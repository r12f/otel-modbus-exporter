use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config;
use crate::decoder;

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
                libc::ioctl(file.as_raw_fd(), I2C_SLAVE, self.address as libc::c_ulong)
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
    device: Box<dyn I2cDevice>,
    bus_path: String,
    address: u8,
    connected: bool,
}

/// Per-bus mutex map for serializing access.
static BUS_LOCKS: std::sync::LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Get or create a per-bus lock.
pub async fn get_bus_lock(bus_path: &str) -> Arc<Mutex<()>> {
    let mut map = BUS_LOCKS.lock().await;
    map.entry(bus_path.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

impl I2cClient {
    pub fn new(device: Box<dyn I2cDevice>, bus_path: String, address: u8) -> Self {
        Self {
            device,
            bus_path,
            address,
            connected: false,
        }
    }

    /// Read bytes from a register address on the I2C device.
    pub fn read_register_sync(&mut self, register: u8, byte_count: usize) -> Result<Vec<u8>> {
        self.device.write_read(&[register], byte_count)
    }
}

/// Read a single I2C metric.
pub async fn read_i2c_metric(
    client: &mut I2cClient,
    metric: &config::Metric,
    bus_lock: &Arc<Mutex<()>>,
) -> Result<f64> {
    let data_type = map_data_type(metric.data_type);
    let byte_order = map_byte_order(metric.byte_order);
    let register = metric.address as u8;
    let num_bytes = decoder::byte_count(data_type);
    let scale = metric.scale;
    let offset = metric.offset;

    // We need to move data out of the client for spawn_blocking
    // Instead, we hold the bus lock and do the blocking read
    let _lock = bus_lock.lock().await;

    let bytes = client
        .read_register_sync(register, num_bytes)
        .with_context(|| {
            format!(
                "reading I2C register {:#04x} ({} bytes) from device {:#04x} on {}",
                register, num_bytes, client.address, client.bus_path
            )
        })?;

    decoder::decode_bytes(&bytes, data_type, byte_order, scale, offset)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn map_byte_order(bo: config::ByteOrder) -> decoder::ByteOrder {
    match bo {
        config::ByteOrder::BigEndian => decoder::ByteOrder::BigEndian,
        config::ByteOrder::LittleEndian => decoder::ByteOrder::LittleEndian,
        config::ByteOrder::MidBigEndian => decoder::ByteOrder::MidBigEndian,
        config::ByteOrder::MidLittleEndian => decoder::ByteOrder::MidLittleEndian,
    }
}

fn map_data_type(dt: config::DataType) -> decoder::DataType {
    match dt {
        config::DataType::U8 => decoder::DataType::U8,
        config::DataType::U16 => decoder::DataType::U16,
        config::DataType::I16 => decoder::DataType::I16,
        config::DataType::U32 => decoder::DataType::U32,
        config::DataType::I32 => decoder::DataType::I32,
        config::DataType::F32 => decoder::DataType::F32,
        config::DataType::U64 => decoder::DataType::U64,
        config::DataType::I64 => decoder::DataType::I64,
        config::DataType::F64 => decoder::DataType::F64,
        config::DataType::Bool => decoder::DataType::Bool,
    }
}

/// Connection/lifecycle trait impl for I2cClient (mirrors ModbusConnection).
#[async_trait]
impl crate::modbus::ModbusConnection for I2cClient {
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
