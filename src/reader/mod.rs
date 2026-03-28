pub mod i2c;
pub mod i3c;
pub mod modbus;
pub mod spi;

use anyhow::Result;
use async_trait::async_trait;

use crate::config::MetricConfig;

/// Describes optional capabilities of a [`MetricReader`] implementation.
#[derive(Debug, Clone, Copy)]
pub struct ReaderCapabilities {
    /// Whether the reader natively supports reading multiple metrics in one call.
    pub batch_read: bool,
}

/// Result of a [`MetricReader::batch_read`] call, including both per-metric
/// results and the number of actual I/O requests performed.
pub struct BatchReadResult<'a> {
    /// Per-metric read results.
    pub results: Vec<(&'a MetricConfig, Result<f64>)>,
    /// Number of actual I/O requests made (e.g., coalesced Modbus reads).
    /// For the default one-read-per-metric implementation this equals `results.len()`.
    pub read_count: usize,
}

/// Unified interface for reading metrics from any bus protocol.
///
/// This trait requires `Send` but intentionally does **not** require `Sync`.
/// The underlying transport (e.g. `tokio_modbus::client::Context`) is `!Sync`,
/// so each reader is owned by a single task (`run_collector`) and accessed via
/// `&mut self` — no shared-reference concurrency is needed.
#[async_trait]
pub trait MetricReader: Send {
    // ── Connection ──────────────────────────────────────────────────

    /// Establish the underlying connection/transport.
    async fn connect(&mut self) -> Result<()>;

    /// Close the underlying connection/transport.
    async fn disconnect(&mut self) -> Result<()>;

    /// Returns `true` when connected.
    fn is_connected(&self) -> bool;

    // ── Capabilities ────────────────────────────────────────────────

    /// Returns the capabilities of this reader.
    fn capabilities(&self) -> ReaderCapabilities;

    // ── Read ────────────────────────────────────────────────────────

    /// Read a single metric, returning its numeric value.
    async fn read(&mut self, metric: &MetricConfig) -> Result<f64>;

    /// Read multiple metrics in one call, returning per-metric results and the
    /// number of actual I/O requests performed.
    ///
    /// The default implementation iterates over `metrics` and calls [`read`](Self::read)
    /// for each one. Implementations that support native batch reads (e.g. Modbus
    /// register coalescing) can override this to return a lower `read_count`.
    async fn batch_read<'a>(&mut self, metrics: &'a [MetricConfig]) -> BatchReadResult<'a> {
        let mut results = Vec::with_capacity(metrics.len());
        for metric in metrics {
            let result = self.read(metric).await;
            results.push((metric, result));
        }
        let read_count = results.len();
        BatchReadResult {
            results,
            read_count,
        }
    }
}
