use super::{BusConnection, ModbusReader};
use anyhow::{bail, Result};
use async_trait::async_trait;

/// A mock Modbus client for testing the trait interface.
struct MockClient {
    connected: bool,
    holding_regs: Vec<u16>,
    input_regs: Vec<u16>,
    coils: Vec<bool>,
    discrete_inputs: Vec<bool>,
}

impl MockClient {
    fn new() -> Self {
        Self {
            connected: false,
            holding_regs: vec![100, 200, 300],
            input_regs: vec![10, 20, 30],
            coils: vec![true, false, true],
            discrete_inputs: vec![false, true, false],
        }
    }
}

#[async_trait]
impl BusConnection for MockClient {
    async fn connect(&mut self) -> Result<()> {
        if self.connected {
            self.disconnect().await.ok();
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
impl ModbusReader for MockClient {
    async fn read_holding_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>> {
        super::validate_register_count(count)?;
        let start = addr as usize;
        let end = start + count as usize;
        if end > self.holding_regs.len() {
            bail!(
                "OOB: addr={addr}, count={count}, len={}",
                self.holding_regs.len()
            );
        }
        Ok(self.holding_regs[start..end].to_vec())
    }

    async fn read_input_registers(&mut self, addr: u16, count: u16) -> Result<Vec<u16>> {
        super::validate_register_count(count)?;
        let start = addr as usize;
        let end = start + count as usize;
        if end > self.input_regs.len() {
            bail!(
                "OOB: addr={addr}, count={count}, len={}",
                self.input_regs.len()
            );
        }
        Ok(self.input_regs[start..end].to_vec())
    }

    async fn read_coils(&mut self, addr: u16, count: u16) -> Result<Vec<bool>> {
        super::validate_coil_count(count)?;
        let start = addr as usize;
        let end = start + count as usize;
        if end > self.coils.len() {
            bail!("OOB: addr={addr}, count={count}, len={}", self.coils.len());
        }
        Ok(self.coils[start..end].to_vec())
    }

    async fn read_discrete_inputs(&mut self, addr: u16, count: u16) -> Result<Vec<bool>> {
        super::validate_coil_count(count)?;
        let start = addr as usize;
        let end = start + count as usize;
        if end > self.discrete_inputs.len() {
            bail!(
                "OOB: addr={addr}, count={count}, len={}",
                self.discrete_inputs.len()
            );
        }
        Ok(self.discrete_inputs[start..end].to_vec())
    }
}

#[tokio::test]
async fn test_mock_connect() {
    let mut client = MockClient::new();
    assert!(!client.is_connected());
    client.connect().await.unwrap();
    assert!(client.is_connected());
}

#[tokio::test]
async fn test_mock_disconnect() {
    let mut client = MockClient::new();
    client.connect().await.unwrap();
    client.disconnect().await.unwrap();
    assert!(!client.is_connected());
}

#[tokio::test]
async fn test_mock_read_holding_registers() {
    let mut client = MockClient::new();
    client.connect().await.unwrap();
    let regs = client.read_holding_registers(0, 2).await.unwrap();
    assert_eq!(regs, vec![100, 200]);
}

#[tokio::test]
async fn test_mock_read_input_registers() {
    let mut client = MockClient::new();
    client.connect().await.unwrap();
    let regs = client.read_input_registers(1, 2).await.unwrap();
    assert_eq!(regs, vec![20, 30]);
}

#[tokio::test]
async fn test_mock_read_coils() {
    let mut client = MockClient::new();
    client.connect().await.unwrap();
    let coils = client.read_coils(0, 3).await.unwrap();
    assert_eq!(coils, vec![true, false, true]);
}

#[tokio::test]
async fn test_mock_read_discrete_inputs() {
    let mut client = MockClient::new();
    client.connect().await.unwrap();
    let inputs = client.read_discrete_inputs(0, 2).await.unwrap();
    assert_eq!(inputs, vec![false, true]);
}

#[tokio::test]
async fn test_mock_oob_returns_error() {
    let mut client = MockClient::new();
    client.connect().await.unwrap();
    // Out-of-bounds should return Err, not panic.
    assert!(client.read_holding_registers(0, 10).await.is_err());
    assert!(client.read_input_registers(5, 1).await.is_err());
    assert!(client.read_coils(0, 10).await.is_err());
    assert!(client.read_discrete_inputs(5, 1).await.is_err());
}

#[tokio::test]
async fn test_count_validation_registers() {
    let mut client = MockClient::new();
    client.connect().await.unwrap();
    // 0 count
    assert!(client.read_holding_registers(0, 0).await.is_err());
    // Over 125
    assert!(client.read_holding_registers(0, 126).await.is_err());
}

#[tokio::test]
async fn test_count_validation_coils() {
    let mut client = MockClient::new();
    client.connect().await.unwrap();
    assert!(client.read_coils(0, 0).await.is_err());
    assert!(client.read_coils(0, 2001).await.is_err());
}

#[tokio::test]
async fn test_double_connect_ok() {
    let mut client = MockClient::new();
    client.connect().await.unwrap();
    // Double connect should succeed (disconnects first).
    client.connect().await.unwrap();
    assert!(client.is_connected());
}
