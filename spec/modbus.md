# Modbus Client Specification

> Part of the [bus-exporter architecture](../README.md#architecture). Modbus readers implement the `MetricReader` trait.

## Overview

The Modbus module provides async clients for RTU (serial) and TCP protocols, abstracting the differences behind a common trait.

### Batch Read

When `batch_read: true` is set in the protocol config, the Modbus reader coalesces adjacent/overlapping register ranges into fewer bus calls. This reduces polling overhead for devices with many contiguous registers. Batch read is disabled by default.

## Modbus RTU Client

- Uses `tokio-serial` for async serial port access.
- Configuration: device path, baud rate, data bits, stop bits, parity.
- Uses `tokio-modbus` `rtu::connect_slave()` with the serial stream.
- Only one outstanding request at a time per serial port (Modbus RTU is half-duplex).

## Modbus TCP Client

- Uses `tokio-modbus` `tcp::connect_slave()` with the target endpoint.
- One TCP connection per collector.
- Supports standard Modbus TCP port 502 or any custom port.

## Register Types

| Register Type | Modbus Function Code | Read Function | Data |
|---------------|---------------------|---------------|------|
| `holding` | FC 03 | `read_holding_registers` | 16-bit registers |
| `input` | FC 04 | `read_input_registers` | 16-bit registers |
| `coil` | FC 01 | `read_coils` | Single bits |
| `discrete` | FC 02 | `read_discrete_inputs` | Single bits |

## Read Operations

- For 16-bit data types (`u16`, `i16`): read 1 register.
- For 32-bit data types (`u32`, `i32`, `f32`): read 2 consecutive registers.
- For 64-bit data types (`u64`, `i64`, `f64`): read 4 consecutive registers.
- For `bool` (coil/discrete): read 1 bit.

## Error Handling

- **Timeout**: default 5s per read operation. Configurable per-collector in future.
- **Retries**: no automatic retry at the Modbus layer — handled by the collector poll loop.
- **Connection errors**: bubble up to the collector for reconnect logic.
- All errors include context: collector name, register address, slave ID.

## Common Trait

```rust
#[async_trait]
pub trait ModbusClient: Send {
    async fn read_holding_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>>;
    async fn read_input_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>>;
    async fn read_coils(&mut self, addr: u16, count: u16) -> Result<Vec<bool>>;
    async fn read_discrete_inputs(&mut self, addr: u16, count: u16) -> Result<Vec<bool>>;
}
```
