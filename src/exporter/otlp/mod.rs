//! OTLP exporter using the official OpenTelemetry SDK.
//!
//! Replaces the previous hand-crafted protobuf encoder with the standard
//! `opentelemetry-otlp` + `opentelemetry-sdk` pipeline.  Reads from
//! [`MetricStore`] cache only — never triggers Modbus calls.
//!
//! Uses **observable instruments** (`f64_observable_gauge` /
//! `f64_observable_counter`) so the SDK receives absolute values and handles
//! delta computation internally.  Instruments are registered lazily and cached
//! so they are created only once per unique metric name.

use crate::config::OtlpExporterConfig;
use crate::metrics::{MetricStore, MetricType, MetricValue};
use anyhow::Result;
use opentelemetry::metrics::{Meter, MeterProvider};
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_otlp::WithHttpConfig;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::Resource;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tracing::{debug, error, info, instrument, warn};

/// Shared metric values that observable instrument callbacks read from.
type SharedMetricValues = Arc<RwLock<Vec<MetricValue>>>;

// ── Helpers ────────────────────────────────────────────────────────────

/// Build an `SdkMeterProvider` wired to an OTLP HTTP exporter.
fn build_meter_provider(
    endpoint: &str,
    headers: &HashMap<String, String>,
    timeout: std::time::Duration,
    interval: std::time::Duration,
    resource: Resource,
) -> Result<SdkMeterProvider> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .with_headers(headers.clone())
        .with_timeout(timeout)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build OTLP metric exporter: {e}"))?;

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
        .with_interval(interval)
        .build();

    let provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(reader)
        .build();

    Ok(provider)
}

/// Register an observable instrument for a metric.  The callback reads the
/// current value from `shared` state, so the SDK always sees the latest
/// absolute value.  For counters the SDK computes deltas internally.
fn register_observable(meter: &Meter, metric: &MetricValue, shared: &SharedMetricValues) {
    let name = metric.name.clone();
    let desc = metric.description.clone();
    let unit = metric.unit.clone();
    let state = shared.clone();
    let metric_name = name.clone();

    match metric.metric_type {
        MetricType::Gauge => {
            // Observable gauge: callback reports the current absolute value.
            let _gauge = meter
                .f64_observable_gauge(name)
                .with_description(desc)
                .with_unit(unit)
                .with_callback(move |observer| {
                    if let Ok(values) = state.read() {
                        for v in values.iter().filter(|v| v.name == metric_name) {
                            let attrs: Vec<KeyValue> = v
                                .labels
                                .iter()
                                .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
                                .collect();
                            observer.observe(v.value, &attrs);
                        }
                    }
                })
                .build();
        }
        MetricType::Counter => {
            // Observable counter: callback reports the cumulative total.
            // The OTel SDK computes deltas between collections automatically.
            let _counter = meter
                .f64_observable_counter(name)
                .with_description(desc)
                .with_unit(unit)
                .with_callback(move |observer| {
                    if let Ok(values) = state.read() {
                        for v in values.iter().filter(|v| v.name == metric_name) {
                            let attrs: Vec<KeyValue> = v
                                .labels
                                .iter()
                                .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
                                .collect();
                            observer.observe(v.value, &attrs);
                        }
                    }
                })
                .build();
        }
    }
}

/// Discover new metric names in `metrics` and register observable instruments
/// for any that haven't been registered yet.
fn register_new_instruments(
    meter: &Meter,
    metrics: &[MetricValue],
    registered: &mut HashSet<String>,
    shared: &SharedMetricValues,
) {
    for m in metrics {
        if registered.insert(m.name.clone()) {
            register_observable(meter, m, shared);
        }
    }
}

/// Build a [`Resource`] from global labels.
fn build_resource(global_labels: &HashMap<String, String>) -> Resource {
    let attrs: Vec<KeyValue> = global_labels
        .iter()
        .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
        .collect();
    Resource::builder().with_attributes(attrs).build()
}

// ── Public API: periodic push loop ────────────────────────────────────

/// Start the periodic OTLP push loop.  Runs until the token is cancelled.
/// Performs one final flush on shutdown.
///
/// Observable instruments are registered lazily as new metric names appear.
/// The `PeriodicReader` handles export scheduling — no manual `force_flush`
/// is needed.  The loop only updates shared state that the observable
/// callbacks read from.
#[instrument(level = "info", skip_all, fields(endpoint))]
pub async fn run(
    config: OtlpExporterConfig,
    store: MetricStore,
    global_labels: HashMap<String, String>,
    cancel: tokio_util::sync::CancellationToken,
    internal_metrics: Option<std::sync::Arc<crate::internal_metrics::InternalMetrics>>,
) {
    let endpoint = match &config.endpoint {
        Some(ep) => ep.trim_end_matches('/').to_string(),
        None => {
            error!("OTLP exporter enabled but no endpoint configured");
            return;
        }
    };

    let resource = build_resource(&global_labels);
    let interval_dur = config.interval;

    let provider = match build_meter_provider(
        &endpoint,
        &config.headers,
        config.timeout,
        interval_dur,
        resource,
    ) {
        Ok(p) => p,
        Err(e) => {
            error!(error = %e, "Failed to create OTLP meter provider");
            return;
        }
    };

    let meter = provider.meter("bus-exporter");
    let internal_meter = provider.meter("bus-exporter-internal");

    info!(%endpoint, ?interval_dur, "OTLP exporter started (SDK-based)");

    // Shared state for observable callbacks.
    let shared_metrics: SharedMetricValues = Arc::new(RwLock::new(Vec::new()));
    let shared_internal: SharedMetricValues = Arc::new(RwLock::new(Vec::new()));
    let mut registered: HashSet<String> = HashSet::new();
    let mut registered_internal: HashSet<String> = HashSet::new();

    let mut interval = tokio::time::interval(interval_dur);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("OTLP exporter shutting down — performing final flush");
                break;
            }
            _ = interval.tick() => {}
        }

        let metrics = store.all_metrics_flat();
        if metrics.is_empty() && internal_metrics.is_none() {
            debug!("No metrics to export");
            continue;
        }

        // Increment OTLP export counter
        if let Some(ref im) = internal_metrics {
            im.otlp_exports_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        // Register instruments for any newly-discovered metric names.
        register_new_instruments(&meter, &metrics, &mut registered, &shared_metrics);

        // Update shared state — observable callbacks will read these values
        // when the PeriodicReader triggers collection.
        if let Ok(mut guard) = shared_metrics.write() {
            *guard = metrics.clone();
        }

        if let Some(ref im) = internal_metrics {
            let internal_values = im.to_metric_values();
            register_new_instruments(
                &internal_meter,
                &internal_values,
                &mut registered_internal,
                &shared_internal,
            );
            if let Ok(mut guard) = shared_internal.write() {
                *guard = internal_values;
            }
        }

        debug!(metrics_count = metrics.len(), "Updated OTLP metric values");
        // No force_flush — PeriodicReader handles export scheduling.
    }

    // Final flush on shutdown — update shared state one last time.
    let metrics = store.all_metrics_flat();
    register_new_instruments(&meter, &metrics, &mut registered, &shared_metrics);
    if let Ok(mut guard) = shared_metrics.write() {
        *guard = metrics;
    }
    if let Some(ref im) = internal_metrics {
        let internal_values = im.to_metric_values();
        register_new_instruments(
            &internal_meter,
            &internal_values,
            &mut registered_internal,
            &shared_internal,
        );
        if let Ok(mut guard) = shared_internal.write() {
            *guard = internal_values;
        }
    }

    if let Err(e) = provider.shutdown() {
        warn!(error = %e, "OTLP meter provider shutdown error");
    }
    info!("OTLP exporter stopped");
}

// ── MetricExporter trait impl ─────────────────────────────────────────

use crate::config::MetricConfig;
use async_trait::async_trait;

/// OTLP exporter that implements [`super::MetricExporter`].
///
/// Uses observable instruments with shared state.  The `export()` method
/// updates the shared values and triggers a flush.  The `PeriodicReader`
/// interval is set very long to avoid double exports — we rely on manual
/// `force_flush()` triggered by the external scheduler.
pub struct OtlpMetricExporter {
    provider: SdkMeterProvider,
    meter: Meter,
    shared_values: SharedMetricValues,
    registered: HashSet<String>,
}

impl OtlpMetricExporter {
    pub fn new(config: OtlpExporterConfig) -> Result<Self> {
        let endpoint = config
            .endpoint
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OTLP exporter enabled but no endpoint configured"))?
            .trim_end_matches('/')
            .to_string();

        let resource = build_resource(&HashMap::new());
        // Use a very long PeriodicReader interval since we flush manually
        // in export().  This avoids double exports.
        let provider = build_meter_provider(
            &endpoint,
            &config.headers,
            config.timeout,
            std::time::Duration::from_secs(86400),
            resource,
        )?;
        let meter = provider.meter("bus-exporter");

        Ok(Self {
            provider,
            meter,
            shared_values: Arc::new(RwLock::new(Vec::new())),
            registered: HashSet::new(),
        })
    }
}

#[async_trait]
impl super::MetricExporter for OtlpMetricExporter {
    async fn export(
        &mut self,
        metrics: &[MetricConfig],
        results: &HashMap<String, Result<(f64, f64)>>,
    ) -> Result<()> {
        let metric_values = super::results_to_metric_values(metrics, results);

        if metric_values.is_empty() {
            return Ok(());
        }

        // Register instruments for any new metric names.
        register_new_instruments(
            &self.meter,
            &metric_values,
            &mut self.registered,
            &self.shared_values,
        );

        // Update shared state — observable callbacks will read from this.
        if let Ok(mut guard) = self.shared_values.write() {
            *guard = metric_values;
        }

        self.provider
            .force_flush()
            .map_err(|e| anyhow::anyhow!("OTLP flush failed: {e}"))
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.provider
            .shutdown()
            .map_err(|e| anyhow::anyhow!("OTLP shutdown failed: {e}"))
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
