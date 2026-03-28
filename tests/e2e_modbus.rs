//! E2E integration test: Modbus TCP simulator → bus-exporter → Prometheus /metrics.
//!
//! Replaces the Docker-based `tests/e2e/run.sh` with a pure-Rust test that
//! embeds a Modbus TCP simulator (via `tokio-modbus` server API), starts the
//! `bus-exporter` binary as a child process, and validates the Prometheus
//! metrics output.

use std::collections::HashMap;
use std::future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio_modbus::prelude::*;
use tokio_modbus::server::tcp::{accept_tcp_connection, Server};
use tokio_modbus::server::Service;

// ── Modbus TCP Simulator ──────────────────────────────────────────────

/// Register store matching `config/modbus-simulator.json`.
#[derive(Clone)]
struct SimulatorService {
    holding: Arc<HashMap<u16, u16>>,
    input: Arc<HashMap<u16, u16>>,
}

impl SimulatorService {
    fn new() -> Self {
        // Values from config/modbus-simulator.json (1-indexed there → 0-indexed Modbus addresses)
        let mut holding = HashMap::new();
        // address 0 (json key "1" → addr 0): 2300
        holding.insert(0, 2300u16);
        // address 16,17 (json keys "17","18" → addr 16,17): u32 = 90000 → hi=1, lo=24464
        holding.insert(16, 1);
        holding.insert(17, 24464);
        // address 32,33 (json keys "33","34" → addr 32,33): f32 = 200.0 → 0x43480000
        holding.insert(32, 0x4348);
        holding.insert(33, 0x0000);
        // address 48,49 (json keys "49","50" → addr 48,49): mid-big u32 = 90000
        // mid_big_endian: CDAB order → register layout [lo_word, hi_word] = [24464, 1]
        holding.insert(48, 24464);
        holding.insert(49, 1);

        let mut input = HashMap::new();
        // address 0 (json key "1" → addr 0): 65436 (i16 = -100)
        input.insert(0, 65436u16);

        Self {
            holding: Arc::new(holding),
            input: Arc::new(input),
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
            Request::ReadCoils(_, count) => Response::ReadCoils(vec![false; count as usize]),
            Request::ReadDiscreteInputs(_, count) => {
                Response::ReadDiscreteInputs(vec![false; count as usize])
            }
            _ => return future::ready(Err(Exception::IllegalFunction)),
        };
        future::ready(Ok(resp))
    }
}

/// Start the Modbus TCP simulator on an OS-assigned port. Returns the bound address.
async fn start_simulator() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Server::new(listener);

    let handle = tokio::spawn(async move {
        let service = SimulatorService::new();
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

/// Write a test config YAML with the given simulator and prometheus addresses.
fn write_test_config(
    dir: &std::path::Path,
    modbus_endpoint: &str,
    prom_listen: &str,
) -> std::path::PathBuf {
    let config = format!(
        r#"global_labels:
  env: test
  site: e2e

logging:
  level: debug
  output: stdout

exporters:
  otlp:
    enabled: false
    endpoint: "http://localhost:4318"
  prometheus:
    enabled: true
    listen: "{prom_listen}"
    path: "/metrics"

collectors:
  - name: test_device
    protocol:
      type: modbus-tcp
      endpoint: "{modbus_endpoint}"
    slave_id: 1
    polling_interval: "1s"
    labels:
      device: simulator
    metrics:
      - name: voltage_phase_a
        description: "Phase A voltage"
        type: gauge
        register_type: holding
        address: 0
        data_type: u16
        byte_order: big_endian
        scale: 0.1
        offset: 0.0
        unit: "V"
      - name: total_energy
        description: "Total energy consumption"
        type: counter
        register_type: holding
        address: 16
        data_type: u32
        byte_order: big_endian
        scale: 0.01
        offset: 0.0
        unit: "kWh"
      - name: temperature
        description: "Temperature sensor"
        type: gauge
        register_type: input
        address: 0
        data_type: i16
        byte_order: big_endian
        scale: 0.1
        offset: 40.0
        unit: "C"
      - name: frequency
        description: "Frequency"
        type: gauge
        register_type: holding
        address: 32
        data_type: f32
        byte_order: big_endian
        scale: 1.0
        offset: 0.0
        unit: "Hz"
      - name: total_energy_mid
        description: "Total energy mid-big-endian"
        type: counter
        register_type: holding
        address: 48
        data_type: u32
        byte_order: mid_big_endian
        scale: 0.01
        offset: 0.0
        unit: "kWh"
"#
    );
    let path = dir.join("test-e2e.yaml");
    std::fs::write(&path, config).unwrap();
    path
}

/// Poll a URL until it returns OK or timeout.
async fn wait_for_url(url: &str, timeout: Duration) -> Result<(), String> {
    let start = std::time::Instant::now();
    let client = reqwest::Client::new();
    loop {
        if start.elapsed() > timeout {
            return Err(format!("timeout waiting for {url}"));
        }
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
}

/// Parse Prometheus text exposition format into a map of metric_name{labels} → value.
fn parse_metrics(body: &str) -> HashMap<String, f64> {
    let mut map = HashMap::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Format: metric_name{label="val",...} value  OR  metric_name value
        let (key, val_str) = if let Some(brace_end) = line.find("} ") {
            let key = &line[..=brace_end];
            let val_str = line[brace_end + 2..].trim();
            (key, val_str)
        } else if let Some(pos) = line.rfind(' ') {
            (&line[..pos], line[pos + 1..].trim())
        } else {
            continue;
        };
        if let Ok(v) = val_str.parse::<f64>() {
            map.insert(key.to_string(), v);
        }
    }
    map
}

fn find_type(body: &str, metric: &str) -> Option<String> {
    for line in body.lines() {
        if line.starts_with(&format!("# TYPE {metric} ")) {
            return line.split_whitespace().last().map(|s| s.to_string());
        }
    }
    None
}

fn assert_close(actual: f64, expected: f64, tolerance: f64, name: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{name}: expected {expected} ±{tolerance}, got {actual}"
    );
}

// ── Test ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_modbus_tcp_prometheus() {
    // 1. Start simulator
    let (sim_addr, sim_handle) = start_simulator().await;

    // 2. Find a free port for Prometheus endpoint
    let prom_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let prom_addr = prom_listener.local_addr().unwrap();
    drop(prom_listener); // free the port for bus-exporter

    // 3. Write config
    let tmp = tempfile::tempdir().unwrap();
    let config_path = write_test_config(
        tmp.path(),
        &format!("{}:{}", sim_addr.ip(), sim_addr.port()),
        &format!("{}:{}", prom_addr.ip(), prom_addr.port()),
    );

    // 4. Build and start bus-exporter
    let exporter_bin = std::env::var("BUS_EXPORTER_BIN").unwrap_or_else(|_| {
        // Use cargo to find/build the binary
        let output = std::process::Command::new("cargo")
            .args(["build", "--release"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("failed to run cargo build");
        if !output.status.success() {
            panic!(
                "cargo build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        format!("{}/target/release/bus-exporter", env!("CARGO_MANIFEST_DIR"))
    });

    let mut child = std::process::Command::new(&exporter_bin)
        .args(["--config", config_path.to_str().unwrap()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to start bus-exporter at {exporter_bin}: {e}"));

    let metrics_url = format!("http://{}:{}/metrics", prom_addr.ip(), prom_addr.port());

    // 5. Wait for metrics endpoint + one poll cycle
    if let Err(e) = wait_for_url(&metrics_url, Duration::from_secs(30)).await {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!(
            "Metrics endpoint not ready: {e}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    // Wait extra for at least one poll cycle
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 6. Scrape metrics
    let client = reqwest::Client::new();
    let body = client
        .get(&metrics_url)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    eprintln!("=== Metrics Output ===\n{body}");

    let metrics = parse_metrics(&body);

    // 7. Assertions

    // -- Metric existence --
    let metric_names = [
        "bus_voltage_phase_a_V",
        "bus_total_energy_kWh",
        "bus_temperature_C",
        "bus_frequency_Hz",
        "bus_total_energy_mid_kWh",
    ];
    for name in &metric_names {
        assert!(
            body.contains(&format!("{name}{{")) || body.contains(&format!("{name} ")),
            "metric '{name}' not found in output"
        );
    }

    // -- Types --
    assert_eq!(
        find_type(&body, "bus_voltage_phase_a_V").as_deref(),
        Some("gauge")
    );
    assert_eq!(
        find_type(&body, "bus_total_energy_kWh").as_deref(),
        Some("counter")
    );
    assert_eq!(
        find_type(&body, "bus_temperature_C").as_deref(),
        Some("gauge")
    );
    assert_eq!(
        find_type(&body, "bus_frequency_Hz").as_deref(),
        Some("gauge")
    );
    assert_eq!(
        find_type(&body, "bus_total_energy_mid_kWh").as_deref(),
        Some("counter")
    );

    // -- Global labels --
    assert!(body.contains(r#"env="test""#), "missing env=test label");
    assert!(body.contains(r#"site="e2e""#), "missing site=e2e label");
    assert!(
        body.contains(r#"device="simulator""#),
        "missing device=simulator label"
    );

    // -- Values (with tolerance) --
    let tolerance = 0.01;

    // Find metric values by prefix
    let find_val = |prefix: &str| -> f64 {
        metrics
            .iter()
            .find(|(k, _)| k.starts_with(&format!("{prefix}{{")))
            .unwrap_or_else(|| panic!("metric '{prefix}' not found in parsed metrics"))
            .1
            .to_owned()
    };

    // voltage_phase_a: register 0 = 2300, scale=0.1 → 230.0
    assert_close(
        find_val("bus_voltage_phase_a_V"),
        230.0,
        tolerance,
        "voltage_phase_a",
    );
    // total_energy: registers 16,17 = 90000, scale=0.01 → 900.0
    assert_close(
        find_val("bus_total_energy_kWh"),
        900.0,
        tolerance,
        "total_energy",
    );
    // temperature: register 0 = 65436 (i16=-100), scale=0.1, offset=40 → 30.0
    assert_close(
        find_val("bus_temperature_C"),
        30.0,
        tolerance,
        "temperature",
    );
    // frequency: registers 32,33 = 0x43480000 (f32=200.0)
    assert_close(find_val("bus_frequency_Hz"), 200.0, tolerance, "frequency");
    // total_energy_mid: registers 48,49 mid-big = 90000, scale=0.01 → 900.0
    assert_close(
        find_val("bus_total_energy_mid_kWh"),
        900.0,
        tolerance,
        "total_energy_mid",
    );

    // -- Internal metrics --
    assert!(
        body.contains("bus_exporter_collectors_total"),
        "missing internal metric collectors_total"
    );
    assert!(
        body.contains("bus_exporter_uptime_seconds"),
        "missing internal metric uptime_seconds"
    );
    assert!(
        body.contains("bus_exporter_polls_total"),
        "missing internal metric polls_total"
    );
    assert!(
        body.contains("bus_exporter_prometheus_scrapes_total"),
        "missing internal metric scrapes_total"
    );

    assert_eq!(
        find_type(&body, "bus_exporter_collectors_total").as_deref(),
        Some("gauge")
    );
    assert_eq!(
        find_type(&body, "bus_exporter_uptime_seconds").as_deref(),
        Some("gauge")
    );
    assert_eq!(
        find_type(&body, "bus_exporter_polls_total").as_deref(),
        Some("counter")
    );
    assert_eq!(
        find_type(&body, "bus_exporter_prometheus_scrapes_total").as_deref(),
        Some("counter")
    );

    // Uptime > 0
    let uptime = metrics
        .iter()
        .find(|(k, _)| k.starts_with("bus_exporter_uptime_seconds"))
        .expect("uptime metric not found")
        .1;
    assert!(*uptime > 0.0, "uptime should be > 0, got {uptime}");

    // 8. Graceful shutdown
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "bus-exporter exited with non-zero status: {status}"
    );

    // Cleanup
    sim_handle.abort();
}
