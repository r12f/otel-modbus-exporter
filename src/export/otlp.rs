//! OTLP protobuf over HTTP push exporter.
//!
//! Reads from [`MetricStore`] cache only — never triggers Modbus calls.

use crate::config::OtlpExporter;
use crate::metrics::{MetricStore, MetricType, MetricValue};
use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info, instrument, warn};

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
pub fn build_request(metrics: &[MetricValue], global_labels: &HashMap<String, String>) -> Vec<u8> {
    // Resource (resource.proto)
    let mut resource = Vec::new();
    let mut sorted_labels: Vec<_> = global_labels.iter().collect();
    sorted_labels.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in &sorted_labels {
        encode_key_value(k, v, &mut resource); // field 1 repeated attributes
    }

    // Scope
    let mut scope = Vec::new();
    encode_ld(1, b"otel-modbus-exporter", &mut scope); // name

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

// ── HTTP push + retry ─────────────────────────────────────────────────

/// Retry / backoff state.
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

    fn next_delay(&mut self) -> Duration {
        let d = self.current;
        self.current = (self.current * 2).min(self.max);
        d
    }

    fn reset(&mut self) {
        self.current = Duration::from_secs(1);
    }
}

/// Send a single export request with retry.
#[instrument(skip_all)]
async fn send_with_retry(
    client: &reqwest::Client,
    url: &str,
    headers: &HashMap<String, String>,
    body: Vec<u8>,
    timeout: Duration,
) -> Result<()> {
    let mut backoff = Backoff::new();

    loop {
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
                if status == 429 {
                    let retry_after = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or_else(|| backoff.next_delay());
                    warn!(status, ?retry_after, "OTLP 429 — backing off");
                    tokio::time::sleep(retry_after).await;
                    continue;
                }
                if status >= 500 {
                    let delay = backoff.next_delay();
                    warn!(status, ?delay, "OTLP 5xx — retrying");
                    tokio::time::sleep(delay).await;
                    continue;
                }
                // 4xx (not 429) — do not retry
                error!(
                    status,
                    "OTLP export failed with client error — not retrying"
                );
                anyhow::bail!("OTLP export failed: HTTP {status}");
            }
            Err(e) => {
                let delay = backoff.next_delay();
                warn!(?delay, error = %e, "OTLP export request error — retrying");
                tokio::time::sleep(delay).await;
            }
        }
    }
}

// ── Public API ─────────────────────────────────────────────────────────

/// Start the periodic OTLP push loop.  Runs until the token is cancelled.
#[instrument(skip_all, fields(endpoint))]
pub async fn run(
    config: OtlpExporter,
    store: MetricStore,
    global_labels: HashMap<String, String>,
    cancel: tokio_util::sync::CancellationToken,
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
    let interval_dur = Duration::from_secs(10);

    info!(%url, "OTLP exporter started");

    let mut interval = tokio::time::interval(interval_dur);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("OTLP exporter shutting down");
                return;
            }
            _ = interval.tick() => {}
        }

        let metrics = store.all_metrics_flat();
        if metrics.is_empty() {
            debug!("No metrics to export");
            continue;
        }

        let body = build_request(&metrics, &global_labels);
        debug!(
            metrics_count = metrics.len(),
            bytes = body.len(),
            "Exporting OTLP batch"
        );

        if let Err(e) = send_with_retry(&client, &url, &config.headers, body, config.timeout).await
        {
            error!(error = %e, "OTLP export failed");
        }
    }
}

#[cfg(test)]
#[path = "otlp_tests.rs"]
mod tests;
