# E2E Testing Specification

## Overview

E2E tests validate the full pipeline for multiple protocols: Modbus TCP, Modbus RTU, I2C, and SPI. All tests use a shared test harness in `tests/common/mod.rs` — no Docker required.

## Architecture

### Shared Test Harness (`tests/common/mod.rs`)

| Component | Description |
|-----------|-------------|
| `TestFixtures` / `standard_fixtures()` | Shared test data (register values, expected metrics) |
| `ConnectionParams` enum | Protocol-specific connection info: `ModbusTcp`, `ModbusRtu`, `I2c`, `Spi` |
| `generate_config()` | Generates a YAML config from fixtures + connection params |
| `run_pull()` | Runs `bus-exporter pull` as an async child process |
| `validate()` | Asserts metric output matches expected values |
| `run_e2e_workflow()` | Orchestrates the full flow: config → pull → assert → validate |

```text
Test harness (generate_config + simulator) → bus-exporter pull (child process) → JSON output → validate()
```

## Protocol-Specific Tests

### `e2e_modbus.rs` — Modbus TCP

- Starts an **in-process Modbus TCP simulator** using `tokio-modbus` server API on a random port.
- Generates test config pointing at the simulator.
- Runs `bus-exporter pull` and validates JSON output.
- No special requirements — runs on any machine with Rust toolchain.

### `e2e_modbus_rtu.rs` — Modbus RTU

- Uses **socat** to create a virtual serial pair.
- Spawns a mock RTU responder that handles Modbus RTU frames with CRC-16.
- Generates test config with the virtual serial device.
- Marked `#[ignore]` — requires `socat` installed.

### `e2e_i2c.rs` — I2C

- Uses the **i2c-stub** kernel module to create a virtual I2C bus.
- Pre-loads register values into the stub device.
- Marked `#[ignore]` — requires root and `i2c-stub` kernel module.

### `e2e_spi.rs` — SPI

- Uses **spidev loopback** for full-duplex testing.
- Marked `#[ignore]` — requires a spidev device.
- Also contains a non-ignored `spi_config_generation` unit test that validates SPI config generation without hardware.

## Running

```bash
# Run non-ignored tests only (Modbus TCP + SPI config generation)
cargo test --test 'e2e_*'

# Run hardware-dependent tests only (requires socat, root, spidev)
cargo test --test 'e2e_*' -- --ignored

# Run all e2e tests
cargo test --test 'e2e_*' -- --include-ignored
```

## CI Integration

- E2E tests run in the `e2e` job in `.github/workflows/ci.yml`:

  ```bash
  cargo test --test 'e2e_*' -- --nocapture
  ```

- Hardware-dependent tests are auto-skipped via `#[ignore]`.
- No Docker required — runs on any GitHub-hosted runner with Rust toolchain.
