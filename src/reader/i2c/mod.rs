use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use super::decoder;
use crate::config;
use crate::config::WriteStep;

/// Type alias for the shared bus lock (std Mutex for use in spawn_blocking).
pub type BusLock = Arc<std::sync::Mutex<()>>;

/// Trait abstracting I2C device operations for testability.
pub trait I2cDevice: Send {
    /// Write bytes to the device, then read `read_len` bytes back.
    fn write_read(&mut self, write_buf: &[u8], read_len: usize) -> Result<Vec<u8>>;

    /// Write bytes to the device (no read).
    fn write(&mut self, write_buf: &[u8]) -> Result<()> {
        // Default: use write_read with 0 read length
        let _ = self.write_read(write_buf, 0)?;
        Ok(())
    }
}

/// Real I2C device using Linux /dev/i2c-N.
/// Only available on Linux targets.
#[cfg(target_os = "linux")]
pub mod linux_device {
    use super::*;
    use std::os::unix::io::AsRawFd;

    const I2C_SLAVE: libc::c_ulong = 0x0703;

    pub struct LinuxI2cDevice {
        bus_path: String,
        address: u8,
        fd: Option<std::fs::File>,
    }

    impl LinuxI2cDevice {
        pub fn new(bus_path: String, address: u8) -> Self {
            Self {
                bus_path,
                address,
                fd: None,
            }
        }

        pub fn open(&mut self) -> Result<()> {
            use std::fs::OpenOptions;

            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.bus_path)
                .with_context(|| format!("opening I2C bus {}", self.bus_path))?;

            let ret = unsafe {
                libc::ioctl(
                    file.as_raw_fd(),
                    I2C_SLAVE as libc::Ioctl,
                    self.address as libc::c_ulong,
                )
            };
            if ret < 0 {
                anyhow::bail!(
                    "ioctl I2C_SLAVE failed for address {:#04x} on {}",
                    self.address,
                    self.bus_path
                );
            }
            self.fd = Some(file);
            Ok(())
        }
    }

    impl I2cDevice for LinuxI2cDevice {
        fn write_read(&mut self, write_buf: &[u8], read_len: usize) -> Result<Vec<u8>> {
            use std::io::{Read, Write};
            let fd = self
                .fd
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("I2C device not opened"))?;
            fd.write_all(write_buf)
                .context("I2C write register address")?;
            let mut buf = vec![0u8; read_len];
            fd.read_exact(&mut buf).context("I2C read data")?;
            Ok(buf)
        }

        fn write(&mut self, write_buf: &[u8]) -> Result<()> {
            use std::io::Write;
            let fd = self
                .fd
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("I2C device not opened"))?;
            fd.write_all(write_buf).context("I2C write")?;
            Ok(())
        }
    }
}

/// Stub I2C device (placeholder for non-Linux platforms where no real hardware is available).
/// Only compiled on non-Linux targets (Linux uses `LinuxI2cDevice`).
#[cfg(not(target_os = "linux"))]
pub struct StubI2cDevice;

#[cfg(not(target_os = "linux"))]
impl I2cDevice for StubI2cDevice {
    fn write_read(&mut self, _write_buf: &[u8], _read_len: usize) -> Result<Vec<u8>> {
        anyhow::bail!("StubI2cDevice: no real I2C hardware available")
    }
}

/// I2C metric reader that wraps a device and provides async read operations.
pub struct I2cMetricReader {
    device: Arc<std::sync::Mutex<Box<dyn I2cDevice>>>,
    bus_path: String,
    address: u8,
    connected: bool,
    bus_lock: BusLock,
    metrics: Vec<config::MetricConfig>,
}

/// Per-bus mutex map for serializing access.
static BUS_LOCKS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, BusLock>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Get or create a per-bus lock.
pub fn get_bus_lock(bus_path: &str) -> BusLock {
    let mut map = BUS_LOCKS.lock().unwrap();
    map.entry(bus_path.to_string())
        .or_insert_with(|| Arc::new(std::sync::Mutex::new(())))
        .clone()
}

impl I2cMetricReader {
    pub fn new(
        device: Box<dyn I2cDevice>,
        bus_path: String,
        address: u8,
        bus_lock: BusLock,
    ) -> Self {
        Self {
            device: Arc::new(std::sync::Mutex::new(device)),
            bus_path,
            address,
            connected: false,
            bus_lock,
            metrics: Vec::new(),
        }
    }

    /// Get a clone of the shared device Arc (for creating a writer that shares the same device).
    pub fn shared_device(&self) -> Arc<std::sync::Mutex<Box<dyn I2cDevice>>> {
        Arc::clone(&self.device)
    }

    /// Get the bus path.
    pub fn bus_path(&self) -> &str {
        &self.bus_path
    }

    /// Get the bus lock.
    pub fn shared_bus_lock(&self) -> BusLock {
        Arc::clone(&self.bus_lock)
    }

    /// Read bytes from a register address on the I2C device.
    pub fn read_register_sync(&self, register: u8, byte_count: usize) -> Result<Vec<u8>> {
        let mut dev = self
            .device
            .lock()
            .map_err(|e| anyhow::anyhow!("device lock poisoned: {e}"))?;
        dev.write_read(&[register], byte_count)
    }
}

/// Read a single I2C metric.
pub async fn read_i2c_metric(
    client: &I2cMetricReader,
    metric: &config::MetricConfig,
    bus_lock: &BusLock,
) -> Result<(f64, f64)> {
    let data_type = decoder::map_data_type(metric.data_type);
    let byte_order = decoder::map_byte_order(metric.byte_order);

    // address validated as present and in u8 range by config
    let register = metric.address.unwrap() as u8;

    let num_bytes = decoder::byte_count(data_type);
    let scale = metric.scale;
    let offset = metric.offset;

    // Clone Arcs for move into spawn_blocking
    let device = Arc::clone(&client.device);
    let bus_lock = Arc::clone(bus_lock);
    let bus_path = client.bus_path.clone();
    let address = client.address;

    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let _lock = bus_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("bus lock poisoned: {e}"))?;
        let mut dev = device
            .lock()
            .map_err(|e| anyhow::anyhow!("device lock poisoned: {e}"))?;
        dev.write_read(&[register], num_bytes).with_context(|| {
            format!(
                "reading I2C register {:#04x} ({} bytes) from device {:#04x} on {}",
                register, num_bytes, address, bus_path
            )
        })
    })
    .await
    .context("spawn_blocking join error")??;

    decoder::decode_bytes(&bytes, data_type, byte_order, scale, offset)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Unified MetricReader implementation for I2C.
#[async_trait]
impl crate::reader::MetricReader for I2cMetricReader {
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
        let bus_lock = Arc::clone(&self.bus_lock);
        for i in 0..self.metrics.len() {
            if cancel.is_cancelled() {
                break;
            }
            let metric = &self.metrics[i];
            let result = read_i2c_metric(self, metric, &bus_lock).await;
            results.insert(metric.name.clone(), result);
        }
        let io_count = results.len();
        crate::reader::ReadResults {
            metrics: results,
            io_count,
        }
    }
}

/// I2C metric writer for executing write steps (init_writes / pre_poll).
pub struct I2cMetricWriter {
    device: Arc<std::sync::Mutex<Box<dyn I2cDevice>>>,
    bus_path: String,
    bus_lock: BusLock,
}

impl I2cMetricWriter {
    pub fn new(
        device: Arc<std::sync::Mutex<Box<dyn I2cDevice>>>,
        bus_path: String,
        bus_lock: BusLock,
    ) -> Self {
        Self {
            device,
            bus_path,
            bus_lock,
        }
    }
}

#[async_trait]
impl crate::reader::MetricWriter for I2cMetricWriter {
    async fn execute_writes(&mut self, steps: &[WriteStep]) -> Result<()> {
        for (idx, step) in steps.iter().enumerate() {
            if let (Some(address), Some(value)) = (step.address, &step.value) {
                let mut buf = vec![address];
                buf.extend_from_slice(&value.as_bytes());
                let device = Arc::clone(&self.device);
                let bus_lock = Arc::clone(&self.bus_lock);
                let bus_path = self.bus_path.clone();
                tokio::task::spawn_blocking(move || -> Result<()> {
                    let _lock = bus_lock
                        .lock()
                        .map_err(|e| anyhow::anyhow!("bus lock poisoned: {e}"))?;
                    let mut dev = device
                        .lock()
                        .map_err(|e| anyhow::anyhow!("device lock poisoned: {e}"))?;
                    dev.write(&buf)
                        .with_context(|| format!("I2C write step {} on {}", idx, bus_path))
                })
                .await
                .context("spawn_blocking join error")??;
            }
            if let Some(delay) = step.delay {
                tokio::time::sleep(delay).await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
