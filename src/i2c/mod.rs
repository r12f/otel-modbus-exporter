use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::bus;
use crate::config;
use crate::decoder;

/// Type alias for the shared bus lock (std Mutex for use in spawn_blocking).
pub type BusLock = Arc<std::sync::Mutex<()>>;

/// Trait abstracting I2C device operations for testability.
pub trait I2cDevice: Send {
    /// Write bytes to the device, then read `read_len` bytes back.
    fn write_read(&mut self, write_buf: &[u8], read_len: usize) -> Result<Vec<u8>>;
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
    }
}

/// Stub I2C device (placeholder — real device is platform-specific).
pub struct StubI2cDevice;

impl I2cDevice for StubI2cDevice {
    fn write_read(&mut self, _write_buf: &[u8], _read_len: usize) -> Result<Vec<u8>> {
        anyhow::bail!("StubI2cDevice: no real I2C hardware available")
    }
}

/// I2C client that wraps a device and provides async read operations.
pub struct I2cClient {
    device: Arc<std::sync::Mutex<Box<dyn I2cDevice>>>,
    bus_path: String,
    address: u8,
    connected: bool,
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

impl I2cClient {
    pub fn new(device: Box<dyn I2cDevice>, bus_path: String, address: u8) -> Self {
        Self {
            device: Arc::new(std::sync::Mutex::new(device)),
            bus_path,
            address,
            connected: false,
        }
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
    client: &I2cClient,
    metric: &config::Metric,
    bus_lock: &BusLock,
) -> Result<f64> {
    let data_type = bus::map_data_type(metric.data_type);
    let byte_order = bus::map_byte_order(metric.byte_order);

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

/// Connection/lifecycle trait impl for I2cClient (mirrors BusConnection).
#[async_trait]
impl crate::modbus::BusConnection for I2cClient {
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
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
