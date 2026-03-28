use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

use crate::config;
use crate::decoder;

/// Type alias for the shared bus lock (std Mutex for use in spawn_blocking).
pub type BusLock = Arc<std::sync::Mutex<()>>;

/// Trait abstracting I3C device operations for testability.
/// Takes `&mut self` (like I2C's `I2cDevice` trait) so no `unsafe` is needed.
pub trait I3cDevice: Send {
    /// Write command bytes (register address) to the device, then read `read_len` bytes back.
    fn write_read(&mut self, address: u8, write_buf: &[u8], read_len: usize) -> Result<Vec<u8>>;
}

/// Real I3C device using Linux /dev/i3c-N character device with ioctl-based addressing.
/// Only available on Linux targets.
#[cfg(target_os = "linux")]
pub mod linux_device {
    use super::*;
    use std::os::unix::io::AsRawFd;

    /// I3C private transfer direction.
    #[repr(u8)]
    #[allow(dead_code)]
    enum I3cTransferDir {
        Write = 0,
        Read = 1,
    }

    /// Kernel struct for `I3C_IOC_PRIV_XFER` ioctl.
    #[repr(C)]
    struct I3cPrivTransfer {
        data: u64, // pointer to buffer
        len: u16,  // buffer length
        rnw: u8,   // 0 = write, 1 = read
        _pad: [u8; 5],
    }

    /// ioctl base type code for I3C: 'i'.
    const I3C_IOC_PRIV_XFER_BASE: u64 = 0x69;
    /// ioctl command number for I3C private transfer.
    const I3C_IOC_PRIV_XFER_NR: u64 = 0x30;

    /// Compute the ioctl request code for `num_xfers` transfers.
    fn i3c_ioc_priv_xfer(num_xfers: usize) -> libc::c_ulong {
        // _IOC(_IOC_READ | _IOC_WRITE, 'i', 0x30, num_xfers * sizeof(I3cPrivTransfer))
        let size = (num_xfers * std::mem::size_of::<I3cPrivTransfer>()) as u64;
        let dir: u64 = 3; // _IOC_READ | _IOC_WRITE
        ((dir << 30) | (I3C_IOC_PRIV_XFER_BASE << 8) | I3C_IOC_PRIV_XFER_NR | (size << 16))
            as libc::c_ulong
    }

    pub struct LinuxI3cDevice {
        bus_path: String,
        fd: Option<std::fs::File>,
    }

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
            self.fd = Some(file);
            Ok(())
        }
    }

    impl I3cDevice for LinuxI3cDevice {
        fn write_read(
            &mut self,
            _address: u8,
            write_buf: &[u8],
            read_len: usize,
        ) -> Result<Vec<u8>> {
            let file = self
                .fd
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("I3C device not opened"))?;
            let raw_fd = file.as_raw_fd();

            // Build two transfers: write command, then read response.
            let mut read_buf = vec![0u8; read_len];
            let mut cmd_buf = write_buf.to_vec();

            let xfers = [
                I3cPrivTransfer {
                    data: cmd_buf.as_mut_ptr() as u64,
                    len: cmd_buf.len() as u16,
                    rnw: I3cTransferDir::Write as u8,
                    _pad: [0; 5],
                },
                I3cPrivTransfer {
                    data: read_buf.as_mut_ptr() as u64,
                    len: read_buf.len() as u16,
                    rnw: I3cTransferDir::Read as u8,
                    _pad: [0; 5],
                },
            ];

            let request = i3c_ioc_priv_xfer(xfers.len());
            // SAFETY: xfers is a valid array of I3cPrivTransfer structs pointing to valid buffers.
            let ret = unsafe { libc::ioctl(raw_fd, request as libc::Ioctl, xfers.as_ptr()) };
            if ret < 0 {
                return Err(std::io::Error::last_os_error())
                    .context("I3C private transfer ioctl failed");
            }

            Ok(read_buf)
        }
    }
}

/// Stub I3C device (placeholder for non-Linux platforms where no real hardware is available).
/// Only compiled on non-Linux targets (Linux uses `LinuxI3cDevice`).
#[cfg(not(target_os = "linux"))]
pub struct StubI3cDevice;

#[cfg(not(target_os = "linux"))]
impl I3cDevice for StubI3cDevice {
    fn write_read(&mut self, _address: u8, _write_buf: &[u8], _read_len: usize) -> Result<Vec<u8>> {
        anyhow::bail!("StubI3cDevice: no real I3C hardware available")
    }
}

// ── I3C-specific types ──────────────────────────────────────────────

/// I3C error classification for retry logic.
#[derive(Debug)]
pub enum I3cErrorKind {
    /// NACK or transfer error — re-enumeration may help.
    TransferError,
    /// Configuration or other non-bus error — do not re-enumerate.
    Other,
}

/// Classify an error to decide whether re-enumeration is warranted.
fn classify_error(err: &anyhow::Error) -> I3cErrorKind {
    let msg = format!("{err:#}").to_lowercase();
    if msg.contains("nack")
        || msg.contains("transfer")
        || msg.contains("i/o error")
        || msg.contains("io error")
        || msg.contains("connection reset")
        || msg.contains("remote i/o")
    {
        I3cErrorKind::TransferError
    } else {
        I3cErrorKind::Other
    }
}

/// Address mode for an I3C device, resolved from config.
#[derive(Debug, Clone)]
pub enum AddressMode {
    /// Provisioned ID — resolved via sysfs enumeration.
    Pid(String),
    /// Static/pre-assigned address.
    Static(u8),
    /// Device class + instance index — resolved via sysfs DCR matching.
    DeviceClass { class: String, instance: u8 },
}

// ── Client ──────────────────────────────────────────────────────────

/// I3C metric reader that wraps a device and provides async read operations.
///
/// Unlike `I2cMetricReader`, the I3C metric reader requires `&mut self` for reads
/// because dynamic address resolution may need to mutate cached state (e.g. after
/// NACK-triggered re-enumeration). The reader is therefore wrapped in
/// `Arc<tokio::sync::Mutex<..>>` at the call site.
pub struct I3cMetricReader {
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
        .or_insert_with(|| Arc::new(std::sync::Mutex::new(())))
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

impl I3cMetricReader {
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

    /// Set the resolved address (for testing without sysfs).
    pub fn set_resolved_address(&mut self, addr: u8) {
        self.resolved_address = Some(addr);
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
    ///
    /// Takes `&mut self` (unlike I2C's `&self`) because dynamic address
    /// resolution and invalidation mutate the cached `resolved_address`.
    pub fn read_register_sync(&mut self, register: u8, byte_count: usize) -> Result<Vec<u8>> {
        let backoffs = [
            std::time::Duration::from_millis(100),
            std::time::Duration::from_millis(500),
            std::time::Duration::from_millis(2000),
        ];

        let address = self.resolve_address()?;
        let mut dev = self
            .device
            .lock()
            .map_err(|e| anyhow::anyhow!("device lock poisoned: {e}"))?;

        match dev.write_read(address, &[register], byte_count) {
            Ok(data) => Ok(data),
            Err(err) => {
                // Only retry on transfer/NACK errors; propagate others immediately.
                if matches!(classify_error(&err), I3cErrorKind::Other) {
                    return Err(err).context("I3C read failed (non-retriable)");
                }
                drop(dev);

                let mut last_err = err;
                for (attempt, backoff) in backoffs.iter().enumerate() {
                    warn!(
                        bus = %self.bus_path,
                        attempt = attempt + 1,
                        backoff_ms = backoff.as_millis() as u64,
                        "I3C transfer error, re-enumerating after backoff"
                    );
                    std::thread::sleep(*backoff);
                    self.invalidate_address();
                    match self.resolve_address() {
                        Ok(new_addr) => {
                            let mut dev = self
                                .device
                                .lock()
                                .map_err(|e| anyhow::anyhow!("device lock poisoned: {e}"))?;
                            match dev.write_read(new_addr, &[register], byte_count) {
                                Ok(data) => return Ok(data),
                                Err(e) => last_err = e,
                            }
                        }
                        Err(e) => last_err = e,
                    }
                }
                Err(last_err).with_context(|| {
                    format!(
                        "I3C read failed after {} retries on {}",
                        backoffs.len(),
                        self.bus_path
                    )
                })
            }
        }
    }

    /// Mark client as connected.
    pub async fn connect(&mut self) -> Result<()> {
        self.connected = true;
        Ok(())
    }

    /// Mark client as disconnected.
    pub async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }

    /// Check if client is connected.
    pub fn is_connected(&self) -> bool {
        self.connected
    }
}

/// Read a single I3C metric.
pub async fn read_i3c_metric(
    client: &Arc<tokio::sync::Mutex<I3cMetricReader>>,
    metric: &config::MetricConfig,
    bus_lock: &BusLock,
) -> Result<f64> {
    let data_type = map_data_type(metric.data_type);
    let byte_order = map_byte_order(metric.byte_order);

    // address validated as present and in u8 range by config
    let register = metric.address.unwrap() as u8;
    let num_bytes = decoder::byte_count(data_type);
    let scale = metric.scale;
    let offset = metric.offset;

    // Clone Arcs for move into spawn_blocking
    let client = Arc::clone(client);
    let bus_lock = bus_lock.clone();

    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let _lock = bus_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("bus lock poisoned: {e}"))?;
        let mut c = client.blocking_lock();
        c.read_register_sync(register, num_bytes)
    })
    .await
    .context("spawn_blocking join error")??;

    decoder::decode_bytes(&bytes, data_type, byte_order, scale, offset)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Map config byte order to decoder byte order.
fn map_byte_order(bo: config::ByteOrder) -> decoder::ByteOrder {
    match bo {
        config::ByteOrder::BigEndian => decoder::ByteOrder::BigEndian,
        config::ByteOrder::LittleEndian => decoder::ByteOrder::LittleEndian,
        config::ByteOrder::MidBigEndian => decoder::ByteOrder::MidBigEndian,
        config::ByteOrder::MidLittleEndian => decoder::ByteOrder::MidLittleEndian,
    }
}

/// Map config data type to decoder data type.
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

/// Wrapper around `Arc<Mutex<I3cMetricReader>>` + `BusLock` that implements `MetricReader`.
pub struct I3cMetricReaderHandle {
    client: Arc<tokio::sync::Mutex<I3cMetricReader>>,
    bus_lock: BusLock,
}

impl I3cMetricReaderHandle {
    pub fn new(client: Arc<tokio::sync::Mutex<I3cMetricReader>>, bus_lock: BusLock) -> Self {
        Self { client, bus_lock }
    }
}

#[async_trait]
impl crate::reader::MetricReader for I3cMetricReaderHandle {
    async fn connect(&mut self) -> Result<()> {
        let mut c = self.client.lock().await;
        c.connect().await
    }

    async fn disconnect(&mut self) -> Result<()> {
        let mut c = self.client.lock().await;
        c.disconnect().await
    }

    fn is_connected(&self) -> bool {
        // If the lock is contended, conservatively report disconnected rather than
        // silently masking a potential disconnection state.
        self.client
            .try_lock()
            .map(|c| c.is_connected())
            .unwrap_or(false)
    }

    fn capabilities(&self) -> crate::reader::ReaderCapabilities {
        crate::reader::ReaderCapabilities { batch_read: false }
    }

    async fn read(&mut self, metric: &config::MetricConfig) -> Result<f64> {
        read_i3c_metric(&self.client, metric, &self.bus_lock).await
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
