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

echo ""
echo "==> Asserting metrics..."

# Check metric existence
assert_metric_exists "voltage_phase_a"
assert_metric_exists "total_energy"
assert_metric_exists "temperature"
assert_metric_exists "frequency"
assert_metric_exists "total_energy_mid"

# Check types
assert_type "voltage_phase_a" "gauge"
assert_type "total_energy" "counter"
assert_type "temperature" "gauge"
assert_type "frequency" "gauge"
assert_type "total_energy_mid" "counter"

# Check global labels
assert_label "voltage_phase_a" 'env="test"'
assert_label "voltage_phase_a" 'site="e2e"'

# Check collector labels
assert_label "voltage_phase_a" 'device="simulator"'

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
