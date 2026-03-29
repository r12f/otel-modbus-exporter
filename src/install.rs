use anyhow::{bail, Result};
use std::path::PathBuf;

const UNIT_TEMPLATE: &str = r#"[Unit]
Description=Bus Exporter - Industrial bus metrics exporter
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={bin_path} run -c {config_path}
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=bus-exporter

[Install]
WantedBy={wanted_by}
"#;

const SERVICE_NAME: &str = "bus-exporter.service";

pub fn run_install(
    user: bool,
    config_path: Option<PathBuf>,
    bin_path: Option<PathBuf>,
    uninstall: bool,
) -> Result<()> {
    // Platform check
    if !cfg!(target_os = "linux") {
        bail!("install command is only supported on Linux with systemd");
    }

    // Check systemctl availability
    let status = std::process::Command::new("systemctl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if status.is_err() || !status.unwrap().success() {
        bail!("systemd is not available on this system");
    }

    let unit_path = if user {
        let home = std::env::var("HOME").unwrap_or_default();
        let dir = PathBuf::from(home).join(".config/systemd/user");
        std::fs::create_dir_all(&dir)?;
        dir.join(SERVICE_NAME)
    } else {
        PathBuf::from("/etc/systemd/system").join(SERVICE_NAME)
    };

    let systemctl = |args: &[&str]| -> Result<()> {
        let mut cmd = std::process::Command::new("systemctl");
        if user {
            cmd.arg("--user");
        }
        cmd.args(args);
        let status = cmd.status()?;
        if !status.success() {
            // Some commands (like stop on non-running service) are okay to fail
        }
        Ok(())
    };

    if uninstall {
        eprintln!("Stopping and disabling bus-exporter...");
        let _ = systemctl(&["stop", "bus-exporter"]);
        let _ = systemctl(&["disable", "bus-exporter"]);

        if unit_path.exists() {
            std::fs::remove_file(&unit_path)?;
            eprintln!("Removed {}", unit_path.display());
        }

        systemctl(&["daemon-reload"])?;
        eprintln!("bus-exporter service uninstalled.");
        return Ok(());
    }

    // Determine binary path
    let bin = bin_path.unwrap_or_else(|| {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("bus-exporter"))
    });

    // Determine config path
    let cfg = config_path.unwrap_or_else(|| PathBuf::from("/etc/bus-exporter/config.yaml"));

    // Generate unit file
    let wanted_by = if user {
        "default.target"
    } else {
        "multi-user.target"
    };
    let unit_content = UNIT_TEMPLATE
        .replace("{bin_path}", &bin.display().to_string())
        .replace("{config_path}", &cfg.display().to_string())
        .replace("{wanted_by}", wanted_by);

    std::fs::write(&unit_path, &unit_content)?;
    eprintln!("Wrote {}", unit_path.display());

    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "bus-exporter"])?;

    eprintln!("bus-exporter service installed and enabled.");
    eprintln!(
        "Start with: systemctl {}start bus-exporter",
        if user { "--user " } else { "" }
    );

    Ok(())
}
