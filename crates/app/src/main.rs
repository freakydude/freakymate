// Do not open a console window in Windows release builds.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use clap::Parser;
use freakymate::app::{Application, Options};
use freakymate::logging;
use freakymate_core::backlight::BacklightBackend;
use freakymate_core::network::NetworkBackend;
use freakymate_core::setup::{self, SetupState};
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
    /// Prints the setup version install.sh writes to /etc/freakymate/setup-version and exits
    #[arg(long, hide = true)]
    setup_version: bool,
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

    /// Whether the setup outside the program folder matches this program (see `freakymate_core::setup`).
    fn setup_state(&self) -> SetupState {
        let marker = std::fs::read_to_string(setup::MARKER_FILE).ok();
        SetupState::detect(marker.as_deref(), self.settings.as_deref())
    }

    /// What the log file says about this run at its top. Display and renderer follow once the window exists.
    fn log_header(&self, setup: SetupState) -> Vec<(String, String)> {
        let command_line: Vec<String> = std::env::args().skip(1).collect();
        let mut header = vec![
            ("Version".to_owned(), freakymate_core::VERSION.to_owned()),
            ("Command line".to_owned(), command_line.join(" ")),
            ("System".to_owned(), format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)),
        ];
        let installed = match setup {
            SetupState::NotInstalled => None,
            SetupState::Current => Some(format!("{} (current)", setup::SETUP_VERSION)),
            SetupState::Outdated { installed } => Some(format!(
                "{} (outdated, this program needs {})",
                installed.map_or_else(|| "none".to_owned(), |version| version.to_string()),
                setup::SETUP_VERSION
            )),
        };
        header.extend(installed.map(|text| ("Setup".to_owned(), text)));
        header
    }
}

/// The command that brings an outdated setup up to date: the `install.sh` next to the program, as Moonraker's updates
/// bring it along.
fn setup_command() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|program| program.parent().map(|folder| folder.join("install.sh")))
        .filter(|script| script.exists())
        .map_or_else(|| "install.sh".to_owned(), |script| script.display().to_string())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.setup_version {
        println!("{}", setup::SETUP_VERSION);
        return ExitCode::SUCCESS;
    }
    let setup = cli.setup_state();
    let config = logging::Config { file: cli.logfile.clone(), header: cli.log_header(setup) };
    if let Err(error) = logging::init(config) {
        eprintln!("cannot start the log: {error}");
        return ExitCode::FAILURE;
    }
    // The kiosk on the touch screen shows no pointer; set up before the window and the runtime's threads exist.
    if cli.fullscreen {
        freakymate::kiosk_pointer::hide();
    }
    let setup_command = matches!(setup, SetupState::Outdated { .. }).then(setup_command);
    if let Some(command) = &setup_command {
        log::warn!("the setup outside the program folder is older than this program; run {command} once");
    }
    let options = Options {
        setup_command,
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
