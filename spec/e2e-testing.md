# E2E Testing Specification

## Overview

- E2E tests validate the full pipeline: Modbus device → collector → cache → Prometheus exporter
- Use `oitc/modbus-server` Docker image as Modbus TCP simulator
- Use Prometheus `/metrics` scrape endpoint to validate exported values

## Architecture

```
oitc/modbus-server (simulator) → modbus-exporter → Prometheus /metrics → test assertions
```

## docker-compose.test.yml

Two services:

1. **`modbus-simulator`** — uses `oitc/modbus-server` image with a JSON config (`config/modbus-simulator.json`) that pre-loads known register values (holding and input registers with deterministic values).
2. **`modbus-exporter`** — built from the local Dockerfile, configured via `config/test.yaml` pointing to `modbus-simulator:5020`. Prometheus endpoint enabled, OTLP disabled.

## Test Config

### Simulator Config (`config/modbus-simulator.json`)

Pre-load specific register values in the simulator so test assertions are deterministic:

| Register Type | Address | Raw Value | Meaning | Data Type | Byte Order |
|---------------|---------|-----------|---------|-----------|------------|
| holding | 0x0000 | 2300 | 230.0V (scale 0.1) | u16 | big_endian |
| holding | 0x0010 | 0x00015F90 (90000) | 900.00 kWh (scale 0.01) | u32 | big_endian |
| input | 0x0000 | 0xFF9C (-100) | -50.0°C (scale 0.1, offset +40.0) | i16 | big_endian |
| holding | 0x0020 | 0x43480000 (200.0) | 200.0 Hz (scale 1.0) | f32 | big_endian |
| holding | 0x0030 | 0x00015F90 (90000) | 900.00 kWh (scale 0.01) | u32 | mid_big_endian |

### Exporter Config (`config/test.yaml`)

- Maps the above registers to named metrics with known scale/offset
- Enables Prometheus exporter on a known port (e.g., `0.0.0.0:9090`)
- Disables OTLP exporter
- Single collector pointing to `modbus-simulator:5020`
- Covers all data types: u16, u32, i16, f32
- Covers byte orders: big_endian, mid_big_endian

### Expected Metrics

| Metric Name | Expected Value | Type | Labels |
|-------------|---------------|------|--------|
| `voltage_phase_a` | 230.0 | gauge | global + collector labels |
| `total_energy` | 900.0 | counter | global + collector labels |
| `temperature` | -50.0 | gauge | global + collector labels |
| `frequency` | 200.0 | gauge | global + collector labels |
| `total_energy_mid` | 900.0 | counter | global + collector labels |

## Test Script (`make e2e`)

The test script (`tests/e2e/run.sh`) performs the following steps:

1. **Start** — `docker-compose -f docker-compose.test.yml up -d --build`
2. **Wait** — poll the exporter's Prometheus endpoint until it returns HTTP 200 with metric data (retry with backoff, timeout after ~30s), ensuring at least one poll cycle has completed
3. **Scrape** — `curl http://localhost:9090/metrics`
4. **Assert**:
   - Expected metric names exist in the output
   - Values match expected (raw × scale + offset) within floating-point tolerance
   - Labels are correct — both global labels and per-collector labels are present
   - Counter vs gauge `# TYPE` annotations are correct
5. **Tear down** — `docker-compose -f docker-compose.test.yml down -v`
6. **Exit** — exit 0 on success, exit 1 on any assertion failure (with diagnostic output)

## Makefile Target

```makefile
e2e:  ## Run E2E tests with docker-compose
	bash tests/e2e/run.sh
```

## CI Integration

- E2E tests can run in GitHub Actions using `docker-compose` (Docker is available in GitHub-hosted runners)
- Should be a separate job or step from unit/integration tests in `ci.yml`
- Runs after the Docker image builds successfully
- Failure in E2E tests should fail the CI pipeline
