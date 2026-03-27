mod config;

use clap::Parser;

fn main() {
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
