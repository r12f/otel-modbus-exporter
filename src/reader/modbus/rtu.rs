use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio_modbus::client::{rtu, Context as ModbusContext, Reader};
use tokio_modbus::Slave;
use tokio_serial::SerialPortBuilder;

use super::{
    validate_coil_count, validate_register_count, BusConnection, ModbusReader, READ_TIMEOUT,
};

/// Modbus RTU (serial) metric reader.
///
/// # Half-duplex / concurrency
///
/// RTU operates over a half-duplex serial bus. All read methods take
/// `&mut self`, which prevents concurrent access at compile time. Do **not**
/// place a `ModbusRtuMetricReader` behind a shared-mutable wrapper unless the wrapper
/// holds the lock for the entire request–response cycle.
pub struct ModbusRtuMetricReader {
    builder: SerialPortBuilder,
    slave_id: u8,
    context: Option<ModbusContext>,
}

impl ModbusRtuMetricReader {
    /// Create a new RTU client (does not connect yet).
    pub fn new(builder: SerialPortBuilder, slave_id: u8) -> Self {
        Self {
            builder,
            slave_id,
            context: None,
        }
    }

    fn ctx(&mut self) -> Result<&mut ModbusContext> {
        self.context
            .as_mut()
            .with_context(|| format!("not connected (slave={})", self.slave_id))
    }
}

#[async_trait]
impl BusConnection for ModbusRtuMetricReader {
    async fn connect(&mut self) -> Result<()> {
        if self.context.is_some() {
            self.disconnect().await.ok();
        }
        let port = tokio_serial::SerialStream::open(&self.builder)
            .with_context(|| format!("failed to open serial port (slave={})", self.slave_id))?;
        let ctx = rtu::attach_slave(port, Slave(self.slave_id));
        self.context = Some(ctx);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.context.take();
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.context.is_some()
    }
}

#[async_trait]
impl ModbusReader for ModbusRtuMetricReader {
    async fn read_holding_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>> {
        validate_register_count(count)?;
        let ctx = self.ctx()?;
        let data = tokio::time::timeout(READ_TIMEOUT, ctx.read_holding_registers(addr, count))
            .await
            .with_context(|| {
                format!(
                    "read_holding_registers timed out (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?
            .with_context(|| {
                format!(
                    "read_holding_registers failed (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?
            .with_context(|| {
                format!(
                    "read_holding_registers empty response (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?;
        Ok(data)
    }

    async fn read_input_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>> {
        validate_register_count(count)?;
        let ctx = self.ctx()?;
        let data = tokio::time::timeout(READ_TIMEOUT, ctx.read_input_registers(addr, count))
            .await
            .with_context(|| {
                format!(
                    "read_input_registers timed out (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?
            .with_context(|| {
                format!(
                    "read_input_registers failed (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?
            .with_context(|| {
                format!(
                    "read_input_registers empty response (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?;
        Ok(data)
    }

    async fn read_coils(&mut self, addr: u16, count: u16) -> Result<Vec<bool>> {
        validate_coil_count(count)?;
        let ctx = self.ctx()?;
        let data = tokio::time::timeout(READ_TIMEOUT, ctx.read_coils(addr, count))
            .await
            .with_context(|| {
                format!(
                    "read_coils timed out (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?
            .with_context(|| {
                format!(
                    "read_coils failed (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?
            .with_context(|| {
                format!(
                    "read_coils empty response (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?;
        Ok(data)
    }

    async fn read_discrete_inputs(&mut self, addr: u16, count: u16) -> Result<Vec<bool>> {
        validate_coil_count(count)?;
        let ctx = self.ctx()?;
        let data = tokio::time::timeout(READ_TIMEOUT, ctx.read_discrete_inputs(addr, count))
            .await
            .with_context(|| {
                format!(
                    "read_discrete_inputs timed out (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?
            .with_context(|| {
                format!(
                    "read_discrete_inputs failed (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?
            .with_context(|| {
                format!(
                    "read_discrete_inputs empty response (addr={addr}, count={count}, slave={})",
                    self.slave_id
                )
            })?;
        Ok(data)
    }
}

#[async_trait]
impl crate::reader::MetricReader for ModbusRtuMetricReader {
    async fn connect(&mut self) -> Result<()> {
        BusConnection::connect(self).await
    }

    async fn disconnect(&mut self) -> Result<()> {
        BusConnection::disconnect(self).await
    }

    fn is_connected(&self) -> bool {
        BusConnection::is_connected(self)
    }

    fn capabilities(&self) -> crate::reader::ReaderCapabilities {
        crate::reader::ReaderCapabilities { batch_read: true }
    }

    async fn read(&mut self, metric: &crate::config::MetricConfig) -> Result<f64> {
        super::read_modbus_metric(self, metric).await
    }

    async fn batch_read<'a>(
        &mut self,
        metrics: &'a [crate::config::MetricConfig],
    ) -> crate::reader::BatchReadResult<'a> {
        let super::batch::BatchReadResult {
            results,
            read_count,
        } = super::batch::batch_read_coalesced(self, metrics).await;
        crate::reader::BatchReadResult {
            results,
            read_count,
        }
    }
}

#[cfg(test)]
#[path = "rtu_tests.rs"]
mod rtu_tests;
