use super::*;
use crate::config::{
    ByteOrder, CollectorConfig, DataType, MetricConfig, MetricType as ConfigMetricType, Protocol,
    RegisterType,
};
use crate::reader::modbus::{BusConnection, ModbusReader};
use crate::reader::MetricReader as MetricReaderTrait;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Mock Modbus client for testing.
struct MockModbusClient {
    connected: bool,
    holding_registers: Arc<Mutex<HashMap<u16, Vec<u16>>>>,
    fail_after: Arc<Mutex<Option<usize>>>,
    read_count: Arc<Mutex<usize>>,
    connect_fail_count: Arc<Mutex<usize>>,
    metrics: Vec<MetricConfig>,
}

impl MockModbusClient {
    fn new() -> Self {
        Self {
            connected: false,
            holding_registers: Arc::new(Mutex::new(HashMap::new())),
            fail_after: Arc::new(Mutex::new(None)),
            read_count: Arc::new(Mutex::new(0)),
            connect_fail_count: Arc::new(Mutex::new(0)),
            metrics: Vec::new(),
        }
    }

    fn with_holding_register(self, addr: u16, values: Vec<u16>) -> Self {
        self.holding_registers.lock().unwrap().insert(addr, values);
        self
    }

    fn with_fail_after(self, n: usize) -> Self {
        *self.fail_after.lock().unwrap() = Some(n);
        self
    }

    fn with_connect_failures(self, n: usize) -> Self {
        *self.connect_fail_count.lock().unwrap() = n;
        self
    }
}

#[async_trait]
impl BusConnection for MockModbusClient {
    async fn connect(&mut self) -> Result<()> {
        let mut count = self.connect_fail_count.lock().unwrap();
        if *count > 0 {
            *count -= 1;
            return Err(anyhow::anyhow!("mock connect failure"));
        }
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
impl ModbusReader for MockModbusClient {
    async fn read_holding_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>> {
        let mut rc = self.read_count.lock().unwrap();
        *rc += 1;
        let current = *rc;
        drop(rc);

        if let Some(fail_after) = *self.fail_after.lock().unwrap() {
            if current > fail_after {
                self.connected = false;
                return Err(anyhow::anyhow!("mock connection lost"));
            }
        }

        let regs = self.holding_registers.lock().unwrap();
        if let Some(values) = regs.get(&addr) {
            Ok(values[..count as usize].to_vec())
        } else {
            Ok(vec![0; count as usize])
        }
    }

    async fn read_input_registers(&mut self, _addr: u16, count: u16) -> Result<Vec<u16>> {
        Ok(vec![0; count as usize])
    }

    async fn read_coils(&mut self, _addr: u16, _count: u16) -> Result<Vec<bool>> {
        Ok(vec![true])
    }

    async fn read_discrete_inputs(&mut self, _addr: u16, _count: u16) -> Result<Vec<bool>> {
        Ok(vec![false])
    }
}

#[async_trait]
impl MetricReaderTrait for MockModbusClient {
    fn set_metrics(&mut self, metrics: Vec<MetricConfig>) {
        self.metrics = metrics;
    }

    async fn connect(&mut self) -> Result<()> {
        BusConnection::connect(self).await
    }

    async fn disconnect(&mut self) -> Result<()> {
        BusConnection::disconnect(self).await
    }

    fn is_connected(&self) -> bool {
        BusConnection::is_connected(self)
    }

    async fn read(
        &mut self,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> crate::reader::ReadResults {
        let mut results = HashMap::new();
        let metrics = std::mem::take(&mut self.metrics);
        for metric in &metrics {
            if cancel.is_cancelled() {
                break;
            }
            let result = crate::reader::modbus::read_modbus_metric(self, metric).await;
            results.insert(metric.name.clone(), result);
        }
        let io_count = results.len();
        self.metrics = metrics;
        crate::reader::ReadResults {
            metrics: results,
            io_count,
        }
    }
}

fn test_collector_config(name: &str) -> CollectorConfig {
    CollectorConfig {
        name: name.to_string(),
        protocol: Protocol::ModbusTcp {
            endpoint: "127.0.0.1:502".to_string(),
        },
        slave_id: Some(1),
        polling_interval: Duration::from_millis(100),
        labels: HashMap::new(),
        metrics_files: None,
        init_writes: Vec::new(),
        pre_poll: Vec::new(),
        metrics: vec![MetricConfig {
            name: "temperature".to_string(),
            description: "Temperature sensor".to_string(),
            metric_type: ConfigMetricType::Gauge,
            register_type: Some(RegisterType::Holding),
            address: Some(100),
            data_type: DataType::U16,
            byte_order: ByteOrder::BigEndian,
            scale: 0.1,
            offset: 0.0,
            unit: "celsius".to_string(),
            command: Vec::new(),
            response_length: None,
            response_offset: 0,
        }],
    }
}

use crate::reader::{MetricFactory, MetricReaderFactory, MetricWriterFactory};

struct MockFactory {
    clients: Mutex<Vec<Box<dyn MetricReaderTrait>>>,
    writers: Mutex<Option<Box<dyn crate::reader::MetricWriter>>>,
}

impl MetricReaderFactory for MockFactory {
    fn create(&self, _collector: &CollectorConfig) -> anyhow::Result<Box<dyn MetricReaderTrait>> {
        let client = self
            .clients
            .lock()
            .unwrap()
            .pop()
            .expect("no mock clients left");
        Ok(client)
    }
}

impl MetricWriterFactory for MockFactory {
    fn create_writer(
        &self,
        _collector: &CollectorConfig,
    ) -> anyhow::Result<Option<Box<dyn crate::reader::MetricWriter>>> {
        let writers = self.writers.lock().unwrap().take();
        Ok(writers)
    }
}

impl MetricFactory for MockFactory {}

#[tokio::test]
async fn test_collector_polls_and_publishes() {
    let store = MetricStore::new();
    let collector_cfg = test_collector_config("test1");

    let mock = MockModbusClient::new().with_holding_register(100, vec![250]); // 250 * 0.1 = 25.0

    let factory = MockFactory {
        clients: Mutex::new(vec![Box::new(mock)]),
        writers: Mutex::new(None),
    };

    let engine = CollectorEngine::spawn(
        vec![collector_cfg],
        store.clone(),
        BTreeMap::new(),
        &factory,
        None,
    );

    // Wait for at least one poll cycle
    tokio::time::sleep(Duration::from_millis(300)).await;

    let metrics = store.metrics_for("test1");
    assert!(!metrics.is_empty(), "expected metrics to be published");
    let temp = metrics.iter().find(|m| m.name == "temperature").unwrap();
    assert!((temp.value - 25.0).abs() < 0.001);

    engine.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn test_collector_graceful_shutdown() {
    let store = MetricStore::new();
    let collector_cfg = test_collector_config("shutdown_test");

    let mock = MockModbusClient::new().with_holding_register(100, vec![100]);

    let factory = MockFactory {
        clients: Mutex::new(vec![Box::new(mock)]),
        writers: Mutex::new(None),
    };

    let engine = CollectorEngine::spawn(
        vec![collector_cfg],
        store.clone(),
        BTreeMap::new(),
        &factory,
        None,
    );

    tokio::time::sleep(Duration::from_millis(200)).await;
    engine.shutdown(Duration::from_secs(2)).await;
    // If we get here without hanging, shutdown works
}

#[tokio::test]
async fn test_collector_reconnects_on_failure() {
    let store = MetricStore::new();
    let collector_cfg = test_collector_config("reconnect_test");

    // Fail after 1 read, then reconnect should succeed
    let mock = MockModbusClient::new()
        .with_holding_register(100, vec![300])
        .with_fail_after(1);

    let factory = MockFactory {
        clients: Mutex::new(vec![Box::new(mock)]),
        writers: Mutex::new(None),
    };

    let engine = CollectorEngine::spawn(
        vec![collector_cfg],
        store.clone(),
        BTreeMap::new(),
        &factory,
        None,
    );

    // First poll succeeds, second fails, reconnect + third succeeds
    tokio::time::sleep(Duration::from_millis(500)).await;

    let metrics = store.metrics_for("reconnect_test");
    // Should have at least the first successful read cached
    assert!(!metrics.is_empty());

    engine.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn test_collector_connect_backoff() {
    let store = MetricStore::new();
    let collector_cfg = test_collector_config("backoff_test");

    // Fail first 2 connect attempts
    let mock = MockModbusClient::new()
        .with_holding_register(100, vec![500])
        .with_connect_failures(2);

    let factory = MockFactory {
        clients: Mutex::new(vec![Box::new(mock)]),
        writers: Mutex::new(None),
    };

    let engine = CollectorEngine::spawn(
        vec![collector_cfg],
        store.clone(),
        BTreeMap::new(),
        &factory,
        None,
    );

    // Need to wait for backoff: 1s + 2s + poll time
    tokio::time::sleep(Duration::from_millis(4000)).await;

    let metrics = store.metrics_for("backoff_test");
    assert!(!metrics.is_empty(), "should eventually connect and poll");

    engine.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn test_multiple_collectors() {
    let store = MetricStore::new();
    let cfg1 = test_collector_config("multi1");
    let mut cfg2 = test_collector_config("multi2");
    cfg2.name = "multi2".to_string();

    let mock1 = MockModbusClient::new().with_holding_register(100, vec![100]);
    let mock2 = MockModbusClient::new().with_holding_register(100, vec![200]);

    let factory = MockFactory {
        clients: Mutex::new(vec![Box::new(mock2), Box::new(mock1)]),
        writers: Mutex::new(None),
    };

    let engine = CollectorEngine::spawn(
        vec![cfg1, cfg2],
        store.clone(),
        BTreeMap::new(),
        &factory,
        None,
    );

    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(!store.metrics_for("multi1").is_empty());
    assert!(!store.metrics_for("multi2").is_empty());

    let m1 = store.metrics_for("multi1");
    let m2 = store.metrics_for("multi2");
    assert!((m1[0].value - 10.0).abs() < 0.001); // 100 * 0.1
    assert!((m2[0].value - 20.0).abs() < 0.001); // 200 * 0.1

    engine.shutdown(Duration::from_secs(2)).await;
}

// ── Mock Writer ──────────────────────────────────────────────────

use crate::config::{ByteValue, WriteStep};

/// Mock writer that can be configured to fail N times then succeed.
struct MockWriter {
    fail_count: Arc<Mutex<u32>>,
    call_count: Arc<Mutex<u32>>,
}

impl MockWriter {
    fn new(fail_count: u32) -> Self {
        Self {
            fail_count: Arc::new(Mutex::new(fail_count)),
            call_count: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl crate::reader::MetricWriter for MockWriter {
    async fn execute_writes(&mut self, _steps: &[WriteStep]) -> Result<()> {
        let mut calls = self.call_count.lock().unwrap();
        *calls += 1;
        let mut remaining = self.fail_count.lock().unwrap();
        if *remaining > 0 {
            *remaining -= 1;
            return Err(anyhow::anyhow!("mock write failure"));
        }
        Ok(())
    }
}

fn write_step_noop() -> WriteStep {
    WriteStep {
        address: Some(0x10),
        value: Some(ByteValue::Single(0x01)),
        command: None,
        delay: None,
    }
}

#[tokio::test]
async fn test_init_writes_failure_triggers_reconnect() {
    // init_writes fails once, then succeeds on reconnect
    let store = MetricStore::new();
    let mut cfg = test_collector_config("init_writes_test");
    cfg.init_writes = vec![write_step_noop()];

    let mock = MockModbusClient::new().with_holding_register(100, vec![250]);
    let writer = MockWriter::new(1); // fail first call, succeed second

    let factory = MockFactory {
        clients: Mutex::new(vec![Box::new(mock)]),
        writers: Mutex::new(Some(Box::new(writer))),
    };

    let engine = CollectorEngine::spawn(vec![cfg], store.clone(), BTreeMap::new(), &factory, None);

    // Wait for backoff (1s) + reconnect + poll
    tokio::time::sleep(Duration::from_millis(3000)).await;

    let metrics = store.metrics_for("init_writes_test");
    assert!(
        !metrics.is_empty(),
        "should eventually connect and poll after init_writes retry"
    );

    engine.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn test_pre_poll_skip_then_reconnect_after_3_failures() {
    // pre_poll fails 4 times: first 2 skip cycles, 3rd triggers reconnect, 4th (after reconnect) succeeds
    let store = MetricStore::new();
    let mut cfg = test_collector_config("pre_poll_test");
    cfg.pre_poll = vec![write_step_noop()];
    cfg.polling_interval = Duration::from_millis(50);

    let mock = MockModbusClient::new().with_holding_register(100, vec![250]);
    let writer = MockWriter::new(3); // fail 3 times then succeed

    let factory = MockFactory {
        clients: Mutex::new(vec![Box::new(mock)]),
        writers: Mutex::new(Some(Box::new(writer))),
    };

    let im = Arc::new(crate::internal_metrics::InternalMetrics::new());
    let engine = CollectorEngine::spawn(
        vec![cfg],
        store.clone(),
        BTreeMap::new(),
        &factory,
        Some(im.clone()),
    );

    // Wait for: 2 skipped cycles + reconnect backoff + successful poll
    tokio::time::sleep(Duration::from_millis(3000)).await;

    let metrics = store.metrics_for("pre_poll_test");
    assert!(
        !metrics.is_empty(),
        "should eventually poll after pre_poll reconnect"
    );

    // Verify error metrics were recorded for skipped cycles
    let stats = im.get_or_create_collector("pre_poll_test");
    assert!(
        stats.polls_error.load(Relaxed) >= 2,
        "should have recorded at least 2 poll errors for skipped cycles"
    );

    engine.shutdown(Duration::from_secs(2)).await;
}
