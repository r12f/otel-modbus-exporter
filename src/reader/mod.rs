pub mod i2c;
pub mod i3c;
pub mod modbus;
pub mod spi;

use anyhow::Result;
use async_trait::async_trait;

use crate::config::Metric as MetricConfig;

/// Describes optional capabilities of a [`MetricReader`] implementation.
#[derive(Debug, Clone, Copy)]
pub struct ReaderCapabilities {
    /// Whether the reader natively supports reading multiple metrics in one call.
    pub batch_read: bool,
}

/// Unified interface for reading metrics from any bus protocol.
#[async_trait]
pub trait MetricReader: Send {
    /// Read a single metric, returning its numeric value.
    async fn read(&mut self, metric: &MetricConfig) -> Result<f64>;

    /// Read multiple metrics in one call.
    ///
    /// The default implementation iterates over `metrics` and calls [`read`](Self::read)
    /// for each one. Implementations that support native batch reads can override this.
    async fn batch_read<'a>(
        &mut self,
        metrics: &'a [MetricConfig],
    ) -> Vec<(&'a MetricConfig, Result<f64>)> {
        let mut results = Vec::with_capacity(metrics.len());
        for metric in metrics {
            let result = self.read(metric).await;
            results.push((metric, result));
        }
        results
    }

    /// Establish the underlying connection/transport.
    async fn connect(&mut self) -> Result<()>;

    /// Close the underlying connection/transport.
    async fn disconnect(&mut self) -> Result<()>;

    /// Returns `true` when connected.
    fn is_connected(&self) -> bool;

    /// Returns the capabilities of this reader.
    fn capabilities(&self) -> ReaderCapabilities;
}
