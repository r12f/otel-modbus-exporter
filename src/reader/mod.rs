pub mod decoder;
pub mod i2c;
pub mod i3c;
pub mod modbus;
pub mod spi;

use std::collections::HashMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::config::{self, MetricConfig, Protocol, WriteStep};

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
    /// Per-metric results: `(raw_value, scaled_value)`.
    pub metrics: HashMap<String, Result<(f64, f64)>>,
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

/// Factory trait for creating metric readers from config.
/// This allows tests to inject mock readers.
pub trait MetricReaderFactory: Send + Sync {
    fn create(&self, collector: &config::CollectorConfig) -> Result<Box<dyn MetricReader>>;
}

/// Trait for writing register values to a bus device.
///
/// Separate from MetricReader — write steps are used for device
/// initialization (`init_writes`) and measurement triggering (`pre_poll`).
#[async_trait]
pub trait MetricWriter: Send {
    /// Execute a sequence of write steps. Each step may write bytes and/or delay.
    async fn execute_writes(&mut self, steps: &[WriteStep]) -> Result<()>;
}

/// Factory trait for creating metric writers from config.
pub trait MetricWriterFactory: Send + Sync {
    /// Create a writer for the given collector, or None if the protocol doesn't support writes.
    fn create_writer(
        &self,
        collector: &config::CollectorConfig,
    ) -> Result<Option<Box<dyn MetricWriter>>>;
}

/// Combined factory trait for creating both readers and writers.
pub trait MetricFactory: MetricReaderFactory + MetricWriterFactory {}

/// Default factory implementation that creates real readers based on protocol config.
pub struct MetricReaderFactoryImpl;

impl MetricReaderFactory for MetricReaderFactoryImpl {
    fn create(&self, collector: &config::CollectorConfig) -> Result<Box<dyn MetricReader>> {
        match &collector.protocol {
            Protocol::ModbusTcp { endpoint } => {
                let slave_id = collector.slave_id.unwrap_or(1);
                Ok(Box::new(modbus::tcp::ModbusTcpMetricReader::new(
                    endpoint.clone(),
                    slave_id,
                )))
            }
            Protocol::ModbusRtu {
                device,
                bps,
                data_bits,
                stop_bits,
                parity,
            } => {
                let slave_id = collector.slave_id.unwrap_or(1);
                let builder = tokio_serial::new(device, *bps)
                    .data_bits(match data_bits {
                        5 => tokio_serial::DataBits::Five,
                        6 => tokio_serial::DataBits::Six,
                        7 => tokio_serial::DataBits::Seven,
                        _ => tokio_serial::DataBits::Eight,
                    })
                    .stop_bits(match stop_bits {
                        2 => tokio_serial::StopBits::Two,
                        _ => tokio_serial::StopBits::One,
                    })
                    .parity(match parity {
                        config::Parity::None => tokio_serial::Parity::None,
                        config::Parity::Even => tokio_serial::Parity::Even,
                        config::Parity::Odd => tokio_serial::Parity::Odd,
                    });
                Ok(Box::new(modbus::rtu::ModbusRtuMetricReader::new(
                    builder, slave_id,
                )))
            }
            Protocol::I2c { bus, address } => {
                #[cfg(target_os = "linux")]
                let device: Box<dyn i2c::I2cDevice> = {
                    let mut dev = i2c::linux_device::LinuxI2cDevice::new(bus.clone(), *address);
                    dev.open().context("failed to open I2C device")?;
                    Box::new(dev)
                };
                #[cfg(not(target_os = "linux"))]
                let device: Box<dyn i2c::I2cDevice> = Box::new(i2c::StubI2cDevice);

                let bus_lock = i2c::get_bus_lock(bus);
                let client = i2c::I2cMetricReader::new(device, bus.clone(), *address, bus_lock);
                Ok(Box::new(client))
            }
            Protocol::Spi {
                device,
                speed_hz,
                mode,
                bits_per_word,
            } => {
                #[cfg(target_os = "linux")]
                let spi_device: Box<dyn spi::SpiDevice> = {
                    let mut dev = spi::linux_device::LinuxSpiDevice::new(
                        device.clone(),
                        *speed_hz,
                        *mode,
                        *bits_per_word,
                    );
                    dev.open().context("failed to open SPI device")?;
                    Box::new(dev)
                };
                #[cfg(not(target_os = "linux"))]
                let spi_device: Box<dyn spi::SpiDevice> = Box::new(spi::StubSpiDevice);

                let device_lock = spi::get_device_lock(device);
                let client = spi::SpiMetricReader::new(spi_device, device.clone(), device_lock);
                Ok(Box::new(client))
            }
            Protocol::I3c {
                bus,
                pid,
                address,
                device_class,
                instance,
            } => {
                let address_mode = if let Some(pid_str) = pid {
                    i3c::AddressMode::Pid(pid_str.clone())
                } else if let Some(addr) = address {
                    i3c::AddressMode::Static(*addr)
                } else {
                    i3c::AddressMode::DeviceClass {
                        class: device_class.clone().unwrap(),
                        instance: instance.unwrap(),
                    }
                };

                #[cfg(target_os = "linux")]
                let device: Box<dyn i3c::I3cDevice> = {
                    let mut dev = i3c::linux_device::LinuxI3cDevice::new(bus.clone());
                    dev.open().context("failed to open I3C device")?;
                    Box::new(dev)
                };
                #[cfg(not(target_os = "linux"))]
                let device: Box<dyn i3c::I3cDevice> = Box::new(i3c::StubI3cDevice);

                let client = i3c::I3cMetricReader::new(device, bus.clone(), address_mode);
                let bus_lock = i3c::get_bus_lock(bus);
                let handle = i3c::I3cMetricReaderHandle::new(
                    std::sync::Arc::new(tokio::sync::Mutex::new(client)),
                    bus_lock,
                );
                Ok(Box::new(handle))
            }
        }
    }
}

impl MetricWriterFactory for MetricReaderFactoryImpl {
    fn create_writer(
        &self,
        collector: &config::CollectorConfig,
    ) -> Result<Option<Box<dyn MetricWriter>>> {
        // Only create writers for protocols that support write steps
        if collector.init_writes.is_empty() && collector.pre_poll.is_empty() {
            return Ok(None);
        }
        match &collector.protocol {
            Protocol::ModbusTcp { .. } | Protocol::ModbusRtu { .. } => {
                // Modbus doesn't support write steps (validated earlier)
                Ok(None)
            }
            Protocol::I2c { bus, address } => {
                #[cfg(target_os = "linux")]
                let device: Box<dyn i2c::I2cDevice> = {
                    let mut dev = i2c::linux_device::LinuxI2cDevice::new(bus.clone(), *address);
                    dev.open().context("failed to open I2C device for writer")?;
                    Box::new(dev)
                };
                #[cfg(not(target_os = "linux"))]
                let device: Box<dyn i2c::I2cDevice> = Box::new(i2c::StubI2cDevice);

                let bus_lock = i2c::get_bus_lock(bus);
                let writer = i2c::I2cMetricWriter::new(
                    std::sync::Arc::new(std::sync::Mutex::new(device)),
                    bus.clone(),
                    bus_lock,
                );
                Ok(Some(Box::new(writer)))
            }
            Protocol::Spi {
                device,
                speed_hz,
                mode,
                bits_per_word,
            } => {
                #[cfg(target_os = "linux")]
                let spi_device: Box<dyn spi::SpiDevice> = {
                    let mut dev = spi::linux_device::LinuxSpiDevice::new(
                        device.clone(),
                        *speed_hz,
                        *mode,
                        *bits_per_word,
                    );
                    dev.open().context("failed to open SPI device for writer")?;
                    Box::new(dev)
                };
                #[cfg(not(target_os = "linux"))]
                let spi_device: Box<dyn spi::SpiDevice> = Box::new(spi::StubSpiDevice);

                let device_lock = spi::get_device_lock(device);
                let writer = spi::SpiMetricWriter::new(
                    std::sync::Arc::new(std::sync::Mutex::new(spi_device)),
                    device.clone(),
                    device_lock,
                );
                Ok(Some(Box::new(writer)))
            }
            Protocol::I3c {
                bus,
                pid,
                address,
                device_class,
                instance,
            } => {
                let address_mode = if let Some(pid_str) = pid {
                    i3c::AddressMode::Pid(pid_str.clone())
                } else if let Some(addr) = address {
                    i3c::AddressMode::Static(*addr)
                } else {
                    i3c::AddressMode::DeviceClass {
                        class: device_class.clone().unwrap(),
                        instance: instance.unwrap(),
                    }
                };

                #[cfg(target_os = "linux")]
                let device: Box<dyn i3c::I3cDevice> = {
                    let mut dev = i3c::linux_device::LinuxI3cDevice::new(bus.clone());
                    dev.open().context("failed to open I3C device for writer")?;
                    Box::new(dev)
                };
                #[cfg(not(target_os = "linux"))]
                let device: Box<dyn i3c::I3cDevice> = Box::new(i3c::StubI3cDevice);

                let client = i3c::I3cMetricReader::new(device, bus.clone(), address_mode);
                let bus_lock = i3c::get_bus_lock(bus);
                let writer = i3c::I3cMetricWriter::new(
                    std::sync::Arc::new(tokio::sync::Mutex::new(client)),
                    bus_lock,
                );
                Ok(Some(Box::new(writer)))
            }
        }
    }
}

impl MetricFactory for MetricReaderFactoryImpl {}
