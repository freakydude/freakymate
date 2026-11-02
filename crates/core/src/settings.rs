//! User settings as a JSON file in the user's configuration directory.

use crate::backlight::BacklightSettings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub(crate) const APP_FOLDER: &str = "freakymate";
const FILE_NAME: &str = "settings.json";
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
    #[error("invalid settings file {path}: {source}")]
    Format { path: PathBuf, source: serde_json::Error },
    #[error("no configuration directory available on this system")]
    NoConfigDirectory,
}

/// Loads and saves settings at a fixed location.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Default location: `<config dir>/freakymate/settings.json`.
    pub fn default_location() -> Result<Self, SettingsError> {
        let directory = dirs::config_dir().ok_or(SettingsError::NoConfigDirectory)?;
        Ok(Self::new(directory.join(APP_FOLDER).join(FILE_NAME)))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// If the file is missing, the defaults apply.
    pub fn load(&self) -> Result<Settings, SettingsError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|source| SettingsError::Format { path: self.path.clone(), source })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(source) => Err(SettingsError::Io { path: self.path.clone(), source }),
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

    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let text = serde_json::to_string_pretty(settings)
            .map_err(|source| SettingsError::Format { path: self.path.clone(), source })?;
        write_atomically(&self.path, &text, false)
            .map_err(|source| SettingsError::Io { path: self.path.clone(), source })
    }
}

/// Writes via a temporary file so a power loss never leaves a half-written file. A `private` file can be read by
/// its owner only (Unix).
pub(crate) fn write_atomically(path: &Path, text: &str, private: bool) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let temporary = path.with_extension("json.tmp");
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
        assert!(text.contains("\"temperature_presets\""));
        assert_eq!(store.load().unwrap(), Settings::default());
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
        std::fs::write(store.path(), r#"{ "fullscreen": true, "steps": { "move_mm": 1 } }"#).unwrap();

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
