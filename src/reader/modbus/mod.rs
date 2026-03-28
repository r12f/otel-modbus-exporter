pub mod batch;
pub mod rtu;
pub mod tcp;

#[cfg(test)]
mod mod_tests;

use anyhow::{bail, Result};
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
