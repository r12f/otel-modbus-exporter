use super::*;
use crate::config::{ByteOrder, DataType, MetricConfig, MetricType};

/// Mock I2C device for testing.
struct MockI2cDevice {
    /// Map of register address -> response bytes
    responses: HashMap<u8, Vec<u8>>,
}

impl MockI2cDevice {
    fn new(responses: HashMap<u8, Vec<u8>>) -> Self {
        Self { responses }
    }
}

impl I2cDevice for MockI2cDevice {
    fn write_read(&mut self, write_buf: &[u8], read_len: usize) -> Result<Vec<u8>> {
        let register = write_buf
            .first()
            .ok_or_else(|| anyhow::anyhow!("empty write buffer"))?;
        let data = self
            .responses
            .get(register)
            .ok_or_else(|| anyhow::anyhow!("no response for register {:#04x}", register))?;
        if data.len() < read_len {
            anyhow::bail!(
                "mock: insufficient data for register {:#04x}: have {}, need {}",
                register,
                data.len(),
                read_len
            );
        }
        Ok(data[..read_len].to_vec())
    }
}

fn make_metric(name: &str, address: u16, data_type: DataType) -> MetricConfig {
    MetricConfig {
        name: name.to_string(),
        description: String::new(),
        metric_type: MetricType::Gauge,
        register_type: None,
        address: Some(address),
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

fn make_bus_lock() -> BusLock {
    Arc::new(std::sync::Mutex::new(()))
}

#[tokio::test]
async fn test_read_u8() {
    let mut responses = HashMap::new();
    responses.insert(0xFA, vec![0x2A]);
    let device = MockI2cDevice::new(responses);
    let client = I2cMetricReader::new(
        Box::new(device),
        "/dev/i2c-1".into(),
        0x76,
        Arc::new(std::sync::Mutex::new(())),
    );

    let metric = make_metric("temp", 0xFA, DataType::U8);
    let bus_lock = make_bus_lock();
    let val = read_i2c_metric(&client, &metric, &bus_lock).await.unwrap();
    assert!((val - 42.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_read_u16_big_endian() {
    let mut responses = HashMap::new();
    responses.insert(0xFA, vec![0x01, 0x00]); // 256 in big endian
    let device = MockI2cDevice::new(responses);
    let client = I2cMetricReader::new(
        Box::new(device),
        "/dev/i2c-1".into(),
        0x76,
        Arc::new(std::sync::Mutex::new(())),
    );

    let metric = make_metric("temp", 0xFA, DataType::U16);
    let bus_lock = make_bus_lock();
    let val = read_i2c_metric(&client, &metric, &bus_lock).await.unwrap();
    assert!((val - 256.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_read_bool() {
    let mut responses = HashMap::new();
    responses.insert(0x10, vec![0x03]); // bit 0 set
    let device = MockI2cDevice::new(responses);
    let client = I2cMetricReader::new(
        Box::new(device),
        "/dev/i2c-1".into(),
        0x48,
        Arc::new(std::sync::Mutex::new(())),
    );

    let metric = make_metric("flag", 0x10, DataType::Bool);
    let bus_lock = make_bus_lock();
    let val = read_i2c_metric(&client, &metric, &bus_lock).await.unwrap();
    assert!((val - 1.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_read_with_scale_offset() {
    let mut responses = HashMap::new();
    responses.insert(0xFA, vec![0x00, 0x64]); // 100 in big endian u16
    let device = MockI2cDevice::new(responses);
    let client = I2cMetricReader::new(
        Box::new(device),
        "/dev/i2c-1".into(),
        0x76,
        Arc::new(std::sync::Mutex::new(())),
    );

    let mut metric = make_metric("temp", 0xFA, DataType::U16);
    metric.scale = 0.01;
    metric.offset = -40.0;

    let bus_lock = make_bus_lock();
    let val = read_i2c_metric(&client, &metric, &bus_lock).await.unwrap();
    // 100 * 0.01 + (-40.0) = -39.0
    assert!((val - (-39.0)).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_read_register_not_found() {
    let responses = HashMap::new(); // empty
    let device = MockI2cDevice::new(responses);
    let client = I2cMetricReader::new(
        Box::new(device),
        "/dev/i2c-1".into(),
        0x76,
        Arc::new(std::sync::Mutex::new(())),
    );

    let metric = make_metric("temp", 0xFA, DataType::U8);
    let bus_lock = make_bus_lock();
    let result = read_i2c_metric(&client, &metric, &bus_lock).await;
    assert!(result.is_err());
}

#[test]
fn test_bus_lock_serialization() {
    let lock1 = get_bus_lock("/dev/i2c-1");
    let lock2 = get_bus_lock("/dev/i2c-1");
    // Same bus -> same lock (Arc pointing to same allocation)
    assert!(Arc::ptr_eq(&lock1, &lock2));

    let lock3 = get_bus_lock("/dev/i2c-2");
    // Different bus -> different lock
    assert!(!Arc::ptr_eq(&lock1, &lock3));
}

#[tokio::test]
async fn test_read_f32() {
    let val: f32 = 3.14;
    let bytes = val.to_be_bytes();
    let mut responses = HashMap::new();
    responses.insert(0x20, bytes.to_vec());
    let device = MockI2cDevice::new(responses);
    let client = I2cMetricReader::new(
        Box::new(device),
        "/dev/i2c-1".into(),
        0x50,
        Arc::new(std::sync::Mutex::new(())),
    );

    let metric = make_metric("pressure", 0x20, DataType::F32);
    let bus_lock = make_bus_lock();
    let result = read_i2c_metric(&client, &metric, &bus_lock).await.unwrap();
    assert!((result - 3.14_f64).abs() < 0.001);
}
