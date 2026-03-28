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
| `endpoint` | `string` | Yes (if enabled) | — | OTLP HTTP endpoint (e.g., `http://host:4318`) |
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
| `slave_id` | `u8` | Modbus only | — | Modbus slave/unit ID (1-247). Not used for I2C/SPI. |
| `polling_interval` | `string` | No | `"10s"` | Poll interval (duration string) |
| `labels` | `map<string, string>` | No | `{}` | Labels for all metrics in this collector |
| `metrics_files` | `list<string>` | No | `[]` | Paths to metrics definition files (see [Metrics Files](#metrics-files)) |
| `metrics` | `list<Metric>` | No | `[]` | Inline metric definitions |

A collector must have at least one metric after merging `metrics_files` and `metrics`.

### Protocol

#### TCP

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | `string` | Yes | — | Must be `"modbus-tcp"` |
| `endpoint` | `string` | Yes | — | `host:port` |

#### RTU

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
| `address` | `u16` | Yes | — | 7-bit device address (`0x03`–`0x77`) |

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
| `output` | `string` | No | `"syslog"` | Output target: `syslog`, `stdout`, `stderr` |
| `syslog_facility` | `string` | No | `"daemon"` | Syslog facility (e.g., `daemon`, `local0`–`local7`) |

```yaml
logging:
  level: "info"              # trace|debug|info|warn|error
  output: "syslog"           # syslog|stdout|stderr
  syslog_facility: "daemon"
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
14. `polling_interval` must be ≥ 100ms.
15. `scale` must not be zero.
16. `counter` metric type is not compatible with `coil`/`discrete` register types or `bool` data type (counters must be numeric).
17. `data_type: u8` is only valid for I2C and SPI collectors (Modbus registers are 16-bit minimum).

## Scale Formula

```text
output_value = raw_value * scale + offset
```

Example: raw register value `245` with `scale: 0.1` and `offset: -40.0` → `245 * 0.1 + (-40.0) = -15.5`
