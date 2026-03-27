use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{error, info, instrument, warn};

use crate::config::{self, RegisterType};
use crate::decoder;
use crate::metrics::{MetricStore, MetricType, MetricValue};
use crate::modbus::ModbusClient;

/// Maximum backoff duration for reconnection attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Initial backoff duration for reconnection attempts.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Map config types to decoder/metrics types.
fn map_byte_order(bo: config::ByteOrder) -> decoder::ByteOrder {
    match bo {
        config::ByteOrder::BigEndian => decoder::ByteOrder::BigEndian,
        config::ByteOrder::LittleEndian => decoder::ByteOrder::LittleEndian,
        config::ByteOrder::MidBigEndian => decoder::ByteOrder::MidBigEndian,
        config::ByteOrder::MidLittleEndian => decoder::ByteOrder::MidLittleEndian,
    }
}

fn map_data_type(dt: config::DataType) -> decoder::DataType {
    match dt {
        config::DataType::U16 => decoder::DataType::U16,
        config::DataType::I16 => decoder::DataType::I16,
        config::DataType::U32 => decoder::DataType::U32,
        config::DataType::I32 => decoder::DataType::I32,
        config::DataType::F32 => decoder::DataType::F32,
        config::DataType::U64 => decoder::DataType::U64,
        config::DataType::I64 => decoder::DataType::I64,
        config::DataType::F64 => decoder::DataType::F64,
        config::DataType::Bool => decoder::DataType::Bool,
    }
}

fn map_metric_type(mt: config::MetricType) -> MetricType {
    match mt {
        config::MetricType::Gauge => MetricType::Gauge,
        config::MetricType::Counter => MetricType::Counter,
    }
}

/// Read a single metric from the Modbus client.
#[instrument(skip(client), fields(metric = %metric.name))]
async fn read_metric(client: &mut dyn ModbusClient, metric: &config::Metric) -> Result<f64> {
    let count = metric.data_type.register_count();
    let data_type = map_data_type(metric.data_type);
    let byte_order = map_byte_order(metric.byte_order);

    match metric.register_type {
        RegisterType::Holding => {
            let regs = client
                .read_holding_registers(metric.address, count)
                .await
                .context("reading holding registers")?;
            decoder::decode(&regs, data_type, byte_order, metric.scale, metric.offset)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        RegisterType::Input => {
            let regs = client
                .read_input_registers(metric.address, count)
                .await
                .context("reading input registers")?;
            decoder::decode(&regs, data_type, byte_order, metric.scale, metric.offset)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        RegisterType::Coil => {
            let bits = client
                .read_coils(metric.address, 1)
                .await
                .context("reading coils")?;
            let raw = if *bits.first().unwrap_or(&false) {
                1.0
            } else {
                0.0
            };
            Ok(raw * metric.scale + metric.offset)
        }
        RegisterType::Discrete => {
            let bits = client
                .read_discrete_inputs(metric.address, 1)
                .await
                .context("reading discrete inputs")?;
            let raw = if *bits.first().unwrap_or(&false) {
                1.0
            } else {
                0.0
            };
            Ok(raw * metric.scale + metric.offset)
        }
    }
}

/// Run a single collector loop. This is the core of each collector task.
#[instrument(skip_all, fields(collector = %collector.name))]
async fn run_collector(
    mut client: Box<dyn ModbusClient>,
    collector: config::Collector,
    store: MetricStore,
    global_labels: BTreeMap<String, String>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let collector_labels: BTreeMap<String, String> = collector
        .labels
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut prev_cache: HashMap<String, MetricValue> = HashMap::new();
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
                    _ = shutdown_rx.changed() => { return; }
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }

    loop {
        let start = Instant::now();
        let mut local_cache: HashMap<String, MetricValue> = HashMap::new();
        let mut connection_error = false;

        for metric_cfg in &collector.metrics {
            // Check shutdown between metrics
            if *shutdown_rx.borrow() {
                info!("shutdown requested, exiting");
                return;
            }

            match read_metric(client.as_mut(), metric_cfg).await {
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
                    // Check if this is a connection-level error
                    if !client.is_connected() {
                        error!(error = %e, "connection lost during poll");
                        connection_error = true;
                        break;
                    }
                    warn!(metric = %metric_cfg.name, error = %e, "metric read failed, retaining previous value");
                }
            }
        }

        if connection_error {
            // Reconnect with backoff
            loop {
                let _ = client.disconnect().await;
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown_rx.changed() => { return; }
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

        // Merge: carry forward previous values for failed metrics
        for (name, prev) in &prev_cache {
            local_cache
                .entry(name.clone())
                .or_insert_with(|| prev.clone());
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
                    return;
                }
            }
        } else {
            // Check shutdown even if no sleep
            if *shutdown_rx.borrow() {
                info!("shutdown requested");
                return;
            }
        }
    }
}

/// Factory trait for creating Modbus clients from config.
/// This allows tests to inject mock clients.
pub trait ModbusClientFactory: Send + Sync {
    fn create(&self, collector: &config::Collector) -> Box<dyn ModbusClient>;
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
        factory: &dyn ModbusClientFactory,
    ) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        let mut handles = Vec::with_capacity(collectors.len());

        for collector_cfg in collectors {
            let client = factory.create(&collector_cfg);
            let store = store.clone();
            let global_labels = global_labels.clone();
            let shutdown_rx = shutdown_tx.subscribe();

            let handle = tokio::spawn(run_collector(
                client,
                collector_cfg,
                store,
                global_labels,
                shutdown_rx,
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
