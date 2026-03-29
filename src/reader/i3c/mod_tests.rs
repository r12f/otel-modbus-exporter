use super::*;
use crate::config::{ByteOrder, DataType, MetricConfig, MetricType};
use std::sync::{Arc, Mutex};

/// Mock I3C device with configurable responses.
struct MockI3cDevice {
    responses: Mutex<Vec<Result<Vec<u8>>>>,
    call_count: Mutex<usize>,
}

impl MockI3cDevice {
    fn new(responses: Vec<Result<Vec<u8>>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            call_count: Mutex::new(0),
        }
    }

    fn with_fixed_response(data: Vec<u8>) -> Self {
        // Return the same response many times
        let responses: Vec<Result<Vec<u8>>> = (0..100).map(|_| Ok(data.clone())).collect();
        Self::new(responses)
    }

    fn get_call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

impl I3cDevice for MockI3cDevice {
    fn write_read(&mut self, _address: u8, _write_buf: &[u8], _read_len: usize) -> Result<Vec<u8>> {
        let mut count = self.call_count.lock().unwrap();
        let mut responses = self.responses.lock().unwrap();
        *count += 1;
        if responses.is_empty() {
            anyhow::bail!("no more mock responses")
        }
        responses.remove(0)
    }
}

fn make_metric(
    name: &str,
    address: u8,
    data_type: DataType,
    byte_order: ByteOrder,
) -> MetricConfig {
    MetricConfig {
        name: name.to_string(),
        description: String::new(),
        metric_type: MetricType::Gauge,
        register_type: None,
        address: Some(address as u16),
        data_type,
        byte_order,
        scale: 1.0,
        offset: 0.0,
        unit: String::new(),
        command: vec![],
        response_length: None,
        response_offset: 0,
    }
}

// ── Address mode tests ──────────────────────────────────────────────

#[test]
fn test_static_address_mode() {
    let device = MockI3cDevice::with_fixed_response(vec![0x42]);
    let mut client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::Static(0x30),
    );
    let addr = client.resolve_address().unwrap();
    assert_eq!(addr, 0x30);
}

#[test]
fn test_pid_address_mode_creation() {
    let device = MockI3cDevice::with_fixed_response(vec![0x42]);
    let client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::Pid("0x0123456789AB".to_string()),
    );
    // PID mode starts with no resolved address (needs sysfs)
    assert!(client.resolved_address.is_none());
}

#[test]
fn test_device_class_mode_creation() {
    let device = MockI3cDevice::with_fixed_response(vec![0x42]);
    let client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::DeviceClass {
            class: "temperature-sensor".to_string(),
            instance: 0,
        },
    );
    assert!(client.resolved_address.is_none());
}

// ── Read with static address ────────────────────────────────────────

#[test]
fn test_read_register_static() {
    let device = MockI3cDevice::with_fixed_response(vec![0xAB, 0xCD]);
    let mut client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::Static(0x30),
    );
    let data = client.read_register_sync(0xFA, 2).unwrap();
    assert_eq!(data, vec![0xAB, 0xCD]);
}

// ── NACK re-enumeration test ────────────────────────────────────────

#[test]
fn test_nack_triggers_reenumeration() {
    // First read fails (NACK), but since we're not on Linux and not static,
    // re-enumeration will also fail. Verify the retry attempt logic.
    let responses: Vec<Result<Vec<u8>>> = vec![Err(anyhow::anyhow!("NACK"))];
    let device = MockI3cDevice::new(responses);
    let mut client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::Static(0x30),
    );
    // Static address re-resolves to same address, but device still fails
    // after the initial NACK — retries should happen
    let result = client.read_register_sync(0xFA, 2);
    assert!(result.is_err());
}

// ── Config validation tests ─────────────────────────────────────────

#[test]
fn test_config_i3c_valid_pid() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      pid: "0x0123456789AB"
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u16
        byte_order: big_endian
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    assert!(config.validate().is_ok());
}

#[test]
fn test_config_i3c_valid_static_address() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      address: 0x30
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u16
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    assert!(config.validate().is_ok());
}

#[test]
fn test_config_i3c_valid_device_class() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      device_class: "temperature-sensor"
      instance: 0
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u16
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    assert!(config.validate().is_ok());
}

#[test]
fn test_config_i3c_no_address_mode() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u16
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("exactly one address mode"));
}

#[test]
fn test_config_i3c_multiple_address_modes() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      pid: "0x0123456789AB"
      address: 0x30
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u16
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("exactly one address mode"));
}

#[test]
fn test_config_i3c_address_out_of_range() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      address: 0x05
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u16
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("0x08"));
}

#[test]
fn test_config_i3c_invalid_pid() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      pid: "0xZZZZ"
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u16
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("PID"));
}

#[test]
fn test_config_i3c_device_class_without_instance() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      device_class: "temperature-sensor"
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u16
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("instance"));
}

#[test]
fn test_config_i3c_mid_endian_rejected() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      address: 0x30
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u32
        byte_order: mid_big_endian
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("mid-endian"));
}

#[test]
fn test_config_i3c_slave_id_rejected() {
    let yaml = r#"
global_labels: {}
logging:
  level: info
  output: stdout
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test-i3c
    slave_id: 1
    protocol:
      type: i3c
      bus: "/dev/i3c-0"
      address: 0x30
    metrics:
      - name: temp
        type: gauge
        address: 0xFA
        data_type: u16
"#;
    let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("slave_id"));
}

// ── Metric reading tests ────────────────────────────────────────────

#[tokio::test]
async fn test_read_i3c_metric_u8() {
    let device = MockI3cDevice::with_fixed_response(vec![0x42]);
    let client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::Static(0x30),
    );
    let client = Arc::new(tokio::sync::Mutex::new(client));
    let bus_lock = Arc::new(std::sync::Mutex::new(()));
    let metric = make_metric("temp", 0xFA, DataType::U8, ByteOrder::BigEndian);

    let (_raw, value) = read_i3c_metric(&client, &metric, &bus_lock).await.unwrap();
    assert!((value - 66.0).abs() < f64::EPSILON); // 0x42 = 66
}

#[tokio::test]
async fn test_read_i3c_metric_u16_big_endian() {
    let device = MockI3cDevice::with_fixed_response(vec![0x01, 0x00]);
    let client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::Static(0x30),
    );
    let client = Arc::new(tokio::sync::Mutex::new(client));
    let bus_lock = Arc::new(std::sync::Mutex::new(()));
    let metric = make_metric("temp", 0xFA, DataType::U16, ByteOrder::BigEndian);

    let (_raw, value) = read_i3c_metric(&client, &metric, &bus_lock).await.unwrap();
    assert!((value - 256.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_read_i3c_metric_u16_little_endian() {
    let device = MockI3cDevice::with_fixed_response(vec![0x00, 0x01]);
    let client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::Static(0x30),
    );
    let client = Arc::new(tokio::sync::Mutex::new(client));
    let bus_lock = Arc::new(std::sync::Mutex::new(()));
    let metric = make_metric("temp", 0xFA, DataType::U16, ByteOrder::LittleEndian);

    let (_raw, value) = read_i3c_metric(&client, &metric, &bus_lock).await.unwrap();
    assert!((value - 256.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_read_i3c_metric_f32_big_endian() {
    // IEEE 754: 42.0f32 = 0x42280000
    let device = MockI3cDevice::with_fixed_response(vec![0x42, 0x28, 0x00, 0x00]);
    let client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::Static(0x30),
    );
    let client = Arc::new(tokio::sync::Mutex::new(client));
    let bus_lock = Arc::new(std::sync::Mutex::new(()));
    let metric = make_metric("temp", 0xFA, DataType::F32, ByteOrder::BigEndian);

    let (_raw, value) = read_i3c_metric(&client, &metric, &bus_lock).await.unwrap();
    assert!((value - 42.0).abs() < 0.001);
}

#[tokio::test]
async fn test_read_i3c_metric_with_scale_offset() {
    let device = MockI3cDevice::with_fixed_response(vec![0x00, 0xF5]); // 245
    let client = I3cMetricReader::new(
        Box::new(device),
        "/dev/i3c-0".to_string(),
        AddressMode::Static(0x30),
    );
    let client = Arc::new(tokio::sync::Mutex::new(client));
    let bus_lock = Arc::new(std::sync::Mutex::new(()));
    let mut metric = make_metric("temp", 0xFA, DataType::U16, ByteOrder::BigEndian);
    metric.scale = 0.1;
    metric.offset = -40.0;

    let (_raw, value) = read_i3c_metric(&client, &metric, &bus_lock).await.unwrap();
    // 245 * 0.1 + (-40.0) = -15.5
    assert!((value - (-15.5)).abs() < 0.001);
}
