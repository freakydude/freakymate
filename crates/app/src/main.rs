// Do not open a console window in Windows release builds.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use clap::Parser;
use freakymate::app::{Application, Options};
use freakymate::logging;
use freakymate_core::backlight::BacklightBackend;
use freakymate_core::network::NetworkBackend;
use std::path::PathBuf;
use std::process::ExitCode;

/// Touch screen UI for Klipper printers via Moonraker.
#[derive(Debug, Parser)]
#[command(version = freakymate_core::VERSION, about)]
struct Cli {
    /// Moonraker address for this start, e.g. http://printer.local:7125 (default: saved setting)
    #[arg(short, long)]
    moonraker: Option<String>,
    /// Kiosk mode on a touch screen: fullscreen, no window frame, no mouse pointer
    #[arg(short, long)]
    fullscreen: bool,
    /// Settings file (default: <config dir>/freakymate/settings.json; the installer uses printer_data/config)
    #[arg(long)]
    settings: Option<PathBuf>,
    /// Log file, rotated daily like those of Klipper and Moonraker (default: stderr only)
    #[arg(short = 'l', long)]
    logfile: Option<PathBuf>,
    /// Simulated Wi-Fi and Ethernet instead of NetworkManager (development build only)
    #[cfg(feature = "dev-mode")]
    #[arg(long)]
    simulate_network: bool,
    /// Simulated backlight with brightness and standby (development build only)
    #[cfg(feature = "dev-mode")]
    #[arg(long)]
    simulate_backlight: bool,
}

impl Cli {
    fn network_backend(&self) -> NetworkBackend {
        #[cfg(feature = "dev-mode")]
        if self.simulate_network {
            return NetworkBackend::Simulated;
        }
        NetworkBackend::System
    }

    fn backlight_backend(&self) -> BacklightBackend {
        #[cfg(feature = "dev-mode")]
        if self.simulate_backlight {
            return BacklightBackend::Simulated;
        }
        BacklightBackend::system()
    }

    /// What the log file says about this run at its top. Display and renderer follow once the window exists.
    fn log_header(&self) -> Vec<(String, String)> {
        let command_line: Vec<String> = std::env::args().skip(1).collect();
        vec![
            ("Version".to_owned(), freakymate_core::VERSION.to_owned()),
            ("Command line".to_owned(), command_line.join(" ")),
            ("System".to_owned(), format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)),
        ]
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let config = logging::Config { file: cli.logfile.clone(), header: cli.log_header() };
    if let Err(error) = logging::init(config) {
        eprintln!("cannot start the log: {error}");
        return ExitCode::FAILURE;
    }
    // The kiosk on the touch screen shows no pointer; set up before the window and the runtime's threads exist.
    if cli.fullscreen {
        freakymate::kiosk_pointer::hide();
    }
    let options = Options {
        network: cli.network_backend(),
        backlight: cli.backlight_backend(),
        moonraker: cli.moonraker,
        fullscreen: cli.fullscreen,
        settings_file: cli.settings,
    };
    let result = Application::build(options).and_then(Application::run);
    if let Err(error) = &result {
        log::error!("{error}");
    }
    log::logger().flush();
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
