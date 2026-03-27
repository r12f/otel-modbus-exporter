use dashmap::DashMap;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::SystemTime;

/// Type of metric — determines exporter semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricType {
    Gauge,
    Counter,
}

/// A single metric value with metadata.
#[derive(Debug, Clone)]
pub struct MetricValue {
    /// Metric name — required by OTLP/Prometheus exporters.
    pub name: String,
    pub value: f64,
    pub metric_type: MetricType,
    pub labels: BTreeMap<String, String>,
    pub description: String,
    /// Unit kept as a dedicated field; exporters handle unit semantics.
    pub unit: String,
    /// Wall-clock timestamp for OTLP/Prometheus export.
    pub updated_at: SystemTime,
}

/// Thread-safe store aggregating per-collector metric caches.
///
/// Collectors call [`publish`] to atomically replace their cache snapshot.
/// Exporters call [`all_metrics`] for a read-only snapshot — they never
/// trigger Modbus calls.
#[derive(Debug, Clone)]
pub struct MetricStore {
    inner: Arc<DashMap<String, Arc<Vec<MetricValue>>>>,
}

impl MetricStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    /// Atomically replace the cache for `collector_name`.
    ///
    /// `global_labels` and `collector_labels` are merged into each metric
    /// following the precedence order: global → collector → metric-level.
    ///
    /// Per-metric dedup: if multiple metrics share the same name, only the
    /// last one wins (HashMap keyed by name).
    pub fn publish(
        &self,
        collector_name: &str,
        metrics: Vec<MetricValue>,
        global_labels: &BTreeMap<String, String>,
        collector_labels: &BTreeMap<String, String>,
    ) {
        // R3-1: compute base labels once, not per metric
        let mut base_labels = global_labels.clone();
        for (k, v) in collector_labels {
            base_labels.insert(k.clone(), v.clone());
        }
        base_labels.insert("collector".to_string(), collector_name.to_string());

        // R1-2: dedup by metric name using HashMap
        let mut deduped: HashMap<String, MetricValue> = HashMap::new();
        for mut m in metrics {
            let mut final_labels = base_labels.clone();
            for (k, v) in &m.labels {
                final_labels.insert(k.clone(), v.clone());
            }
            // R2-2: do NOT inject unit into labels
            m.labels = final_labels;
            deduped.insert(m.name.clone(), m);
        }

        let merged: Vec<MetricValue> = deduped.into_values().collect();
        self.inner
            .insert(collector_name.to_string(), Arc::new(merged));
    }

    /// Return a flat snapshot of all metrics across all collectors.
    ///
    /// Returns `Arc<Vec<MetricValue>>` per collector to avoid deep cloning
    /// on every scrape. Callers that need a flat `Vec` can collect.
    pub fn all_metrics(&self) -> Vec<Arc<Vec<MetricValue>>> {
        self.inner.iter().map(|e| Arc::clone(e.value())).collect()
    }

    /// Convenience: flat snapshot (allocates a new Vec but shares Arc'd inner vecs).
    pub fn all_metrics_flat(&self) -> Vec<MetricValue> {
        let mut out = Vec::new();
        for entry in self.inner.iter() {
            out.extend(entry.value().as_ref().clone());
        }
        out
    }

    /// Return metrics for a single collector.
    pub fn metrics_for(&self, collector_name: &str) -> Vec<MetricValue> {
        self.inner
            .get(collector_name)
            .map(|e| e.value().as_ref().clone())
            .unwrap_or_default()
    }

    /// Number of collectors currently in the store.
    pub fn collector_count(&self) -> usize {
        self.inner.len()
    }

    /// Remove a collector's metrics (R3-2: staleness eviction).
    pub fn remove_collector(&self, collector_name: &str) -> bool {
        self.inner.remove(collector_name).is_some()
    }
}

impl Default for MetricStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
