//! E2E integration test: Modbus TCP simulator → bus-exporter run (OTLP export)
//! → otel-collector → Prometheus scrape → validation.
//!
//! Requires `otelcol-contrib` (or `otelcol`) on PATH. Skipped otherwise.

#[allow(dead_code)]
mod common;

use std::collections::HashMap;
use std::future;
use std::io;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::process::Command;
use tokio_modbus::prelude::*;
use tokio_modbus::server::tcp::{accept_tcp_connection, Server};
use tokio_modbus::server::Service;

use common::{standard_fixtures, TestFixtures};

// ── Modbus TCP Simulator (same as e2e_modbus.rs) ─────────────────────

#[derive(Clone)]
struct SimulatorService {
    holding: Arc<HashMap<u16, u16>>,
    input: Arc<HashMap<u16, u16>>,
    coils: Arc<HashMap<u16, bool>>,
}

impl SimulatorService {
    fn from_fixtures(fixtures: &TestFixtures) -> Self {
        let mut holding = HashMap::new();
        let mut input = HashMap::new();
        let mut coils = HashMap::new();

        for m in &fixtures.metrics {
            match m.register_type {
                "holding" => {
                    for (i, &val) in m.raw_registers.iter().enumerate() {
                        holding.insert(m.address + i as u16, val);
                    }
                }
                "input" => {
                    for (i, &val) in m.raw_registers.iter().enumerate() {
                        input.insert(m.address + i as u16, val);
                    }
                }
                "coil" => {
                    for (i, &val) in m.raw_registers.iter().enumerate() {
                        coils.insert(m.address + i as u16, val != 0);
                    }
                }
                _ => {}
            }
        }

        Self {
            holding: Arc::new(holding),
            input: Arc::new(input),
            coils: Arc::new(coils),
        }
    }

    fn read_holding(&self, addr: u16, count: u16) -> Vec<u16> {
        (addr..addr + count)
            .map(|a| self.holding.get(&a).copied().unwrap_or(0))
            .collect()
    }

    fn read_input(&self, addr: u16, count: u16) -> Vec<u16> {
        (addr..addr + count)
            .map(|a| self.input.get(&a).copied().unwrap_or(0))
            .collect()
    }

    fn read_coils(&self, addr: u16, count: u16) -> Vec<bool> {
        (addr..addr + count)
            .map(|a| self.coils.get(&a).copied().unwrap_or(false))
            .collect()
    }
}

impl Service for SimulatorService {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = Exception;
    type Future = future::Ready<Result<Self::Response, Self::Exception>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let resp = match req {
            Request::ReadHoldingRegisters(addr, count) => {
                Response::ReadHoldingRegisters(self.read_holding(addr, count))
            }
            Request::ReadInputRegisters(addr, count) => {
                Response::ReadInputRegisters(self.read_input(addr, count))
            }
            Request::ReadCoils(addr, count) => Response::ReadCoils(self.read_coils(addr, count)),
            Request::ReadDiscreteInputs(_, count) => {
                Response::ReadDiscreteInputs(vec![false; count as usize])
            }
            _ => return future::ready(Err(Exception::IllegalFunction)),
        };
        future::ready(Ok(resp))
    }
}

async fn start_simulator(fixtures: &TestFixtures) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Server::new(listener);
    let service = SimulatorService::from_fixtures(fixtures);

    let handle = tokio::spawn(async move {
        let new_service = Arc::new(move |_socket_addr: SocketAddr| {
            let svc = service.clone();
            Ok(Some(svc)) as io::Result<Option<SimulatorService>>
        });
        let on_connected = |stream: tokio::net::TcpStream, socket_addr: SocketAddr| {
            let ns = Arc::clone(&new_service);
            async move { accept_tcp_connection(stream, socket_addr, &*ns) }
        };
        let _ = server
            .serve(&on_connected, |err: io::Error| {
                eprintln!("simulator process error: {err}");
            })
            .await;
    });

    (addr, handle)
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Find a free TCP port by binding to :0 and returning the assigned port.
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
            .is_ok()
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
    sim_addr: &SocketAddr,
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
    endpoint: "http://127.0.0.1:{otlp_port}"
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
    let (sim_addr, sim_handle) = start_simulator(&fixtures).await;

    // 2. Allocate ports and generate configs
    let otlp_port = free_port();
    let prom_port = free_port();
    let tmp = tempfile::tempdir().unwrap();

    let otelcol_config = generate_otelcol_config(tmp.path(), otlp_port, prom_port);
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

    // Verify it's still running
    assert!(
        otelcol_child.try_wait().unwrap().is_none(),
        "otel-collector exited prematurely"
    );

    // 4. Start bus-exporter run
    let binary = find_binary();
    let mut bus_child = Command::new(&binary)
        .args(["run", "-c", bus_config.to_str().unwrap()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to start bus-exporter");

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
fn validate_prometheus_output(body: &str, fixtures: &TestFixtures) {
    let tolerance = 0.01;

    for fixture in &fixtures.metrics {
        let metric_name = fixture.name;
        // Find lines that contain the metric name and have a value (not comments)
        let matching_lines: Vec<&str> = body
            .lines()
            .filter(|line| {
                !line.starts_with('#')
                    && (line.starts_with(metric_name)
                        || line.starts_with(&format!("{metric_name}{{"))
                        || line.starts_with(&format!("{metric_name}_total"))
                        || line.starts_with(&format!("{metric_name}_total{{")))
            })
            .collect();

        assert!(
            !matching_lines.is_empty(),
            "metric '{}' not found in Prometheus output.\nBody:\n{}",
            metric_name,
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
