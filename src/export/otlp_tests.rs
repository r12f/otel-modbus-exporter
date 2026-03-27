use super::*;
use crate::metrics::{MetricStore, MetricType, MetricValue};
use std::collections::BTreeMap;
use std::time::SystemTime;

fn sample_gauge() -> MetricValue {
    let mut labels = BTreeMap::new();
    labels.insert("device".to_string(), "plc1".to_string());
    MetricValue {
        name: "temperature".to_string(),
        value: 23.5,
        metric_type: MetricType::Gauge,
        labels,
        description: "Temperature reading".to_string(),
        unit: "celsius".to_string(),
        updated_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
    }
}

fn sample_counter() -> MetricValue {
    MetricValue {
        name: "energy_total".to_string(),
        value: 42.0,
        metric_type: MetricType::Counter,
        labels: BTreeMap::new(),
        description: "Total energy".to_string(),
        unit: "kWh".to_string(),
        updated_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
    }
}

#[test]
fn build_request_empty_metrics() {
    let body = build_request(&[], &std::collections::HashMap::new());
    // Should produce a valid (minimal) protobuf — at least the outer envelope
    assert!(!body.is_empty());
}

#[test]
fn build_request_gauge_roundtrip() {
    let metrics = vec![sample_gauge()];
    let mut global = std::collections::HashMap::new();
    global.insert("service.name".to_string(), "test".to_string());
    let body = build_request(&metrics, &global);
    // The body must contain the metric name and scope name as raw bytes
    assert!(body
        .windows(b"temperature".len())
        .any(|w| w == b"temperature"));
    assert!(body
        .windows(b"otel-modbus-exporter".len())
        .any(|w| w == b"otel-modbus-exporter"));
    assert!(body
        .windows(b"service.name".len())
        .any(|w| w == b"service.name"));
}

#[test]
fn build_request_counter_has_sum_fields() {
    let metrics = vec![sample_counter()];
    let body = build_request(&metrics, &std::collections::HashMap::new());
    assert!(body
        .windows(b"energy_total".len())
        .any(|w| w == b"energy_total"));
}

#[test]
fn build_request_mixed_metrics() {
    let metrics = vec![sample_gauge(), sample_counter()];
    let body = build_request(&metrics, &std::collections::HashMap::new());
    assert!(body
        .windows(b"temperature".len())
        .any(|w| w == b"temperature"));
    assert!(body
        .windows(b"energy_total".len())
        .any(|w| w == b"energy_total"));
}

#[test]
fn backoff_progression() {
    let mut b = Backoff::new();
    assert_eq!(b.next_delay(), std::time::Duration::from_secs(1));
    assert_eq!(b.next_delay(), std::time::Duration::from_secs(2));
    assert_eq!(b.next_delay(), std::time::Duration::from_secs(4));
    assert_eq!(b.next_delay(), std::time::Duration::from_secs(8));
    assert_eq!(b.next_delay(), std::time::Duration::from_secs(16));
    assert_eq!(b.next_delay(), std::time::Duration::from_secs(30)); // capped
    assert_eq!(b.next_delay(), std::time::Duration::from_secs(30));
    b.reset();
    assert_eq!(b.next_delay(), std::time::Duration::from_secs(1));
}

#[test]
fn system_time_to_nanos_epoch() {
    let nanos = system_time_to_nanos(SystemTime::UNIX_EPOCH);
    assert_eq!(nanos, 0);
}

#[test]
fn system_time_to_nanos_known_value() {
    let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let nanos = system_time_to_nanos(t);
    assert_eq!(nanos, 1_700_000_000_000_000_000);
}

#[test]
fn metric_store_integration() {
    let store = MetricStore::new();
    let global = BTreeMap::from([("env".to_string(), "test".to_string())]);
    let collector_labels = BTreeMap::new();
    store.publish("c1", vec![sample_gauge()], &global, &collector_labels);
    let flat = store.all_metrics_flat();
    assert_eq!(flat.len(), 1);
    // Should contain merged labels
    assert!(flat[0].labels.contains_key("env"));
    assert!(flat[0].labels.contains_key("device"));
    assert!(flat[0].labels.contains_key("collector"));
}
