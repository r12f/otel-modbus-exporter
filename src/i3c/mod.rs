use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::bus;
use crate::config;
use crate::decoder;

/// Type alias for per-bus mutex (serializes access to a single I3C controller).
pub type BusLock = Arc<tokio::sync::Mutex<()>>;

/// Trait abstracting I3C device operations for testability.
pub trait I3cDevice: Send {
    /// Read from a device at the given dynamic address.
    /// Sends `command` bytes (register address), then reads `response_length` bytes.
    fn read(&self, address: u8, command: &[u8], response_length: usize) -> Result<Vec<u8>>;
}

/// Real I3C device using Linux /dev/i3c-N character device.
#[cfg(target_os = "linux")]
pub mod linux_device {
    use super::*;
    use std::io::{Read, Write};

    pub struct LinuxI3cDevice {
        bus_path: String,
        fd: Option<std::cell::UnsafeCell<std::fs::File>>,
    }

    // SAFETY: We ensure single-threaded access through external bus locking.
    unsafe impl Send for LinuxI3cDevice {}
    unsafe impl Sync for LinuxI3cDevice {}

    impl LinuxI3cDevice {
        pub fn new(bus_path: String) -> Self {
            Self { bus_path, fd: None }
        }

        pub fn open(&mut self) -> Result<()> {
            use std::fs::OpenOptions;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.bus_path)
                .with_context(|| format!("opening I3C bus {}", self.bus_path))?;
            self.fd = Some(std::cell::UnsafeCell::new(file));
            Ok(())
        }
    }

    impl I3cDevice for LinuxI3cDevice {
        fn read(&self, _address: u8, command: &[u8], response_length: usize) -> Result<Vec<u8>> {
            let cell = self
                .fd
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("I3C device not opened"))?;
            // SAFETY: Access is serialized by the bus lock.
            let fd_mut = unsafe { &mut *cell.get() };
            fd_mut
                .write_all(command)
                .context("I3C write register address")?;
            let mut buf = vec![0u8; response_length];
            fd_mut.read_exact(&mut buf).context("I3C read data")?;
            Ok(buf)
        }
    }
}

/// Stub I3C device for tests and non-Linux platforms.
pub struct StubI3cDevice;

impl I3cDevice for StubI3cDevice {
    fn read(&self, _address: u8, _command: &[u8], _response_length: usize) -> Result<Vec<u8>> {
        anyhow::bail!("StubI3cDevice: no real I3C hardware available")
    }
}

/// Address mode for an I3C device, resolved from config.
#[derive(Debug, Clone)]
pub enum AddressMode {
    Pid(String),
    Static(u8),
    DeviceClass { class: String, instance: u8 },
}

/// I3C client wrapping a device with address resolution and caching.
pub struct I3cClient {
    device: Arc<std::sync::Mutex<Box<dyn I3cDevice>>>,
    bus_path: String,
    address_mode: AddressMode,
    resolved_address: Option<u8>,
    connected: bool,
}

/// Per-bus mutex map for serializing access.
static BUS_LOCKS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, BusLock>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Get or create a per-bus lock.
pub fn get_bus_lock(bus_path: &str) -> BusLock {
    let mut map = BUS_LOCKS.lock().unwrap();
    map.entry(bus_path.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Resolve a dynamic address by scanning /sys/bus/i3c/devices/.
/// This is a best-effort implementation for Linux; in tests/non-Linux it returns an error.
fn resolve_address_from_sysfs(address_mode: &AddressMode) -> Result<u8> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        let sysfs_path = "/sys/bus/i3c/devices/";
        let entries = fs::read_dir(sysfs_path)
            .with_context(|| format!("enumerating I3C devices at {}", sysfs_path))?;

        match address_mode {
            AddressMode::Pid(pid) => {
                let pid_normalized = pid.to_lowercase().replace("0x", "");
                for entry in entries {
                    let entry = entry?;
                    let pid_file = entry.path().join("pid");
                    if let Ok(content) = fs::read_to_string(&pid_file) {
                        let dev_pid = content.trim().to_lowercase().replace("0x", "");
                        if dev_pid == pid_normalized {
                            // Read dynamic address from the device directory name or address file
                            let addr_file = entry.path().join("dynamic_address");
                            if let Ok(addr_str) = fs::read_to_string(&addr_file) {
                                let addr = u8::from_str_radix(
                                    addr_str.trim().trim_start_matches("0x"),
                                    16,
                                )?;
                                return Ok(addr);
                            }
                        }
                    }
                }
                anyhow::bail!("no I3C device found with PID {}", pid);
            }
            AddressMode::Static(addr) => Ok(*addr),
            AddressMode::DeviceClass { class, instance } => {
                let mut matches = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    let dcr_file = entry.path().join("dcr");
                    if let Ok(content) = fs::read_to_string(&dcr_file) {
                        if content.trim() == *class {
                            matches.push(entry.path());
                        }
                    }
                }
                matches.sort();
                let dev_path = matches.get(*instance as usize).with_context(|| {
                    format!(
                        "no I3C device found for class '{}' instance {}",
                        class, instance
                    )
                })?;
                let addr_file = dev_path.join("dynamic_address");
                let addr_str =
                    fs::read_to_string(&addr_file).with_context(|| "reading dynamic_address")?;
                let addr = u8::from_str_radix(addr_str.trim().trim_start_matches("0x"), 16)?;
                Ok(addr)
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        match address_mode {
            AddressMode::Static(addr) => Ok(*addr),
            _ => anyhow::bail!("I3C address resolution from sysfs is only available on Linux"),
        }
    }
}

impl I3cClient {
    pub fn new(device: Box<dyn I3cDevice>, bus_path: String, address_mode: AddressMode) -> Self {
        let resolved = match &address_mode {
            AddressMode::Static(addr) => Some(*addr),
            _ => None,
        };
        Self {
            device: Arc::new(std::sync::Mutex::new(device)),
            bus_path,
            address_mode,
            resolved_address: resolved,
            connected: false,
        }
    }

    /// Resolve address (if not already cached).
    pub fn resolve_address(&mut self) -> Result<u8> {
        if let Some(addr) = self.resolved_address {
            return Ok(addr);
        }
        let addr = resolve_address_from_sysfs(&self.address_mode)?;
        self.resolved_address = Some(addr);
        Ok(addr)
    }

    /// Invalidate the cached address (for re-enumeration after NACK).
    pub fn invalidate_address(&mut self) {
        if !matches!(self.address_mode, AddressMode::Static(_)) {
            self.resolved_address = None;
        }
    }

    /// Read bytes from a register on the I3C device with NACK retry logic.
    pub fn read_register_sync(&mut self, register: u8, byte_count: usize) -> Result<Vec<u8>> {
        let backoffs = [
            std::time::Duration::from_millis(100),
            std::time::Duration::from_millis(500),
            std::time::Duration::from_millis(2000),
        ];

        let address = self.resolve_address()?;
        let dev = self
            .device
            .lock()
            .map_err(|e| anyhow::anyhow!("device lock poisoned: {e}"))?;

        let result = dev.read(address, &[register], byte_count);
        if let Ok(data) = result {
            return Ok(data);
        }
        let first_err = result.unwrap_err();
        drop(dev);

        let mut last_err = first_err;
        for (attempt, backoff) in backoffs.iter().enumerate() {
            std::thread::sleep(*backoff);
            self.invalidate_address();
            match self.resolve_address() {
                Ok(new_addr) => {
                    let dev = self
                        .device
                        .lock()
                        .map_err(|e| anyhow::anyhow!("device lock poisoned: {e}"))?;
                    match dev.read(new_addr, &[register], byte_count) {
                        Ok(data) => return Ok(data),
                        Err(e) => last_err = e,
                    }
                }
                Err(e) => last_err = e,
            }
            if attempt == backoffs.len() - 1 {
                return Err(last_err).with_context(|| {
                    format!(
                        "I3C read failed after {} retries on {}",
                        backoffs.len(),
                        self.bus_path
                    )
                });
            }
        }
        Err(last_err)
    }
}

/// Read a single I3C metric.
pub async fn read_i3c_metric(
    client: &Arc<tokio::sync::Mutex<I3cClient>>,
    metric: &config::Metric,
    bus_lock: &BusLock,
) -> Result<f64> {
    let data_type = bus::map_data_type(metric.data_type);
    let byte_order = bus::map_byte_order(metric.byte_order);

    let register = metric.address.unwrap() as u8;
    let num_bytes = decoder::byte_count(data_type);
    let scale = metric.scale;
    let offset = metric.offset;

    let client = Arc::clone(client);
    let bus_lock = bus_lock.clone();

    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let _lock = bus_lock.blocking_lock();
        let mut c = client.blocking_lock();
        c.read_register_sync(register, num_bytes)
    })
    .await
    .context("spawn_blocking join error")??;

    decoder::decode_bytes(&bytes, data_type, byte_order, scale, offset)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Connection/lifecycle trait impl for I3cClient.
#[async_trait]
impl crate::modbus::BusConnection for I3cClient {
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
