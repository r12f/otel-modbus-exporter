//! Internal exporter metrics for self-observability.
//!
//! All counters use `AtomicU64` / `AtomicU64`-backed f64 for lock-free updates.
//! The struct is wrapped in `Arc` and shared across all tasks.

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Relaxed ordering — sufficient for monotonic counters where we only need
/// eventual visibility, not cross-field consistency.
const ORD: Ordering = Ordering::Relaxed;

/// Per-collector statistics.
pub struct CollectorStats {
    pub polls_total: AtomicU64,
    pub polls_success: AtomicU64,
    pub polls_error: AtomicU64,
    pub modbus_requests: AtomicU64,
    pub modbus_errors: AtomicU64,
    /// Last poll duration stored as f64 bits via `AtomicU64`.
    pub last_poll_duration_secs: AtomicU64,
}

impl CollectorStats {
    pub fn new() -> Self {
        Self {
            polls_total: AtomicU64::new(0),
            polls_success: AtomicU64::new(0),
            polls_error: AtomicU64::new(0),
            modbus_requests: AtomicU64::new(0),
            modbus_errors: AtomicU64::new(0),
            last_poll_duration_secs: AtomicU64::new(0f64.to_bits()),
        }
    }

    pub fn set_poll_duration(&self, secs: f64) {
        self.last_poll_duration_secs.store(secs.to_bits(), ORD);
    }

    pub fn get_poll_duration(&self) -> f64 {
        f64::from_bits(self.last_poll_duration_secs.load(ORD))
    }
}

/// Global internal metrics for the exporter process.
impl std::fmt::Debug for InternalMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InternalMetrics").finish_non_exhaustive()
    }
}

pub struct InternalMetrics {
    pub start_time: Instant,
    pub collectors_total: AtomicU64,
    pub collector_stats: DashMap<String, CollectorStats>,
    pub otlp_exports_total: AtomicU64,
    pub otlp_errors_total: AtomicU64,
    pub prometheus_scrapes_total: AtomicU64,
}

impl InternalMetrics {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            collectors_total: AtomicU64::new(0),
            collector_stats: DashMap::new(),
            otlp_exports_total: AtomicU64::new(0),
            otlp_errors_total: AtomicU64::new(0),
            prometheus_scrapes_total: AtomicU64::new(0),
        }
    }

    /// Get or create stats for a collector.
    pub fn get_or_create_collector(&self, name: &str) -> dashmap::mapref::one::Ref<'_, String, CollectorStats> {
        self.collector_stats.entry(name.to_string()).or_insert_with(CollectorStats::new);
        self.collector_stats.get(name).unwrap()
    }

    /// Uptime in seconds.
    pub fn uptime_seconds(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Render internal metrics in Prometheus exposition format.
    pub fn render_prometheus(&self) -> String {
        let mut buf = String::new();

        // collectors_total
        buf.push_str("# HELP modbus_exporter_collectors_total Total number of configured collectors\n");
        buf.push_str("# TYPE modbus_exporter_collectors_total gauge\n");
        buf.push_str(&format!("modbus_exporter_collectors_total {}\n", self.collectors_total.load(ORD)));

        // uptime_seconds
        buf.push_str("# HELP modbus_exporter_uptime_seconds Seconds since exporter started\n");
        buf.push_str("# TYPE modbus_exporter_uptime_seconds gauge\n");
        buf.push_str(&format!("modbus_exporter_uptime_seconds {:.1}\n", self.uptime_seconds()));

        // Per-collector metrics
        let collectors: Vec<_> = {
            let mut v: Vec<_> = self.collector_stats.iter().map(|e| e.key().clone()).collect();
            v.sort();
            v
        };

        if !collectors.is_empty() {
            buf.push_str("# HELP modbus_exporter_polls_total Total poll cycles per collector\n");
            buf.push_str("# TYPE modbus_exporter_polls_total counter\n");
            for c in &collectors {
                if let Some(s) = self.collector_stats.get(c) {
                    buf.push_str(&format!("modbus_exporter_polls_total{{collector=\"{c}\"}} {}\n", s.polls_total.load(ORD)));
                }
            }

            buf.push_str("# HELP modbus_exporter_polls_success_total Successful poll cycles per collector\n");
            buf.push_str("# TYPE modbus_exporter_polls_success_total counter\n");
            for c in &collectors {
                if let Some(s) = self.collector_stats.get(c) {
                    buf.push_str(&format!("modbus_exporter_polls_success_total{{collector=\"{c}\"}} {}\n", s.polls_success.load(ORD)));
                }
            }

            buf.push_str("# HELP modbus_exporter_polls_error_total Poll cycles with errors per collector\n");
            buf.push_str("# TYPE modbus_exporter_polls_error_total counter\n");
            for c in &collectors {
                if let Some(s) = self.collector_stats.get(c) {
                    buf.push_str(&format!("modbus_exporter_polls_error_total{{collector=\"{c}\"}} {}\n", s.polls_error.load(ORD)));
                }
            }

            buf.push_str("# HELP modbus_exporter_modbus_requests_total Total Modbus register read requests per collector\n");
            buf.push_str("# TYPE modbus_exporter_modbus_requests_total counter\n");
            for c in &collectors {
                if let Some(s) = self.collector_stats.get(c) {
                    buf.push_str(&format!("modbus_exporter_modbus_requests_total{{collector=\"{c}\"}} {}\n", s.modbus_requests.load(ORD)));
                }
            }

            buf.push_str("# HELP modbus_exporter_modbus_errors_total Failed Modbus register read requests per collector\n");
            buf.push_str("# TYPE modbus_exporter_modbus_errors_total counter\n");
            for c in &collectors {
                if let Some(s) = self.collector_stats.get(c) {
                    buf.push_str(&format!("modbus_exporter_modbus_errors_total{{collector=\"{c}\"}} {}\n", s.modbus_errors.load(ORD)));
                }
            }

            buf.push_str("# HELP modbus_exporter_poll_duration_seconds Duration of the last poll cycle in seconds\n");
            buf.push_str("# TYPE modbus_exporter_poll_duration_seconds gauge\n");
            for c in &collectors {
                if let Some(s) = self.collector_stats.get(c) {
                    buf.push_str(&format!("modbus_exporter_poll_duration_seconds{{collector=\"{c}\"}} {:.6}\n", s.get_poll_duration()));
                }
            }
        }

        // Export metrics
        buf.push_str("# HELP modbus_exporter_otlp_exports_total Total OTLP export attempts\n");
        buf.push_str("# TYPE modbus_exporter_otlp_exports_total counter\n");
        buf.push_str(&format!("modbus_exporter_otlp_exports_total {}\n", self.otlp_exports_total.load(ORD)));

        buf.push_str("# HELP modbus_exporter_otlp_errors_total Failed OTLP exports\n");
        buf.push_str("# TYPE modbus_exporter_otlp_errors_total counter\n");
        buf.push_str(&format!("modbus_exporter_otlp_errors_total {}\n", self.otlp_errors_total.load(ORD)));

        buf.push_str("# HELP modbus_exporter_prometheus_scrapes_total Total Prometheus scrape requests\n");
        buf.push_str("# TYPE modbus_exporter_prometheus_scrapes_total counter\n");
        buf.push_str(&format!("modbus_exporter_prometheus_scrapes_total {}\n", self.prometheus_scrapes_total.load(ORD)));

        buf
    }

    /// Build OTLP metric values for the internal scope.
    pub fn to_metric_values(&self) -> Vec<crate::metrics::MetricValue> {
        use crate::metrics::{MetricType, MetricValue};
        use std::collections::BTreeMap;
        use std::time::SystemTime;

        let now = SystemTime::now();
        let mut out = Vec::new();

        out.push(MetricValue {
            name: "modbus_exporter_collectors_total".into(),
            value: self.collectors_total.load(ORD) as f64,
            metric_type: MetricType::Gauge,
            labels: BTreeMap::new(),
            description: "Total number of configured collectors".into(),
            unit: String::new(),
            updated_at: now,
        });

        out.push(MetricValue {
            name: "modbus_exporter_uptime_seconds".into(),
            value: self.uptime_seconds(),
            metric_type: MetricType::Gauge,
            labels: BTreeMap::new(),
            description: "Seconds since exporter started".into(),
            unit: String::new(),
            updated_at: now,
        });

        for entry in self.collector_stats.iter() {
            let c = entry.key();
            let s = entry.value();
            let mut labels = BTreeMap::new();
            labels.insert("collector".to_string(), c.clone());

            let counter_metrics = [
                ("modbus_exporter_polls_total", s.polls_total.load(ORD), "Total poll cycles per collector"),
                ("modbus_exporter_polls_success_total", s.polls_success.load(ORD), "Successful poll cycles per collector"),
                ("modbus_exporter_polls_error_total", s.polls_error.load(ORD), "Poll cycles with errors per collector"),
                ("modbus_exporter_modbus_requests_total", s.modbus_requests.load(ORD), "Total Modbus register read requests per collector"),
                ("modbus_exporter_modbus_errors_total", s.modbus_errors.load(ORD), "Failed Modbus register read requests per collector"),
            ];

            for (name, val, desc) in counter_metrics {
                out.push(MetricValue {
                    name: name.into(),
                    value: val as f64,
                    metric_type: MetricType::Counter,
                    labels: labels.clone(),
                    description: desc.into(),
                    unit: String::new(),
                    updated_at: now,
                });
            }

            out.push(MetricValue {
                name: "modbus_exporter_poll_duration_seconds".into(),
                value: s.get_poll_duration(),
                metric_type: MetricType::Gauge,
                labels: labels.clone(),
                description: "Duration of the last poll cycle in seconds".into(),
                unit: String::new(),
                updated_at: now,
            });
        }

        out.push(MetricValue {
            name: "modbus_exporter_otlp_exports_total".into(),
            value: self.otlp_exports_total.load(ORD) as f64,
            metric_type: MetricType::Counter,
            labels: BTreeMap::new(),
            description: "Total OTLP export attempts".into(),
            unit: String::new(),
            updated_at: now,
        });

        out.push(MetricValue {
            name: "modbus_exporter_otlp_errors_total".into(),
            value: self.otlp_errors_total.load(ORD) as f64,
            metric_type: MetricType::Counter,
            labels: BTreeMap::new(),
            description: "Failed OTLP exports".into(),
            unit: String::new(),
            updated_at: now,
        });

        out.push(MetricValue {
            name: "modbus_exporter_prometheus_scrapes_total".into(),
            value: self.prometheus_scrapes_total.load(ORD) as f64,
            metric_type: MetricType::Counter,
            labels: BTreeMap::new(),
            description: "Total Prometheus scrape requests".into(),
            unit: String::new(),
            updated_at: now,
        });

        out
    }
}

#[cfg(test)]
#[path = "internal_metrics_tests.rs"]
mod tests;
