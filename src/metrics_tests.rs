use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

fn make_metric(name: &str, value: f64, metric_type: MetricType) -> MetricValue {
    MetricValue {
        name: name.to_string(),
        value,
        metric_type,
        labels: BTreeMap::new(),
        description: name.to_string(),
        unit: String::new(),
        updated_at: SystemTime::now(),
    }
}

#[test]
fn test_publish_and_read() {
    let store = MetricStore::new();
    let global = BTreeMap::new();
    let coll_labels = BTreeMap::new();

    store.publish(
        "collector1",
        vec![make_metric("temp", 22.5, MetricType::Gauge)],
        &global,
        &coll_labels,
    );

    let all = store.all_metrics_flat();
    assert_eq!(all.len(), 1);
    assert!((all[0].value - 22.5).abs() < f64::EPSILON);
    assert_eq!(all[0].labels.get("collector").unwrap(), "collector1");
    assert_eq!(all[0].name, "temp");
}

#[test]
fn test_cache_overwrite() {
    let store = MetricStore::new();
    let g = BTreeMap::new();
    let c = BTreeMap::new();

    store.publish("c1", vec![make_metric("a", 1.0, MetricType::Gauge)], &g, &c);
    store.publish("c1", vec![make_metric("b", 2.0, MetricType::Gauge)], &g, &c);

    let all = store.all_metrics_flat();
    assert_eq!(all.len(), 1);
    assert!((all[0].value - 2.0).abs() < f64::EPSILON);
}

#[test]
fn test_label_merging_precedence() {
    let store = MetricStore::new();

    let mut global = BTreeMap::new();
    global.insert("env".to_string(), "prod".to_string());
    global.insert("region".to_string(), "us-east".to_string());

    let mut coll_labels = BTreeMap::new();
    coll_labels.insert("region".to_string(), "eu-west".to_string());

    let mut metric = make_metric("voltage", 230.0, MetricType::Gauge);
    metric
        .labels
        .insert("region".to_string(), "ap-south".to_string());
    metric.unit = "V".to_string();

    store.publish("plc1", vec![metric], &global, &coll_labels);

    let all = store.all_metrics_flat();
    assert_eq!(all.len(), 1);
    let labels = &all[0].labels;
    assert_eq!(labels.get("region").unwrap(), "ap-south");
    assert_eq!(labels.get("env").unwrap(), "prod");
    assert_eq!(labels.get("collector").unwrap(), "plc1");
    // R2-2: unit must NOT be in labels
    assert!(!labels.contains_key("unit"));
    // unit is on the struct field instead
    assert_eq!(all[0].unit, "V");
}

#[test]
fn test_gauge_vs_counter() {
    let store = MetricStore::new();
    let g = BTreeMap::new();
    let c = BTreeMap::new();

    store.publish(
        "c1",
        vec![
            make_metric("temperature", 25.0, MetricType::Gauge),
            make_metric("total_energy", 1000.0, MetricType::Counter),
        ],
        &g,
        &c,
    );

    let all = store.all_metrics_flat();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_multiple_collectors() {
    let store = MetricStore::new();
    let g = BTreeMap::new();
    let c = BTreeMap::new();

    store.publish("c1", vec![make_metric("a", 1.0, MetricType::Gauge)], &g, &c);
    store.publish("c2", vec![make_metric("b", 2.0, MetricType::Gauge)], &g, &c);

    assert_eq!(store.collector_count(), 2);
    assert_eq!(store.all_metrics_flat().len(), 2);
    assert_eq!(store.metrics_for("c1").len(), 1);
    assert_eq!(store.metrics_for("c2").len(), 1);
    assert!(store.metrics_for("c3").is_empty());
}

#[test]
fn test_concurrent_reads() {
    let store = MetricStore::new();
    let g = BTreeMap::new();
    let c = BTreeMap::new();
    store.publish(
        "c1",
        vec![make_metric("x", 42.0, MetricType::Gauge)],
        &g,
        &c,
    );

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let s = store.clone();
            std::thread::spawn(move || {
                for _ in 0..100 {
                    let metrics = s.all_metrics_flat();
                    assert!(!metrics.is_empty());
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn test_unit_not_in_labels() {
    let store = MetricStore::new();
    let g = BTreeMap::new();
    let c = BTreeMap::new();

    // With empty unit
    store.publish(
        "c1",
        vec![make_metric("temp", 1.0, MetricType::Gauge)],
        &g,
        &c,
    );
    let labels = &store.all_metrics_flat()[0].labels;
    assert!(!labels.contains_key("unit"));

    // With non-empty unit — still should NOT be in labels
    let mut m = make_metric("voltage", 230.0, MetricType::Gauge);
    m.unit = "V".to_string();
    store.publish("c2", vec![m], &g, &c);
    for metric in store.metrics_for("c2") {
        assert!(!metric.labels.contains_key("unit"));
        assert_eq!(metric.unit, "V");
    }
}

#[test]
fn test_dedup_by_name() {
    let store = MetricStore::new();
    let g = BTreeMap::new();
    let c = BTreeMap::new();

    // Publish two metrics with the same name — should dedup to one
    store.publish(
        "c1",
        vec![
            make_metric("temp", 1.0, MetricType::Gauge),
            make_metric("temp", 2.0, MetricType::Gauge),
        ],
        &g,
        &c,
    );

    let all = store.all_metrics_flat();
    assert_eq!(all.len(), 1);
    // Last one wins
    assert!((all[0].value - 2.0).abs() < f64::EPSILON);
}

#[test]
fn test_concurrent_read_write_contention() {
    let store = MetricStore::new();
    let g = BTreeMap::new();
    let c = BTreeMap::new();
    store.publish("c1", vec![make_metric("x", 0.0, MetricType::Gauge)], &g, &c);

    let writer_store = store.clone();
    let writer = std::thread::spawn(move || {
        let g = BTreeMap::new();
        let c = BTreeMap::new();
        for i in 0..200 {
            writer_store.publish(
                "c1",
                vec![make_metric("x", i as f64, MetricType::Gauge)],
                &g,
                &c,
            );
        }
    });

    let reader_handles: Vec<_> = (0..4)
        .map(|_| {
            let s = store.clone();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    // Should never panic even under contention
                    let _metrics = s.all_metrics_flat();
                    let _metrics2 = s.all_metrics();
                }
            })
        })
        .collect();

    writer.join().unwrap();
    for h in reader_handles {
        h.join().unwrap();
    }
}

#[test]
fn test_all_metrics_returns_arcs() {
    let store = MetricStore::new();
    let g = BTreeMap::new();
    let c = BTreeMap::new();
    store.publish("c1", vec![make_metric("a", 1.0, MetricType::Gauge)], &g, &c);

    let arcs = store.all_metrics();
    assert_eq!(arcs.len(), 1);
    assert_eq!(arcs[0].len(), 1);
    // Cloning the Arc is cheap — verify it's actually an Arc
    let arc2 = Arc::clone(&arcs[0]);
    assert_eq!(arc2.len(), 1);
}

#[test]
fn test_remove_collector() {
    let store = MetricStore::new();
    let g = BTreeMap::new();
    let c = BTreeMap::new();

    store.publish("c1", vec![make_metric("a", 1.0, MetricType::Gauge)], &g, &c);
    store.publish("c2", vec![make_metric("b", 2.0, MetricType::Gauge)], &g, &c);
    assert_eq!(store.collector_count(), 2);

    assert!(store.remove_collector("c1"));
    assert_eq!(store.collector_count(), 1);
    assert!(store.metrics_for("c1").is_empty());

    // Removing non-existent returns false
    assert!(!store.remove_collector("c1"));
}

#[test]
fn test_systemtime_timestamp() {
    let before = SystemTime::now();
    let m = make_metric("t", 1.0, MetricType::Gauge);
    let after = SystemTime::now();

    assert!(m.updated_at >= before);
    assert!(m.updated_at <= after);
}
