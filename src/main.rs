#![allow(dead_code)]
mod config;
mod decoder;
pub mod export;
mod logging;
pub mod metrics;
mod modbus;

use clap::Parser;
use logging::{init_logging, LoggingConfig};

fn main() {
    let config = LoggingConfig::default();
    if let Err(e) = init_logging(&config) {
        eprintln!("failed to initialize logging: {e}");
    }

    let cli = config::Cli::parse();
    match config::Config::load(&cli.config) {
        Ok(config) => {
            println!(
                "Loaded config with {} collector(s)",
                config.collectors.len()
            );
        }
        Err(e) => {
            eprintln!("Error loading config: {e:#}");
            std::process::exit(1);
        }
    }
}
