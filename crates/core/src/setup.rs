//! The setup outside the program folder that `install.sh` and the Debian package write as root: service unit and its
//! arguments, udev and polkit rules, groups. Updates through Moonraker only replace the program folder, so an
//! installation can lag behind the program it runs; running `install.sh` again catches up.

use std::path::Path;

/// Raise by one whenever `install.sh` or the Debian package set up something new outside the program folder, and note
/// what below. 1: `--logfile` in the service, the settings file `freakymate.conf` (the program also finds it when an
/// older service still passes `freakymate.json`), the backlight udev rule, the groups `video` and `i2c`.
pub const SETUP_VERSION: u32 = 1;

/// Written by `install.sh` and the Debian package: the setup version they set up.
pub const MARKER_FILE: &str = "/etc/freakymate/setup-version";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupState {
    /// Not started as an installed service (development, a desktop, Windows): nothing to say.
    NotInstalled,
    Current,
    /// Set up for an older program; `installed` is `None` for an installation from before the marker file.
    Outdated {
        installed: Option<u32>,
    },
}

impl SetupState {
    /// From the content of [`MARKER_FILE`] (if it exists) and the settings file the program was started with. Without
    /// the marker, the installer's own convention tells an old installation: settings in `printer_data/config`.
    pub fn detect(marker: Option<&str>, settings_file: Option<&Path>) -> Self {
        match marker.map(|text| text.trim().parse::<u32>()) {
            Some(Ok(installed)) if installed >= SETUP_VERSION => Self::Current,
            Some(Ok(installed)) => Self::Outdated { installed: Some(installed) },
            Some(Err(_)) => Self::Outdated { installed: None },
            None if settings_file.is_some_and(is_installer_settings_file) => Self::Outdated { installed: None },
            None => Self::NotInstalled,
        }
    }
}

/// `<printer_data>/config/freakymate.conf` (before: `.json`), where `install.sh` and the Debian package put the settings.
fn is_installer_settings_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "freakymate.conf" || name == "freakymate.json")
        && path.parent().and_then(Path::file_name).is_some_and(|folder| folder == "config")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLED: &str = "/home/pi/printer_data/config/freakymate.conf";

    #[test]
    fn the_marker_tells_the_installed_version() {
        let current = SETUP_VERSION.to_string();
        let older = (SETUP_VERSION - 1).to_string();

        assert_eq!(SetupState::detect(Some(&format!("{current}\n")), None), SetupState::Current);
        assert_eq!(
            SetupState::detect(Some(&older), Some(Path::new(INSTALLED))),
            SetupState::Outdated { installed: Some(SETUP_VERSION - 1) }
        );
        assert_eq!(SetupState::detect(Some("garbage"), None), SetupState::Outdated { installed: None });
    }

    #[test]
    fn without_marker_only_the_installer_layout_counts_as_outdated() {
        assert_eq!(SetupState::detect(None, Some(Path::new(INSTALLED))), SetupState::Outdated { installed: None });
        assert_eq!(
            SetupState::detect(None, Some(Path::new("/home/me/.config/freakymate/settings.conf"))),
            SetupState::NotInstalled
        );
        assert_eq!(SetupState::detect(None, None), SetupState::NotInstalled);
    }
}
