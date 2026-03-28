use super::*;
use crate::config::{
    ByteOrder, Collector, DataType, Metric, MetricType as ConfigMetricType, Protocol, RegisterType,
};
use crate::modbus::{ModbusConnection, ModbusReader};
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
}

impl MockModbusClient {
    fn new() -> Self {
        Self {
            connected: false,
            holding_registers: Arc::new(Mutex::new(HashMap::new())),
            fail_after: Arc::new(Mutex::new(None)),
            read_count: Arc::new(Mutex::new(0)),
            connect_fail_count: Arc::new(Mutex::new(0)),
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
impl ModbusConnection for MockModbusClient {
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

fn test_collector_config(name: &str) -> Collector {
    Collector {
        name: name.to_string(),
        protocol: Protocol::ModbusTcp {
            endpoint: "127.0.0.1:502".to_string(),
        },
        slave_id: 1,
        polling_interval: Duration::from_millis(100),
        labels: HashMap::new(),
        metrics_files: None,
        metrics: vec![Metric {
            name: "temperature".to_string(),
            description: "Temperature sensor".to_string(),
            metric_type: ConfigMetricType::Gauge,
            register_type: RegisterType::Holding,
            address: 100,
            data_type: DataType::U16,
            byte_order: ByteOrder::BigEndian,
            scale: 0.1,
            offset: 0.0,
            unit: "celsius".to_string(),
        }],
    }
}

struct MockFactory {
    clients: Mutex<Vec<Box<dyn ModbusClient>>>,
}

impl ModbusClientFactory for MockFactory {
    fn create(&self, _collector: &Collector) -> Box<dyn ModbusClient> {
        self.clients
            .lock()
            .unwrap()
            .pop()
            .expect("no mock clients left")
    }
}

#[tokio::test]
async fn test_collector_polls_and_publishes() {
    let store = MetricStore::new();
    let collector_cfg = test_collector_config("test1");

    let mock = MockModbusClient::new().with_holding_register(100, vec![250]); // 250 * 0.1 = 25.0

    let factory = MockFactory {
        clients: Mutex::new(vec![Box::new(mock)]),
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
        clients: Mutex::new(vec![Box::new(mock2), Box::new(mock1)]), // reversed since pop
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
