use super::*;
use std::sync::atomic::Ordering::Relaxed;

#[test]
fn test_internal_metrics_new() {
    let m = InternalMetrics::new();
    assert_eq!(m.collectors_total.load(Relaxed), 0);
    assert_eq!(m.otlp_exports_total.load(Relaxed), 0);
    assert_eq!(m.otlp_errors_total.load(Relaxed), 0);
    assert_eq!(m.prometheus_scrapes_total.load(Relaxed), 0);
    assert!(m.collector_stats.is_empty());
}

#[test]
fn test_collector_stats_increments() {
    let stats = CollectorStats::new();
    stats.polls_total.fetch_add(1, Relaxed);
    stats.polls_total.fetch_add(1, Relaxed);
    stats.polls_success.fetch_add(1, Relaxed);
    stats.polls_error.fetch_add(1, Relaxed);
    stats.read_requests.fetch_add(5, Relaxed);
    stats.read_errors.fetch_add(2, Relaxed);

    assert_eq!(stats.polls_total.load(Relaxed), 2);
    assert_eq!(stats.polls_success.load(Relaxed), 1);
    assert_eq!(stats.polls_error.load(Relaxed), 1);
    assert_eq!(stats.read_requests.load(Relaxed), 5);
    assert_eq!(stats.read_errors.load(Relaxed), 2);
}

#[test]
fn test_poll_duration_f64() {
    let stats = CollectorStats::new();
    assert_eq!(stats.get_poll_duration(), 0.0);
    stats.set_poll_duration(1.234);
    assert!((stats.get_poll_duration() - 1.234).abs() < f64::EPSILON);
}

#[test]
fn test_get_or_create_collector() {
    let m = InternalMetrics::new();
    {
        let s = m.get_or_create_collector("test_collector");
        s.polls_total.fetch_add(1, Relaxed);
    }
    {
        let s = m.get_or_create_collector("test_collector");
        assert_eq!(s.polls_total.load(Relaxed), 1);
    }
    assert_eq!(m.collector_stats.len(), 1);
}

#[test]
fn test_uptime_is_positive() {
    let m = InternalMetrics::new();
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(m.uptime_seconds() > 0.0);
}

#[test]
fn test_render_prometheus_contains_expected_metrics() {
    let m = InternalMetrics::new();
    m.collectors_total.store(3, Relaxed);
    m.otlp_exports_total.store(10, Relaxed);
    m.otlp_errors_total.store(1, Relaxed);
    m.prometheus_scrapes_total.store(5, Relaxed);

    // Add a collector
    m.collector_stats
        .insert("meter_1".to_string(), CollectorStats::new());
    if let Some(s) = m.collector_stats.get("meter_1") {
        s.polls_total.store(42, Relaxed);
        s.polls_success.store(40, Relaxed);
        s.polls_error.store(2, Relaxed);
        s.read_requests.store(100, Relaxed);
        s.read_errors.store(3, Relaxed);
        s.set_poll_duration(0.5);
    }

    let output = m.render_prometheus();

    assert!(output.contains("bus_exporter_collectors_total 3"));
    assert!(output.contains("bus_exporter_uptime_seconds"));
    assert!(output.contains("bus_exporter_polls_total{collector=\"meter_1\"} 42"));
    assert!(output.contains("bus_exporter_polls_success_total{collector=\"meter_1\"} 40"));
    assert!(output.contains("bus_exporter_polls_error_total{collector=\"meter_1\"} 2"));
    assert!(output.contains("bus_exporter_modbus_requests_total{collector=\"meter_1\"} 100"));
    assert!(output.contains("bus_exporter_modbus_errors_total{collector=\"meter_1\"} 3"));
    assert!(output.contains("bus_exporter_poll_duration_seconds{collector=\"meter_1\"} 0.5"));
    assert!(output.contains("bus_exporter_otlp_exports_total 10"));
    assert!(output.contains("bus_exporter_otlp_errors_total 1"));
    assert!(output.contains("bus_exporter_prometheus_scrapes_total 5"));

    // Check TYPE annotations
    assert!(output.contains("# TYPE bus_exporter_collectors_total gauge"));
    assert!(output.contains("# TYPE bus_exporter_polls_total counter"));
    assert!(output.contains("# TYPE bus_exporter_uptime_seconds gauge"));
}

#[test]
fn test_to_metric_values() {
    let m = InternalMetrics::new();
    m.collectors_total.store(2, Relaxed);
    m.collector_stats
        .insert("c1".to_string(), CollectorStats::new());

    let values = m.to_metric_values();
    let names: Vec<&str> = values.iter().map(|v| v.name.as_str()).collect();

    assert!(names.contains(&"bus_exporter_collectors_total"));
    assert!(names.contains(&"bus_exporter_uptime_seconds"));
    assert!(names.contains(&"bus_exporter_polls_total"));
    assert!(names.contains(&"bus_exporter_otlp_exports_total"));
    assert!(names.contains(&"bus_exporter_prometheus_scrapes_total"));
}
