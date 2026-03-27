use super::*;
use crate::metrics::{MetricStore, MetricType, MetricValue};
use std::collections::BTreeMap;
use std::time::SystemTime;

fn make_metric(
    name: &str,
    value: f64,
    mt: MetricType,
    unit: &str,
    desc: &str,
    labels: BTreeMap<String, String>,
) -> MetricValue {
    MetricValue {
        name: name.to_string(),
        value,
        metric_type: mt,
        labels,
        description: desc.to_string(),
        unit: unit.to_string(),
        updated_at: SystemTime::now(),
    }
}

#[test]
fn test_sanitize_name() {
    assert_eq!(sanitize_name("hello-world"), "hello_world");
    assert_eq!(sanitize_name("123abc"), "_123abc");
    assert_eq!(sanitize_name("ok_name"), "ok_name");
    assert_eq!(sanitize_name(""), "_");
}

#[test]
fn test_build_metric_name_with_unit() {
    assert_eq!(
        build_metric_name("voltage", "volts"),
        "modbus_voltage_volts"
    );
}

#[test]
fn test_build_metric_name_without_unit() {
    assert_eq!(build_metric_name("temperature", ""), "modbus_temperature");
}

#[test]
fn test_render_metrics_empty() {
    let store = MetricStore::new();
    let output = render_metrics(&store);
    assert!(output.is_empty());
}

#[test]
fn test_render_metrics_gauge() {
    let store = MetricStore::new();
    let mut labels = BTreeMap::new();
    labels.insert("phase".to_string(), "a".to_string());

    let m = make_metric(
        "voltage",
        230.5,
        MetricType::Gauge,
        "volts",
        "Phase voltage",
        labels,
    );
    store.publish(
        "test-collector",
        vec![m],
        &BTreeMap::new(),
        &BTreeMap::new(),
    );

    let output = render_metrics(&store);
    assert!(output.contains("# HELP modbus_voltage_volts Phase voltage"));
    assert!(output.contains("# TYPE modbus_voltage_volts gauge"));
    assert!(output.contains("modbus_voltage_volts{"));
    assert!(output.contains("230.5"));
}

#[test]
fn test_render_metrics_counter() {
    let store = MetricStore::new();
    let m = make_metric(
        "requests",
        42.0,
        MetricType::Counter,
        "",
        "Total requests",
        BTreeMap::new(),
    );
    store.publish("c1", vec![m], &BTreeMap::new(), &BTreeMap::new());

    let output = render_metrics(&store);
    assert!(output.contains("# TYPE modbus_requests counter"));
}

#[test]
fn test_label_escaping() {
    let mut labels = BTreeMap::new();
    labels.insert(
        "desc".to_string(),
        "has \"quotes\" and \nnewline".to_string(),
    );

    let line = format_metric_line("test_metric", &labels, 1.0);
    assert!(line.contains("\\\"quotes\\\""));
    assert!(line.contains("\\n"));
}

#[tokio::test]
async fn test_serve_disabled() {
    let config = crate::config::PrometheusExporter {
        enabled: false,
        listen: "127.0.0.1:0".to_string(),
        path: "/metrics".to_string(),
    };
    let store = MetricStore::new();
    let cancel = tokio_util::sync::CancellationToken::new();
    let result = serve(&config, store, cancel).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_http_endpoint() {
    let store = MetricStore::new();
    let m = make_metric(
        "temp",
        22.5,
        MetricType::Gauge,
        "celsius",
        "Temperature",
        BTreeMap::new(),
    );
    store.publish("sensor", vec![m], &BTreeMap::new(), &BTreeMap::new());

    let config = crate::config::PrometheusExporter {
        enabled: true,
        listen: "127.0.0.1:0".to_string(),
        path: "/metrics".to_string(),
    };

    let state = std::sync::Arc::new(PrometheusState { store });
    let app = axum::Router::new()
        .route("/metrics", axum::routing::get(metrics_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let resp = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.contains("text/plain"));
    let body = resp.text().await.unwrap();
    assert!(body.contains("modbus_temp_celsius"));
    assert!(body.contains("22.5"));
}
