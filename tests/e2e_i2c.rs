//! E2E integration test: I2C via i2c-stub kernel module → bus-exporter pull → JSON validation.
//!
//! Uses the shared test harness from `tests/common/mod.rs` for config generation,
//! pull execution, and validation. The i2c-stub kernel module provides a simulated
//! I2C device.
//!
//! **Requirements:** root privileges, `i2c-stub` kernel module available.
//! The test is marked `#[ignore]` and skips gracefully when prerequisites are missing.

mod common;

use common::{ConnectionParams, TestFixtures, TestMetric};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

// ── I2C-specific test fixtures ────────────────────────────────────────

/// I2C test fixtures using only holding registers with u8/u16 reads.
/// Addresses are I2C register byte offsets (u8 range).
fn i2c_fixtures() -> TestFixtures {
    TestFixtures {
        metrics: vec![
            // u16 big-endian at register 0x00: raw=2300, scale=0.1 → 230.0
            TestMetric {
                name: "voltage",
                description: "I2C voltage sensor",
                metric_type: "gauge",
                register_type: "",
                address: 0x00,
                data_type: "u16",
                byte_order: "big_endian",
                scale: 0.1,
                offset: 0.0,
                unit: "V",
                raw_registers: vec![2300],
                expected_value: 230.0,
            },
            // u16 big-endian at register 0x10: raw=500, scale=0.1, offset=-10 → 40.0
            TestMetric {
                name: "temperature",
                description: "I2C temperature sensor",
                metric_type: "gauge",
                register_type: "",
                address: 0x10,
                data_type: "u16",
                byte_order: "big_endian",
                scale: 0.1,
                offset: -10.0,
                unit: "C",
                raw_registers: vec![500],
                expected_value: 40.0,
            },
        ],
    }
}

// ── i2c-stub helpers ──────────────────────────────────────────────────

const I2C_STUB_CHIP_ADDR: u8 = 0x50;

/// Try to load the i2c-stub kernel module. Returns true if successful.
fn load_i2c_stub() -> bool {
    let status = Command::new("modprobe")
        .args([
            "i2c-stub",
            &format!("chip_addr=0x{:02x}", I2C_STUB_CHIP_ADDR),
        ])
        .status();
    match status {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

/// Unload the i2c-stub kernel module.
fn unload_i2c_stub() {
    let _ = Command::new("rmmod").arg("i2c-stub").status();
}

/// Find the I2C bus number for the i2c-stub adapter by scanning sysfs.
/// Returns the bus path (e.g., "/dev/i2c-3") if found.
fn find_stub_bus() -> Option<PathBuf> {
    let devices_dir = PathBuf::from("/sys/bus/i2c/devices");
    if !devices_dir.exists() {
        return None;
    }

    for entry in fs::read_dir(&devices_dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();

        // Look for adapter directories (i2c-N)
        let name = path.file_name()?.to_str()?;
        if !name.starts_with("i2c-") {
            continue;
        }

        // Check if this adapter is the stub
        let adapter_name_path = path.join("name");
        if let Ok(adapter_name) = fs::read_to_string(&adapter_name_path) {
            if adapter_name.trim().contains("SMBus stub") {
                let bus_num = name.strip_prefix("i2c-")?;
                return Some(PathBuf::from(format!("/dev/i2c-{}", bus_num)));
            }
        }
    }

    None
}

/// Write a u16 word value to a register on the i2c-stub device using i2cset word mode.
/// Uses SMBus word write: i2cset -y <bus> <addr> <reg> <value> w
fn i2c_write_word(bus_num: &str, register: u8, value: u16) -> bool {
    let status = Command::new("i2cset")
        .args([
            "-y",
            bus_num,
            &format!("0x{:02x}", I2C_STUB_CHIP_ADDR),
            &format!("0x{:02x}", register),
            &format!("0x{:04x}", value),
            "w",
        ])
        .status();
    matches!(status, Ok(s) if s.success())
}

/// Write fixture values to the i2c-stub device.
/// Uses SMBus word writes so that smbus_read_word_data can read them back.
fn write_fixtures_to_stub(bus_num: &str, fixtures: &TestFixtures) -> bool {
    for m in &fixtures.metrics {
        for (i, &raw_word) in m.raw_registers.iter().enumerate() {
            let reg = m.address as u8 + (i as u8 * 2);
            if !i2c_write_word(bus_num, reg, raw_word) {
                eprintln!(
                    "Failed to write word for metric '{}' at register 0x{:02x}",
                    m.name, reg
                );
                return false;
            }
        }
    }
    true
}

// ── Test ──────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore] // Requires root + i2c-stub kernel module
async fn e2e_i2c_pull() {
    // 1. Load i2c-stub module
    if !load_i2c_stub() {
        eprintln!("Skipping I2C e2e test: cannot load i2c-stub module (need root + module)");
        return;
    }

    // Ensure cleanup on exit
    struct StubGuard;
    impl Drop for StubGuard {
        fn drop(&mut self) {
            unload_i2c_stub();
        }
    }
    let _guard = StubGuard;

    // 2. Find the stub bus
    let bus_path = match find_stub_bus() {
        Some(p) => p,
        None => {
            eprintln!("Skipping I2C e2e test: could not find i2c-stub bus in sysfs");
            return;
        }
    };
    let bus_path_str = bus_path.to_str().unwrap();

    // Extract bus number for i2cset
    let bus_num = bus_path_str
        .strip_prefix("/dev/i2c-")
        .expect("unexpected bus path format");

    let fixtures = i2c_fixtures();

    // 3. Write test register values to the stub
    if !write_fixtures_to_stub(bus_num, &fixtures) {
        panic!("Failed to write fixture data to i2c-stub device");
    }

    // 4. Run shared e2e workflow
    let connection = ConnectionParams::I2c {
        bus: bus_path_str.to_string(),
        address: I2C_STUB_CHIP_ADDR,
    };
    common::run_e2e_workflow("i2c_test_device", &connection, &fixtures).await;
}
