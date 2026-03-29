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

    /// Read all configured metrics. Returns results and I/O count.
    ///
    /// The `cancel` token allows cooperative cancellation between individual
    /// metric reads for fast shutdown on slow buses.
    async fn read(&mut self, cancel: &CancellationToken) -> ReadResults;
}

/// Result of a [`MetricReader::read`] call, including I/O request count.
pub struct ReadResults {
    /// Per-metric results: `(raw_value, scaled_value)`.
    pub metrics: HashMap<String, Result<(f64, f64)>>,
    /// Number of actual I/O operations performed (e.g. coalesced Modbus reads).
    pub io_count: usize,
}
```

**Error handling:** `Result` uses `anyhow::Result` — the collector logs errors and updates internal metrics (error counters). Transient errors (connection lost, timeout) trigger reconnect; permanent errors (invalid config) are logged once.

## MetricReaderFactory

A factory trait decouples reader creation from the collector, allowing tests to inject mock readers:

```rust
pub trait MetricReaderFactory: Send + Sync {
    fn create(&self, collector: &config::CollectorConfig) -> Result<Box<dyn MetricReader>>;
}

pub struct MetricReaderFactoryImpl;
```

`MetricReaderFactoryImpl` inspects the protocol in `CollectorConfig` and creates the appropriate reader (`ModbusTcpMetricReader`, `ModbusRtuMetricReader`, `I2cMetricReader`, `SpiMetricReader`, `I3cMetricReaderHandle`).

## Design

Each reader stores its configured metrics internally via `set_metrics()`. When `read()` is called, the reader iterates over all configured metrics and returns a `ReadResults` struct containing a `HashMap<String, Result<(f64, f64)>>` mapping metric names to `(raw_value, scaled_value)` tuples, plus the number of actual I/O operations performed.

Protocol-specific optimizations (e.g., Modbus register coalescing) are handled internally by each reader implementation — the collector doesn't need to know about batch capabilities.

## Implementations

| Protocol | Internal optimization | Notes |
|----------|----------------------|-------|
| [Modbus TCP/RTU](modbus.md) | Register coalescing | Coalesces adjacent register ranges automatically |
| [I2C](i2c.md) | None | Single-device reads |
| [SPI](spi.md) | None | Single-device reads |
| [I3C](i3c.md) | None | Single-device reads |

> **Note:** Non-Modbus readers (I2C, SPI, I3C) always return `metrics.len()` for `io_count`, since each metric requires a separate I/O operation (no coalescing).

## Source Layout

```text
src/reader/
  mod.rs              — MetricReader trait, MetricReaderFactory
  decoder.rs          — Register/byte decoding
  decoder_tests.rs
  modbus/
    mod.rs            — Modbus MetricReader impl
    tcp.rs            — TCP transport
    tcp_tests.rs
    rtu.rs            — RTU transport
    rtu_tests.rs
    batch.rs          — Register coalescing
    batch/
      batch_tests.rs
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
