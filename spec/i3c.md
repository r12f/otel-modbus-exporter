# I3C Client Specification

> Part of the [bus-exporter architecture](../README.md#architecture). The I3C reader implements the `MetricReader` trait.

## Overview

The I3C module provides an async client for reading data from I3C devices on Linux.
I3C (Improved Inter-Integrated Circuit) is the successor to I2C, offering higher speed,
dynamic addressing, and in-band interrupts. Like the I2C, SPI, and Modbus clients,
it abstracts device communication behind a common read interface so the collector and
export pipeline are fully reusable.

## Platform

- **Linux only** — uses the Linux I3C subsystem (`/sys/bus/i3c/devices/`) and
  `/dev/i3c-N` device files.
- Requires kernel I3C support (v4.20+) with appropriate controller drivers.
- The process must have read/write permission on the I3C device file (write permission is required for `init_writes` and `pre_poll`).

## Crate

- Direct ioctl via the Linux I3C character device interface.
- Wrap in `tokio::task::spawn_blocking` since I3C operations are synchronous.

## Address Resolution

I3C supports three mutually exclusive address modes. Exactly one must be configured
per device. If zero or multiple modes are set, validation fails at config load.

### Mode 1: Provisioned ID (`pid`) — Preferred

The 48-bit Provisioned ID (PID) uniquely identifies each I3C device. At bus
initialization, the controller enumerates devices and assigns dynamic addresses.
The exporter resolves the PID to a dynamic address by scanning
`/sys/bus/i3c/devices/` for a matching PID entry.

```yaml
protocol:
  type: i3c
  bus: "/dev/i3c-0"
  pid: "0x0123456789AB"
```

### Mode 2: Static Address (`address`)

For devices with a pre-assigned static address (common during bring-up or for
legacy I3C devices), the address can be specified directly.

```yaml
protocol:
  type: i3c
  bus: "/dev/i3c-0"
  address: 0x30
```

### Mode 3: Device Class Discovery (`device_class` + `instance`)

For environments with known device types but dynamic topology, devices can be
identified by class name and instance index. The exporter queries the I3C subsystem
for devices matching the class, then selects by instance index.

```yaml
protocol:
  type: i3c
  bus: "/dev/i3c-0"
  device_class: "temperature-sensor"
  instance: 0
```

### Resolution Priority

When validating, the exporter checks for address modes in this order:

1. **`pid`** — highest priority
2. **`address`** — static/legacy fallback
3. **`device_class` + `instance`** — discovery-based

Exactly one mode must be present. Setting multiple modes is a validation error.

## Bus Interaction

### Linux I3C Subsystem

The exporter interacts with I3C through two interfaces:

1. **`/sys/bus/i3c/devices/`** — sysfs entries for device enumeration and PID
   matching. Each device directory exposes attributes including `pid`, `dcr`
   (Device Characteristic Register), and `bcr` (Bus Characteristic Register).

2. **`/dev/i3c-N`** — controller character device for data transfers (where `N` is
   the controller index, not a per-device file). Individual device access is
   routed through the controller using the resolved dynamic address. Register
   reads follow the same write-address-then-read pattern as I2C.

### Address Resolution Phase

Address resolution is a separate phase from data reading:

1. On startup, enumerate devices on the bus via `/sys/bus/i3c/devices/`.
2. Match the configured address mode (PID lookup, static address, or class discovery).
3. Resolve to a dynamic address for subsequent reads.
4. Cache the resolved address for the polling lifecycle.

## Re-enumeration on Bus Reset

I3C buses can be reset (by the controller or external events), which invalidates
all dynamic address assignments. The exporter handles this:

1. On **NACK** or transfer error, assume a potential bus reset.
2. Re-enumerate devices via `/sys/bus/i3c/devices/` before retrying.
3. Re-resolve the configured address mode to obtain the new dynamic address.
4. Retry the read with the updated address.
5. If re-resolution fails after **3 attempts** (with exponential backoff: 100ms,
   500ms, 2s), report `read_error` to the collector and skip that polling cycle.
6. The next polling cycle will attempt re-resolution again from scratch.

## Configuration

### Full Example

```yaml
collectors:
  # PID-based addressing (preferred)
  - name: "temp-sensor-pid"
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      pid: "0x0123456789AB"
    polling_interval: "5s"
    init_writes:                     # optional: one-time setup on startup/reconnect
      - address: 0x20
        value: 0x01                  # e.g., set measurement mode
    pre_poll:                        # optional: trigger before each read cycle
      - address: 0x20
        value: 0x02                  # e.g., trigger measurement
      - delay: "20ms"               # wait for conversion
    metrics:
      - name: temperature
        type: gauge
        address: 0xFA
        data_type: u16
        byte_order: big_endian
        scale: 0.01
        offset: -40.0
        unit: "°C"

  # Static address
  - name: "temp-sensor-static"
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      address: 0x30
    polling_interval: "5s"
    metrics:
      - name: humidity
        type: gauge
        address: 0xFD
        data_type: u16
        byte_order: big_endian
        scale: 0.01
        unit: "%"

  # Discovery-based
  - name: "temp-sensor-discovery"
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      device_class: "temperature-sensor"
      instance: 0
    polling_interval: "5s"
    metrics:
      - name: temperature
        type: gauge
        address: 0xFA
        data_type: u16
        byte_order: big_endian
        scale: 0.01
        offset: -40.0
        unit: "°C"
```

### Protocol Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | `string` | Yes | — | Must be `"i3c"` |
| `bus` | `string` | Yes | — | I3C bus device path (e.g., `/dev/i3c-0`) |
| `pid` | `string` | Mode 1 | — | 48-bit Provisioned ID as hex string (e.g., `"0x0123456789AB"`) |
| `address` | `u8` | Mode 2 | — | Static I3C device address (`0x08`–`0x3D`) |
| `device_class` | `string` | Mode 3 | — | Device class name for discovery |
| `instance` | `u8` | Mode 3 | — | Instance index when using `device_class` |

### Metric Fields (I3C-specific)

I3C metrics reuse the standard `Metric` schema with these differences:

| Field | Modbus equivalent | I3C behavior |
|-------|-------------------|--------------|
| `register_type` | `holding`/`input`/`coil`/`discrete` | **Not used** — omit or ignore |
| `address` | Modbus register address | I3C register address (0x00–0xFF) |
| `data_type` | Same | Same: `u8`, `u16`, `i16`, `u32`, `i32`, `f32`, `u64`, `i64`, `f64`, `bool` |
| `byte_order` | Same | Same (applies to multi-byte reads). `mid_big_endian`/`mid_little_endian` are Modbus-specific — use only `big_endian`/`little_endian` for I3C. |

**`u8` data type** is available for I3C (single byte read, same as I2C).

## Write Operations

I3C writes are used by `init_writes` and `pre_poll` (see [config.md](config.md#register-writes)):

1. **Write** the register address byte followed by the value byte(s).

I3C uses the same write-register model as I2C. Sensors on I3C buses often need initialization or measurement triggers just like their I2C counterparts.

## Read Operations

I3C read follows the same **write-register-then-read** pattern as I2C:

1. **Resolve** the device address (from PID, static address, or class discovery).
2. **Write** the register address byte to the device.
3. **Read** N bytes from the device.

Byte counts by data type:

| Data Type | Bytes Read |
|-----------|-----------|
| `bool` | 1 (bit 0) |
| `u8` | 1 |
| `u16`, `i16` | 2 |
| `u32`, `i32`, `f32` | 4 |
| `u64`, `i64`, `f64` | 8 |

## Bus Locking

I3C buses are shared — multiple devices on the same bus. The kernel handles low-level
bus arbitration, but we must ensure only one operation per bus at a time from our process:

- **One connection per bus** (not per device). Multiple collectors on the same bus
  share a single file descriptor.
- Use a `tokio::sync::Mutex` per bus path to serialize access.
- Different buses (`/dev/i3c-0` vs `/dev/i3c-1`) are independent.

## Error Handling

- **Device not found**: If the I3C bus file doesn't exist → error on startup.
- **PID not found**: No device with the configured PID on the bus → error on startup
  (retried on bus reset).
- **Device class not found**: No device matching class/instance → error on startup
  (retried on bus reset).
- **NACK**: Device doesn't acknowledge → trigger re-enumeration, then retry.
  If still failing, report `read_error` to collector.
- **Bus reset**: Detected via NACK or transfer error → re-enumerate and re-resolve
  addresses before retry.
- **Permission denied**: Clear error message suggesting `udev` rules or running as root.
- All errors include context: collector name, bus path, address mode details, register.

## Validation Rules

1. `bus` must be a valid path (checked at config load, opened at runtime).
2. Exactly one address mode must be set: `pid`, `address`, or `device_class` + `instance`.
   Zero or multiple modes is a validation error.
3. `pid` must be a valid 48-bit hex string (6 bytes, e.g., `"0x0123456789AB"`).
4. `address` must be in the I3C dynamic address range: `0x08`–`0x3D`.
5. `device_class` requires `instance` to also be set (and vice versa).
6. `register_type` is ignored for I3C collectors (not validated).
7. `data_type: bool` reads one byte and extracts bit 0.
8. `slave_id` is **not used** for I3C — omit from collector config.
9. `byte_order` only supports `big_endian` and `little_endian` (`mid_*` variants
   are Modbus-specific).
