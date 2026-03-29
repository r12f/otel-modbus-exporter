use anyhow::Result;
use clap::Parser;

use std::path::Path;

use bus_exporter::commands;
use bus_exporter::config::{Cli, Command};

// ── Entry point ───────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Install {
            user,
            config,
            bin,
            uninstall,
        }) => commands::install::run_install(user, config, bin, uninstall),
        Some(Command::Pull { collector, metric }) => {
            commands::pull::pull_command(
                cli.config.as_deref().map(Path::new),
                collector.as_deref(),
                metric.as_deref(),
            )
            .await
        }
        Some(Command::Run) | None => commands::run::run_daemon(cli).await,
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
