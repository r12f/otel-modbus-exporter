use anyhow::Result;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};

use crate::config;
use crate::internal_metrics::InternalMetrics;
use crate::metrics::{MetricStore, MetricType, MetricValue};
use crate::reader::{MetricReader, ReadResults};

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

/// Factory trait for creating metric readers from config.
/// This allows tests to inject mock clients.
pub trait MetricReaderFactory: Send + Sync {
    fn create(&self, collector: &config::CollectorConfig) -> Result<Box<dyn MetricReader>>;
}

/// Run a single collector loop. This is the core of each collector task.
#[instrument(level = "info", skip_all, fields(collector = %collector.name))]
async fn run_collector(
    mut client: Box<dyn MetricReader>,
    collector: config::CollectorConfig,
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

    // CancellationToken for cooperative shutdown inside read()
    let cancel = CancellationToken::new();

    // Configure which metrics to read
    client.set_metrics(collector.metrics.clone());

    // Initial connect
    loop {
        match client.connect().await {
            Ok(()) => {
                info!("connected");
                backoff = INITIAL_BACKOFF;
                break;
            }
            Err(e) => {
                warn!(error = %e, backoff_secs = backoff.as_secs(), "connection failed, retrying");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown_rx.changed() => {
                        cancel.cancel();
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

        // Check shutdown before read
        if *shutdown_rx.borrow() {
            info!("shutdown requested, exiting");
            cancel.cancel();
            let _ = client.disconnect().await;
            return;
        }

        let ReadResults {
            metrics: read_results,
            io_count,
        } = client.read(&cancel).await;

        // Increment modbus_requests by actual I/O count
        if let Some(ref im) = internal_metrics {
            let stats = im.get_or_create_collector(&collector.name);
            stats.modbus_requests.fetch_add(io_count as u64, Relaxed);
        }

        for (metric_name, result) in read_results {
            // Find the metric config for this name
            let metric_cfg = match collector.metrics.iter().find(|m| m.name == metric_name) {
                Some(cfg) => cfg,
                None => continue,
            };

            match result {
                Ok(value) => {
                    local_cache.insert(
                        metric_name,
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

                    if !client.is_connected() {
                        warn!(error = %e, "connection lost during poll");
                        connection_error = true;
                        poll_had_error = true;
                        break;
                    }
                    poll_had_error = true;
                    let count = error_counts.entry(metric_name).or_insert(0);
                    *count += 1;
                    warn!(metric = %metric_cfg.name, error = %e, error_count = *count, "metric read failed, retaining previous value");
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
                        cancel.cancel();
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
                        warn!(error = %e, backoff_secs = backoff.as_secs(), "reconnect failed");
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
                    cancel.cancel();
                    let _ = client.disconnect().await;
                    return;
                }
            }
        } else {
            // Check shutdown even if no sleep
            if *shutdown_rx.borrow() {
                info!("shutdown requested");
                cancel.cancel();
                let _ = client.disconnect().await;
                return;
            }
        }
    }
}

/// Handle for managing all collector tasks.
pub struct CollectorEngine {
    shutdown_tx: watch::Sender<bool>,
    handles: Vec<JoinHandle<()>>,
}

impl CollectorEngine {
    /// Spawn one async task per collector. Returns a handle for shutdown.
    pub fn spawn(
        collectors: Vec<config::CollectorConfig>,
        store: MetricStore,
        global_labels: BTreeMap<String, String>,
        factory: &dyn MetricReaderFactory,
        internal_metrics: Option<Arc<InternalMetrics>>,
    ) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        let mut handles = Vec::with_capacity(collectors.len());

        for collector_cfg in collectors {
            let client = match factory.create(&collector_cfg) {
                Ok(c) => c,
                Err(e) => {
                    error!(collector = %collector_cfg.name, error = %e, "failed to create metric reader, skipping collector");
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
