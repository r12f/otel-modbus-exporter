//! E2E integration test: Modbus RTU over virtual serial pair → bus-exporter pull → JSON validation.
//!
//! Uses `socat` to create a virtual serial (PTY) pair, then runs a mock RTU
//! responder on one end and points bus-exporter at the other.
//!
//! Requires `socat` and the `serialport` crate's system deps to be installed.

mod common;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;

use common::{standard_fixtures, ConnectionParams, TestFixtures};

// ── CRC-16/Modbus ─────────────────────────────────────────────────────

/// Compute Modbus CRC-16 (polynomial 0xA001).
fn modbus_crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

/// Append CRC-16 to a frame (little-endian: crc_lo, crc_hi).
fn append_crc(frame: &mut Vec<u8>) {
    let crc = modbus_crc16(frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);
}

/// Verify CRC of a received frame. Returns true if valid.
fn verify_crc(frame: &[u8]) -> bool {
    if frame.len() < 3 {
        return false;
    }
    let crc = modbus_crc16(&frame[..frame.len() - 2]);
    let expected = u16::from_le_bytes([frame[frame.len() - 2], frame[frame.len() - 1]]);
    crc == expected
}

// ── Register store ────────────────────────────────────────────────────

struct RegisterStore {
    holding: HashMap<u16, u16>,
    input: HashMap<u16, u16>,
    coils: HashMap<u16, bool>,
}

impl RegisterStore {
    fn from_fixtures(fixtures: &TestFixtures) -> Self {
        let mut holding = HashMap::new();
        let mut input = HashMap::new();
        let mut coils = HashMap::new();

        for m in &fixtures.metrics {
            match m.register_type {
                "holding" => {
                    for (i, &val) in m.raw_registers.iter().enumerate() {
                        holding.insert(m.address + i as u16, val);
                    }
                }
                "input" => {
                    for (i, &val) in m.raw_registers.iter().enumerate() {
                        input.insert(m.address + i as u16, val);
                    }
                }
                "coil" => {
                    for (i, &val) in m.raw_registers.iter().enumerate() {
                        coils.insert(m.address + i as u16, val != 0);
                    }
                }
                _ => {}
            }
        }

        Self {
            holding,
            input,
            coils,
        }
    }
}

// ── Mock RTU responder ────────────────────────────────────────────────

/// Handle a single Modbus RTU request frame and return the response frame.
fn handle_rtu_request(frame: &[u8], store: &RegisterStore) -> Option<Vec<u8>> {
    // Minimum RTU request: slave(1) + fc(1) + addr(2) + count(2) + crc(2) = 8
    if frame.len() < 8 {
        return None;
    }
    if !verify_crc(frame) {
        return None;
    }

    let slave_id = frame[0];
    let function_code = frame[1];
    let addr = u16::from_be_bytes([frame[2], frame[3]]);
    let count = u16::from_be_bytes([frame[4], frame[5]]);

    let mut response = Vec::new();
    response.push(slave_id);
    response.push(function_code);

    match function_code {
        // 0x03: Read Holding Registers
        0x03 => {
            let byte_count = (count * 2) as u8;
            response.push(byte_count);
            for i in 0..count {
                let val = store.holding.get(&(addr + i)).copied().unwrap_or(0);
                response.extend_from_slice(&val.to_be_bytes());
            }
        }
        // 0x04: Read Input Registers
        0x04 => {
            let byte_count = (count * 2) as u8;
            response.push(byte_count);
            for i in 0..count {
                let val = store.input.get(&(addr + i)).copied().unwrap_or(0);
                response.extend_from_slice(&val.to_be_bytes());
            }
        }
        // 0x01: Read Coils
        0x01 => {
            let byte_count = ((count + 7) / 8) as u8;
            response.push(byte_count);
            let mut bytes = vec![0u8; byte_count as usize];
            for i in 0..count {
                let val = store.coils.get(&(addr + i)).copied().unwrap_or(false);
                if val {
                    bytes[i as usize / 8] |= 1 << (i % 8);
                }
            }
            response.extend_from_slice(&bytes);
        }
        _ => {
            // Exception response
            response.clear();
            response.push(slave_id);
            response.push(function_code | 0x80);
            response.push(0x01); // Illegal Function
        }
    }

    append_crc(&mut response);
    Some(response)
}

/// Try to extract a complete Modbus RTU frame from the buffer.
/// Returns the frame length if a valid frame is found, or `None` if more data is needed.
fn try_extract_frame(buf: &[u8]) -> Option<usize> {
    // Minimum RTU request: slave(1) + fc(1) + addr(2) + count(2) + crc(2) = 8
    if buf.len() < 8 {
        return None;
    }
    // Try all plausible lengths starting from 8
    for len in 8..=buf.len() {
        if verify_crc(&buf[..len]) {
            return Some(len);
        }
    }
    None
}

/// Run the mock RTU responder on the given PTY device path.
/// Blocks until the stop flag is set or an error occurs.
fn run_mock_rtu_responder(
    pty_path: &str,
    store: Arc<RegisterStore>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    let mut port = serialport::new(pty_path, 9600)
        .timeout(std::time::Duration::from_millis(200))
        .open()
        .unwrap_or_else(|e| panic!("failed to open PTY {}: {}", pty_path, e));

    let mut accum = Vec::with_capacity(256);
    let mut tmp = [0u8; 256];
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        match port.read(&mut tmp) {
            Ok(n) => {
                accum.extend_from_slice(&tmp[..n]);
                // Try to extract a complete frame from accumulated bytes
                while let Some(frame_len) = try_extract_frame(&accum) {
                    let frame: Vec<u8> = accum.drain(..frame_len).collect();
                    if let Some(response) = handle_rtu_request(&frame, &store) {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        let _ = port.write_all(&response);
                        let _ = port.flush();
                    }
                }
                // Prevent unbounded accumulation if we get garbage
                if accum.len() > 512 {
                    accum.clear();
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
}

// ── Socat PTY pair helper ─────────────────────────────────────────────

/// RAII guard that cleans up the socat child process and responder thread on drop.
/// Prevents zombie processes if assertions panic.
struct TestGuard {
    socat_child: Option<std::process::Child>,
    responder_handle: Option<std::thread::JoinHandle<()>>,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
}

impl TestGuard {
    fn new(
        socat_child: std::process::Child,
        stop_flag: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            socat_child: Some(socat_child),
            responder_handle: None,
            stop_flag,
        }
    }

    fn set_responder(&mut self, handle: std::thread::JoinHandle<()>) {
        self.responder_handle = Some(handle);
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        self.stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(handle) = self.responder_handle.take() {
            let _ = handle.join();
        }
        if let Some(mut child) = self.socat_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Spawn socat to create a virtual serial pair. Returns (pty1, pty2, child).
fn spawn_socat() -> (String, String, std::process::Child) {
    use std::io::BufRead;

    let mut child = std::process::Command::new("socat")
        .args(["-d", "-d", "pty,raw,echo=0", "pty,raw,echo=0"])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn socat - is it installed?");

    let stderr = child.stderr.take().unwrap();
    let reader = std::io::BufReader::new(stderr);

    let mut ptys = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut remaining_stderr: Option<std::io::BufReader<std::process::ChildStderr>> = None;

    // We need to consume lines to find PTY paths, then drain the rest in a thread.
    // BufReader consumes the inner reader, so we use lines() and break out.
    let mut lines_iter = reader;

    loop {
        if std::time::Instant::now() > deadline {
            panic!("timed out waiting for socat PTY paths");
        }
        let mut line = String::new();
        match lines_iter.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                if let Some(pos) = line.find("PTY is ") {
                    let pty_path = line[pos + 7..].trim().to_string();
                    ptys.push(pty_path);
                    if ptys.len() == 2 {
                        remaining_stderr = Some(lines_iter);
                        break;
                    }
                }
            }
            Err(e) => panic!("failed to read socat stderr: {}", e),
        }
    }

    // Spawn a thread to drain remaining socat stderr to prevent blocking/SIGPIPE
    if let Some(mut stderr_reader) = remaining_stderr {
        std::thread::spawn(move || {
            let mut discard = [0u8; 1024];
            loop {
                match std::io::Read::read(&mut stderr_reader, &mut discard) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });
    }

    assert_eq!(
        ptys.len(),
        2,
        "expected 2 PTY paths from socat, got {:?}",
        ptys
    );

    (ptys[0].clone(), ptys[1].clone(), child)
}

// ── Test ──────────────────────────────────────────────────────────────

/// E2E test for Modbus RTU using a virtual serial pair.
///
/// Ignored by default because it requires `socat` to be installed on the
/// system. Run with: `cargo test --test e2e_modbus_rtu -- --ignored`
#[tokio::test]
#[ignore]
async fn e2e_modbus_rtu_pull() {
    let fixtures = standard_fixtures();

    // 1. Create virtual serial pair via socat
    let (pty_responder, pty_exporter, socat_child) = spawn_socat();
    eprintln!(
        "socat PTYs: responder={}, exporter={}",
        pty_responder, pty_exporter
    );

    let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut guard = TestGuard::new(socat_child, stop_flag.clone());

    // Small delay for PTYs to be ready
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // 2. Start mock RTU responder on one PTY
    let store = Arc::new(RegisterStore::from_fixtures(&fixtures));
    let stop_clone = stop_flag.clone();
    let pty_resp = pty_responder.clone();
    let responder_handle = std::thread::spawn(move || {
        run_mock_rtu_responder(&pty_resp, store, stop_clone);
    });
    guard.set_responder(responder_handle);

    // 3. Generate config pointing at the other PTY
    let tmp = tempfile::tempdir().unwrap();
    let connection = ConnectionParams::ModbusRtu {
        device: pty_exporter.clone(),
        bps: 9600,
        slave_id: 1,
    };
    let config_path =
        common::generate_config(tmp.path(), "test_rtu_device", &connection, &fixtures);

    // 4. Run pull
    let result = common::run_pull(&config_path).await;
    assert_eq!(
        result.exit_code,
        Some(0),
        "pull failed:\nstderr: {}",
        result.stderr
    );

    // 5. Validate results
    common::validate(&result, &fixtures);

    // Cleanup handled by TestGuard's Drop impl
    drop(guard);
}
