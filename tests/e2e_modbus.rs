//! E2E integration test: Modbus TCP simulator → bus-exporter pull → JSON validation.
//!
//! Uses the shared test harness from `tests/common/mod.rs` for fixtures,
//! config generation, pull execution, and validation. The Modbus TCP simulator
//! is also shared from the common module.

#[allow(dead_code)]
mod common;

use common::{standard_fixtures, ConnectionParams};

// ── Test ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_modbus_tcp_pull() {
    let fixtures = standard_fixtures();

    // 1. Start simulator populated from shared fixtures
    let (sim_addr, sim_handle) = common::start_simulator(&fixtures).await;

    // 2. Run shared e2e workflow
    let connection = ConnectionParams::ModbusTcp {
        endpoint: format!("{}:{}", sim_addr.ip(), sim_addr.port()),
        slave_id: 1,
    };
    common::run_e2e_workflow("test_device", &connection, &fixtures).await;

    // Cleanup
    sim_handle.abort();
}
