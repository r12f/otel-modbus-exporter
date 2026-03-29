use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use super::decoder;
use crate::config;

/// Type alias for per-device mutex (different chip-selects are independent).
pub type DeviceLock = Arc<tokio::sync::Mutex<()>>;

/// Trait abstracting SPI device operations for testability.
pub trait SpiDevice: Send {
    /// Perform a full-duplex SPI transfer: send tx_buf, return rx_buf of same length.
    fn transfer(&mut self, tx_buf: &[u8]) -> Result<Vec<u8>>;
}

/// Real SPI device using Linux spidev.
#[cfg(target_os = "linux")]
pub mod linux_device {
    use super::*;

    pub struct LinuxSpiDevice {
        device_path: String,
        speed_hz: u32,
        mode: u8,
        bits_per_word: u8,
        inner: Option<spidev::Spidev>,
    }

    impl LinuxSpiDevice {
        pub fn new(device_path: String, speed_hz: u32, mode: u8, bits_per_word: u8) -> Self {
            Self {
                device_path,
                speed_hz,
                mode,
                bits_per_word,
                inner: None,
            }
        }

        pub fn open(&mut self) -> Result<()> {
            use spidev::{SpiModeFlags, Spidev, SpidevOptions};

            let mut spi = Spidev::open(&self.device_path)
                .with_context(|| format!("opening SPI device {}", self.device_path))?;

            let mode_flags = match self.mode {
                0 => SpiModeFlags::SPI_MODE_0,
                1 => SpiModeFlags::SPI_MODE_1,
                2 => SpiModeFlags::SPI_MODE_2,
                3 => SpiModeFlags::SPI_MODE_3,
                _ => anyhow::bail!("invalid SPI mode: {}", self.mode),
            };

            let options = SpidevOptions::new()
                .bits_per_word(self.bits_per_word)
                .max_speed_hz(self.speed_hz)
                .mode(mode_flags)
                .build();

            spi.configure(&options)
                .with_context(|| format!("configuring SPI device {}", self.device_path))?;

            self.inner = Some(spi);
            Ok(())
        }
    }

    impl SpiDevice for LinuxSpiDevice {
        fn transfer(&mut self, tx_buf: &[u8]) -> Result<Vec<u8>> {
            use spidev::SpidevTransfer;

            let spi = self
                .inner
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("SPI device not opened"))?;

            let mut rx_buf = vec![0u8; tx_buf.len()];
            let mut transfer = SpidevTransfer::read_write(tx_buf, &mut rx_buf);
            spi.transfer(&mut transfer).context("SPI transfer failed")?;
            Ok(rx_buf)
        }
    }
}

/// Stub SPI device for non-Linux platforms.
/// Only compiled on non-Linux targets (Linux uses `LinuxSpiDevice`).
#[cfg(not(target_os = "linux"))]
pub struct StubSpiDevice;

#[cfg(not(target_os = "linux"))]
impl SpiDevice for StubSpiDevice {
    fn transfer(&mut self, _tx_buf: &[u8]) -> Result<Vec<u8>> {
        anyhow::bail!("StubSpiDevice: no real SPI hardware available")
    }
}

/// SPI metric reader that wraps a device for async read operations.
pub struct SpiMetricReader {
    device: Arc<std::sync::Mutex<Box<dyn SpiDevice>>>,
    device_path: String,
    connected: bool,
    device_lock: DeviceLock,
    metrics: Vec<config::MetricConfig>,
}

/// Per-device mutex map for serializing access to same chip-select.
static DEVICE_LOCKS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, DeviceLock>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Get or create a per-device lock.
pub fn get_device_lock(device_path: &str) -> DeviceLock {
    let mut map = DEVICE_LOCKS.lock().unwrap();
    map.entry(device_path.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

impl SpiMetricReader {
    pub fn new(device: Box<dyn SpiDevice>, device_path: String, device_lock: DeviceLock) -> Self {
        Self {
            device: Arc::new(std::sync::Mutex::new(device)),
            device_path,
            connected: false,
            device_lock,
            metrics: Vec::new(),
        }
    }

    /// Perform a synchronous SPI transfer.
    pub fn transfer_sync(&self, tx_buf: &[u8]) -> Result<Vec<u8>> {
        let mut dev = self
            .device
            .lock()
            .map_err(|e| anyhow::anyhow!("device lock poisoned: {e}"))?;
        dev.transfer(tx_buf)
    }
}

/// Read a single SPI metric.
pub async fn read_spi_metric(
    client: &SpiMetricReader,
    metric: &config::MetricConfig,
    device_lock: &DeviceLock,
) -> Result<f64> {
    let data_type = decoder::map_data_type(metric.data_type);
    let byte_order = decoder::map_byte_order(metric.byte_order);

    // All validation (mid-endian, empty command, response bounds) already done by config.

    let response_length = metric
        .response_length
        .unwrap_or(metric.command.len() as u16) as usize;
    let response_offset = metric.response_offset as usize;
    let num_bytes = decoder::byte_count(data_type);

    // Build TX buffer: command bytes, zero-padded to response_length
    let mut tx_buf = metric.command.clone();
    if tx_buf.len() < response_length {
        tx_buf.resize(response_length, 0);
    }

    let scale = metric.scale;
    let offset = metric.offset;

    let device = Arc::clone(&client.device);
    let device_lock = device_lock.clone();
    let device_path = client.device_path.clone();

    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let _lock = device_lock.blocking_lock();
        let mut dev = device
            .lock()
            .map_err(|e| anyhow::anyhow!("device lock poisoned: {e}"))?;
        dev.transfer(&tx_buf)
            .with_context(|| format!("SPI transfer on {}", device_path))
    })
    .await
    .context("spawn_blocking join error")??;

    // Extract payload from response at offset
    let payload = &bytes[response_offset..response_offset + num_bytes];

    decoder::decode_bytes(payload, data_type, byte_order, scale, offset)
        .map(|(_raw, scaled)| scaled)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Unified MetricReader implementation for SPI.
#[async_trait]
impl crate::reader::MetricReader for SpiMetricReader {
    fn set_metrics(&mut self, metrics: Vec<config::MetricConfig>) {
        crate::reader::warn_duplicate_metric_names(&metrics);
        self.metrics = metrics;
    }

    async fn connect(&mut self) -> Result<()> {
        self.connected = true;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn read(
        &mut self,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> crate::reader::ReadResults {
        let mut results = HashMap::new();
        let device_lock = Arc::clone(&self.device_lock);
        for i in 0..self.metrics.len() {
            if cancel.is_cancelled() {
                break;
            }
            let metric = &self.metrics[i];
            let result = read_spi_metric(self, metric, &device_lock).await;
            results.insert(metric.name.clone(), result);
        }
        let io_count = results.len();
        crate::reader::ReadResults {
            metrics: results,
            io_count,
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
