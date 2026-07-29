use clap::{Parser, Subcommand};
use std::process::ExitCode;

mod daemon;
mod tui;
mod tray;

#[derive(Parser)]
#[command(name = "audiosource")]
#[command(version = "1.2.0")]
#[command(about = "Audio Source Linux Client", long_about = None)]
struct Cli {
    #[arg(short, long, help = "Use device with given serial (overrides $ANDROID_SERIAL)")]
    serial: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run Audio Source and start forwarding
    Run {
        /// Automatically restart
        #[arg(short, long)]
        restart: bool,
    },
    /// Set volume
    Volume {
        /// Volume level (e.g. 250%)
        level: String,
    },
    /// Run the Terminal User Interface
    Tui,
    /// Run the System Tray Daemon
    Tray,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Run { restart }) => {
            if let Err(e) = daemon::run_bridge(cli.serial, *restart) {
                eprintln!("Error: {:?}", e);
                return ExitCode::FAILURE;
            }
        }
        Some(Commands::Volume { level }) => {
            if let Err(e) = daemon::set_volume(cli.serial, level) {
                eprintln!("Error: {:?}", e);
                return ExitCode::FAILURE;
            }
        }
        Some(Commands::Tui) => {
            if let Err(e) = tui::run_tui() {
                eprintln!("TUI Error: {:?}", e);
            }
        }
        Some(Commands::Tray) => {
            if let Err(e) = tray::run_tray() {
                eprintln!("Tray Error: {:?}", e);
            }
        }
        None => {
            // Default to TUI if no arguments provided (as in python original)
            if let Err(e) = tui::run_tui() {
                eprintln!("TUI Error: {:?}", e);
            }
        }
    }
    
    ExitCode::SUCCESS
}
