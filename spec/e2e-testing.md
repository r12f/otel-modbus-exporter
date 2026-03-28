# E2E Testing Specification

## Overview

- E2E tests validate the full pipeline: Modbus device → collector → cache → Prometheus exporter
- Uses a **Rust-native Modbus TCP simulator** (via `tokio-modbus` server API) — no Docker required
- Uses Prometheus `/metrics` scrape endpoint to validate exported values

## Architecture

```text
Rust Modbus TCP simulator (in-process) → bus-exporter (child process) → Prometheus /metrics → test assertions
```

## Test Implementation

The e2e test is a Rust integration test at `tests/e2e_modbus.rs` that:

1. **Starts an in-process Modbus TCP simulator** using `tokio-modbus` server API on a random port
2. **Generates a test config** pointing bus-exporter at the simulator
3. **Starts bus-exporter** as a child process with `--config <temp-config>`
4. **Waits** for the Prometheus `/metrics` endpoint to become available
5. **Scrapes and validates** metric output (names, types, labels, values with float tolerance)
6. **Sends SIGTERM** and verifies graceful shutdown (exit code 0)

## Simulator Register Values

Pre-loaded register values matching `config/modbus-simulator.json`:

| Register Type | Address | Raw Value | Meaning | Data Type | Byte Order |
|---------------|---------|-----------|---------|-----------|------------|
| holding | 0 | 2300 | 230.0V (scale 0.1) | u16 | big_endian |
| holding | 16,17 | 1, 24464 (u32=90000) | 900.00 kWh (scale 0.01) | u32 | big_endian |
| input | 0 | 65436 (i16=-100) | 30.0°C (scale 0.1, offset +40.0) | i16 | big_endian |
| holding | 32,33 | 0x4348, 0x0000 (f32=200.0) | 200.0 Hz (scale 1.0) | f32 | big_endian |
| holding | 48,49 | 24464, 1 (u32=90000) | 900.00 kWh (scale 0.01) | u32 | mid_big_endian |

## Expected Metrics

| Metric Name | Expected Value | Type | Labels |
|-------------|---------------|------|--------|
| `bus_voltage_phase_a_V` | 230.0 | gauge | env="test", site="e2e", device="simulator" |
| `bus_total_energy_kWh` | 900.0 | counter | env="test", site="e2e", device="simulator" |
| `bus_temperature_C` | 30.0 | gauge | env="test", site="e2e", device="simulator" |
| `bus_frequency_Hz` | 200.0 | gauge | env="test", site="e2e", device="simulator" |
| `bus_total_energy_mid_kWh` | 900.0 | counter | env="test", site="e2e", device="simulator" |

## Running

```bash
# Via Makefile
make e2e

# Via cargo directly
cargo test --test e2e_modbus -- --nocapture
```

## CI Integration

- E2E tests run in the `e2e` job in `.github/workflows/ci.yml`
- No Docker required — runs on any GitHub-hosted runner with Rust toolchain
- Previously paused due to Docker Hub rate-limiting of `oitc/modbus-server`; now fully native
