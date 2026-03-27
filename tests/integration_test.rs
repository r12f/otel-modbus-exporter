//! Integration tests for otel-modbus-exporter.

use std::collections::BTreeMap;
use std::io::Write;
use std::time::SystemTime;

// ── Config loading & validation ───────────────────────────────────────

#[test]
fn config_load_valid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        r#"
exporters:
  prometheus:
    enabled: true
    listen: "0.0.0.0:9090"
    path: "/metrics"
collectors:
  - name: dev1
    protocol:
      type: tcp
      endpoint: "127.0.0.1:502"
    slave_id: 1
    polling_interval: "1s"
    metrics:
      - name: voltage
        type: gauge
        register_type: holding
        address: 0
        data_type: u16
        byte_order: big_endian
        scale: 1.0
        offset: 0.0
"#,
    )
    .unwrap();

    let config = modbus_exporter::config::Config::load(&path);
    assert!(config.is_ok(), "valid config should load: {config:?}");
    let config = config.unwrap();
    assert_eq!(config.collectors.len(), 1);
    assert_eq!(config.collectors[0].name, "dev1");
}

#[test]
fn config_load_invalid_no_exporter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.yaml");
    std::fs::write(
        &path,
        r#"
exporters:
  otlp:
    enabled: false
  prometheus:
    enabled: false
collectors:
  - name: dev1
    protocol:
      type: tcp
      endpoint: "127.0.0.1:502"
    slave_id: 1
    polling_interval: "1s"
    metrics:
      - name: voltage
        type: gauge
        register_type: holding
        address: 0
        data_type: u16
        byte_order: big_endian
        scale: 1.0
        offset: 0.0
"#,
    )
    .unwrap();

    let result = modbus_exporter::config::Config::load(&path);
    assert!(result.is_err(), "should fail: no exporter enabled");
}

// ── MetricStore round-trip ────────────────────────────────────────────

#[test]
fn metric_store_publish_and_read() {
    let store = modbus_exporter::metrics::MetricStore::new();
    let global = BTreeMap::from([("env".to_string(), "test".to_string())]);
    let collector_labels = BTreeMap::new();

    let metric = modbus_exporter::metrics::MetricValue {
        name: "temp".to_string(),
        value: 22.5,
        metric_type: modbus_exporter::metrics::MetricType::Gauge,
        labels: BTreeMap::from([("sensor".to_string(), "s1".to_string())]),
        description: "Temperature".to_string(),
        unit: "C".to_string(),
        updated_at: SystemTime::now(),
    };

    store.publish("c1", vec![metric], &global, &collector_labels);
    let flat = store.all_metrics_flat();
    assert_eq!(flat.len(), 1);
    assert!((flat[0].value - 22.5).abs() < f64::EPSILON);
    assert!(flat[0].labels.contains_key("env"));
    assert!(flat[0].labels.contains_key("sensor"));
}

// ── Decoder round-trip ────────────────────────────────────────────────

#[test]
fn decoder_u16_roundtrip() {
    let value = 12345u16;
    let registers = [value];
    let result = modbus_exporter::decoder::decode(
        &registers,
        modbus_exporter::decoder::DataType::U16,
        modbus_exporter::decoder::ByteOrder::BigEndian,
        1.0,
        0.0,
    )
    .unwrap();
    assert!((result - 12345.0).abs() < f64::EPSILON);
}

#[test]
fn decoder_i16_negative() {
    let value = (-100i16) as u16;
    let registers = [value];
    let result = modbus_exporter::decoder::decode(
        &registers,
        modbus_exporter::decoder::DataType::I16,
        modbus_exporter::decoder::ByteOrder::BigEndian,
        1.0,
        0.0,
    )
    .unwrap();
    assert!((result - (-100.0)).abs() < f64::EPSILON);
}

#[test]
fn decoder_f32_big_endian() {
    let val: f32 = 3.14;
    let bytes = val.to_be_bytes();
    let r0 = u16::from_be_bytes([bytes[0], bytes[1]]);
    let r1 = u16::from_be_bytes([bytes[2], bytes[3]]);
    let registers = [r0, r1];
    let result = modbus_exporter::decoder::decode(
        &registers,
        modbus_exporter::decoder::DataType::F32,
        modbus_exporter::decoder::ByteOrder::BigEndian,
        1.0,
        0.0,
    )
    .unwrap();
    assert!((result - 3.14_f64).abs() < 0.001);
}

#[test]
fn decoder_with_scale_and_offset() {
    let registers = [100u16];
    let result = modbus_exporter::decoder::decode(
        &registers,
        modbus_exporter::decoder::DataType::U16,
        modbus_exporter::decoder::ByteOrder::BigEndian,
        0.1,
        -10.0,
    )
    .unwrap();
    // 100 * 0.1 + (-10.0) = 0.0
    assert!((result - 0.0).abs() < f64::EPSILON);
}

#[test]
fn decoder_insufficient_registers() {
    let registers = [1u16]; // need 2 for u32
    let result = modbus_exporter::decoder::decode(
        &registers,
        modbus_exporter::decoder::DataType::U32,
        modbus_exporter::decoder::ByteOrder::BigEndian,
        1.0,
        0.0,
    );
    assert!(result.is_err());
}
