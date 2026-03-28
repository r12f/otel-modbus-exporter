use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{error, info, instrument, warn};

use crate::bus;
use crate::config::{self, RegisterType};
use crate::decoder;
use crate::internal_metrics::InternalMetrics;
use crate::metrics::{MetricStore, MetricType, MetricValue};
use crate::reader::i2c::{self, I2cClient};
use crate::reader::i3c;
use crate::reader::modbus::batch::batch_read_coalesced;
use crate::reader::modbus::{BusConnection, ModbusClient};
use crate::reader::spi::{self, SpiClient};

/// Maximum backoff duration for reconnection attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Initial backoff duration for reconnection attempts.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Default shutdown timeout.
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Map config types to decoder/metrics types.
fn map_metric_type(mt: config::MetricType) -> MetricType {
    match mt {
        config::MetricType::Gauge => MetricType::Gauge,
        config::MetricType::Counter => MetricType::Counter,
    }
}

/// Abstraction over Modbus and I2C clients.
pub enum BusClient {
    Modbus(Box<dyn ModbusClient>),
    I2c {
        client: I2cClient,
        bus_lock: i2c::BusLock,
    },
    Spi {
        client: SpiClient,
        device_lock: spi::DeviceLock,
    },
    I3c {
        client: Arc<tokio::sync::Mutex<i3c::I3cClient>>,
        bus_lock: i3c::BusLock,
    },
}

impl BusClient {
    async fn connect(&mut self) -> Result<()> {
        match self {
            BusClient::Modbus(c) => c.connect().await,
            BusClient::I2c { client, .. } => client.connect().await,
            BusClient::Spi { client, .. } => client.connect().await,
            BusClient::I3c { client, .. } => {
                let mut c = client.lock().await;
                c.connect().await
            }
        }
    }

    async fn disconnect(&mut self) -> Result<()> {
        match self {
            BusClient::Modbus(c) => c.disconnect().await,
            BusClient::I2c { client, .. } => client.disconnect().await,
            BusClient::Spi { client, .. } => client.disconnect().await,
            BusClient::I3c { client, .. } => {
                let mut c = client.lock().await;
                c.disconnect().await
            }
        }
    }

    fn is_connected(&self) -> bool {
        match self {
            BusClient::Modbus(c) => c.is_connected(),
            BusClient::I2c { client, .. } => client.is_connected(),
            BusClient::Spi { client, .. } => client.is_connected(),
            BusClient::I3c { client, .. } => {
                // Best-effort: try_lock to avoid blocking
                client.try_lock().map(|c| c.is_connected()).unwrap_or(true)
            }
        }
    }
}

/// Read a single metric from the Modbus client.
#[instrument(skip(client), fields(metric = %metric.name))]
async fn read_metric(client: &mut dyn ModbusClient, metric: &config::Metric) -> Result<f64> {
    let count = metric.data_type.register_count();
    let data_type = bus::map_data_type(metric.data_type);
    let byte_order = bus::map_byte_order(metric.byte_order);
    let register_type = metric.register_type.unwrap_or(RegisterType::Holding);

    match register_type {
        RegisterType::Holding => {
            let regs = client
                .read_holding_registers(metric.address.unwrap(), count)
                .await
                .context("reading holding registers")?;
            decoder::decode(&regs, data_type, byte_order, metric.scale, metric.offset)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        RegisterType::Input => {
            let regs = client
                .read_input_registers(metric.address.unwrap(), count)
                .await
                .context("reading input registers")?;
            decoder::decode(&regs, data_type, byte_order, metric.scale, metric.offset)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        RegisterType::Coil => {
            let bits = client
                .read_coils(metric.address.unwrap(), 1)
                .await
                .context("reading coils")?;
            let val = bits
                .first()
                .ok_or_else(|| anyhow::anyhow!("empty coil response"))?;
            let raw = if *val { 1.0 } else { 0.0 };
            Ok(raw * metric.scale + metric.offset)
        }
        RegisterType::Discrete => {
            let bits = client
                .read_discrete_inputs(metric.address.unwrap(), 1)
                .await
                .context("reading discrete inputs")?;
            let val = bits
                .first()
                .ok_or_else(|| anyhow::anyhow!("empty discrete input response"))?;
            let raw = if *val { 1.0 } else { 0.0 };
            Ok(raw * metric.scale + metric.offset)
        }
    }
}

/// Read a single metric from any bus client.
async fn read_bus_metric(client: &mut BusClient, metric: &config::Metric) -> Result<f64> {
    match client {
        BusClient::Modbus(c) => read_metric(c.as_mut(), metric).await,
        BusClient::I2c { client, bus_lock } => i2c::read_i2c_metric(client, metric, bus_lock).await,
        BusClient::Spi {
            client,
            device_lock,
        } => spi::read_spi_metric(client, metric, device_lock).await,
        BusClient::I3c { client, bus_lock } => i3c::read_i3c_metric(client, metric, bus_lock).await,
    }
}

/// Run a single collector loop. This is the core of each collector task.
#[instrument(skip_all, fields(collector = %collector.name))]
async fn run_collector(
    mut client: BusClient,
    collector: config::Collector,
    store: MetricStore,
    global_labels: BTreeMap<String, String>,
    mut shutdown_rx: watch::Receiver<bool>,
    internal_metrics: Option<Arc<InternalMetrics>>,
) {
    let collector_labels: BTreeMap<String, String> = collector
        .labels
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut prev_cache: HashMap<String, MetricValue> = HashMap::new();
    let mut error_counts: HashMap<String, u64> = HashMap::new();
    let mut backoff = INITIAL_BACKOFF;
    let poll_interval = collector.polling_interval;
    let warn_threshold = poll_interval.mul_f64(0.8);

    // Initial connect
    loop {
        match client.connect().await {
            Ok(()) => {
                info!("connected");
                backoff = INITIAL_BACKOFF;
                break;
            }
            Err(e) => {
                error!(error = %e, backoff_secs = backoff.as_secs(), "connection failed, retrying");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown_rx.changed() => {
                        let _ = client.disconnect().await;
                        return;
                    }
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }

    loop {
        let start = Instant::now();
        let mut local_cache: HashMap<String, MetricValue> = HashMap::new();
        let mut connection_error = false;
        let mut poll_had_error = false;

        // Increment polls_total
        if let Some(ref im) = internal_metrics {
            let stats = im.get_or_create_collector(&collector.name);
            stats.polls_total.fetch_add(1, Relaxed);
        }

        // Use batch_read when configured and the client is Modbus
        let use_batch = collector.batch_read && matches!(client, BusClient::Modbus(_));

        if use_batch {
            // Batch path: coalesce registers and read in bulk
            if let BusClient::Modbus(ref mut modbus_client) = client {
                if let Some(ref im) = internal_metrics {
                    let stats = im.get_or_create_collector(&collector.name);
                    stats.modbus_requests.fetch_add(1, Relaxed);
                }

                let batch_results =
                    batch_read_coalesced(modbus_client.as_mut(), &collector.metrics).await;

                for (metric_cfg, result) in batch_results {
                    match result {
                        Ok(value) => {
                            local_cache.insert(
                                metric_cfg.name.clone(),
                                MetricValue {
                                    name: metric_cfg.name.clone(),
                                    value,
                                    metric_type: map_metric_type(metric_cfg.metric_type),
                                    labels: BTreeMap::new(),
                                    description: metric_cfg.description.clone(),
                                    unit: metric_cfg.unit.clone(),
                                    updated_at: SystemTime::now(),
                                },
                            );
                        }
                        Err(e) => {
                            if let Some(ref im) = internal_metrics {
                                let stats = im.get_or_create_collector(&collector.name);
                                stats.modbus_errors.fetch_add(1, Relaxed);
                            }

                            if !modbus_client.is_connected() {
                                error!(error = %e, "connection lost during batch poll");
                                connection_error = true;
                                poll_had_error = true;
                                break;
                            }
                            poll_had_error = true;
                            let count = error_counts.entry(metric_cfg.name.clone()).or_insert(0);
                            *count += 1;
                            warn!(metric = %metric_cfg.name, error = %e, error_count = *count, "metric read failed, retaining previous value");
                        }
                    }
                }
            }
        } else {
            // Individual read path (original logic)
            for metric_cfg in &collector.metrics {
                // Check shutdown between metrics
                if *shutdown_rx.borrow() {
                    info!("shutdown requested, exiting");
                    let _ = client.disconnect().await;
                    return;
                }

                // Increment modbus_requests
                if let Some(ref im) = internal_metrics {
                    let stats = im.get_or_create_collector(&collector.name);
                    stats.modbus_requests.fetch_add(1, Relaxed);
                }

                match read_bus_metric(&mut client, metric_cfg).await {
                    Ok(value) => {
                        local_cache.insert(
                            metric_cfg.name.clone(),
                            MetricValue {
                                name: metric_cfg.name.clone(),
                                value,
                                metric_type: map_metric_type(metric_cfg.metric_type),
                                labels: BTreeMap::new(),
                                description: metric_cfg.description.clone(),
                                unit: metric_cfg.unit.clone(),
                                updated_at: SystemTime::now(),
                            },
                        );
                    }
                    Err(e) => {
                        // Increment modbus_errors
                        if let Some(ref im) = internal_metrics {
                            let stats = im.get_or_create_collector(&collector.name);
                            stats.modbus_errors.fetch_add(1, Relaxed);
                        }

                        // Check if this is a connection-level error
                        if !client.is_connected() {
                            error!(error = %e, "connection lost during poll");
                            connection_error = true;
                            poll_had_error = true;
                            break;
                        }
                        poll_had_error = true;
                        let count = error_counts.entry(metric_cfg.name.clone()).or_insert(0);
                        *count += 1;
                        warn!(metric = %metric_cfg.name, error = %e, error_count = *count, "metric read failed, retaining previous value");
                    }
                }
            }
        }

        if connection_error {
            // Record error metrics before reconnecting
            if let Some(ref im) = internal_metrics {
                let elapsed_secs = start.elapsed().as_secs_f64();
                let stats = im.get_or_create_collector(&collector.name);
                stats.set_poll_duration(elapsed_secs);
                stats.polls_error.fetch_add(1, Relaxed);
            }

            // Reconnect with backoff
            loop {
                let _ = client.disconnect().await;
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown_rx.changed() => {
                        let _ = client.disconnect().await;
                        return;
                    }
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
                match client.connect().await {
                    Ok(()) => {
                        info!("reconnected");
                        backoff = INITIAL_BACKOFF;
                        break;
                    }
                    Err(e) => {
                        error!(error = %e, backoff_secs = backoff.as_secs(), "reconnect failed");
                    }
                }
            }
            continue;
        }

        // Successful poll cycle — reset backoff
        backoff = INITIAL_BACKOFF;

        // Record poll duration and success/error
        if let Some(ref im) = internal_metrics {
            let elapsed_secs = start.elapsed().as_secs_f64();
            let stats = im.get_or_create_collector(&collector.name);
            stats.set_poll_duration(elapsed_secs);
            if poll_had_error {
                stats.polls_error.fetch_add(1, Relaxed);
            } else {
                stats.polls_success.fetch_add(1, Relaxed);
            }
        }

        // Merge: carry forward previous values for failed metrics, updating timestamp
        let now = SystemTime::now();
        for (name, prev) in &prev_cache {
            local_cache.entry(name.clone()).or_insert_with(|| {
                let mut carried = prev.clone();
                carried.updated_at = now;
                carried
            });
        }

        // Publish per-metric error counts to store
        for (metric_name, &count) in &error_counts {
            local_cache
                .entry(format!("{}_errors", metric_name))
                .or_insert_with(|| MetricValue {
                    name: format!("{}_errors", metric_name),
                    value: count as f64,
                    metric_type: MetricType::Counter,
                    labels: BTreeMap::new(),
                    description: format!("Error count for metric {}", metric_name),
                    unit: String::new(),
                    updated_at: now,
                });
            // Update existing error counter value
            if let Some(m) = local_cache.get_mut(&format!("{}_errors", metric_name)) {
                m.value = count as f64;
                m.updated_at = now;
            }
        }

        // Publish to store
        let metrics_vec: Vec<MetricValue> = local_cache.values().cloned().collect();
        store.publish(
            &collector.name,
            metrics_vec,
            &global_labels,
            &collector_labels,
        );
        prev_cache = local_cache;

        let elapsed = start.elapsed();
        if elapsed > warn_threshold {
            warn!(
                elapsed_ms = elapsed.as_millis() as u64,
                interval_ms = poll_interval.as_millis() as u64,
                "poll cycle exceeded 80% of interval"
            );
        }

        if elapsed < poll_interval {
            let remaining = poll_interval - elapsed;
            tokio::select! {
                _ = tokio::time::sleep(remaining) => {}
                _ = shutdown_rx.changed() => {
                    info!("shutdown requested");
                    let _ = client.disconnect().await;
                    return;
                }
            }
        } else {
            // Check shutdown even if no sleep
            if *shutdown_rx.borrow() {
                info!("shutdown requested");
                let _ = client.disconnect().await;
                return;
            }
        }
    }
}

/// Factory trait for creating bus clients from config.
/// This allows tests to inject mock clients.
pub trait BusClientFactory: Send + Sync {
    fn create(&self, collector: &config::Collector) -> Result<BusClient>;
}

/// Handle for managing all collector tasks.
pub struct CollectorEngine {
    shutdown_tx: watch::Sender<bool>,
    handles: Vec<JoinHandle<()>>,
}

impl CollectorEngine {
    /// Spawn one async task per collector. Returns a handle for shutdown.
    pub fn spawn(
        collectors: Vec<config::Collector>,
        store: MetricStore,
        global_labels: BTreeMap<String, String>,
        factory: &dyn BusClientFactory,
        internal_metrics: Option<Arc<InternalMetrics>>,
    ) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        let mut handles = Vec::with_capacity(collectors.len());

        for collector_cfg in collectors {
            let client = match factory.create(&collector_cfg) {
                Ok(c) => c,
                Err(e) => {
                    error!(collector = %collector_cfg.name, error = %e, "failed to create bus client, skipping collector");
                    continue;
                }
            };
            let store = store.clone();
            let global_labels = global_labels.clone();
            let shutdown_rx = shutdown_tx.subscribe();
            let im = internal_metrics.clone();

            let handle = tokio::spawn(run_collector(
                client,
                collector_cfg,
                store,
                global_labels,
                shutdown_rx,
                im,
            ));
            handles.push(handle);
        }

        Self {
            shutdown_tx,
            handles,
        }
    }

    /// Signal all collectors to shut down and wait for them (up to timeout).
    pub async fn shutdown(self, timeout: Duration) {
        let _ = self.shutdown_tx.send(true);
        let all = futures::future::join_all(self.handles);
        match tokio::time::timeout(timeout, all).await {
            Ok(_) => info!("all collectors shut down gracefully"),
            Err(_) => warn!("collector shutdown timed out"),
        }
    }
}

#[cfg(test)]
#[path = "collector_tests.rs"]
mod tests;
