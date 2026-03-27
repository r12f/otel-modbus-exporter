use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Router};
use std::sync::Arc;
use tracing::{info, instrument};

use crate::config::PrometheusExporter;
use crate::metrics::{MetricStore, MetricType, MetricValue};

/// Shared state for the Prometheus HTTP handler.
#[derive(Debug, Clone)]
struct PrometheusState {
    store: MetricStore,
}

/// Sanitise a string so it matches `[a-zA-Z_][a-zA-Z0-9_]*`.
fn sanitize_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_alphanumeric() || c == '_' {
            if i == 0 && c.is_ascii_digit() {
                out.push('_');
            }
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Build the fully-qualified metric name: `otel_modbus_{name}[_{unit}]`.
fn build_metric_name(name: &str, unit: &str) -> String {
    let sname = sanitize_name(name);
    if unit.is_empty() {
        format!("otel_modbus_{sname}")
    } else {
        let sunit = sanitize_name(unit);
        format!("otel_modbus_{sname}_{sunit}")
    }
}

/// Format a single metric value line with labels.
fn format_metric_line(
    fqname: &str,
    labels: &std::collections::BTreeMap<String, String>,
    value: f64,
) -> String {
    if labels.is_empty() {
        format!("{fqname} {value}")
    } else {
        let label_str: Vec<String> = labels
            .iter()
            .map(|(k, v)| {
                let sk = sanitize_name(k);
                let escaped = v
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n");
                format!("{sk}=\"{escaped}\"")
            })
            .collect();
        format!("{fqname}{{{labels}}} {value}", labels = label_str.join(","))
    }
}

/// Render all metrics from the store in Prometheus exposition format.
fn render_metrics(store: &MetricStore) -> String {
    let all = store.all_metrics_flat();

    // Group by fully-qualified name to emit HELP/TYPE once per metric name.
    let mut grouped: std::collections::BTreeMap<String, Vec<&MetricValue>> =
        std::collections::BTreeMap::new();
    for m in &all {
        let fqname = build_metric_name(&m.name, &m.unit);
        grouped.entry(fqname).or_default().push(m);
    }

    let mut buf = String::new();
    for (fqname, values) in &grouped {
        // Use the first value for HELP and TYPE metadata.
        let first = values[0];
        let type_str = match first.metric_type {
            MetricType::Gauge => "gauge",
            MetricType::Counter => "counter",
        };
        let escaped_desc = first.description.replace('\\', "\\\\").replace('\n', "\\n");
        buf.push_str(&format!("# HELP {fqname} {escaped_desc}\n"));
        buf.push_str(&format!("# TYPE {fqname} {type_str}\n"));
        for m in values {
            buf.push_str(&format_metric_line(fqname, &m.labels, m.value));
            buf.push('\n');
        }
    }
    buf
}

/// Handler for `/metrics` (or configured path).
#[instrument(skip_all)]
async fn metrics_handler(State(state): State<Arc<PrometheusState>>) -> impl IntoResponse {
    let body = render_metrics(&state.store);
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

/// Start the Prometheus scrape HTTP server.
///
/// This function runs until the server is shut down or the process exits.
#[instrument(skip(store))]
pub async fn serve(config: &PrometheusExporter, store: MetricStore) -> anyhow::Result<()> {
    if !config.enabled {
        info!("Prometheus exporter disabled");
        return Ok(());
    }

    let state = Arc::new(PrometheusState { store });
    let path = config.path.clone();

    let app = Router::new()
        .route(&path, get(metrics_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    info!(listen = %config.listen, path = %path, "Prometheus exporter started");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
#[path = "prometheus_tests.rs"]
mod tests;
