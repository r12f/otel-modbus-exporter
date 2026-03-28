use super::*;
use crate::config::{ByteOrder, DataType, Metric, MetricType};

/// Mock SPI device for testing.
struct MockSpiDevice {
    /// Expected TX -> RX response mapping (by TX prefix).
    responses: HashMap<Vec<u8>, Vec<u8>>,
}

impl MockSpiDevice {
    fn new(responses: HashMap<Vec<u8>, Vec<u8>>) -> Self {
        Self { responses }
    }
}

impl SpiDevice for MockSpiDevice {
    fn transfer(&mut self, tx_buf: &[u8]) -> Result<Vec<u8>> {
        for (tx_key, rx_data) in &self.responses {
            if tx_buf.starts_with(tx_key) {
                let mut rx = rx_data.clone();
                // Pad or truncate to match tx_buf length (SPI is full-duplex)
                rx.resize(tx_buf.len(), 0);
                return Ok(rx);
            }
        }
        anyhow::bail!("no mock response for TX: {:?}", tx_buf)
    }
}

fn make_spi_metric(
    name: &str,
    command: Vec<u8>,
    response_length: Option<u16>,
    response_offset: u16,
    data_type: DataType,
) -> Metric {
    Metric {
        name: name.to_string(),
        description: String::new(),
        metric_type: MetricType::Gauge,
        register_type: None,
        address: None,
        data_type,
        byte_order: ByteOrder::BigEndian,
        scale: 1.0,
        offset: 0.0,
        unit: String::new(),
        command,
        response_length,
        response_offset,
    }
}

fn make_device_lock() -> DeviceLock {
    Arc::new(tokio::sync::Mutex::new(()))
}

#[tokio::test]
async fn test_read_u8() {
    let mut responses = HashMap::new();
    responses.insert(vec![0x01], vec![0x2A]);
    let device = MockSpiDevice::new(responses);
    let client = SpiClient::new(Box::new(device), "/dev/spidev0.0".into());

    let metric = make_spi_metric("val", vec![0x01], None, 0, DataType::U8);
    let lock = make_device_lock();
    let val = read_spi_metric(&client, &metric, &lock).await.unwrap();
    assert!((val - 42.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_read_u16_with_offset() {
    let mut responses = HashMap::new();
    // Command: 3 bytes, response: [0x00, 0x01, 0x00] -> skip byte 0, read u16 from bytes 1-2 = 256
    responses.insert(vec![0x06, 0x00, 0x00], vec![0x00, 0x01, 0x00]);
    let device = MockSpiDevice::new(responses);
    let client = SpiClient::new(Box::new(device), "/dev/spidev0.0".into());

    let metric = make_spi_metric("adc", vec![0x06, 0x00, 0x00], Some(3), 1, DataType::U16);
    let lock = make_device_lock();
    let val = read_spi_metric(&client, &metric, &lock).await.unwrap();
    assert!((val - 256.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_read_with_scale_offset() {
    let mut responses = HashMap::new();
    responses.insert(vec![0x06, 0x00, 0x00], vec![0x00, 0x00, 0x64]);
    let device = MockSpiDevice::new(responses);
    let client = SpiClient::new(Box::new(device), "/dev/spidev0.0".into());

    let mut metric = make_spi_metric("adc", vec![0x06, 0x00, 0x00], Some(3), 1, DataType::U16);
    metric.scale = 0.01;
    metric.offset = -40.0;

    let lock = make_device_lock();
    let val = read_spi_metric(&client, &metric, &lock).await.unwrap();
    // bytes[1..3] = [0x00, 0x64] = 100 big-endian u16
    // 100 * 0.01 + (-40.0) = -39.0
    assert!((val - (-39.0)).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_device_lock_per_device() {
    let lock1 = get_device_lock("/dev/spidev0.0");
    let lock2 = get_device_lock("/dev/spidev0.0");
    assert!(Arc::ptr_eq(&lock1, &lock2));

    let lock3 = get_device_lock("/dev/spidev0.1");
    assert!(!Arc::ptr_eq(&lock1, &lock3));
}

#[tokio::test]
async fn test_read_f32() {
    let val: f32 = 3.14;
    let bytes = val.to_be_bytes();
    let mut responses = HashMap::new();
    responses.insert(vec![0x01], bytes.to_vec());
    let device = MockSpiDevice::new(responses);
    let client = SpiClient::new(Box::new(device), "/dev/spidev0.0".into());

    let metric = make_spi_metric(
        "pressure",
        vec![0x01, 0x00, 0x00, 0x00],
        Some(4),
        0,
        DataType::F32,
    );
    let lock = make_device_lock();
    let result = read_spi_metric(&client, &metric, &lock).await.unwrap();
    assert!((result - 3.14_f64).abs() < 0.001);
}

#[tokio::test]
async fn test_zero_pad_tx_buffer() {
    // Command is 1 byte but response_length is 3 -> TX should be zero-padded
    let mut responses = HashMap::new();
    responses.insert(vec![0x06], vec![0x00, 0x01, 0x00]);
    let device = MockSpiDevice::new(responses);
    let client = SpiClient::new(Box::new(device), "/dev/spidev0.0".into());

    let metric = make_spi_metric("adc", vec![0x06], Some(3), 1, DataType::U16);
    let lock = make_device_lock();
    let val = read_spi_metric(&client, &metric, &lock).await.unwrap();
    assert!((val - 256.0).abs() < f64::EPSILON);
}
