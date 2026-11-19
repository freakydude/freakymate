//! UI-independent core of sailhand: Moonraker connection, printer state, commands and settings.
//!
//! Layers, each using only the ones before it: [`moonraker`] is the transport and knows no printer objects;
//! [`sources`] has one module per origin of data (Klipper, Moonraker's components, Happy Hare, Spoolman, ...);
//! [`domain`] answers questions that need several sources; [`service`] only collects. The entry point is
//! [`PrinterService`]; the UI reads [`Snapshot`]s and [`Event`]s and sends [`Command`]s.

pub mod atomic_file;
pub mod backlight;
pub mod color;
pub mod commands;
pub mod credentials;
pub mod domain;
mod klipper_ini;
pub mod moonraker;
pub mod network;
pub mod printers;
pub mod report;
pub mod service;
pub mod settings;
pub mod setup;
pub mod sourced;
pub mod sources;
pub mod step_choices;

pub use commands::Command;
pub use credentials::CredentialStore;
pub use moonraker::Endpoint;
pub use service::{Access, ConnectionState, Event, PrinterService, Query, ServiceChannels, Snapshot};
pub use settings::{Settings, SettingsStore};
pub use sourced::{Provenance, Sourced};

/// Version of this build: set by CI from git tags, otherwise the crate version marked as a local build.
pub const VERSION: &str = match option_env!("SAILHAND_VERSION") {
    Some(version) => version,
    None => concat!(env!("CARGO_PKG_VERSION"), "-dev"),
};

/// Only builds from main (releases X.Y.0 and pre-releases X.Y.Z) are Sailhand; a version with a suffix (`-dev`
/// locally and on other branches, `-pr.N.M` for a pull request, see `.github/scripts/version.sh`) is a test build.
pub fn is_test_build() -> bool {
    is_test_version(VERSION)
}

/// The folder in the user's configuration directory: test builds keep their settings and logins apart, so trying one
/// never changes the installed display.
pub fn config_folder() -> &'static str {
    if is_test_build() { "sailhand-dev" } else { "sailhand" }
}

fn is_test_version(version: &str) -> bool {
    version.contains('-')
}

/// Log target for facts about the machine and the printer (`name: value`); the log file repeats the latest of each
/// in its header.
pub const HEADER_TARGET: &str = "sailhand::header";

#[cfg(test)]
mod tests {
    use super::is_test_version;

    #[test]
    fn only_versions_from_main_are_no_test_builds() {
        assert!(!is_test_version("0.6.0"), "release");
        assert!(!is_test_version("0.6.3"), "pre-release from main");
        assert!(is_test_version("0.6.3-pr.214.2"), "pull request");
        assert!(is_test_version("0.6.3-dev.ea5d135"), "other branch");
        assert!(is_test_version("0.0.1-dev"), "local build");
    }
}
