//! OTLP protobuf over HTTP push exporter.
//!
//! Reads from [`MetricStore`] cache only — never triggers Modbus calls.

use crate::config::OtlpExporterConfig;
use crate::metrics::{MetricStore, MetricType, MetricValue};
use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info, instrument, warn};

/// Maximum number of retry attempts for retryable errors (429, 5xx, network).
const MAX_RETRIES: u32 = 3;

// ── OTLP protobuf wire types ──────────────────────────────────────────
// We hand-roll minimal protobuf encoding to avoid pulling in the full
// opentelemetry-proto crate which has heavy gRPC deps.  The wire format
// follows opentelemetry/proto/collector/metrics/v1/metrics_service.proto
// and opentelemetry/proto/metrics/v1/metrics.proto.

/// Encode a protobuf varint.
fn encode_varint(mut v: u64, buf: &mut Vec<u8>) {
    while v >= 0x80 {
        buf.push((v as u8) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

/// Encode a length-delimited field (field_number, wire_type=2).
fn encode_ld(field: u32, data: &[u8], buf: &mut Vec<u8>) {
    encode_varint(((field as u64) << 3) | 2, buf);
    encode_varint(data.len() as u64, buf);
    buf.extend_from_slice(data);
}

/// Encode a fixed64 field.
fn encode_fixed64(field: u32, v: u64, buf: &mut Vec<u8>) {
    encode_varint(((field as u64) << 3) | 1, buf);
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Encode a double field (wire type 1 = fixed64).
fn encode_double(field: u32, v: f64, buf: &mut Vec<u8>) {
    encode_fixed64(field, v.to_bits(), buf);
}

/// Encode a varint field.
fn encode_varint_field(field: u32, v: u64, buf: &mut Vec<u8>) {
    encode_varint((field as u64) << 3, buf);
    encode_varint(v, buf);
}

/// Encode a KeyValue (common.proto).
fn encode_key_value(key: &str, value: &str, buf: &mut Vec<u8>) {
    let mut kv = Vec::new();
    encode_ld(1, key.as_bytes(), &mut kv); // key
                                           // AnyValue with string_value (field 1)
    let mut any = Vec::new();
    encode_ld(1, value.as_bytes(), &mut any);
    encode_ld(2, &any, &mut kv); // value
    encode_ld(1, &kv, buf); // KeyValue as repeated in parent
}

fn system_time_to_nanos(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64
}

/// Build `ExportMetricsServiceRequest` protobuf bytes from a flat metric list.
///
/// `process_start` is recorded at process startup and used as
/// `start_time_unix_nano` for cumulative Sum data points.
pub fn build_request(
    metrics: &[MetricValue],
    global_labels: &HashMap<String, String>,
    process_start: SystemTime,
) -> Vec<u8> {
    // Resource (resource.proto)
    let mut resource = Vec::new();
    let mut sorted_labels: Vec<_> = global_labels.iter().collect();
    sorted_labels.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in &sorted_labels {
        encode_key_value(k, v, &mut resource); // field 1 repeated attributes
    }

    // Scope
    let mut scope = Vec::new();
    encode_ld(1, b"bus-exporter", &mut scope); // name

    // Build Metric entries
    let mut otlp_metrics_buf = Vec::new();
    for m in metrics {
        let mut metric = Vec::new();
        encode_ld(1, m.name.as_bytes(), &mut metric); // name
        encode_ld(2, m.description.as_bytes(), &mut metric); // description
        encode_ld(3, m.unit.as_bytes(), &mut metric); // unit

        // NumberDataPoint
        let mut dp = Vec::new();
        // attributes (field 7)
        let mut sorted_m_labels: Vec<_> = m.labels.iter().collect();
        sorted_m_labels.sort_by_key(|(k, _)| k.as_str());
        for (k, v) in &sorted_m_labels {
            let mut kv = Vec::new();
            encode_ld(1, k.as_bytes(), &mut kv);
            let mut any = Vec::new();
            encode_ld(1, v.as_bytes(), &mut any);
            encode_ld(2, &any, &mut kv);
            encode_ld(7, &kv, &mut dp);
        }
        // start_time_unix_nano (field 2) — required for cumulative Sum
        if m.metric_type == MetricType::Counter {
            encode_fixed64(2, system_time_to_nanos(process_start), &mut dp);
        }
        encode_fixed64(3, system_time_to_nanos(m.updated_at), &mut dp); // time_unix_nano
        encode_double(4, m.value, &mut dp); // as_double (field 4)

        match m.metric_type {
            MetricType::Gauge => {
                // Gauge message (field 5 of Metric)
                let mut gauge = Vec::new();
                encode_ld(1, &dp, &mut gauge); // data_points
                encode_ld(5, &gauge, &mut metric);
            }
            MetricType::Counter => {
                // Sum message (field 7 of Metric)
                let mut sum = Vec::new();
                encode_ld(1, &dp, &mut sum); // data_points
                encode_varint_field(2, 2, &mut sum); // AGGREGATION_TEMPORALITY_CUMULATIVE = 2
                encode_varint_field(3, 1, &mut sum); // is_monotonic = true
                encode_ld(7, &sum, &mut metric);
            }
        }
        otlp_metrics_buf.push(metric);
    }

    // ScopeMetrics
    let mut scope_metrics = Vec::new();
    encode_ld(1, &scope, &mut scope_metrics); // scope
    for m_bytes in &otlp_metrics_buf {
        encode_ld(2, m_bytes, &mut scope_metrics); // metrics
    }

    // ResourceMetrics
    let mut resource_metrics = Vec::new();
    encode_ld(1, &resource, &mut resource_metrics); // resource
    encode_ld(2, &scope_metrics, &mut resource_metrics); // scope_metrics

    // ExportMetricsServiceRequest
    let mut request = Vec::new();
    encode_ld(1, &resource_metrics, &mut request); // resource_metrics
    request
}

/// Build `ExportMetricsServiceRequest` with a separate internal scope.
pub fn build_request_with_internal(
    device_metrics: &[MetricValue],
    internal_metrics: &[MetricValue],
    global_labels: &HashMap<String, String>,
    process_start: SystemTime,
) -> Vec<u8> {
    // Resource
    let mut resource = Vec::new();
    let mut sorted_labels: Vec<_> = global_labels.iter().collect();
    sorted_labels.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in &sorted_labels {
        encode_key_value(k, v, &mut resource);
    }

    // Device scope
    let mut device_scope = Vec::new();
    encode_ld(1, b"bus-exporter", &mut device_scope);

    let mut device_scope_metrics = Vec::new();
    encode_ld(1, &device_scope, &mut device_scope_metrics);
    for m in device_metrics {
        let metric_bytes = encode_single_metric(m, process_start);
        encode_ld(2, &metric_bytes, &mut device_scope_metrics);
    }

    // Internal scope
    let mut internal_scope = Vec::new();
    encode_ld(1, b"bus-exporter-internal", &mut internal_scope);

    let mut internal_scope_metrics = Vec::new();
    encode_ld(1, &internal_scope, &mut internal_scope_metrics);
    for m in internal_metrics {
        let metric_bytes = encode_single_metric(m, process_start);
        encode_ld(2, &metric_bytes, &mut internal_scope_metrics);
    }

    // ResourceMetrics
    let mut resource_metrics = Vec::new();
    encode_ld(1, &resource, &mut resource_metrics);
    encode_ld(2, &device_scope_metrics, &mut resource_metrics);
    encode_ld(2, &internal_scope_metrics, &mut resource_metrics);

    // ExportMetricsServiceRequest
    let mut request = Vec::new();
    encode_ld(1, &resource_metrics, &mut request);
    request
}

/// Encode a single Metric message.
fn encode_single_metric(m: &MetricValue, process_start: SystemTime) -> Vec<u8> {
    let mut metric = Vec::new();
    encode_ld(1, m.name.as_bytes(), &mut metric);
    encode_ld(2, m.description.as_bytes(), &mut metric);
    encode_ld(3, m.unit.as_bytes(), &mut metric);

    let mut dp = Vec::new();
    let mut sorted_labels: Vec<_> = m.labels.iter().collect();
    sorted_labels.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in &sorted_labels {
        let mut kv = Vec::new();
        encode_ld(1, k.as_bytes(), &mut kv);
        let mut any = Vec::new();
        encode_ld(1, v.as_bytes(), &mut any);
        encode_ld(2, &any, &mut kv);
        encode_ld(7, &kv, &mut dp);
    }
    if m.metric_type == MetricType::Counter {
        encode_fixed64(2, system_time_to_nanos(process_start), &mut dp);
    }
    encode_fixed64(3, system_time_to_nanos(m.updated_at), &mut dp);
    encode_double(4, m.value, &mut dp);

    match m.metric_type {
        MetricType::Gauge => {
            let mut gauge = Vec::new();
            encode_ld(1, &dp, &mut gauge);
            encode_ld(5, &gauge, &mut metric);
        }
        MetricType::Counter => {
            let mut sum = Vec::new();
            encode_ld(1, &dp, &mut sum);
            encode_varint_field(2, 2, &mut sum);
            encode_varint_field(3, 1, &mut sum);
            encode_ld(7, &sum, &mut metric);
        }
    }
    metric
}

// ── HTTP push + retry ─────────────────────────────────────────────────

/// Retry / backoff state with jitter.
struct Backoff {
    current: Duration,
    max: Duration,
}

impl Backoff {
    fn new() -> Self {
        Self {
            current: Duration::from_secs(1),
            max: Duration::from_secs(30),
        }
    }

    /// Return the next delay with ±25% random jitter applied.
    fn next_delay(&mut self) -> Duration {
        let base = self.current;
        self.current = (self.current * 2).min(self.max);
        // Apply ±25% jitter
        let base_ms = base.as_millis() as u64;
        let jitter_range = base_ms / 4; // 25%
        if jitter_range == 0 {
            return base;
        }
        // Simple deterministic-seed-free jitter using current time nanos
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as u64;
        let offset = nanos % (jitter_range * 2 + 1);
        let jittered = base_ms - jitter_range + offset;
        Duration::from_millis(jittered)
    }

    fn reset(&mut self) {
        self.current = Duration::from_secs(1);
    }
}

/// Send a single export request with retry.
///
/// The `cancel` token makes backoff sleeps cancellation-aware so shutdown
/// is not blocked for up to 30 s waiting on a retry delay.
#[instrument(level = "debug", skip_all)]
async fn send_with_retry(
    client: &reqwest::Client,
    url: &str,
    headers: &HashMap<String, String>,
    body: Vec<u8>,
    timeout: Duration,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<()> {
    // Convert to Bytes upfront so retries are O(1) clones instead of
    // copying the entire Vec on every attempt.
    let body: bytes::Bytes = body.into();

    let mut backoff = Backoff::new();
    let mut attempts: u32 = 0;

    loop {
        attempts += 1;
        let mut req = client
            .post(url)
            .header("Content-Type", "application/x-protobuf")
            .timeout(timeout)
            .body(body.clone());

        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if (200..300).contains(&status) {
                    debug!(status, "OTLP export succeeded");
                    return Ok(());
                }

                // Extract Retry-After header before consuming body.
                let retry_after_header = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok());

                // Read response body for diagnostics before deciding on retry.
                let resp_body = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "<failed to read body>".to_string());

                if status == 429 {
                    if attempts >= MAX_RETRIES {
                        anyhow::bail!(
                            "OTLP export failed: HTTP 429 after {attempts} attempts: {resp_body}"
                        );
                    }
                    // Respect Retry-After header if present; fall back to exponential backoff.
                    let retry_after = retry_after_header
                        .map(Duration::from_secs)
                        .unwrap_or_else(|| backoff.next_delay());
                    warn!(status, ?retry_after, %resp_body, "OTLP 429 — backing off");
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            anyhow::bail!("OTLP export cancelled during retry backoff");
                        }
                        _ = tokio::time::sleep(retry_after) => {}
                    }
                    continue;
                }
                if status >= 500 {
                    if attempts >= MAX_RETRIES {
                        anyhow::bail!(
                            "OTLP export failed: HTTP {status} after {attempts} attempts: {resp_body}"
                        );
                    }
                    let delay = backoff.next_delay();
                    warn!(status, ?delay, %resp_body, "OTLP 5xx — retrying");
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            anyhow::bail!("OTLP export cancelled during retry backoff");
                        }
                        _ = tokio::time::sleep(delay) => {}
                    }
                    continue;
                }
                // 4xx (not 429) — do not retry
                error!(
                    status,
                    %resp_body,
                    "OTLP export failed with client error — not retrying"
                );
                anyhow::bail!("OTLP export failed: HTTP {status}: {resp_body}");
            }
            Err(e) => {
                if attempts >= MAX_RETRIES {
                    anyhow::bail!("OTLP export failed after {attempts} attempts: {e}");
                }
                let delay = backoff.next_delay();
                warn!(?delay, error = %e, "OTLP export request error — retrying");
                tokio::select! {
                    _ = cancel.cancelled() => {
                        anyhow::bail!("OTLP export cancelled during retry backoff");
                    }
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}

// ── Public API ─────────────────────────────────────────────────────────

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
    let url = format!("{endpoint}/v1/metrics");
    let client = reqwest::Client::new();
    let interval_dur = config.interval;
    let process_start = SystemTime::now();

    info!(%url, ?interval_dur, "OTLP exporter started");

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

        export_once(
            &client,
            &url,
            &config,
            &store,
            &global_labels,
            process_start,
            &cancel,
            internal_metrics.as_deref(),
        )
        .await;
    }

    // Final flush on shutdown — pass a non-cancelled token so the final
    // flush can complete its retries without being immediately cancelled.
    let flush_token = tokio_util::sync::CancellationToken::new();
    export_once(
        &client,
        &url,
        &config,
        &store,
        &global_labels,
        process_start,
        &flush_token,
        internal_metrics.as_deref(),
    )
    .await;
    info!("OTLP exporter stopped");
}

/// Export current metrics once.
#[allow(clippy::too_many_arguments)]
async fn export_once(
    client: &reqwest::Client,
    url: &str,
    config: &OtlpExporterConfig,
    store: &MetricStore,
    global_labels: &HashMap<String, String>,
    process_start: SystemTime,
    cancel: &tokio_util::sync::CancellationToken,
    internal_metrics: Option<&crate::internal_metrics::InternalMetrics>,
) {
    let metrics = store.all_metrics_flat();
    if metrics.is_empty() && internal_metrics.is_none() {
        debug!("No metrics to export");
        return;
    }

    // Increment OTLP export counter
    if let Some(im) = internal_metrics {
        im.otlp_exports_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    // Build request with device metrics + internal scope
    let body = if let Some(im) = internal_metrics {
        let internal_values = im.to_metric_values();
        build_request_with_internal(&metrics, &internal_values, global_labels, process_start)
    } else {
        build_request(&metrics, global_labels, process_start)
    };

    debug!(
        metrics_count = metrics.len(),
        bytes = body.len(),
        "Exporting OTLP batch"
    );

    if let Err(e) =
        send_with_retry(client, url, &config.headers, body, config.timeout, cancel).await
    {
        // Increment OTLP error counter
        if let Some(im) = internal_metrics {
            im.otlp_errors_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        error!(error = %e, "OTLP export failed");
    }
}

// ── MetricExporter trait impl ─────────────────────────────────────────

use crate::config::MetricConfig;
use async_trait::async_trait;

/// OTLP exporter that implements [`super::MetricExporter`].
///
/// Wraps the existing push logic.  Each call to [`export()`] performs a
/// single OTLP push with the supplied metrics/results.
pub struct OtlpMetricExporter {
    config: OtlpExporterConfig,
    client: reqwest::Client,
    url: String,
    process_start: SystemTime,
    cancel: tokio_util::sync::CancellationToken,
}

impl OtlpMetricExporter {
    pub fn new(config: OtlpExporterConfig) -> Result<Self> {
        let endpoint = config
            .endpoint
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OTLP exporter enabled but no endpoint configured"))?
            .trim_end_matches('/')
            .to_string();
        let url = format!("{endpoint}/v1/metrics");
        Ok(Self {
            config,
            client: reqwest::Client::new(),
            url,
            process_start: SystemTime::now(),
            cancel: tokio_util::sync::CancellationToken::new(),
        })
    }
}

#[async_trait]
impl super::MetricExporter for OtlpMetricExporter {
    async fn export(
        &mut self,
        metrics: &[MetricConfig],
        results: &HashMap<String, Result<f64>>,
    ) -> Result<()> {
        let metric_values = super::results_to_metric_values(metrics, results);

        if metric_values.is_empty() {
            return Ok(());
        }

        let body = build_request(&metric_values, &HashMap::new(), self.process_start);
        send_with_retry(
            &self.client,
            &self.url,
            &self.config.headers,
            body,
            self.config.timeout,
            &self.cancel,
        )
        .await
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.cancel.cancel();
        Ok(())
    }
}

#[cfg(test)]
#[path = "otlp_tests.rs"]
mod tests;
