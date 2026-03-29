//! OTLP exporter using the official OpenTelemetry SDK.
//!
//! Replaces the previous hand-crafted protobuf encoder with the standard
//! `opentelemetry-otlp` + `opentelemetry-sdk` pipeline.  Reads from
//! [`MetricStore`] cache only — never triggers Modbus calls.

use crate::config::OtlpExporterConfig;
use crate::metrics::{MetricStore, MetricType, MetricValue};
use anyhow::Result;
use opentelemetry::metrics::{Meter, MeterProvider};
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_otlp::WithHttpConfig;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::Resource;
use std::collections::HashMap;
use tracing::{debug, error, info, instrument, warn};

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

/// Record a slice of [`MetricValue`]s into OTel instruments on the given meter.
fn record_metrics(meter: &Meter, metrics: &[MetricValue]) {
    for m in metrics {
        let attrs: Vec<KeyValue> = m
            .labels
            .iter()
            .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
            .collect();

        let name = m.name.clone();
        let desc = m.description.clone();
        let unit = m.unit.clone();

        match m.metric_type {
            MetricType::Gauge => {
                let gauge = meter
                    .f64_gauge(name)
                    .with_description(desc)
                    .with_unit(unit)
                    .build();
                gauge.record(m.value, &attrs);
            }
            MetricType::Counter => {
                let counter = meter
                    .f64_counter(name)
                    .with_description(desc)
                    .with_unit(unit)
                    .build();
                counter.add(m.value, &attrs);
            }
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

        record_metrics(&meter, &metrics);

        if let Some(ref im) = internal_metrics {
            let internal_values = im.to_metric_values();
            record_metrics(&internal_meter, &internal_values);
        }

        debug!(metrics_count = metrics.len(), "Recorded OTLP metrics batch");

        // The PeriodicReader handles the actual export on its own schedule.
        // Force flush to send immediately.
        if let Err(e) = provider.force_flush() {
            if let Some(ref im) = internal_metrics {
                im.otlp_errors_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            error!(error = %e, "OTLP export flush failed");
        }
    }

    // Final flush on shutdown
    let metrics = store.all_metrics_flat();
    if !metrics.is_empty() {
        record_metrics(&meter, &metrics);
    }
    if let Some(ref im) = internal_metrics {
        let internal_values = im.to_metric_values();
        record_metrics(&internal_meter, &internal_values);
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
/// Uses the standard OpenTelemetry SDK pipeline instead of hand-crafted
/// protobuf encoding.
pub struct OtlpMetricExporter {
    provider: SdkMeterProvider,
    meter: Meter,
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
        let provider = build_meter_provider(
            &endpoint,
            &config.headers,
            config.timeout,
            config.interval,
            resource,
        )?;
        let meter = provider.meter("bus-exporter");

        Ok(Self { provider, meter })
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

        record_metrics(&self.meter, &metric_values);

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
