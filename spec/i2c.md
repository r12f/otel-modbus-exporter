# I2C Client Specification

> Part of the [bus-exporter architecture](../README.md#architecture). The I2C reader implements the `MetricReader` trait.

## Overview

The I2C module provides an async client for reading registers from I2C devices on Linux.
It follows the same pattern as the Modbus clients — abstracting device communication behind
a common read interface so the collector and export pipeline are fully reusable.

## Platform

- **Linux only** — uses `/dev/i2c-N` device files via the `i2c-dev` kernel interface.
- Requires the `i2c-dev` kernel module loaded (`modprobe i2c-dev`).
- The process must have read/write permission on the I2C device file (write permission is required for `init_writes` and `pre_poll`).

## Crate

- **`linux-embedded-hal`** for I2C access, or **`i2cdev`** (`i2c-linux` crate) for direct ioctl.
- Wrap in `tokio::task::spawn_blocking` since I2C operations are synchronous.

## Configuration

```yaml
collectors:
  - name: "bme280"
    protocol:
      type: i2c
      bus: "/dev/i2c-1"     # I2C bus device path
      address: 0x76          # 7-bit device address (hex or decimal)
    polling_interval: "5s"
    init_writes:              # optional: one-time setup on startup/reconnect
      - address: 0xF2
        value: 0x01           # humidity oversampling x1
      - address: 0xF4
        value: 0x24           # temp x1, pressure x1, sleep mode
    pre_poll:                 # optional: trigger before each read cycle
      - address: 0xF4
        value: 0x25           # forced mode trigger (sleep → forced)
      - delay: "50ms"         # wait for conversion
    metrics:
      - name: temperature
        type: gauge
        address: 0xFA        # I2C register address to read
        data_type: u16
        byte_order: big_endian
        scale: 0.01
        offset: -40.0
        unit: "°C"
```

### Protocol Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | `string` | Yes | — | Must be `"i2c"` |
| `bus` | `string` | Yes | — | I2C bus device path (e.g., `/dev/i2c-1`) |
| `address` | `u8` | Yes | — | 7-bit I2C device address (`0x03`–`0x77`) |

### Metric Fields (I2C-specific)

I2C metrics reuse the standard `Metric` schema with these differences:

| Field | Modbus equivalent | I2C behavior |
|-------|-------------------|--------------|
| `register_type` | `holding`/`input`/`coil`/`discrete` | **Not used** — omit or ignore |
| `address` | Modbus register address | I2C register address (0x00–0xFF) |
| `data_type` | Same | Same: `u8`, `u16`, `i16`, `u32`, `i32`, `f32`, `u64`, `i64`, `f64`, `bool` |
| `byte_order` | Same | Same (applies to multi-byte reads). `mid_big_endian`/`mid_little_endian` are Modbus-specific — use only `big_endian`/`little_endian` for I2C. |

**New data type**: `u8` — single byte read, common in I2C. Not available in Modbus
(which is 16-bit register based).

## Read Operations

I2C read follows the standard **write-register-then-read** pattern:

1. **Write** the register address byte to the device.
2. **Read** N bytes from the device.

## Write Operations

I2C writes are used by `init_writes` and `pre_poll` (see [config.md](config.md#register-writes)):

1. **Write** the register address byte followed by the value byte(s).

Many sensors require writes to configure operating modes, oversampling, or trigger measurements before registers contain valid data. Common patterns:

| Sensor | Init Writes | Pre-Poll |
|--------|------------|----------|
| BME280/BME680 | Set oversampling registers | Trigger forced mode |
| ADS1115 | Configure multiplexer, gain, rate | Start conversion |
| SHT31 | — | Send measurement command |

See the collector `init_writes` / `pre_poll` fields for the full schema.

Byte counts by data type:

| Data Type | Bytes Read |
|-----------|-----------|
| `bool` | 1 (bit 0) |
| `u8` | 1 |
| `u16`, `i16` | 2 |
| `u32`, `i32`, `f32` | 4 |
| `u64`, `i64`, `f64` | 8 |

## Bus Locking

I2C buses are shared — multiple devices on the same bus. The kernel handles low-level
bus arbitration, but we must ensure only one operation per bus at a time from our process:

- **One connection per bus** (not per device). Multiple collectors on the same bus
  share a single file descriptor.
- Use a `tokio::sync::Mutex` per bus path to serialize access.
- Different buses (`/dev/i2c-1` vs `/dev/i2c-2`) are independent.

## Error Handling

- **Device not found**: If the I2C bus file doesn't exist → error on startup.
- **NACK**: Device doesn't acknowledge → `read_error`, reported to collector.
- **Timeout**: Kernel-level timeout (not configurable from userspace).
- **Permission denied**: Clear error message suggesting `udev` rules or running as root.
- All errors include context: collector name, bus path, device address, register.

## Validation Rules

1. `bus` must be a valid path (checked at config load, opened at runtime).
2. `address` must be in the valid 7-bit range: `0x03`–`0x77`.
3. `register_type` is ignored for I2C collectors (not validated).
4. `data_type: bool` reads one byte and extracts bit 0.
5. `slave_id` is **not used** for I2C — omit from collector config.
