pub mod i2c;
pub mod i3c;
pub mod modbus;
pub mod spi;

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::config::MetricConfig;

/// Check for duplicate metric names and warn about them.
pub fn warn_duplicate_metric_names(metrics: &[MetricConfig]) {
    let mut seen = std::collections::HashSet::new();
    for m in metrics {
        if !seen.insert(&m.name) {
            tracing::warn!(name = %m.name, "duplicate metric name in set_metrics — last occurrence wins");
        }
    }
}

/// Result of a [`MetricReader::read`] call, including I/O request count.
pub struct ReadResults {
    /// Per-metric results keyed by metric name.
    pub metrics: HashMap<String, Result<f64>>,
    /// Number of actual I/O operations performed (e.g. coalesced Modbus reads).
    /// Non-Modbus readers return `metrics.len()` (one I/O per metric).
    pub io_count: usize,
}

/// Unified interface for reading metrics from any bus protocol.
///
/// Each reader is configured with a set of metrics via [`set_metrics`](Self::set_metrics),
/// then [`read`](Self::read) returns all configured metric values in one call.
///
/// Metric names must be unique within a reader; [`set_metrics`] implementations
/// should validate this and warn on duplicates.
///
/// This trait requires `Send` but intentionally does **not** require `Sync`.
/// The underlying transport (e.g. `tokio_modbus::client::Context`) is `!Sync`,
/// so each reader is owned by a single task (`run_collector`) and accessed via
/// `&mut self` — no shared-reference concurrency is needed.
#[async_trait]
pub trait MetricReader: Send {
    /// Configure which metrics this reader should collect.
    ///
    /// Warns if duplicate metric names are found; only the last occurrence is kept.
    fn set_metrics(&mut self, metrics: Vec<MetricConfig>);

    /// Establish the underlying connection/transport.
    async fn connect(&mut self) -> Result<()>;

    /// Close the underlying connection/transport.
    async fn disconnect(&mut self) -> Result<()>;

    /// Returns `true` when connected.
    fn is_connected(&self) -> bool;

    /// Read all configured metrics. Returns results and I/O count.
    ///
    /// The `cancel` token allows cooperative cancellation between individual
    /// metric reads for fast shutdown on slow buses.
    async fn read(&mut self, cancel: &CancellationToken) -> ReadResults;
}
