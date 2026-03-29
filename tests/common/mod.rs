//! Shared e2e test harness for bus-exporter.
//!
//! Provides test fixtures, config generation, pull execution, and validation
//! so that protocol-specific e2e tests only need to set up a mock device and
//! call the shared workflow.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Test fixture definitions ──────────────────────────────────────────

/// A single test metric with its configuration, raw mock data, and expected result.
#[derive(Debug, Clone)]
pub struct TestMetric {
    /// Metric name used in config and JSON output.
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Metric type: "gauge" or "counter".
    pub metric_type: &'static str,
    /// Register type: "holding", "input", "coil", or "discrete".
    pub register_type: &'static str,
    /// Starting register address.
    pub address: u16,
    /// Data type: "u16", "i16", "u32", "f32", "bool", etc.
    pub data_type: &'static str,
    /// Byte order: "big_endian", "little_endian", "mid_big_endian", "mid_little_endian".
    pub byte_order: &'static str,
    /// Scale factor applied after raw decode.
    pub scale: f64,
    /// Offset added after scaling.
    pub offset: f64,
    /// Unit string.
    pub unit: &'static str,
    /// Raw u16 register words the mock device should serve at `address`.
    pub raw_registers: Vec<u16>,
    /// Expected decoded value after scale + offset.
    pub expected_value: f64,
}

/// A complete test fixture set.
#[derive(Debug, Clone)]
pub struct TestFixtures {
    pub metrics: Vec<TestMetric>,
}

/// Standard test fixtures covering all supported data types.
pub fn standard_fixtures() -> TestFixtures {
    TestFixtures {
        metrics: vec![
            // u16 holding register with scale: raw=2300, scale=0.1 → 230.0
            TestMetric {
                name: "voltage",
                description: "Phase A voltage",
                metric_type: "gauge",
                register_type: "holding",
                address: 0,
                data_type: "u16",
                byte_order: "big_endian",
                scale: 0.1,
                offset: 0.0,
                unit: "V",
                raw_registers: vec![2300],
                expected_value: 230.0,
            },
            // i16 input register with scale+offset: raw=65436 (i16=-100), scale=0.1, offset=40 → 30.0
            TestMetric {
                name: "temperature",
                description: "Temperature sensor",
                metric_type: "gauge",
                register_type: "input",
                address: 0,
                data_type: "i16",
                byte_order: "big_endian",
                scale: 0.1,
                offset: 40.0,
                unit: "C",
                raw_registers: vec![65436],
                expected_value: 30.0,
            },
            // u32 big-endian: raw=90000 (hi=1, lo=24464), scale=0.01 → 900.0
            TestMetric {
                name: "total_energy",
                description: "Total energy consumption",
                metric_type: "counter",
                register_type: "holding",
                address: 16,
                data_type: "u32",
                byte_order: "big_endian",
                scale: 0.01,
                offset: 0.0,
                unit: "kWh",
                raw_registers: vec![1, 24464],
                expected_value: 900.0,
            },
            // u32 mid-big-endian: same value=90000 with swapped word order
            TestMetric {
                name: "energy_mid",
                description: "Total energy mid-big-endian",
                metric_type: "counter",
                register_type: "holding",
                address: 48,
                data_type: "u32",
                byte_order: "mid_big_endian",
                scale: 0.01,
                offset: 0.0,
                unit: "kWh",
                raw_registers: vec![24464, 1],
                expected_value: 900.0,
            },
            // f32 big-endian: 200.0 → 0x43480000 → registers [0x4348, 0x0000]
            TestMetric {
                name: "frequency",
                description: "Frequency",
                metric_type: "gauge",
                register_type: "holding",
                address: 32,
                data_type: "f32",
                byte_order: "big_endian",
                scale: 1.0,
                offset: 0.0,
                unit: "Hz",
                raw_registers: vec![0x4348, 0x0000],
                expected_value: 200.0,
            },
            // bool coil: true → 1.0
            TestMetric {
                name: "switch_state",
                description: "Switch state",
                metric_type: "gauge",
                register_type: "coil",
                address: 0,
                data_type: "bool",
                byte_order: "big_endian",
                scale: 1.0,
                offset: 0.0,
                unit: "",
                raw_registers: vec![1], // 1 = true
                expected_value: 1.0,
            },
        ],
    }
}

// ── Config generator ──────────────────────────────────────────────────

/// Protocol-specific connection parameters for config generation.
#[derive(Debug, Clone)]
pub enum ConnectionParams {
    ModbusTcp {
        endpoint: String,
        slave_id: u8,
    },
    #[allow(dead_code)]
    ModbusRtu {
        device: String,
        bps: u32,
        slave_id: u8,
    },
}

/// Generate a bus-exporter YAML config and write it to `dir/config.yaml`.
/// Returns the path to the written config file.
pub fn generate_config(
    dir: &Path,
    collector_name: &str,
    connection: &ConnectionParams,
    fixtures: &TestFixtures,
) -> PathBuf {
    let (protocol_yaml, slave_id) = match connection {
        ConnectionParams::ModbusTcp { endpoint, slave_id } => {
            (
                format!(
                    "    protocol:\n      type: modbus-tcp\n      endpoint: \"{}\"",
                    endpoint
                ),
                *slave_id,
            )
        }
        ConnectionParams::ModbusRtu {
            device,
            bps,
            slave_id,
        } => {
            (
                format!(
                    "    protocol:\n      type: modbus-rtu\n      device: \"{}\"\n      bps: {}",
                    device, bps
                ),
                *slave_id,
            )
        }
    };

    let mut metrics_yaml = String::new();
    for m in &fixtures.metrics {
        metrics_yaml.push_str(&format!(
            "      - name: {}\n        description: \"{}\"\n        type: {}\n        register_type: {}\n        address: {}\n        data_type: {}\n        byte_order: {}\n        scale: {}\n        offset: {}\n        unit: \"{}\"\n",
            m.name, m.description, m.metric_type, m.register_type, m.address,
            m.data_type, m.byte_order, m.scale, m.offset, m.unit,
        ));
    }

    let config = format!(
        r#"exporters:
  otlp:
    enabled: false
    endpoint: "http://localhost:4318"
  prometheus:
    enabled: false
    listen: "127.0.0.1:0"

collectors:
  - name: {}
{}
    slave_id: {}
    polling_interval: "1s"
    metrics:
{}
"#,
        collector_name, protocol_yaml, slave_id, metrics_yaml,
    );

    let config_path = dir.join("config.yaml");
    std::fs::write(&config_path, &config).expect("failed to write test config");
    config_path
}

// ── Pull runner ───────────────────────────────────────────────────────

/// Output from a `bus-exporter pull` invocation.
#[derive(Debug)]
pub struct PullResult {
    /// Parsed JSON output from stdout.
    pub json: Value,
    /// Raw stdout string.
    pub stdout: String,
    /// Raw stderr string.
    pub stderr: String,
    /// Process exit code.
    pub exit_code: Option<i32>,
}

/// Find the bus-exporter binary. Uses the cargo-built debug binary.
fn find_binary() -> PathBuf {
    // `cargo test` puts the binary in target/debug/
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push("bus-exporter");
    if path.exists() {
        return path;
    }
    // Fallback: assume it's on PATH
    PathBuf::from("bus-exporter")
}

/// Run `bus-exporter pull -c <config>` and parse the JSON output.
pub fn run_pull(config_path: &Path) -> PullResult {
    let binary = find_binary();
    let output = Command::new(&binary)
        .args(["pull", "-c", config_path.to_str().unwrap()])
        .output()
        .unwrap_or_else(|e| panic!("failed to run bus-exporter binary at {:?}: {}", binary, e));

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code();

    let json: Value = if stdout.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!(
                "failed to parse pull JSON output: {}\nstdout: {}\nstderr: {}",
                e, stdout, stderr
            )
        })
    };

    PullResult {
        json,
        stdout,
        stderr,
        exit_code,
    }
}

// ── Validator ─────────────────────────────────────────────────────────

/// Default float comparison tolerance.
const DEFAULT_TOLERANCE: f64 = 0.001;

/// Validate pull JSON output against test fixtures.
///
/// Checks that each expected metric appears in the output with the correct value.
/// Panics with a descriptive message on mismatch.
pub fn validate(result: &PullResult, fixtures: &TestFixtures) {
    validate_with_tolerance(result, fixtures, DEFAULT_TOLERANCE);
}

/// Validate with a custom tolerance for float comparisons.
pub fn validate_with_tolerance(result: &PullResult, fixtures: &TestFixtures, tolerance: f64) {
    assert!(
        !result.json.is_null(),
        "pull produced no JSON output.\nstderr: {}",
        result.stderr
    );

    let collectors = result.json["collectors"]
        .as_array()
        .expect("expected 'collectors' array in pull output");

    // Build a map of metric_name → value from all collectors
    let mut actual_values: std::collections::HashMap<String, Option<f64>> =
        std::collections::HashMap::new();
    let mut actual_errors: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();

    for collector in collectors {
        let metrics = collector["metrics"]
            .as_array()
            .expect("expected 'metrics' array in collector");
        for metric in metrics {
            let name = metric["name"].as_str().unwrap().to_string();
            let value = metric["value"].as_f64();
            let error = metric["error"].as_str().map(|s| s.to_string());
            actual_values.insert(name.clone(), value);
            actual_errors.insert(name, error);
        }
    }

    for fixture in &fixtures.metrics {
        let name = fixture.name;
        let actual = actual_values
            .get(name)
            .unwrap_or_else(|| panic!("metric '{}' not found in pull output", name));

        // Check for errors
        if let Some(Some(err)) = actual_errors.get(name) {
            panic!("metric '{}' has error: {}", name, err);
        }

        let actual_val = actual.unwrap_or_else(|| {
            panic!("metric '{}' has null value in pull output", name);
        });

        let diff = (actual_val - fixture.expected_value).abs();
        assert!(
            diff <= tolerance,
            "metric '{}': expected {}, got {} (diff={}, tolerance={})",
            name,
            fixture.expected_value,
            actual_val,
            diff,
            tolerance,
        );
    }
}
