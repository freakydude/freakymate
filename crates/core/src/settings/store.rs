//! Loads and saves the settings file. Saving changes only the values that changed, so comments and the order written
//! by hand stay.

use super::{Settings, file_format};
use crate::atomic_file::write_atomically;
use crate::klipper_ini::ConfigFile;
use std::path::{Path, PathBuf};
use thiserror::Error;

const FILE_NAME: &str = "settings.conf";
/// Appended to the name of a settings file in the old TOML format when it is set aside.
const SET_ASIDE_SUFFIX: &str = "old";
/// The top of a new settings file.
const HEADER: &str = "# Sailhand settings, written like Klipper's printer.cfg. Edit them here or in Mainsail or Fluidd,
# then restart Sailhand; comments stay when Sailhand saves a change made on the screen.
# Documentation: https://github.com/freakydude/sailhand/blob/main/docs/configuration.md#settings-file
";

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
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Default location: `<config dir>/sailhand/settings.conf`.
    pub fn default_location() -> Result<Self, SettingsError> {
        let directory = dirs::config_dir().ok_or(SettingsError::NoConfigDirectory)?;
        Ok(Self::new(directory.join(crate::config_folder()).join(FILE_NAME)))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// If the file is missing, the defaults apply.
    pub fn load(&self) -> Result<Settings, SettingsError> {
        match self.read()? {
            Some(text) => self.settings_from(&text),
            None => Ok(Settings::default()),
        }
    }

    /// Like [`Self::load`], but writes the defaults when the file is missing, so every setting (for example the
    /// temperature presets) is there to edit by hand. A file in the old TOML format is renamed to `<name>.old` and
    /// replaced by the defaults.
    pub fn load_or_create(&self) -> Result<Settings, SettingsError> {
        match self.read()? {
            Some(text) if is_old_toml(&text) => {
                self.set_aside()?;
                self.create()
            }
            Some(text) => self.settings_from(&text),
            None => self.create(),
        }
    }

    /// Changes only what differs from the file. A file that cannot be read is left alone (the error says why),
    /// rather than replaced by the settings in memory.
    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let (mut file, new) = match self.read()? {
            Some(text) => {
                let file = self.parse(&text)?;
                // A value that cannot be read would be replaced by the default in memory: keep what was written.
                file_format::read(&file).map_err(|message| self.format_error(message))?;
                (file, false)
            }
            None => (ConfigFile::new(HEADER), true),
        };
        file_format::write(settings, &mut file, new);
        write_atomically(&self.path, &file.render(), false)
            .map_err(|source| SettingsError::Io { path: self.path.clone(), source })
    }

    fn create(&self) -> Result<Settings, SettingsError> {
        let settings = Settings::default();
        self.save(&settings)?;
        Ok(settings)
    }

    fn settings_from(&self, text: &str) -> Result<Settings, SettingsError> {
        if is_old_toml(text) {
            return Err(self.format_error(
                "it is in the old TOML format; Sailhand replaces it with the defaults at its next start",
            ));
        }
        file_format::read(&self.parse(text)?).map_err(|message| self.format_error(message))
    }

    fn parse(&self, text: &str) -> Result<ConfigFile, SettingsError> {
        ConfigFile::parse(text).map_err(|error| self.format_error(error))
    }

    fn set_aside(&self) -> Result<(), SettingsError> {
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".{SET_ASIDE_SUFFIX}"));
        let aside = self.path.with_file_name(name);
        std::fs::rename(&self.path, &aside).map_err(|source| SettingsError::Io { path: aside.clone(), source })?;
        log::warn!(
            "settings: {} was in the old TOML format; renamed it to {} and started with the defaults",
            self.path.display(),
            aside.display()
        );
        Ok(())
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

/// A file Sailhand wrote up to now in TOML: not readable as INI, and it has a `[[list]]` or a value before the first
/// section (Sailhand wrote `moonraker = "…"` at the top).
fn is_old_toml(text: &str) -> bool {
    if ConfigFile::parse(text).is_ok() {
        return false;
    }
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with(['#', ';']));
    let starts_with_value = lines.next().is_some_and(|first| !first.starts_with('['));
    starts_with_value || text.lines().any(|line| line.trim_start().starts_with("[["))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moonraker::Endpoint;
    use crate::settings::{DEFAULT_MOONRAKER, SavedPrinter, Steps};

    /// The example of `docs/configuration.md`: three printers, the first in use.
    const THREE_PRINTERS: &str = "# Sailhand settings
[sailhand]
fullscreen: True
keep_screen_on: False
hide_temperature_reports: True
verbose_log: False

[printer voron]
moonraker: http://192.168.40.6:7125
hidden_macros:
    LOAD_FILAMENT
    UNLOAD_FILAMENT

[printer ratrig]
moonraker: http://ratrig.local:7125

[printer werkstatt]
moonraker: https://192.168.40.20:7130

[preset PLA]
extruder: 210
bed: 60

[preset PETG]
extruder: 240
bed: 80

[step_choices]
move_mm: 0.1, 1, 10, 50, 100

[backlight]
brightness: 80
off_when_idle_minutes: 5

#*# <---------------------- SAVE_CONFIG ---------------------->
#*# DO NOT EDIT THIS BLOCK OR BELOW. The contents are auto-generated.
#*#
#*# [sailhand]
#*# printer = voron
#*#
#*# [selected_steps]
#*# move_mm = 1
#*# z_offset_mm = 0.025
#*# temperature = 5
#*# extrude_mm = 10
#*# extrude_speed = 5
#*#
#*# [printer voron]
#*# uuid = 29f7565a940247049493e8ff2a0d0c87
#*# known_macros = PARK, LOAD_FILAMENT, UNLOAD_FILAMENT
#*#
#*# [printer ratrig]
#*# known_macros = PARK
";

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    /// A store in a folder that does not exist yet; the folder is removed when the returned guard is dropped.
    fn temporary_store() -> (tempfile::TempDir, SettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("sailhand").join(FILE_NAME));
        (dir, store)
    }

    fn store_with(text: &str) -> (tempfile::TempDir, SettingsStore) {
        let (dir, store) = temporary_store();
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        std::fs::write(store.path(), text).unwrap();
        (dir, store)
    }

    fn file_text(store: &SettingsStore) -> String {
        std::fs::read_to_string(store.path()).unwrap()
    }

    #[test]
    fn three_printers_are_read_with_what_sailhand_kept_about_them() {
        let (_dir, store) = store_with(THREE_PRINTERS);

        let settings = store.load().unwrap();

        assert!(settings.fullscreen && !settings.keep_screen_on);
        assert_eq!(settings.moonraker.as_deref(), Some("http://192.168.40.6:7125"));
        let voron = SavedPrinter {
            name: "voron".to_owned(),
            address: "http://192.168.40.6:7125".to_owned(),
            uuid: Some("29f7565a940247049493e8ff2a0d0c87".to_owned()),
            known_macros: names(&["PARK", "LOAD_FILAMENT", "UNLOAD_FILAMENT"]),
            hidden_macros: names(&["LOAD_FILAMENT", "UNLOAD_FILAMENT"]),
        };
        assert_eq!(settings.printers[0], voron);
        assert_eq!(settings.printers[1].known_macros, names(&["PARK"]));
        assert_eq!(settings.printers[2].address, "https://192.168.40.20:7130");
        assert_eq!(settings.temperature_presets.len(), 2);
        assert_eq!(settings.temperature_presets[1].bed, 80.0);
        assert_eq!(settings.step_choices.move_mm, Some(vec![0.1, 1.0, 10.0, 50.0, 100.0]));
        assert_eq!(settings.backlight.off_when_idle_minutes, Some(5));
        assert_eq!(settings.steps, Steps { move_mm: 1.0, ..Steps::default() });
    }

    #[test]
    fn saving_unchanged_settings_leaves_the_file_as_it_was() {
        let (_dir, store) = store_with(THREE_PRINTERS);

        store.save(&store.load().unwrap()).unwrap();

        assert_eq!(file_text(&store), THREE_PRINTERS);
    }

    #[test]
    fn changes_on_the_screen_touch_only_their_lines() {
        let (_dir, store) = store_with(THREE_PRINTERS);
        let mut settings = store.load().unwrap();
        settings.backlight.brightness = Some(40);
        settings.forget_printer("http://ratrig.local:7125");
        settings.printer_mut("http://192.168.40.6:7125").unwrap().hidden_macros.clear();

        store.save(&settings).unwrap();

        let expected = THREE_PRINTERS
            .replace("hidden_macros:\n    LOAD_FILAMENT\n    UNLOAD_FILAMENT\n", "")
            .replace("[printer ratrig]\nmoonraker: http://ratrig.local:7125\n\n", "")
            .replace("brightness: 80", "brightness: 40")
            .replace("#*#\n#*# [printer ratrig]\n#*# known_macros = PARK\n", "");
        assert_eq!(file_text(&store), expected);
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn a_printer_that_is_not_saved_is_kept_in_use_by_its_address() {
        let (_dir, store) = temporary_store();
        let mut settings = Settings::default();
        settings.choose_printer(&Endpoint::parse("voron.local:7125").unwrap(), "voron", None);
        settings.forget_printer("http://voron.local:7125");

        store.save(&settings).unwrap();

        assert!(file_text(&store).contains("#*# [sailhand]\n#*# moonraker = http://voron.local:7125\n"));
        assert_eq!(store.load().unwrap().moonraker.as_deref(), Some("http://voron.local:7125"));
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let (_dir, store) = temporary_store();

        assert_eq!(store.load().unwrap(), Settings::default());
    }

    #[test]
    fn a_missing_file_is_created_with_the_defaults() {
        let (_dir, store) = temporary_store();

        assert_eq!(store.load_or_create().unwrap(), Settings::default());

        let text = file_text(&store);
        assert!(text.starts_with("# Sailhand settings"));
        assert!(text.contains("\n[sailhand]\nfullscreen: False\n"), "{text}");
        assert!(text.contains("\n[preset PLA]\nextruder: 210\nbed: 60\n"), "{text}");
        assert!(!text.contains("[backlight]") && !text.contains("[step_choices]"), "{text}");
        assert_eq!(store.load().unwrap(), Settings::default());
    }

    #[test]
    fn the_first_printer_is_written_once_it_is_kept() {
        let (_dir, store) = temporary_store();
        let mut settings = store.load_or_create().unwrap();
        settings.keep_first_printer(&Endpoint::parse(DEFAULT_MOONRAKER).unwrap());

        store.save(&settings).unwrap();

        assert!(file_text(&store).contains("\n[printer 127.0.0.1]\nmoonraker: http://127.0.0.1:7125\n"));
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn an_old_toml_file_is_set_aside_and_replaced_by_the_defaults() {
        let old = "moonraker = \"http://192.168.40.6:7125\"\n\n[[temperature_presets]]\nname = \"PLA\"\n";
        let (_dir, store) = store_with(old);

        assert!(matches!(store.load(), Err(SettingsError::Format { .. })));
        assert_eq!(store.load_or_create().unwrap(), Settings::default());

        let aside = store.path().with_file_name("settings.conf.old");
        assert_eq!(std::fs::read_to_string(aside).unwrap(), old);
        assert!(file_text(&store).starts_with("# Sailhand settings"));
    }

    #[test]
    fn a_broken_file_is_reported_and_not_overwritten() {
        let (_dir, store) = store_with("[sailhand]\nfullscreen: tru\n");

        let error = store.load_or_create().unwrap_err().to_string();

        assert!(error.contains("[sailhand] fullscreen (line 2)"), "{error}");
        assert!(matches!(store.save(&Settings::default()), Err(SettingsError::Format { .. })));
        assert_eq!(file_text(&store), "[sailhand]\nfullscreen: tru\n");
        let (_dir, store) = store_with("[sailhand\n");
        assert!(matches!(store.save(&Settings::default()), Err(SettingsError::Format { .. })));
        assert_eq!(file_text(&store), "[sailhand\n");
    }

    #[test]
    fn a_partial_file_keeps_the_defaults_for_the_rest_except_presets() {
        let (_dir, store) = store_with("[sailhand]\nfullscreen = yes\n\n[step_choices]\nmove_mm: 1\n");

        let settings = store.load().unwrap();

        assert!(settings.fullscreen);
        assert_eq!(settings.step_choices.move_mm, Some(vec![1.0]));
        assert_eq!(settings.steps, Steps::default());
        assert!(settings.temperature_presets.is_empty());
    }

    #[test]
    fn rotation_is_one_of_four_angles_and_defaults_to_none() {
        let (_dir, store) = store_with("[sailhand]\nrotation: 270\n");
        assert_eq!(store.load().unwrap().rotation, 270);
        let (_dir, store) = store_with("[sailhand]\nfullscreen: True\n");
        assert_eq!(store.load().unwrap().rotation, 0);
        let (_dir, store) = store_with("[sailhand]\nrotation: 45\n");
        assert!(matches!(store.load(), Err(SettingsError::Format { .. })));
    }

    #[test]
    fn a_printer_or_preset_without_what_it_needs_is_an_error() {
        for text in ["[printer voron]\n", "[printer]\nmoonraker: x\n", "[preset PLA]\nbed: 60\n"] {
            let (_dir, store) = store_with(text);
            assert!(matches!(store.load(), Err(SettingsError::Format { .. })), "{text}");
        }
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
}
