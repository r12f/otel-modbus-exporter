use super::RtuClient;
use crate::reader::modbus::{BusConnection, ModbusReader};

#[test]
fn test_rtu_client_new_not_connected() {
    let builder = tokio_serial::new("/dev/null", 9600);
    let client = RtuClient::new(builder, 1);
    assert!(!client.is_connected());
}

#[tokio::test]
async fn test_rtu_client_read_without_connect_fails() {
    let builder = tokio_serial::new("/dev/null", 9600);
    let mut client = RtuClient::new(builder, 1);
    let result = client.read_holding_registers(0, 1).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not connected"));
}

#[tokio::test]
async fn test_rtu_client_disconnect() {
    let builder = tokio_serial::new("/dev/null", 9600);
    let mut client = RtuClient::new(builder, 1);
    // disconnect when not connected is fine
    client.disconnect().await.unwrap();
    assert!(!client.is_connected());
}

#[tokio::test]
async fn test_rtu_count_validation() {
    let builder = tokio_serial::new("/dev/null", 9600);
    let mut client = RtuClient::new(builder, 1);
    assert!(client.read_holding_registers(0, 0).await.is_err());
    assert!(client.read_holding_registers(0, 126).await.is_err());
    assert!(client.read_coils(0, 2001).await.is_err());
}
