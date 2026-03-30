# SPI Client Specification

> Part of the [bus-exporter architecture](../README.md#architecture). The SPI reader implements the `MetricReader` trait.

## Overview

The SPI module provides an async client for reading data from SPI devices on Linux.
Like the I2C and Modbus clients, it abstracts device communication behind a common
read interface so the collector and export pipeline are fully reusable.

## Platform

- **Linux only** — uses `/dev/spidevB.C` device files (bus B, chip-select C).
- Requires the `spidev` kernel module.
- The process must have read/write permission on the SPI device file.

## Crate

- **`spidev`** crate for Linux SPI access via ioctl.
- Wrap in `tokio::task::spawn_blocking` since SPI operations are synchronous.

## Configuration

```yaml
collectors:
  - name: "adc-ch0"
    protocol:
      type: spi
      device: "/dev/spidev0.0"    # SPI bus.chip-select
      speed_hz: 1000000            # Clock speed in Hz
      mode: 0                      # SPI mode (0-3)
    polling_interval: "1s"
    init_writes:                    # optional: one-time setup on startup/reconnect
      - command: [0x01, 0x83]      # e.g., write config register
    pre_poll:                       # optional: trigger before each read cycle
      - command: [0x01, 0x80]      # e.g., start conversion
      - delay: "10ms"              # wait for conversion
    metrics:
      - name: adc_voltage
        type: gauge
        command: [0x06, 0x00, 0x00]   # bytes to send
        response_length: 3             # bytes to read back
        response_offset: 1             # skip first N bytes of response
        data_type: u16
        byte_order: big_endian
        scale: 0.000805               # 3.3V / 4096 for 12-bit ADC
        unit: "V"
```

### Protocol Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | `string` | Yes | — | Must be `"spi"` |
| `device` | `string` | Yes | — | SPI device path (e.g., `/dev/spidev0.0`) |
| `speed_hz` | `u32` | No | `1000000` | SPI clock speed in Hz |
| `mode` | `u8` | No | `0` | SPI mode: `0`, `1`, `2`, or `3` |
| `bits_per_word` | `u8` | No | `8` | Bits per word |

### Metric Fields (SPI-specific)

SPI devices don't have a standard register model like Modbus or I2C. Instead, each
read is a **command → response** transfer. SPI metrics use different addressing fields:

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | `string` | Yes | — | Metric name |
| `type` | `string` | Yes | — | `"counter"` or `"gauge"` |
| `command` | `list<u8>` | Yes | — | Bytes to transmit (TX buffer) |
| `response_length` | `u16` | No | auto | Total response bytes. Defaults to `command` length since SPI is full-duplex (TX and RX are the same length). Set explicitly if response is longer than command. |
| `response_offset` | `u16` | No | `0` | Skip first N bytes of response before decoding |
| `data_type` | `string` | Yes | — | `u8`, `u16`, `i16`, `u32`, `i32`, `f32`, `u64`, `i64`, `f64`, `bool` |
| `byte_order` | `string` | No | `"big_endian"` | `big_endian` or `little_endian` only (`mid_*` are Modbus-specific) |
| `scale` | `f64` | No | `1.0` | Scale factor |
| `offset` | `f64` | No | `0.0` | Additive offset |
| `unit` | `string` | No | `""` | Unit label |
| `description` | `string` | No | `""` | Human-readable description |

**Not used** for SPI: `register_type`, `address` (replaced by `command`), `slave_id`.

## Write Operations

SPI writes are used by `init_writes` and `pre_poll` (see [config.md](config.md#register-writes)):

1. **Transmit** the `command` bytes to the device.

SPI has no register addressing model — writes are just byte sequences sent over the bus. Common patterns:

| Device | Init Writes | Pre-Poll |
|--------|------------|----------|
| AD7124 (SPI ADC) | Configure filter, gain, channel map | Start conversion |
| MAX31855 | — | — (always ready) |
| MCP3008 | — | — (conversion per read) |

For SPI, the `WriteStep` uses `command` instead of `address`+`value`:

```yaml
init_writes:
  - command: [0x01, 0x83]     # bytes to transmit
pre_poll:
  - command: [0x01, 0x80]     # start conversion
  - delay: "10ms"
```

## Read Operations

SPI is full-duplex — data is clocked in and out simultaneously:

1. Build TX buffer from `command` bytes, zero-padded to `response_length` if needed.
2. Perform SPI `transfer` (simultaneous TX/RX).
3. Extract payload from RX buffer starting at `response_offset`.
4. Decode according to `data_type` and `byte_order`.
5. Apply `scale` and `offset`.

### Example: MCP3008 12-bit ADC (channel 0)

```text
TX: [0x06, 0x00, 0x00]    → start bit + single-ended + channel 0
RX: [0x??, 0x0N, 0xNN]    → ignore first byte, bits [1:0] of byte 2 + byte 3 = 10-bit value
```

Config:

```yaml
command: [0x06, 0x00, 0x00]
response_length: 3
response_offset: 1       # skip first RX byte
data_type: u16           # decode 2 bytes
byte_order: big_endian
scale: 0.003222          # 3.3V / 1023 for 10-bit
```

## Device Sharing

Each SPI chip-select (`/dev/spidevB.C`) is a separate device — no bus sharing
concern like I2C. However:

- **One file descriptor per device path**. Multiple collectors using the same
  device path share the FD with a `tokio::sync::Mutex`.
- Different chip-selects on the same bus are independent at the kernel level.

## Error Handling

- **Device not found**: SPI device file doesn't exist → error on startup.
- **Permission denied**: Clear error suggesting `udev` rules or group membership.
- **Transfer failure**: Kernel ioctl error → `read_error`, reported to collector.
- **Timeout**: SPI transfers are synchronous at the kernel level with no user-configurable timeout. The `spawn_blocking` wrapper uses the collector's `polling_interval` as an implicit upper bound.
- All errors include context: collector name, device path, command bytes.

## Validation Rules

1. `device` must be a valid path (checked at config load, opened at runtime).
2. `mode` must be 0–3.
3. `speed_hz` must be > 0.
4. `command` must have at least one byte.
5. `response_length` must be ≥ bytes needed for `data_type` + `response_offset`.
6. `response_offset` + data_type bytes must not exceed `response_length`.
7. `register_type`, `address`, and `slave_id` are **not used** for SPI.
8. `bits_per_word` must be 1–32 (kernel constraint).
