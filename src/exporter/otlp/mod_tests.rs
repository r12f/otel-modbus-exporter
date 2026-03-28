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
    let body = build_request(
        &[],
        &std::collections::HashMap::new(),
        SystemTime::UNIX_EPOCH,
    );
    // Should produce a valid (minimal) protobuf — at least the outer envelope
    assert!(!body.is_empty());
}

#[test]
fn build_request_gauge_roundtrip() {
    let metrics = vec![sample_gauge()];
    let mut global = std::collections::HashMap::new();
    global.insert("service.name".to_string(), "test".to_string());
    let body = build_request(&metrics, &global, SystemTime::UNIX_EPOCH);
    // The body must contain the metric name and scope name as raw bytes
    assert!(body
        .windows(b"temperature".len())
        .any(|w| w == b"temperature"));
    assert!(body
        .windows(b"bus-exporter".len())
        .any(|w| w == b"bus-exporter"));
    assert!(body
        .windows(b"service.name".len())
        .any(|w| w == b"service.name"));
}

#[test]
fn build_request_counter_has_sum_fields() {
    let metrics = vec![sample_counter()];
    let body = build_request(
        &metrics,
        &std::collections::HashMap::new(),
        SystemTime::UNIX_EPOCH,
    );
    assert!(body
        .windows(b"energy_total".len())
        .any(|w| w == b"energy_total"));
}

#[test]
fn build_request_mixed_metrics() {
    let metrics = vec![sample_gauge(), sample_counter()];
    let body = build_request(
        &metrics,
        &std::collections::HashMap::new(),
        SystemTime::UNIX_EPOCH,
    );
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
    // With ±25% jitter, 1s base → 750ms..1250ms
    let d1 = b.next_delay();
    assert!(
        d1 >= std::time::Duration::from_millis(750) && d1 <= std::time::Duration::from_millis(1250)
    );
    let d2 = b.next_delay(); // base 2s → 1500..2500
    assert!(
        d2 >= std::time::Duration::from_millis(1500)
            && d2 <= std::time::Duration::from_millis(2500)
    );
    let d3 = b.next_delay(); // base 4s → 3000..5000
    assert!(
        d3 >= std::time::Duration::from_millis(3000)
            && d3 <= std::time::Duration::from_millis(5000)
    );
    b.reset();
    let d_reset = b.next_delay();
    assert!(
        d_reset >= std::time::Duration::from_millis(750)
            && d_reset <= std::time::Duration::from_millis(1250)
    );
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

#[tokio::test]
async fn send_with_retry_respects_retry_after_header() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let call_count = Arc::new(AtomicU32::new(0));
    let call_count2 = call_count.clone();

    // Start a mock server that returns 429 with Retry-After: 1 on the first call,
    // then 200 on the second.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let call_count2 = call_count2.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let mut stream = stream;
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let _ = stream.read(&mut buf).await;

                let n = call_count2.fetch_add(1, Ordering::SeqCst);
                let response = if n == 0 {
                    "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 11\r\n\r\nrate limited"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
                };
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });

    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/metrics");
    let cancel = tokio_util::sync::CancellationToken::new();
    let start = std::time::Instant::now();

    let result = send_with_retry(
        &client,
        &url,
        &HashMap::new(),
        vec![0u8; 10],
        Duration::from_secs(5),
        &cancel,
    )
    .await;

    let elapsed = start.elapsed();
    assert!(result.is_ok(), "expected success after retry: {result:?}");
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
    // Should have waited ~1s (Retry-After: 1), not the default exponential backoff
    assert!(
        elapsed >= Duration::from_millis(900),
        "should wait at least ~1s per Retry-After"
    );

    server.abort();
}
