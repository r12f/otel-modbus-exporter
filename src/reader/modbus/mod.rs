pub mod batch;
pub mod rtu;
pub mod tcp;

#[cfg(test)]
mod mod_tests;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use std::time::Duration;

/// Maximum number of registers per read for FC03/FC04 (Modbus spec).
pub const MAX_REGISTERS_PER_READ: u16 = 125;
/// Maximum number of coils/discrete inputs per read for FC01/FC02 (Modbus spec).
pub const MAX_COILS_PER_READ: u16 = 2000;
/// Default read timeout for all Modbus operations.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Validate register count for FC03/FC04.
pub fn validate_register_count(count: u16) -> Result<()> {
    if count == 0 || count > MAX_REGISTERS_PER_READ {
        bail!("register count {count} out of range (1..={MAX_REGISTERS_PER_READ})");
    }
    Ok(())
}

/// Validate coil/discrete input count for FC01/FC02.
pub fn validate_coil_count(count: u16) -> Result<()> {
    if count == 0 || count > MAX_COILS_PER_READ {
        bail!("coil/discrete count {count} out of range (1..={MAX_COILS_PER_READ})");
    }
    Ok(())
}

/// Lifecycle management for bus clients (Modbus, I2C, SPI).
///
/// Separated from [`ModbusReader`] so callers can hold a narrow read-only
/// interface when lifecycle control is not needed.
#[async_trait]
pub trait BusConnection: Send {
    /// Establish the connection.
    ///
    /// If already connected, the previous connection is closed first
    /// (see [`disconnect`](Self::disconnect)).
    async fn connect(&mut self) -> Result<()>;

    /// Explicitly close the underlying transport (TCP socket / serial port).
    ///
    /// After this call, [`is_connected`](Self::is_connected) returns `false`.
    async fn disconnect(&mut self) -> Result<()>;

    /// Returns `true` when a transport handle exists.
    ///
    /// **Note:** This only tracks whether [`connect`](Self::connect) has been
    /// called (and [`disconnect`](Self::disconnect) has not). It does **not**
    /// perform a health-check on the underlying socket or serial port—the
    /// next read may still fail if the remote end has gone away.
    fn is_connected(&self) -> bool;
}

/// Read interface for Modbus operations.
///
/// All read methods enforce Modbus-spec count limits and apply a 5-second
/// timeout around the underlying I/O.
///
/// # Concurrency (RTU / half-duplex)
///
/// All methods take `&mut self`, which guarantees at the type level that only
/// one operation is in-flight at a time. This is **required** for RTU
/// (half-duplex serial) transports where concurrent bus access would corrupt
/// frames. Callers must not wrap a `ModbusRtuMetricReader` in shared-mutable containers
/// (`Arc<Mutex<_>>` is acceptable only if the critical section spans the
/// entire request–response cycle).
#[async_trait]
pub trait ModbusReader: Send {
    /// Read holding registers (FC03). `count` must be in 1..=125.
    async fn read_holding_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>>;

    /// Read input registers (FC04). `count` must be in 1..=125.
    async fn read_input_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>>;

    /// Read coils (FC01). `count` must be in 1..=2000.
    async fn read_coils(&mut self, addr: u16, count: u16) -> Result<Vec<bool>>;

    /// Read discrete inputs (FC02). `count` must be in 1..=2000.
    async fn read_discrete_inputs(&mut self, addr: u16, count: u16) -> Result<Vec<bool>>;
}

/// Combined trait for convenience — a full Modbus client.
pub trait ModbusClient: BusConnection + ModbusReader {}
impl<T: BusConnection + ModbusReader> ModbusClient for T {}

/// Read a single metric value from any Modbus client (used by MetricReader impls).
pub(crate) async fn read_modbus_metric(
    client: &mut dyn ModbusClient,
    metric: &crate::config::MetricConfig,
) -> Result<f64> {
    use crate::config::RegisterType;

    let count = metric.data_type.register_count();
    let data_type = crate::bus::map_data_type(metric.data_type);
    let byte_order = crate::bus::map_byte_order(metric.byte_order);
    let register_type = metric.register_type.unwrap_or(RegisterType::Holding);

    match register_type {
        RegisterType::Holding => {
            let regs = client
                .read_holding_registers(metric.address.unwrap(), count)
                .await
                .context("reading holding registers")?;
            crate::decoder::decode(&regs, data_type, byte_order, metric.scale, metric.offset)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        RegisterType::Input => {
            let regs = client
                .read_input_registers(metric.address.unwrap(), count)
                .await
                .context("reading input registers")?;
            crate::decoder::decode(&regs, data_type, byte_order, metric.scale, metric.offset)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        RegisterType::Coil => {
            let bits = client
                .read_coils(metric.address.unwrap(), 1)
                .await
                .context("reading coils")?;
            let val = bits
                .first()
                .ok_or_else(|| anyhow::anyhow!("empty coil response"))?;
            let raw = if *val { 1.0 } else { 0.0 };
            Ok(raw * metric.scale + metric.offset)
        }
        RegisterType::Discrete => {
            let bits = client
                .read_discrete_inputs(metric.address.unwrap(), 1)
                .await
                .context("reading discrete inputs")?;
            let val = bits
                .first()
                .ok_or_else(|| anyhow::anyhow!("empty discrete input response"))?;
            let raw = if *val { 1.0 } else { 0.0 };
            Ok(raw * metric.scale + metric.offset)
        }
    }
}
