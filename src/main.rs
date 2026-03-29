#![allow(dead_code)]

use anyhow::{Context, Result};
use clap::Parser;

use bus_exporter::commands;
use bus_exporter::config::{find_config_file, Cli, Command, Config};
use bus_exporter::logging::{init_logging, LogOutput, LoggingConfig};
use commands::run::map_logging_config;

// ── Entry point ───────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Install {
            user,
            config,
            bin,
            uninstall,
        }) => {
            return commands::install::run_install(user, config, bin, uninstall);
        }
        Some(Command::Pull { collector, metric }) => {
            let config_path = find_config_file(cli.config.as_deref())
                .context("failed to find configuration file");
            let config_path = match config_path {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Fatal: {e:#}");
                    std::process::exit(1);
                }
            };
            let config = match Config::load_for_pull(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Fatal: failed to load configuration: {e:#}");
                    std::process::exit(1);
                }
            };
            let logging_cfg = map_logging_config(&config.logging);
            // For pull, force stderr output
            let pull_logging = LoggingConfig {
                level: logging_cfg.level,
                output: LogOutput::Stderr,
            };
            init_logging(&pull_logging).context("failed to initialize logging")?;

            let exit_code = match commands::pull::run_pull(
                &config,
                collector.as_deref(),
                metric.as_deref(),
            )
            .await
            {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("Fatal: {e:#}");
                    std::process::exit(1);
                }
            };
            std::process::exit(exit_code);
        }
        Some(Command::Run) | None => commands::run::run_daemon(cli).await,
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
