# Configuration Specification

## Overview

Configuration is loaded from a YAML file. The file location is determined as follows:

### Config File Resolution

If `--config <path>` is specified, use that path exactly (error if not found).

If `--config` is **not** specified, search in order (first match wins):

1. `./config.yaml` (current working directory — highest priority)
2. `~/.config/bus-exporter/config.yaml` (user config)
3. `/etc/bus-exporter/config.yaml` (system config)

If none found, exit with an error listing all searched paths.

### Path Resolution

All relative paths within the config file (e.g., `metrics_files` entries) are resolved relative to the **parent directory of the config file that was loaded**, not the current working directory.

Example: config loaded from `~/.config/bus-exporter/config.yaml` with `metrics_files: ["devices/sdm630.yaml"]` → resolves to `~/.config/bus-exporter/devices/sdm630.yaml`.

## Example

See [`config/example.yaml`](../config/example.yaml) for a complete annotated example.

## Schema

### Top-level

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `global_labels` | `map<string, string>` | No | `{}` | Labels applied to all metrics |
| `logging` | `Logging` | No | See below | Logging configuration |
| `exporters` | `Exporters` | Yes | — | Export configuration |
| `collectors` | `list<Collector>` | Yes | — | At least one collector required |

### Exporters

#### `exporters.otlp`

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `enabled` | `bool` | No | `false` | Enable OTLP export |
| `endpoint` | `string` | Yes (if enabled) | — | Full OTLP HTTP endpoint including signal path (e.g., `http://host:4318/v1/metrics`) |
| `timeout` | `string` | No | `"10s"` | Request timeout (duration string) |
| `headers` | `map<string, string>` | No | `{}` | Additional HTTP headers |

#### `exporters.prometheus`

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `enabled` | `bool` | No | `false` | Enable Prometheus endpoint |
| `listen` | `string` | No | `"0.0.0.0:9090"` | Listen address |
| `path` | `string` | No | `"/metrics"` | Metrics path |

#### `exporters.mqtt`

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `enabled` | `bool` | No | `false` | Enable MQTT export |
| `endpoint` | `string` | Yes (if enabled) | — | Broker URL (`mqtt://host:port` or `mqtts://host:port`) |
| `client_id` | `string` | No | auto-generated | MQTT client identifier |
| `topic_prefix` | `string` | No | `"modbus/metrics"` | Base topic prefix |
| `auth` | `MqttAuth` | No | — | Authentication credentials |
| `tls` | `MqttTls` | No | — | TLS configuration (for `mqtts://`) |
| `qos` | `u8` | No | `1` | QoS level: `0`, `1`, or `2` |
| `retain` | `bool` | No | `false` | Retain flag on metric messages |
| `interval` | `string` | No | `"10s"` | Publish interval (duration string) |
| `timeout` | `string` | No | `"5s"` | Connection/publish timeout |

##### `exporters.mqtt.auth`

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `username` | `string` | Yes | — | MQTT username |
| `password` | `string` | Yes | — | MQTT password |

##### `exporters.mqtt.tls`

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `ca_cert` | `string` | No | system roots | Path to CA certificate |
| `client_cert` | `string` | No | — | Path to client certificate (mutual TLS) |
| `client_key` | `string` | No | — | Path to client private key (mutual TLS) |
| `insecure` | `bool` | No | `false` | Skip server certificate verification |

See [export-mqtt.md](export-mqtt.md) for full MQTT export specification.

### Collector

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | `string` | Yes | — | Unique collector name (used as label) |
| `protocol` | `Protocol` | Yes | — | Connection protocol |
| `slave_id` | `u8` | Modbus only | — | Modbus slave/unit ID (1-247). Not used for I2C/SPI/I3C. |
| `polling_interval` | `string` | No | `"10s"` | Poll interval (duration string) |
| `init_writes` | `list<WriteStep>` | No | `[]` | Register writes executed once at startup/reconnect. I2C/SPI/I3C only. See [Register Writes](#register-writes). |
| `pre_poll` | `list<WriteStep>` | No | `[]` | Register writes executed before each poll cycle. I2C/SPI/I3C only. See [Register Writes](#register-writes). |
| `labels` | `map<string, string>` | No | `{}` | Labels for all metrics in this collector |
| `metrics_files` | `list<string>` | No | `[]` | Paths to metrics definition files (see [Metrics Files](#metrics-files)) |
| `metrics` | `list<Metric>` | No | `[]` | Inline metric definitions |

A collector must have at least one metric after merging `metrics_files` and `metrics`.

### Register Writes

`init_writes` and `pre_poll` allow register writes for device initialization and measurement triggering. **Only valid for I2C, SPI, and I3C collectors** — a validation error is raised if set on Modbus collectors.

The write step schema differs by protocol — I2C/I3C use register addressing, while SPI uses raw byte commands.

#### WriteStep (I2C / I3C)

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `address` | `u8` | Conditional | — | Register address to write, `0x00`–`0xFF` (omit for delay-only steps) |
| `value` | `u8 \| list<u8>` | Conditional | — | Byte(s) to write. Required if `address` is set. Single integer or list for multi-byte. |
| `delay` | `string` | No | — | Duration to wait after this step (e.g., `"50ms"`, `"1s"`) |

A step must have at least one of `address`+`value` or `delay`. A step with both writes first, then waits.

#### WriteStep (SPI)

SPI has no register addressing — writes are raw byte sequences sent over the bus.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `command` | `list<u8>` | Conditional | — | Bytes to transmit (omit for delay-only steps) |
| `delay` | `string` | No | — | Duration to wait after this step |

#### Example: BME680 on I2C

```yaml
collectors:
  - name: "bme680"
    protocol:
      type: i2c
      bus: "/dev/i2c-1"
      address: 0x76
    polling_interval: "5s"
    init_writes:
      - address: 0x72
        value: 0x01              # humidity oversampling x1
      - address: 0x74
        value: 0x24              # temp x1, pressure x1, sleep mode
    pre_poll:
      - address: 0x74
        value: 0x25              # forced mode trigger (sleep → forced)
      - delay: "50ms"            # wait for measurement
    metrics:
      - name: temperature
        type: gauge
        address: 0x22            # BME680 temp MSB register
        data_type: u16
        byte_order: big_endian
        scale: 0.01
        offset: -40.0
        unit: "°C"
```

#### Validation

- `init_writes` and `pre_poll` on Modbus TCP/RTU collectors → **validation error**.
- Each step must have at least one of:
  - `address` **and** `value` (I2C/I3C write step),
  - `command` (SPI write step), or
  - `delay`.
- For I2C/I3C steps, `address` and `value` must appear together: specifying one without the other is a **validation error**.
- For I2C/I3C steps, `value` must represent at least one byte — `value: []` is a **validation error**.
- For SPI steps, `command` must contain at least one byte — `command: []` is a **validation error**.
- `delay` values must be valid duration strings and ≤ 10s (to prevent blocking the poll loop).

### Protocol

#### Modbus TCP

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | `string` | Yes | — | Must be `"modbus-tcp"` |
| `endpoint` | `string` | Yes | — | `host:port` |

#### Modbus RTU

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | `string` | Yes | — | Must be `"modbus-rtu"` |
| `device` | `string` | Yes | — | Serial device path (e.g., `/dev/ttyUSB0`) |
| `bps` | `u32` | No | `9600` | Baud rate |
| `data_bits` | `u8` | No | `8` | Data bits (5-8) |
| `stop_bits` | `u8` | No | `1` | Stop bits (1-2) |
| `parity` | `string` | No | `"none"` | `"none"`, `"even"`, or `"odd"` |

#### I2C

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | `string` | Yes | — | Must be `"i2c"` |
| `bus` | `string` | Yes | — | I2C bus device path (e.g., `/dev/i2c-1`) |
| `address` | `u8` | Yes | — | 7-bit device address (`0x03`–`0x77`) |

See [i2c.md](i2c.md) for full I2C specification.

#### SPI

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | `string` | Yes | — | Must be `"spi"` |
| `device` | `string` | Yes | — | SPI device path (e.g., `/dev/spidev0.0`) |
| `speed_hz` | `u32` | No | `1000000` | SPI clock speed in Hz |
| `mode` | `u8` | No | `0` | SPI mode: `0`, `1`, `2`, or `3` |
| `bits_per_word` | `u8` | No | `8` | Bits per word |

See [spi.md](spi.md) for full SPI specification.

#### I3C

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | `string` | Yes | — | Must be `"i3c"` |
| `bus` | `string` | Yes | — | I3C bus device path (e.g., `/dev/i3c-0`) |
| `pid` | `string` | Mode 1 | — | 48-bit Provisioned ID as hex string (e.g., `"0x0123456789AB"`) |
| `address` | `u8` | Mode 2 | — | Static I3C device address (`0x08`–`0x3D`) |
| `device_class` | `string` | Mode 3 | — | Device class name for discovery |
| `instance` | `u8` | Mode 3 | — | Instance index when using `device_class` |

Exactly one address mode must be set: `pid`, `address`, or `device_class` + `instance`.
Setting zero or multiple modes is a validation error. `pid` must be a valid 48-bit hex
string. `address` must be in range `0x08`–`0x3D`. `device_class` requires `instance`
(and vice versa).

See [i3c.md](i3c.md) for full I3C specification.

### Metric

The metric schema varies slightly by protocol. Modbus uses `address` + `register_type`.
I2C uses `address` only (no register types). SPI uses `command` + `response_length` + `response_offset`. Common fields apply to all.

#### Common Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | `string` | Yes | — | Metric name (snake_case recommended) |
| `description` | `string` | No | `""` | Human-readable description |
| `type` | `string` | Yes | — | `"counter"` or `"gauge"` |
| `data_type` | `string` | Yes | — | One of: `u8`, `u16`, `i16`, `u32`, `i32`, `f32`, `u64`, `i64`, `f64`, `bool` |
| `byte_order` | `string` | No | `"big_endian"` | `"big_endian"`, `"little_endian"`, `"mid_big_endian"`, `"mid_little_endian"` |
| `scale` | `f64` | No | `1.0` | Multiplicative scale factor |
| `offset` | `f64` | No | `0.0` | Additive offset |
| `unit` | `string` | No | `""` | Unit label (e.g., `"V"`, `"kWh"`, `"°C"`) |

#### Modbus-specific Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `register_type` | `string` | Yes | — | `"holding"`, `"input"`, `"coil"`, or `"discrete"` |
| `address` | `u16` | Yes | — | Starting register address (0-based) |

Note: `u8` data type is **not** available for Modbus (16-bit register based). `data_type: bool` requires `coil` or `discrete` register type.

#### I2C-specific Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `address` | `u16` | Yes | — | I2C register address (0x00–0xFF) |

Note: `register_type` is not used for I2C. `u8` data type is available.

#### SPI-specific Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `command` | `list<u8>` | Yes | — | Bytes to transmit (TX buffer) |
| `response_length` | `u16` | No | auto | Total response bytes (defaults to `command` length — SPI is full-duplex) |
| `response_offset` | `u16` | No | `0` | Skip first N bytes of response before decoding |

Note: `register_type` and `address` are not used for SPI. `u8` data type is available.

#### I3C-specific Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `address` | `u8` | Yes | — | I3C register address (0x00–0xFF) |

Note: `register_type` is not used for I3C. `u8` data type is available. `byte_order` supports only `big_endian`/`little_endian` (`mid_*` variants are Modbus-specific).

### Metrics Files

Metrics files allow reusable metric definitions across multiple collectors with the same device type.

#### File Path Resolution

Relative paths are resolved against the **config file's parent directory**.
Example: config at `/etc/bus-exporter/config.yaml` + `metrics_files: ["devices/sdm630.yaml"]` → `/etc/bus-exporter/devices/sdm630.yaml`.
Absolute paths are used as-is.

#### File Format

```yaml
# Optional: shared defaults applied to all metrics in this file.
# Individual metrics can override any default field.
defaults:
  register_type: holding
  data_type: f32
  byte_order: big_endian

metrics:
  - name: voltage_l1
    description: "Phase 1 line-to-neutral voltage"
    type: gauge
    address: 0
    unit: "V"
  - name: current_l1
    description: "Phase 1 current"
    type: gauge
    address: 6
    unit: "A"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `defaults` | `Partial<Metric>` | No | Default values applied to every metric in this file. Only non-required fields (`description`, `type`, `register_type`, `data_type`, `byte_order`, `scale`, `offset`, `unit`) may be set. |
| `metrics` | `list<Metric>` | Yes | At least one metric. Each metric inherits from `defaults`, then its own fields override. |

#### Merge Order

Metrics are merged by **name** (the `name` field is the key):

1. Process `metrics_files` in list order (first file → last file).
2. For each file, apply `defaults` to its metrics, then add/replace into the merged map by name.
3. Process inline `metrics` last — these always have the **highest priority**.
4. When a later entry has the same `name` as an earlier one, the later entry **fully replaces** the earlier one (no field-level merge). If a field is absent in the replacing entry, it is removed (reverts to schema default or becomes required-missing), not inherited from the replaced entry.

**Example:**

```yaml
# devices/base.yaml
defaults:
  register_type: holding
  data_type: f32
metrics:
  - name: voltage
    type: gauge
    address: 0
    unit: "V"
    description: "Voltage reading"
  - name: current
    type: gauge
    address: 6
    unit: "A"

# devices/override.yaml
metrics:
  - name: voltage          # replaces base.yaml's voltage entirely
    type: gauge
    register_type: input   # different register type
    data_type: f32
    address: 100
    unit: "V"

# config.yaml
collectors:
  - name: meter1
    protocol: { type: modbus-tcp, endpoint: "192.168.1.10:502" }
    slave_id: 1
    metrics_files:
      - "devices/base.yaml"       # loaded first
      - "devices/override.yaml"   # voltage replaced, current kept
    metrics:                       # inline: highest priority
      - name: power               # new metric added
        type: gauge
        register_type: holding
        data_type: f32
        address: 12
        unit: "W"
```

Result for `meter1`: `voltage` from override.yaml (no description — it wasn't in the override), `current` from base.yaml, `power` from inline.

### Logging

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `level` | `string` | No | `"info"` | Log level: `trace`, `debug`, `info`, `warn`, `error` |
| `output` | `string` | No | `"syslog"` | Output target: `syslog`, `json`, `stdout`, `stderr`. `syslog` sends to the system syslog daemon via unix socket and is the default on Linux. |
| `syslog_facility` | `string` | No | `"daemon"` | Syslog facility when output is `"syslog"` (`daemon`\|`local0`–`local7`) |

```yaml
logging:
  level: "info"              # trace|debug|info|warn|error
  output: "syslog"           # syslog|json|stdout|stderr
  syslog_facility: "daemon"  # daemon|local0-local7
```

## Validation Rules

1. At least one exporter must be enabled.
2. At least one collector must be defined.
3. Each collector must have at least one metric **after merging** `metrics_files` and `metrics`.
4. Collector names must be unique.
5. Metric names must be unique within a collector (enforced after merge — last one wins).
6. `slave_id` must be 1-247.
7. `coil` and `discrete` register types must use `data_type: bool`.
8. `bool` data type must use `coil` or `discrete` register types.
9. Duration strings must parse (e.g., `"5s"`, `"1m"`, `"500ms"`).
10. `byte_order` is ignored for `u16`, `i16`, and `bool` (single register).
11. All `metrics_files` paths must exist and be readable.
12. Each metrics file must contain a valid `metrics` list with at least one entry.
13. After merge, all required fields (`name`, `type`, `register_type`, `address`, `data_type`) must be present on every metric. Missing required fields from partial overrides are a validation error.
14. `polling_interval` must be ≥ 1ms.
15. `scale` must not be zero.
16. `counter` metric type is not compatible with `coil`/`discrete` register types or `bool` data type (counters must be numeric).
17. `data_type: u8` is only valid for I2C, I3C, and SPI collectors (Modbus registers are 16-bit minimum).
18. For I3C collectors, exactly one address mode must be set: `pid`, `address`, or `device_class` + `instance`. Zero or multiple is an error.
19. I3C `pid` must be a valid 48-bit hex string (e.g., `"0x0123456789AB"`).
20. I3C `address` must be in the dynamic address range: `0x08`–`0x3D`.
21. I3C `device_class` requires `instance` to also be set (and vice versa).

## Scale Formula

```text
output_value = raw_value * scale + offset
```

Example: raw register value `245` with `scale: 0.1` and `offset: -40.0` → `245 * 0.1 + (-40.0) = -15.5`
