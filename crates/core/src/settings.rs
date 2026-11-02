//! User settings as a TOML file with the ending `.conf`, like `moonraker.conf` and the other non-Klipper files in `printer_data/config`. FreakyMate
//! saves it by changing only the values that changed, so comments written by hand stay.

use crate::backlight::BacklightSettings;
use crate::toml_merge;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use toml_edit::DocumentMut;

pub(crate) const APP_FOLDER: &str = "freakymate";
const FILE_NAME: &str = "settings.conf";
const EXTENSION: &str = "conf";
/// Up to 0.3.8 the settings were JSON; such a file stays where it is and is no longer read.
const OLD_EXTENSION: &str = "json";
/// The top of a new settings file.
const HEADER: &str = "# FreakyMate settings (TOML). Comments stay when FreakyMate saves a change.\n\
# Documentation: https://github.com/freakydude/freakymate/blob/main/docs/configuration.md#settings-file\n\n";
/// Presets the Thermal page has room for beside the heaters.
pub const MAX_SHOWN_PRESETS: usize = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Address of the Moonraker server, e.g. `http://127.0.0.1:7125`.
    pub moonraker: String,
    pub fullscreen: bool,
    pub steps: Steps,
    /// Edited in the settings file only; the Thermal page shows the first [`MAX_SHOWN_PRESETS`].
    pub temperature_presets: Vec<TemperaturePreset>,
    /// Macros the printer had when the touch screen last saw it; a macro missing here counts as new.
    pub known_macros: Vec<String>,
    /// Known macros the user hides from the console.
    pub hidden_macros: Vec<String>,
    /// Leaves out the temperature reports that Klipper sends while waiting for a heater (like Mainsail).
    pub hide_temperature_reports: bool,
    /// Logs the program's own debug messages as well (for a bug report); `RUST_LOG` overrides it.
    pub verbose_log: bool,
    /// Brightness and standby of this display; only used where a backlight can be controlled.
    pub backlight: BacklightSettings,
}

impl Settings {
    /// Brings the macro lists in line with the macros the printer currently has.
    /// Macros that vanished are forgotten (also as hidden), new ones are known and visible.
    /// Returns whether anything changed.
    pub fn reconcile_macros(&mut self, current: &[String]) -> bool {
        let before = (self.known_macros.clone(), self.hidden_macros.clone());
        self.known_macros.retain(|name| current.contains(name));
        self.hidden_macros.retain(|name| current.contains(name));
        for name in current {
            if !self.known_macros.contains(name) {
                self.known_macros.push(name.clone());
            }
        }
        before != (self.known_macros.clone(), self.hidden_macros.clone())
    }
}

/// Last chosen step sizes; the UI offers fixed steps and remembers the selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Steps {
    pub move_mm: f64,
    pub z_offset_mm: f64,
    pub temperature: f64,
    pub extrude_mm: f64,
    pub extrude_speed: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemperaturePreset {
    pub name: String,
    pub extruder: f64,
    pub bed: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            moonraker: "http://127.0.0.1:7125".to_owned(),
            fullscreen: false,
            steps: Steps::default(),
            temperature_presets: vec![
                TemperaturePreset::new("PLA", 210.0, 60.0),
                TemperaturePreset::new("PETG", 240.0, 80.0),
                TemperaturePreset::new("ABS", 250.0, 105.0),
                TemperaturePreset::new("TPU", 225.0, 50.0),
            ],
            known_macros: Vec::new(),
            hidden_macros: Vec::new(),
            hide_temperature_reports: true,
            verbose_log: false,
            backlight: BacklightSettings::default(),
        }
    }
}

impl Default for Steps {
    fn default() -> Self {
        Self { move_mm: 10.0, z_offset_mm: 0.025, temperature: 5.0, extrude_mm: 10.0, extrude_speed: 5.0 }
    }
}

impl TemperaturePreset {
    pub fn new(name: &str, extruder: f64, bed: f64) -> Self {
        Self { name: name.to_owned(), extruder, bed }
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("cannot access {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("invalid settings file {path}: {message}")]
    Format { path: PathBuf, message: String },
    #[error("no configuration directory available on this system")]
    NoConfigDirectory,
}

/// Loads and saves settings at a fixed location.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    /// A path ending in `.json` (a service set up for the old JSON settings) means the `.conf` next to it; the JSON
    /// file stays untouched.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if path.extension().is_some_and(|extension| extension == OLD_EXTENSION) {
            let replacement = path.with_extension(EXTENSION);
            log::info!("settings: {} is no longer read, using {}", path.display(), replacement.display());
            return Self { path: replacement };
        }
        Self { path }
    }

    /// Default location: `<config dir>/freakymate/settings.conf`.
    pub fn default_location() -> Result<Self, SettingsError> {
        let directory = dirs::config_dir().ok_or(SettingsError::NoConfigDirectory)?;
        Ok(Self::new(directory.join(APP_FOLDER).join(FILE_NAME)))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// If the file is missing, the defaults apply.
    pub fn load(&self) -> Result<Settings, SettingsError> {
        match self.read()? {
            Some(text) => toml::from_str(&text).map_err(|error| self.format_error(error)),
            None => Ok(Settings::default()),
        }
    }

    /// Like [`Self::load`], but writes the defaults when the file is missing, so every setting (for example the
    /// temperature presets) is there to edit by hand.
    pub fn load_or_create(&self) -> Result<Settings, SettingsError> {
        if self.path.exists() {
            return self.load();
        }
        let settings = Settings::default();
        self.save(&settings)?;
        Ok(settings)
    }

    /// Changes only what differs from the file, so comments and the order written by hand stay. A file that cannot be
    /// read as TOML is left alone (the error says why), rather than replaced by the settings in memory.
    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let fresh: DocumentMut = toml::to_string(settings)
            .map_err(|error| self.format_error(error))?
            .parse()
            .map_err(|error| self.format_error(error))?;
        let text = match self.read()? {
            Some(existing) => {
                let mut document: DocumentMut = existing.parse().map_err(|error| self.format_error(error))?;
                toml_merge::merge_table(document.as_table_mut(), fresh.as_table());
                document.to_string()
            }
            None => format!("{HEADER}{fresh}"),
        };
        write_atomically(&self.path, &text, false)
            .map_err(|source| SettingsError::Io { path: self.path.clone(), source })
    }

    /// The text of the file; `None` if there is none yet.
    fn read(&self) -> Result<Option<String>, SettingsError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SettingsError::Io { path: self.path.clone(), source }),
        }
    }

    fn format_error(&self, error: impl std::fmt::Display) -> SettingsError {
        SettingsError::Format { path: self.path.clone(), message: error.to_string() }
    }
}

/// Writes via a temporary file so a power loss never leaves a half-written file. A `private` file can be read by
/// its owner only (Unix).
pub(crate) fn write_atomically(path: &Path, text: &str, private: bool) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    // The mode only applies to a new file; a leftover of an interrupted write keeps its own.
    let _ = std::fs::remove_file(&temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    std::io::Write::write_all(&mut options.open(&temporary)?, text.as_bytes())?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn new_macros_are_known_and_visible() {
        let mut settings = Settings::default();

        assert!(settings.reconcile_macros(&names(&["PARK", "PRINT_END"])));
        assert_eq!(settings.known_macros, names(&["PARK", "PRINT_END"]));
        assert!(settings.hidden_macros.is_empty());
        assert!(!settings.reconcile_macros(&names(&["PARK", "PRINT_END"])));
    }

    #[test]
    fn vanished_macros_are_forgotten_including_hidden() {
        let mut settings = Settings {
            known_macros: names(&["PARK", "OLD"]),
            hidden_macros: names(&["PARK", "OLD"]),
            ..Settings::default()
        };

        assert!(settings.reconcile_macros(&names(&["PARK"])));
        assert_eq!(settings.known_macros, names(&["PARK"]));
        assert_eq!(settings.hidden_macros, names(&["PARK"]));
    }

    #[test]
    fn returning_macro_counts_as_new_and_visible_again() {
        let mut settings =
            Settings { known_macros: names(&["PARK"]), hidden_macros: names(&["PARK"]), ..Settings::default() };

        settings.reconcile_macros(&[]);
        settings.reconcile_macros(&names(&["PARK"]));

        assert!(settings.hidden_macros.is_empty());
        assert_eq!(settings.known_macros, names(&["PARK"]));
    }

    /// A store in a folder that does not exist yet; the folder is removed when the returned guard is dropped.
    fn temporary_store() -> (tempfile::TempDir, SettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("freakymate").join(FILE_NAME));
        (dir, store)
    }

    #[test]
    fn missing_file_gives_defaults() {
        let (_dir, store) = temporary_store();

        assert_eq!(store.load().unwrap(), Settings::default());
    }

    #[test]
    fn a_missing_file_is_created_with_the_defaults() {
        let (_dir, store) = temporary_store();

        assert_eq!(store.load_or_create().unwrap(), Settings::default());

        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(text.starts_with("# FreakyMate settings"));
        assert!(text.contains("[[temperature_presets]]"));
        assert_eq!(store.load().unwrap(), Settings::default());
    }

    #[test]
    fn saving_keeps_comments_written_by_hand() {
        let (_dir, store) = temporary_store();
        store.load_or_create().unwrap();
        let edited = std::fs::read_to_string(store.path()).unwrap().replace(
            "[[temperature_presets]]\nname = \"PLA\"",
            "[[temperature_presets]]\n# satin PLA wants it hotter\nname = \"PLA\"",
        );
        std::fs::write(store.path(), &edited).unwrap();
        let mut settings = store.load().unwrap();
        settings.steps.move_mm = 1.0;

        store.save(&settings).unwrap();

        let saved = std::fs::read_to_string(store.path()).unwrap();
        assert!(saved.contains("# satin PLA wants it hotter\nname = \"PLA\""));
        assert!(saved.starts_with("# FreakyMate settings"));
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn a_broken_file_is_not_overwritten() {
        let (_dir, store) = temporary_store();
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        std::fs::write(store.path(), "fullscreen = tru").unwrap();

        assert!(matches!(store.save(&Settings::default()), Err(SettingsError::Format { .. })));
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), "fullscreen = tru");
    }

    #[test]
    fn an_old_json_path_means_the_conf_next_to_it_and_the_json_stays() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("freakymate.json");
        std::fs::write(&json, r#"{ "fullscreen": true }"#).unwrap();

        let store = SettingsStore::new(&json);
        store.load_or_create().unwrap();

        assert_eq!(store.path(), dir.path().join("freakymate.conf"));
        assert!(!store.load().unwrap().fullscreen);
        assert_eq!(std::fs::read_to_string(&json).unwrap(), r#"{ "fullscreen": true }"#);
    }

    #[test]
    fn standby_times_and_brightness_survive_a_restart() {
        let (_dir, store) = temporary_store();
        let mut settings = Settings::default();
        settings.backlight.brightness = Some(40);
        settings.backlight.off_when_idle_minutes = Some(5);
        settings.backlight.off_when_printing_minutes = Some(30);

        store.save(&settings).unwrap();
        let loaded = store.load().unwrap().backlight;

        assert_eq!(loaded, settings.backlight);
        let timeouts = loaded.timeouts();
        assert_eq!(timeouts.idle, Some(std::time::Duration::from_secs(300)));
        assert_eq!(timeouts.printing, Some(std::time::Duration::from_secs(1800)));
    }

    #[test]
    fn saved_settings_load_again() {
        let (_dir, store) = temporary_store();
        let settings = Settings {
            moonraker: "http://voron.local".to_owned(),
            steps: Steps { move_mm: 50.0, ..Steps::default() },
            ..Settings::default()
        };

        store.save(&settings).unwrap();

        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn partial_file_keeps_defaults_for_missing_fields() {
        let (_dir, store) = temporary_store();
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        std::fs::write(store.path(), "fullscreen = true\n\n[steps]\nmove_mm = 1.0\n").unwrap();

        let settings = store.load().unwrap();

        assert!(settings.fullscreen);
        assert_eq!(settings.steps.move_mm, 1.0);
        assert_eq!(settings.steps.temperature, Steps::default().temperature);
        assert_eq!(settings.temperature_presets.len(), 4);
    }

    #[test]
    fn broken_file_is_reported() {
        let (_dir, store) = temporary_store();
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        std::fs::write(store.path(), "{ nope").unwrap();

        assert!(matches!(store.load(), Err(SettingsError::Format { .. })));
    }
}
