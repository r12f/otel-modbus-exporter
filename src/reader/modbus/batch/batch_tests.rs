//! Unit and integration tests for batch read coalescing.

use super::*;
use crate::config::{ByteOrder, DataType, MetricConfig, MetricType, RegisterType};
use crate::reader::modbus::{BusConnection, ModbusReader};
use anyhow::{bail, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Mock Modbus client that tracks read calls.
struct MockBatchClient {
    connected: bool,
    /// Holding register data: address -> value. For simplicity, maps each address to a single u16.
    holding: std::collections::HashMap<u16, u16>,
    input: std::collections::HashMap<u16, u16>,
    holding_read_count: AtomicUsize,
    input_read_count: AtomicUsize,
}

impl MockBatchClient {
    fn new() -> Self {
        Self {
            connected: true,
            holding: std::collections::HashMap::new(),
            input: std::collections::HashMap::new(),
            holding_read_count: AtomicUsize::new(0),
            input_read_count: AtomicUsize::new(0),
        }
    }

    fn with_holding(mut self, data: &[(u16, u16)]) -> Self {
        for &(addr, val) in data {
            self.holding.insert(addr, val);
        }
        self
    }

    fn with_input(mut self, data: &[(u16, u16)]) -> Self {
        for &(addr, val) in data {
            self.input.insert(addr, val);
        }
        self
    }
}

#[async_trait]
impl BusConnection for MockBatchClient {
    async fn connect(&mut self) -> Result<()> {
        self.connected = true;
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
    fn is_connected(&self) -> bool {
        self.connected
    }
}

#[async_trait]
impl ModbusReader for MockBatchClient {
    async fn read_holding_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>> {
        self.holding_read_count.fetch_add(1, Ordering::Relaxed);
        let mut result = Vec::with_capacity(count as usize);
        for i in 0..count {
            result.push(*self.holding.get(&(addr + i)).unwrap_or(&0));
        }
        Ok(result)
    }

    async fn read_input_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>> {
        self.input_read_count.fetch_add(1, Ordering::Relaxed);
        let mut result = Vec::with_capacity(count as usize);
        for i in 0..count {
            result.push(*self.input.get(&(addr + i)).unwrap_or(&0));
        }
        Ok(result)
    }

    async fn read_coils(&mut self, _addr: u16, _count: u16) -> Result<Vec<bool>> {
        Ok(vec![true])
    }

    async fn read_discrete_inputs(&mut self, _addr: u16, _count: u16) -> Result<Vec<bool>> {
        Ok(vec![false])
    }
}

fn make_metric(name: &str, addr: u16, data_type: DataType, reg_type: RegisterType) -> MetricConfig {
    MetricConfig {
        name: name.to_string(),
        description: String::new(),
        metric_type: MetricType::Gauge,
        register_type: Some(reg_type),
        address: Some(addr),
        data_type,
        byte_order: ByteOrder::BigEndian,
        scale: 1.0,
        offset: 0.0,
        unit: String::new(),
        command: Vec::new(),
        response_length: None,
        response_offset: 0,
    }
}

// ---------- Coalescing unit tests ----------

#[test]
fn test_coalesce_adjacent_ranges_merged() {
    let m1 = make_metric("a", 0, DataType::U16, RegisterType::Holding);
    let m2 = make_metric("b", 1, DataType::U16, RegisterType::Holding);
    let m3 = make_metric("c", 2, DataType::U16, RegisterType::Holding);
    let items = vec![
        IndexedMetric {
            idx: 0,
            metric: &m1,
            addr: 0,
            count: 1,
        },
        IndexedMetric {
            idx: 1,
            metric: &m2,
            addr: 1,
            count: 1,
        },
        IndexedMetric {
            idx: 2,
            metric: &m3,
            addr: 2,
            count: 1,
        },
    ];
    let ranges = coalesce(items);
    assert_eq!(ranges.len(), 1, "adjacent ranges should merge into one");
    assert_eq!(ranges[0].start, 0);
    assert_eq!(ranges[0].end, 3);
    assert_eq!(ranges[0].members.len(), 3);
}

#[test]
fn test_coalesce_gap_within_threshold() {
    let m1 = make_metric("a", 0, DataType::U16, RegisterType::Holding);
    let m2 = make_metric("b", 10, DataType::U16, RegisterType::Holding);
    let items = vec![
        IndexedMetric {
            idx: 0,
            metric: &m1,
            addr: 0,
            count: 1,
        },
        IndexedMetric {
            idx: 1,
            metric: &m2,
            addr: 10,
            count: 1,
        },
    ];
    let ranges = coalesce(items);
    // gap = 10 - 1 = 9 <= 10, should merge
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].start, 0);
    assert_eq!(ranges[0].end, 11);
}

#[test]
fn test_coalesce_gap_exceeds_threshold() {
    let m1 = make_metric("a", 0, DataType::U16, RegisterType::Holding);
    let m2 = make_metric("b", 12, DataType::U16, RegisterType::Holding);
    let items = vec![
        IndexedMetric {
            idx: 0,
            metric: &m1,
            addr: 0,
            count: 1,
        },
        IndexedMetric {
            idx: 1,
            metric: &m2,
            addr: 12,
            count: 1,
        },
    ];
    let ranges = coalesce(items);
    // gap = 12 - 1 = 11 > 10, should NOT merge
    assert_eq!(ranges.len(), 2);
}

#[test]
fn test_coalesce_overlapping_ranges() {
    let m1 = make_metric("a", 0, DataType::U32, RegisterType::Holding); // addr 0, count 2
    let m2 = make_metric("b", 1, DataType::U32, RegisterType::Holding); // addr 1, count 2
    let items = vec![
        IndexedMetric {
            idx: 0,
            metric: &m1,
            addr: 0,
            count: 2,
        },
        IndexedMetric {
            idx: 1,
            metric: &m2,
            addr: 1,
            count: 2,
        },
    ];
    let ranges = coalesce(items);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].start, 0);
    assert_eq!(ranges[0].end, 3);
}

#[test]
fn test_coalesce_empty() {
    let ranges = coalesce(vec![]);
    assert!(ranges.is_empty());
}

#[test]
fn test_coalesce_single() {
    let m = make_metric("a", 5, DataType::U16, RegisterType::Holding);
    let items = vec![IndexedMetric {
        idx: 0,
        metric: &m,
        addr: 5,
        count: 1,
    }];
    let ranges = coalesce(items);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].start, 5);
    assert_eq!(ranges[0].end, 6);
}

// ---------- Integration tests: batch vs single equivalence ----------

#[tokio::test]
async fn test_batch_read_values_match_individual() {
    let mut client = MockBatchClient::new().with_holding(&[(0, 100), (1, 200), (10, 300)]);

    let metrics = vec![
        make_metric("m0", 0, DataType::U16, RegisterType::Holding),
        make_metric("m1", 1, DataType::U16, RegisterType::Holding),
        make_metric("m10", 10, DataType::U16, RegisterType::Holding),
    ];

    // Individual reads
    let mut individual_results = Vec::new();
    for m in &metrics {
        individual_results.push(read_single(&mut client, m).await.unwrap());
    }

    // Reset counters
    client.holding_read_count.store(0, Ordering::Relaxed);

    // Batch read
    let batch_result = batch_read_coalesced(&mut client, &metrics).await;
    let batch_results = batch_result.results;

    for (i, (_, result)) in batch_results.iter().enumerate() {
        let batch_val = result.as_ref().unwrap();
        assert!(
            (batch_val.1 - individual_results[i].1).abs() < f64::EPSILON,
            "metric {} mismatch: batch={} individual={}",
            metrics[i].name,
            batch_val.1,
            individual_results[i].1
        );
    }
}

#[tokio::test]
async fn test_batch_read_coalesces_adjacent() {
    // 3 adjacent holding registers should be read with 1 Modbus call
    let mut client = MockBatchClient::new().with_holding(&[(0, 10), (1, 20), (2, 30)]);

    let metrics = vec![
        make_metric("a", 0, DataType::U16, RegisterType::Holding),
        make_metric("b", 1, DataType::U16, RegisterType::Holding),
        make_metric("c", 2, DataType::U16, RegisterType::Holding),
    ];

    let batch_result = batch_read_coalesced(&mut client, &metrics).await;
    let results = batch_result.results;

    // Should have been only 1 holding read call
    assert_eq!(client.holding_read_count.load(Ordering::Relaxed), 1);

    assert_eq!(results[0].1.as_ref().unwrap().1, 10.0);
    assert_eq!(results[1].1.as_ref().unwrap().1, 20.0);
    assert_eq!(results[2].1.as_ref().unwrap().1, 30.0);
}

#[tokio::test]
async fn test_batch_read_separate_register_types() {
    let mut client = MockBatchClient::new()
        .with_holding(&[(0, 100)])
        .with_input(&[(0, 200)]);

    let metrics = vec![
        make_metric("h", 0, DataType::U16, RegisterType::Holding),
        make_metric("i", 0, DataType::U16, RegisterType::Input),
    ];

    let batch_result = batch_read_coalesced(&mut client, &metrics).await;
    let results = batch_result.results;

    assert_eq!(client.holding_read_count.load(Ordering::Relaxed), 1);
    assert_eq!(client.input_read_count.load(Ordering::Relaxed), 1);
    assert_eq!(results[0].1.as_ref().unwrap().1, 100.0);
    assert_eq!(results[1].1.as_ref().unwrap().1, 200.0);
}

#[tokio::test]
async fn test_batch_read_non_adjacent_separate_calls() {
    // Gap > 10 should result in 2 separate reads
    let mut client = MockBatchClient::new().with_holding(&[(0, 10), (50, 500)]);

    let metrics = vec![
        make_metric("a", 0, DataType::U16, RegisterType::Holding),
        make_metric("b", 50, DataType::U16, RegisterType::Holding),
    ];

    let batch_result = batch_read_coalesced(&mut client, &metrics).await;
    let results = batch_result.results;

    assert_eq!(client.holding_read_count.load(Ordering::Relaxed), 2);
    assert_eq!(results[0].1.as_ref().unwrap().1, 10.0);
    assert_eq!(results[1].1.as_ref().unwrap().1, 500.0);
}

/// Mock that fails batch reads to test fallback.
struct FailingBatchClient {
    holding_call_count: AtomicUsize,
}

impl FailingBatchClient {
    fn new() -> Self {
        Self {
            holding_call_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl BusConnection for FailingBatchClient {
    async fn connect(&mut self) -> Result<()> {
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }
    fn is_connected(&self) -> bool {
        true
    }
}

#[async_trait]
impl ModbusReader for FailingBatchClient {
    async fn read_holding_registers(&mut self, _addr: u16, count: u16) -> Result<Vec<u16>> {
        let _n = self.holding_call_count.fetch_add(1, Ordering::Relaxed);
        if count > 1 {
            // Fail batch reads (count > 1), succeed individual
            bail!("simulated batch failure");
        }
        // Individual reads succeed
        Ok(vec![42])
    }
    async fn read_input_registers(&mut self, _addr: u16, _count: u16) -> Result<Vec<u16>> {
        Ok(vec![0])
    }
    async fn read_coils(&mut self, _addr: u16, _count: u16) -> Result<Vec<bool>> {
        Ok(vec![true])
    }
    async fn read_discrete_inputs(&mut self, _addr: u16, _count: u16) -> Result<Vec<bool>> {
        Ok(vec![false])
    }
}

#[tokio::test]
async fn test_batch_read_fallback_on_failure() {
    let mut client = FailingBatchClient::new();

    let metrics = vec![
        make_metric("a", 0, DataType::U16, RegisterType::Holding),
        make_metric("b", 1, DataType::U16, RegisterType::Holding),
    ];

    let batch_result = batch_read_coalesced(&mut client, &metrics).await;
    let results = batch_result.results;

    // First call (batch) fails, then 2 individual calls succeed
    assert_eq!(client.holding_call_count.load(Ordering::Relaxed), 3);
    assert_eq!(results[0].1.as_ref().unwrap().1, 42.0);
    assert_eq!(results[1].1.as_ref().unwrap().1, 42.0);
}

#[test]
fn test_coalesce_splits_at_125_register_limit() {
    // Two metrics that together span more than 125 registers should be split
    // into separate ranges even though the gap is within threshold.
    // Metric A: addr 0, count 1 (U16)
    // Metric B: addr 124, count 2 (U32) => end = 126, merged_count = 126 > 125
    let m1 = make_metric("a", 0, DataType::U16, RegisterType::Holding);
    let m2 = make_metric("b", 124, DataType::U32, RegisterType::Holding);
    let items = vec![
        IndexedMetric {
            idx: 0,
            metric: &m1,
            addr: 0,
            count: 1,
        },
        IndexedMetric {
            idx: 1,
            metric: &m2,
            addr: 124,
            count: 2,
        },
    ];
    let ranges = coalesce(items);
    assert_eq!(
        ranges.len(),
        2,
        "metrics spanning >125 registers should produce 2 ranges"
    );
    assert_eq!(ranges[0].start, 0);
    assert_eq!(ranges[0].end, 1);
    assert_eq!(ranges[1].start, 124);
    assert_eq!(ranges[1].end, 126);
}

#[tokio::test]
async fn test_batch_read_125_limit_produces_multiple_reads() {
    // Verify that metrics spanning >125 registers result in multiple Modbus reads.
    let mut client = MockBatchClient::new().with_holding(&[(0, 10), (124, 20), (125, 30)]);

    let metrics = vec![
        make_metric("a", 0, DataType::U16, RegisterType::Holding),
        make_metric("b", 124, DataType::U32, RegisterType::Holding),
    ];

    let result = batch_read_coalesced(&mut client, &metrics).await;

    // Should be 2 separate read calls due to 125-register limit
    assert_eq!(
        client.holding_read_count.load(Ordering::Relaxed),
        2,
        "should issue 2 reads when range exceeds 125 registers"
    );
    assert_eq!(result.read_count, 2);
    assert_eq!(result.results[0].1.as_ref().unwrap().1, 10.0);
}
