# MetricReader Specification

> Part of the [bus-exporter architecture](../README.md#architecture).

## Overview

`MetricReader` is the common trait for all protocol readers. The collector uses it to read metrics from devices — it doesn't need to know the underlying protocol.

## Ownership Model

Each collector owns exactly one `MetricReader` instance. Readers are not shared across collectors — there is no concurrent access to a single reader. The collector calls `set_metrics()` once at startup, then calls `read()` sequentially within its polling loop, so `&mut self` is the correct receiver.

## Trait

```rust
#[async_trait]
pub trait MetricReader: Send {
    /// Configure which metrics this reader should collect.
    fn set_metrics(&mut self, metrics: Vec<MetricConfig>);

    /// Connect to the device/bus.
    async fn connect(&mut self) -> Result<()>;

    /// Disconnect.
    async fn disconnect(&mut self) -> Result<()>;

    /// Whether the reader is connected.
    fn is_connected(&self) -> bool;

    /// Read all configured metrics. Returns name → result mapping.
    async fn read(&mut self) -> HashMap<String, Result<f64>>;
}
```

**Error handling:** `Result` uses `anyhow::Result` — the collector logs errors and updates internal metrics (error counters). Transient errors (connection lost, timeout) trigger reconnect; permanent errors (invalid config) are logged once.

## Design

Each reader stores its configured metrics internally via `set_metrics()`. When `read()` is called, the reader iterates over all configured metrics and returns a `HashMap<String, Result<f64>>` mapping metric names to their results.

Protocol-specific optimizations (e.g., Modbus register coalescing) are handled internally by each reader implementation — the collector doesn't need to know about batch capabilities.

## Implementations

| Protocol | Internal optimization | Notes |
|----------|----------------------|-------|
| [Modbus TCP/RTU](modbus.md) | Register coalescing | Coalesces adjacent register ranges automatically |
| [I2C](i2c.md) | None | Single-device reads |
| [SPI](spi.md) | None | Single-device reads |
| [I3C](i3c.md) | None | Single-device reads |

## Source Layout

```text
src/reader/
  mod.rs              — MetricReader trait
  modbus/
    mod.rs            — Modbus MetricReader impl
    tcp.rs            — TCP transport
    tcp_tests.rs
    rtu.rs            — RTU transport
    rtu_tests.rs
    mod_tests.rs
  i2c/
    mod.rs
    mod_tests.rs
  spi/
    mod.rs
    mod_tests.rs
  i3c/
    mod.rs
    mod_tests.rs
```
