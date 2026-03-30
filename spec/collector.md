# Collector Specification

## Overview

Each collector runs as an independent async task (tokio::spawn) that periodically polls a device via its [`MetricReader`](reader.md) and updates the in-memory metric store. The collector doesn't know the underlying protocol — it works through the common reader interface.

## Per-Collector Cache

Each collector maintains its own **local cache** (`CollectorCache`) — a `HashMap<String, MetricValue>` keyed by metric name. This cache holds the latest decoded values from the most recent successful poll cycle.

After each complete poll cycle, the collector **atomically publishes** its cache snapshot to the shared `MetricStore` via `store.publish(collector_name, cache_snapshot)`. This is the **only write path** into the metric store. Exporters (OTLP, Prometheus, MQTT) are pure readers — they never trigger bus calls.

This strict **producer/consumer separation** ensures:

- Bus I/O is fully contained within collector tasks.
- Exporters see a consistent snapshot, never partial poll results.
- A slow or failing collector cannot block or delay metric export.

## Poll Engine Design

### One Task Per Collector

- On startup, spawn one `tokio::task` per configured collector.
- Each task owns its Modbus client connection **and** its local `CollectorCache`.
- Tasks are independent — a failure in one collector does not affect others.

### Polling Loop

```rust
// CancellationToken for cooperative shutdown inside read()
let cancel = CancellationToken::new();

// Execute init_writes once at startup (I2C/SPI/I3C only)
// Writes are performed via a separate MetricWriter trait,
// not through the MetricReader interface. The collector
// holds both a reader and an optional writer for the same
// bus connection. See reader.md for the trait boundary.
if let Some(writer) = &mut writer {
    writer.execute_writes(&collector.init_writes).await?;
}

loop {
    let start = Instant::now();
    let mut local_cache = HashMap::new();
    let mut had_error = false;

    // Execute pre_poll writes before each read cycle (I2C/SPI/I3C only)
    if let Some(writer) = &mut writer {
        writer.execute_writes(&collector.pre_poll).await?;
    }

    // Batch read all metrics via reader — returns ReadResults
    let ReadResults { metrics: read_results, io_count } = client.read(&cancel).await;

    for (metric_name, result) in read_results {
        match result {
            Ok((_raw, scaled)) => {
                local_cache.insert(metric_name, MetricValue { value: scaled, /* ... */ });
            },
            Err(e) => {
                had_error = true;
                log per-metric error;
                increment error counter;
                // Retain previous cached value (not inserted into local_cache)
            }
        }
    }
    // Merge: for metrics that failed, carry forward from previous cache
    for (name, prev) in &prev_cache {
        local_cache.entry(name.clone()).or_insert_with(|| {
            let mut carried = prev.clone();
            carried.updated_at = SystemTime::now();
            carried
        });
    }
    // Atomically publish to shared store
    store.publish(&collector.name, &local_cache);
    prev_cache = local_cache;

    let elapsed = start.elapsed();
    if elapsed < polling_interval {
        sleep(polling_interval - elapsed).await;
    }
}
```

### Polling Interval

- Measured from the **start** of each poll cycle, not the end.
- If a poll cycle exceeds the interval, the next cycle starts immediately (no negative sleep).
- A warning is logged if poll duration exceeds 80% of the interval.

## Reconnect and Backoff Strategy

When a connection fails or is lost:

1. Log the error with collector name and endpoint/device.
2. Wait with exponential backoff: 1s → 2s → 4s → 8s → … → max 60s.
3. Reset backoff to 1s after a successful poll cycle.
4. During backoff, the collector task is sleeping (not consuming CPU).
5. After reconnect, re-execute `init_writes` before resuming the poll loop.

## Error Handling

### Per-Metric Errors

- A single metric read failure does NOT abort the poll cycle.
- The failed metric retains its previous value (stale).
- An error counter is incremented per metric.
- Errors are logged at `warn` level.

### Per-Collector Errors

- Connection-level failures (timeout, disconnect) affect all metrics.
- The entire poll cycle is aborted and reconnect logic kicks in.
- Errors are logged at `error` level.

## Graceful Shutdown

- On SIGTERM/SIGINT, all collector tasks receive a cancellation signal via `tokio::sync::watch` or `CancellationToken`.
- Each task completes its current poll cycle (or aborts within 5s), then exits.
- The main task waits for all collector tasks to finish before shutting down exporters.
