#!/usr/bin/env bash
# E2E test script for otel-modbus-exporter
# Starts modbus simulator + exporter via docker compose,
# polls Prometheus /metrics endpoint, and validates output.
set -euo pipefail

COMPOSE_FILE="docker-compose.test.yml"
METRICS_URL="http://localhost:9090/metrics"
MAX_RETRIES=30
RETRY_INTERVAL=2

cleanup() {
  echo "==> Tearing down..."
  docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
}
trap cleanup EXIT

echo "==> Starting services..."
docker compose -f "$COMPOSE_FILE" up -d --build

echo "==> Waiting for metrics endpoint..."
for i in $(seq 1 $MAX_RETRIES); do
  if curl -sf "$METRICS_URL" > /dev/null 2>&1; then
    echo "    Endpoint ready after ${i} attempts"
    break
  fi
  if [ "$i" -eq "$MAX_RETRIES" ]; then
    echo "FAIL: Metrics endpoint not available after $((MAX_RETRIES * RETRY_INTERVAL))s"
    docker compose -f "$COMPOSE_FILE" logs
    exit 1
  fi
  sleep "$RETRY_INTERVAL"
done

# Wait extra cycle for poll to complete
sleep 5

echo "==> Scraping metrics..."
METRICS=$(curl -sf "$METRICS_URL")
echo "$METRICS"

FAILURES=0

assert_metric_exists() {
  local name="$1"
  if echo "$METRICS" | grep -q "^${name}"; then
    echo "  PASS: metric '${name}' found"
  else
    echo "  FAIL: metric '${name}' NOT found"
    FAILURES=$((FAILURES + 1))
  fi
}

assert_type() {
  local name="$1"
  local expected_type="$2"
  if echo "$METRICS" | grep -q "^# TYPE ${name} ${expected_type}"; then
    echo "  PASS: '${name}' has TYPE ${expected_type}"
  else
    echo "  FAIL: '${name}' expected TYPE ${expected_type}"
    FAILURES=$((FAILURES + 1))
  fi
}

assert_label() {
  local name="$1"
  local label="$2"
  if echo "$METRICS" | grep "^${name}" | grep -q "${label}"; then
    echo "  PASS: '${name}' has label '${label}'"
  else
    echo "  FAIL: '${name}' missing label '${label}'"
    FAILURES=$((FAILURES + 1))
  fi
}

assert_value() {
  local name="$1"
  local expected="$2"
  local tolerance="${3:-0.01}"
  # Extract the numeric value from the last field of the metric line
  local actual
  actual=$(echo "$METRICS" | grep "^${name}{" | head -1 | awk '{print $NF}')
  if [ -z "$actual" ]; then
    echo "  FAIL: '${name}' value not found"
    FAILURES=$((FAILURES + 1))
    return
  fi
  if awk "BEGIN { diff = $actual - $expected; if (diff < 0) diff = -diff; exit !(diff <= $tolerance) }"; then
    echo "  PASS: '${name}' value=${actual} (expected=${expected} ±${tolerance})"
  else
    echo "  FAIL: '${name}' value=${actual} (expected=${expected} ±${tolerance})"
    FAILURES=$((FAILURES + 1))
  fi
}

echo ""
echo "==> Asserting metrics..."

# Check metric existence
assert_metric_exists "otel_modbus_voltage_phase_a_V"
assert_metric_exists "otel_modbus_total_energy_kWh"
assert_metric_exists "otel_modbus_temperature_C"
assert_metric_exists "otel_modbus_frequency_Hz"
assert_metric_exists "otel_modbus_total_energy_mid_kWh"

# Check types
assert_type "otel_modbus_voltage_phase_a_V" "gauge"
assert_type "otel_modbus_total_energy_kWh" "counter"
assert_type "otel_modbus_temperature_C" "gauge"
assert_type "otel_modbus_frequency_Hz" "gauge"
assert_type "otel_modbus_total_energy_mid_kWh" "counter"

# Check global labels
assert_label "otel_modbus_voltage_phase_a_V" 'env="test"'
assert_label "otel_modbus_voltage_phase_a_V" 'site="e2e"'

# Check collector labels
assert_label "otel_modbus_voltage_phase_a_V" 'device="simulator"'

# Check metric values (deterministic simulator registers + scale/offset)
# voltage_phase_a: register 0 = 2300 (u16), scale=0.1, offset=0.0 → 230.0
assert_value "otel_modbus_voltage_phase_a_V" 230.0
# total_energy: registers 16,17 = (1<<16)|24464 = 90000 (u32), scale=0.01, offset=0.0 → 900.0
assert_value "otel_modbus_total_energy_kWh" 900.0
# temperature: register 0 = 65436 (i16 = -100), scale=0.1, offset=40.0 → -10.0+40.0 = 30.0
assert_value "otel_modbus_temperature_C" 30.0
# frequency: registers 32,33 = 0x43480000 (f32 = 200.0), scale=1.0, offset=0.0 → 200.0
assert_value "otel_modbus_frequency_Hz" 200.0
# total_energy_mid: registers 48,49 mid-big = same value 90000 (u32), scale=0.01 → 900.0
assert_value "otel_modbus_total_energy_mid_kWh" 900.0

echo ""
echo "==> Testing graceful shutdown..."
EXPORTER_CONTAINER=$(docker compose -f "$COMPOSE_FILE" ps -q otel-modbus-exporter)
if [ -n "$EXPORTER_CONTAINER" ]; then
  docker stop --time=10 "$EXPORTER_CONTAINER"
  EXIT_CODE=$(docker inspect "$EXPORTER_CONTAINER" --format='{{.State.ExitCode}}')
  if [ "$EXIT_CODE" = "0" ]; then
    echo "  PASS: graceful shutdown (exit code 0)"
  else
    echo "  FAIL: exit code was ${EXIT_CODE}, expected 0"
    FAILURES=$((FAILURES + 1))
  fi
fi

echo ""
if [ "$FAILURES" -gt 0 ]; then
  echo "FAILED: ${FAILURES} assertion(s) failed"
  exit 1
else
  echo "ALL TESTS PASSED"
  exit 0
fi
