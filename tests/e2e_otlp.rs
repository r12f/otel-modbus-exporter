//! E2E integration test: Modbus TCP simulator → bus-exporter run (OTLP export)
//! → otel-collector → Prometheus scrape → validation.
//!
//! Requires `otelcol-contrib` (or `otelcol`) on PATH. Skipped otherwise.

#[allow(dead_code)]
mod common;

use std::net::TcpListener as StdTcpListener;
use std::time::Duration;

use tokio::process::Command;

use common::{standard_fixtures, TestFixtures};

// ── Helpers ───────────────────────────────────────────────────────────

/// Find a free TCP port by binding to :0 and returning the assigned port.
///
/// TODO: There is an inherent TOCTOU race here — the port may be claimed by
/// another process between the time we release the listener and the time the
/// target process binds to it. For now this is acceptable in test code; a more
/// robust approach would hold all listeners simultaneously and drop them just
/// before spawning the processes.
fn free_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Locate the otel-collector binary. Returns None if not found.
fn find_otelcol() -> Option<String> {
    for name in &["otelcol-contrib", "otelcol"] {
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Find the bus-exporter binary (same logic as common harness).
fn find_binary() -> std::path::PathBuf {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push("bus-exporter");
    if path.exists() {
        return path;
    }
    std::path::PathBuf::from("bus-exporter")
}

/// Generate an otel-collector config that receives OTLP/HTTP and exports to Prometheus.
fn generate_otelcol_config(
    dir: &std::path::Path,
    otlp_port: u16,
    prom_port: u16,
    telemetry_port: u16,
) -> std::path::PathBuf {
    let config = format!(
        r#"receivers:
  otlp:
    protocols:
      http:
        endpoint: "127.0.0.1:{otlp_port}"

exporters:
  prometheus:
    endpoint: "127.0.0.1:{prom_port}"

service:
  telemetry:
    metrics:
      address: "127.0.0.1:{telemetry_port}"
  pipelines:
    metrics:
      receivers: [otlp]
      exporters: [prometheus]
"#
    );

    let path = dir.join("otelcol-config.yaml");
    std::fs::write(&path, config).unwrap();
    path
}

/// Generate bus-exporter config with OTLP enabled pointing at the collector.
fn generate_bus_exporter_config(
    dir: &std::path::Path,
    sim_addr: &std::net::SocketAddr,
    otlp_port: u16,
    fixtures: &TestFixtures,
) -> std::path::PathBuf {
    let mut metrics_yaml = String::new();
    for m in &fixtures.metrics {
        let register_type_line = if m.register_type.is_empty() {
            String::new()
        } else {
            format!("        register_type: {}\n", m.register_type)
        };
        metrics_yaml.push_str(&format!(
            "      - name: {}\n        description: \"{}\"\n        type: {}\n{}        address: {}\n        data_type: {}\n        byte_order: {}\n        scale: {}\n        offset: {}\n        unit: \"{}\"\n",
            m.name, m.description, m.metric_type, register_type_line, m.address,
            m.data_type, m.byte_order, m.scale, m.offset, m.unit,
        ));
    }

    let config = format!(
        r#"exporters:
  otlp:
    enabled: true
    endpoint: "http://127.0.0.1:{otlp_port}/v1/metrics"
    interval: "2s"
    timeout: "5s"
  prometheus:
    enabled: false
    listen: "127.0.0.1:0"

collectors:
  - name: test_device
    protocol:
      type: modbus-tcp
      endpoint: "{}:{}"
    slave_id: 1
    polling_interval: "1s"
    metrics:
{metrics_yaml}
"#,
        sim_addr.ip(),
        sim_addr.port(),
    );

    let path = dir.join("config.yaml");
    std::fs::write(&path, &config).unwrap();
    path
}

// ── Test ──────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore] // Requires otelcol-contrib on PATH
async fn e2e_otlp_export() {
    // 0. Skip if no otel-collector binary
    let otelcol_bin = match find_otelcol() {
        Some(bin) => bin,
        None => {
            eprintln!("SKIP: otelcol-contrib / otelcol not found on PATH");
            return;
        }
    };

    let fixtures = standard_fixtures();

    // 1. Start Modbus TCP simulator
    let (sim_addr, sim_handle) = common::start_simulator(&fixtures).await;

    // 2. Allocate ports and generate configs
    let otlp_port = free_port();
    let prom_port = free_port();
    let telemetry_port = free_port();
    let tmp = tempfile::tempdir().unwrap();

    let otelcol_config = generate_otelcol_config(tmp.path(), otlp_port, prom_port, telemetry_port);
    let bus_config = generate_bus_exporter_config(tmp.path(), &sim_addr, otlp_port, &fixtures);

    // 3. Start otel-collector
    let mut otelcol_child = Command::new(&otelcol_bin)
        .arg("--config")
        .arg(&otelcol_config)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to start otel-collector");

    // Give the collector a moment to start
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify it's still running; log stderr on premature exit
    if let Some(status) = otelcol_child.try_wait().unwrap() {
        let stderr = otelcol_child.stderr.take();
        let stderr_output = if let Some(mut se) = stderr {
            use tokio::io::AsyncReadExt;
            let mut buf = String::new();
            let _ = se.read_to_string(&mut buf).await;
            buf
        } else {
            String::from("<stderr not captured>")
        };
        panic!("otel-collector exited prematurely with status {status}\nstderr:\n{stderr_output}");
    }

    // 4. Start bus-exporter run
    let binary = find_binary();
    let mut bus_child = Command::new(&binary)
        .args(["run", "-c", bus_config.to_str().unwrap()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to start bus-exporter");

    // Verify bus-exporter is still running after a brief warmup
    tokio::time::sleep(Duration::from_millis(500)).await;
    if let Some(status) = bus_child.try_wait().unwrap() {
        let stderr = bus_child.stderr.take();
        let stderr_output = if let Some(mut se) = stderr {
            use tokio::io::AsyncReadExt;
            let mut buf = String::new();
            let _ = se.read_to_string(&mut buf).await;
            buf
        } else {
            String::from("<stderr not captured>")
        };
        panic!("bus-exporter exited prematurely with status {status}\nstderr:\n{stderr_output}");
    }

    // 5. Wait for metrics to flow through the pipeline, then scrape Prometheus
    let prom_url = format!("http://127.0.0.1:{prom_port}/metrics");
    let client = reqwest::Client::new();

    let mut found_metrics = false;
    // Retry for up to 30 seconds (OTLP interval is 2s, collector needs warmup)
    for attempt in 1..=15 {
        tokio::time::sleep(Duration::from_secs(2)).await;

        let resp = match client.get(&prom_url).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("attempt {attempt}: Prometheus scrape failed: {e}");
                continue;
            }
        };

        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("attempt {attempt}: failed to read body: {e}");
                continue;
            }
        };

        // Check if we see at least one of our expected metrics
        // (names may be normalized by otel-collector, e.g. "voltage" → "voltage_volts")
        if body.contains("voltage") && body.contains("temperature") {
            eprintln!("attempt {attempt}: found metrics in Prometheus output");
            // Validate all expected metrics
            validate_prometheus_output(&body, &fixtures);
            found_metrics = true;
            break;
        } else {
            eprintln!(
                "attempt {attempt}: metrics not yet available ({} bytes)",
                body.len()
            );
        }
    }

    assert!(found_metrics, "timed out waiting for metrics in Prometheus");

    // 6. Cleanup
    let _ = bus_child.kill().await;
    let _ = otelcol_child.kill().await;
    sim_handle.abort();
}

/// Parse Prometheus text format and validate expected metric values.
///
/// The otel-collector Prometheus exporter normalizes metric names per
/// OpenMetrics conventions: units are appended as suffixes, counters get
/// `_total`, and some unit names are expanded (e.g. "V" → "volts",
/// "Hz" → "hertz").  The prefix `total_` may also be reordered to a
/// `_total` suffix.  We search for non-comment lines whose metric name
/// portion contains the fixture name (or its stem without `total_`).
fn validate_prometheus_output(body: &str, fixtures: &TestFixtures) {
    let tolerance = 0.01;

    for fixture in &fixtures.metrics {
        let metric_name = fixture.name;
        // The Prometheus exporter may reorder "total_X" → "X_..._total",
        // so also try the name with the "total_" prefix stripped.
        let alt_name = metric_name.strip_prefix("total_").unwrap_or(metric_name);

        let matching_lines: Vec<&str> = body
            .lines()
            .filter(|line| {
                if line.starts_with('#') || line.is_empty() {
                    return false;
                }
                // Extract the metric-name portion (everything before '{' or ' ')
                let name_part = line.find(['{', ' ']).map(|i| &line[..i]).unwrap_or(line);
                // Skip internal bus_exporter metrics
                if name_part.starts_with("bus_exporter_") {
                    return false;
                }
                name_part.contains(metric_name) || name_part.contains(alt_name)
            })
            .collect();

        assert!(
            !matching_lines.is_empty(),
            "metric '{}' (alt '{}') not found in Prometheus output.\nBody:\n{}",
            metric_name,
            alt_name,
            body
        );

        // Parse the value from the last matching line
        let line = matching_lines.last().unwrap();
        let value_str = line.rsplit_once(' ').expect("no space in metric line").1;
        let actual: f64 = value_str.parse().unwrap_or_else(|e| {
            panic!(
                "failed to parse value '{}' for metric '{}': {}",
                value_str, metric_name, e
            )
        });

        let diff = (actual - fixture.expected_value).abs();
        assert!(
            diff <= tolerance,
            "metric '{}': expected {}, got {} (diff={}, tolerance={})",
            metric_name,
            fixture.expected_value,
            actual,
            diff,
            tolerance,
        );
    }
}
