use super::*;
use std::io::Write;
use tempfile::NamedTempFile;

fn sample_config_yaml() -> &'static str {
    r#"
logging:
  level: info
  output: stdout

global_labels:
  env: test

exporters:
  prometheus:
    enabled: true
    listen: "0.0.0.0:9090"
  mqtt:
    enabled: true
    endpoint: "mqtt://localhost:1883"
    auth:
      username: exporter
      password: supersecret

collectors:
  - name: sensor_a
    protocol:
      type: i2c
      bus: "/dev/i2c-1"
      address: 0x76
    polling_interval: "5s"
    metrics:
      - name: temperature
        type: gauge
        address: 0x22
        data_type: u16
        byte_order: big_endian
        scale: 0.01
        offset: -40.0
        unit: "°C"
      - name: humidity
        type: gauge
        address: 0x24
        data_type: u16
        byte_order: big_endian
        scale: 0.01
        unit: "%"
  - name: meter_b
    protocol:
      type: modbus-tcp
      endpoint: "192.168.1.100:502"
    slave_id: 1
    polling_interval: "10s"
    metrics:
      - name: voltage
        type: gauge
        register_type: holding
        address: 0
        data_type: u16
        byte_order: big_endian
        scale: 0.1
        unit: "V"
  - name: sensor_i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
    polling_interval: "10s"
    metrics: []
"#
}

fn write_temp_config(yaml: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    f
}

#[test]
fn test_yaml_output_valid() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    let output = serde_yaml::to_string(&cfg).unwrap();
    // Should parse back successfully
    let _: serde_yaml::Value = serde_yaml::from_str(&output).unwrap();
    assert!(output.contains("sensor_a"));
    assert!(output.contains("logging"));
    assert!(output.contains("exporters"));
}

#[test]
fn test_json_output_valid() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    let output = serde_json::to_string_pretty(&cfg).unwrap();
    let _: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert!(output.contains("sensor_a"));
    assert!(output.contains("logging"));
    assert!(output.contains("exporters"));
}

#[test]
fn test_collector_filter() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    let filtered =
        crate::commands::filter_collectors(&cfg.collectors, Some("sensor"), None).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "sensor_a");
}

#[test]
fn test_metric_filter_removes_empty_collectors() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    // "temperature" only in sensor_a, not in meter_b
    let filtered =
        crate::commands::filter_collectors(&cfg.collectors, None, Some("temperature")).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "sensor_a");
    assert_eq!(filtered[0].metrics.len(), 1);
    assert_eq!(filtered[0].metrics[0].name, "temperature");
}

#[test]
fn test_password_redaction() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    let output = serde_yaml::to_string(&cfg).unwrap();
    assert!(output.contains("'***'") || output.contains("\"***\"") || output.contains("***"));
    assert!(!output.contains("supersecret"));

    let json_output = serde_json::to_string_pretty(&cfg).unwrap();
    assert!(json_output.contains("***"));
    assert!(!json_output.contains("supersecret"));
}

#[test]
fn test_empty_filter_warning() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    let filtered =
        crate::commands::filter_collectors(&cfg.collectors, Some("nonexistent"), None).unwrap();
    assert!(filtered.is_empty());
    // The warning is printed in show_config_command; here we verify filter returns empty
}

#[test]
fn test_metrics_files_not_serialized() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    let output = serde_yaml::to_string(&cfg).unwrap();
    assert!(!output.contains("metrics_files"));
}

#[test]
fn test_invalid_regex_returns_error() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    let result = crate::commands::filter_collectors(&cfg.collectors, Some("[invalid"), None);
    assert!(result.is_err());
}

#[test]
fn test_no_null_fields_in_yaml_output() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    let output = serde_yaml::to_string(&cfg).unwrap();
    assert!(
        !output.contains(": null"),
        "YAML output should not contain null fields, got:\n{}",
        output
    );
}

#[test]
fn test_no_null_fields_in_json_output() {
    let f = write_temp_config(sample_config_yaml());
    let path = f.path();
    let cfg = config::Config::load_for_pull(path).unwrap();
    let output = serde_json::to_string_pretty(&cfg).unwrap();
    assert!(
        !output.contains(": null"),
        "JSON output should not contain null fields, got:\n{}",
        output
    );
}
