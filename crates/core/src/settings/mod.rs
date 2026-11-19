//! User settings: what the screen offers and what belongs to each saved printer. They live in a file in the dialect of
//! Klipper's `printer.cfg` ([`file_format`]), so they read and edit like the other files in `printer_data/config`.

mod file_format;
mod store;

pub use store::{SettingsError, SettingsStore};

use crate::backlight::BacklightSettings;
use crate::moonraker::Endpoint;
use crate::step_choices::StepChoices;
use serde::Serialize;

/// The local Moonraker, used when no printer is chosen yet and the command line names none.
pub const DEFAULT_MOONRAKER: &str = "http://127.0.0.1:7125";
/// Presets the Thermal page has room for beside the heaters.
pub const MAX_SHOWN_PRESETS: usize = 4;
/// Not allowed in a printer name: brackets end the `[printer name]` header, `#` and `;` start a comment.
const NAME_FORBIDDEN: [char; 4] = ['[', ']', '#', ';'];
/// Name of a printer whose name and host give nothing usable.
const FALLBACK_PRINTER_NAME: &str = "printer";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settings {
    /// Address of the printer chosen last, e.g. `http://127.0.0.1:7125`; the next start uses it. `None` until one
    /// is chosen.
    pub moonraker: Option<String>,
    /// The printers the user chose once, each with its own Moonraker; switched on the Printers page.
    pub printers: Vec<SavedPrinter>,
    pub fullscreen: bool,
    /// Android: the screen stays on while Sailhand is in front, like a touch screen at the printer.
    pub keep_screen_on: bool,
    pub steps: Steps,
    /// The step sizes the UI offers; edited in the settings file only. Missing keys mean the defaults.
    #[serde(skip_serializing_if = "StepChoices::is_empty")]
    pub step_choices: StepChoices,
    /// Edited in the settings file only; the Thermal page shows the first [`MAX_SHOWN_PRESETS`].
    pub temperature_presets: Vec<TemperaturePreset>,
    /// Leaves out the temperature reports that Klipper sends while waiting for a heater (like Mainsail).
    pub hide_temperature_reports: bool,
    /// Logs the program's own debug messages as well (for a bug report); `RUST_LOG` overrides it.
    pub verbose_log: bool,
    /// Rotation of picture and touch in degrees (0, 90, 180 or 270, clockwise); only the framebuffer fallback uses it.
    pub rotation: u16,
    /// Brightness and standby of this display; only used where a backlight can be controlled.
    pub backlight: BacklightSettings,
}

/// A printer the user chose once, kept with what belongs to it alone.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct SavedPrinter {
    /// Unique among the saved printers; the name of its `[printer name]` section in the file.
    pub name: String,
    /// Moonraker's address as [`Endpoint::display`] writes it.
    pub address: String,
    /// The `uuid` Moonraker announces over zeroconf; finds the printer again when its address changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    /// Macros the printer had when the touch screen last saw it; a macro missing here counts as new.
    pub known_macros: Vec<String>,
    /// Known macros the user hides from the console.
    pub hidden_macros: Vec<String>,
}

impl Settings {
    /// The printer to connect to at start: the one chosen last, else the one from the command line, else the local
    /// Moonraker.
    pub fn start_address(&self, start_argument: Option<&str>) -> String {
        self.moonraker.as_deref().or(start_argument).unwrap_or(DEFAULT_MOONRAKER).to_owned()
    }

    /// Keeps the printer used at start as a saved printer when none is saved yet, so the macros hidden from its
    /// console stay hidden after a restart.
    pub fn keep_first_printer(&mut self, endpoint: &Endpoint) {
        if !self.printers.is_empty() {
            return;
        }
        let name = self.unique_name(endpoint.host(), endpoint);
        self.printers.push(SavedPrinter { name, address: endpoint.display(), ..SavedPrinter::default() });
    }

    /// The saved printer at this address.
    pub fn printer(&self, address: &str) -> Option<&SavedPrinter> {
        self.printers.iter().find(|printer| printer.address == address)
    }

    pub fn printer_mut(&mut self, address: &str) -> Option<&mut SavedPrinter> {
        self.printers.iter_mut().find(|printer| printer.address == address)
    }

    /// Makes this printer the one in use and keeps it. A saved printer with the same `uuid` takes the new address
    /// (it got another one from DHCP) and keeps its name and macros. A new printer gets a name no other one has.
    pub fn choose_printer(&mut self, endpoint: &Endpoint, name: &str, uuid: Option<&str>) {
        let address = endpoint.display();
        let same_uuid = |printer: &SavedPrinter| uuid.is_some() && printer.uuid.as_deref() == uuid;
        let position = self.printers.iter().position(|printer| printer.address == address || same_uuid(printer));
        match position {
            Some(index) => {
                let printer = &mut self.printers[index];
                printer.address.clone_from(&address);
                if uuid.is_some() {
                    printer.uuid = uuid.map(str::to_owned);
                }
            }
            None => {
                let name = self.unique_name(name, endpoint);
                self.printers.push(SavedPrinter {
                    name,
                    address: address.clone(),
                    uuid: uuid.map(str::to_owned),
                    ..SavedPrinter::default()
                });
            }
        }
        self.moonraker = Some(address);
    }

    /// Removes the saved printer; returns whether there was one. `moonraker` stays, so the display keeps using it
    /// until another printer is chosen.
    pub fn forget_printer(&mut self, address: &str) -> bool {
        let before = self.printers.len();
        self.printers.retain(|printer| printer.address != address);
        self.printers.len() != before
    }

    /// The hidden macros of this printer; none for a printer that is not saved (an address from the command line).
    pub fn hidden_macros(&self, address: &str) -> Vec<String> {
        self.printer(address).map(|printer| printer.hidden_macros.clone()).unwrap_or_default()
    }

    /// The name without what a section header cannot hold, else the host; with ` 2`, ` 3` … when it is taken.
    fn unique_name(&self, wanted: &str, endpoint: &Endpoint) -> String {
        let base = [wanted, endpoint.host()]
            .into_iter()
            .map(clean_name)
            .find(|name| !name.is_empty())
            .unwrap_or_else(|| FALLBACK_PRINTER_NAME.to_owned());
        let taken = |name: &str| self.printers.iter().any(|printer| printer.name.eq_ignore_ascii_case(name));
        (1..)
            .map(|number| if number == 1 { base.clone() } else { format!("{base} {number}") })
            .find(|name| !taken(name))
            .unwrap_or(base)
    }
}

/// Without the characters a `[printer name]` header cannot hold, and with single spaces.
fn clean_name(name: &str) -> String {
    let kept: String = name.chars().filter(|c| !NAME_FORBIDDEN.contains(c) && !c.is_control()).collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl SavedPrinter {
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
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Steps {
    pub move_mm: f64,
    pub z_offset_mm: f64,
    pub temperature: f64,
    pub extrude_mm: f64,
    pub extrude_speed: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TemperaturePreset {
    pub name: String,
    pub extruder: f64,
    pub bed: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            moonraker: None,
            printers: Vec::new(),
            fullscreen: false,
            keep_screen_on: true,
            steps: Steps::default(),
            step_choices: StepChoices::default(),
            temperature_presets: vec![
                TemperaturePreset::new("PLA", 210.0, 60.0),
                TemperaturePreset::new("PETG", 240.0, 80.0),
                TemperaturePreset::new("ABS", 250.0, 105.0),
                TemperaturePreset::new("TPU", 225.0, 50.0),
            ],
            hide_temperature_reports: true,
            verbose_log: false,
            rotation: 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    fn endpoint(address: &str) -> Endpoint {
        Endpoint::parse(address).unwrap()
    }

    /// The defaults after the first start against the local Moonraker.
    fn first_start() -> Settings {
        let mut settings = Settings::default();
        settings.keep_first_printer(&endpoint(DEFAULT_MOONRAKER));
        settings
    }

    #[test]
    fn new_macros_are_known_and_visible() {
        let mut printer = SavedPrinter::default();

        assert!(printer.reconcile_macros(&names(&["PARK", "PRINT_END"])));
        assert_eq!(printer.known_macros, names(&["PARK", "PRINT_END"]));
        assert!(printer.hidden_macros.is_empty());
        assert!(!printer.reconcile_macros(&names(&["PARK", "PRINT_END"])));
    }

    #[test]
    fn vanished_macros_are_forgotten_including_hidden() {
        let mut printer = SavedPrinter {
            known_macros: names(&["PARK", "OLD"]),
            hidden_macros: names(&["PARK", "OLD"]),
            ..SavedPrinter::default()
        };

        assert!(printer.reconcile_macros(&names(&["PARK"])));
        assert_eq!(printer.known_macros, names(&["PARK"]));
        assert_eq!(printer.hidden_macros, names(&["PARK"]));
    }

    #[test]
    fn returning_macro_counts_as_new_and_visible_again() {
        let mut printer =
            SavedPrinter { known_macros: names(&["PARK"]), hidden_macros: names(&["PARK"]), ..SavedPrinter::default() };

        printer.reconcile_macros(&[]);
        printer.reconcile_macros(&names(&["PARK"]));

        assert!(printer.hidden_macros.is_empty());
        assert_eq!(printer.known_macros, names(&["PARK"]));
    }

    #[test]
    fn the_start_uses_the_printer_chosen_last_then_the_command_line_then_the_local_one() {
        let mut settings = Settings::default();
        assert_eq!(settings.start_address(None), DEFAULT_MOONRAKER);
        assert_eq!(settings.start_address(Some("http://voron:7125")), "http://voron:7125");

        settings.choose_printer(&endpoint("ratrig.local:7125"), "ratrig", None);

        assert_eq!(settings.start_address(Some("http://voron:7125")), "http://ratrig.local:7125");
    }

    #[test]
    fn the_first_printer_is_kept_only_when_none_is_saved() {
        let mut settings = first_start();
        settings.keep_first_printer(&endpoint("voron.local:7125"));

        assert_eq!(settings.printers.len(), 1);
        assert_eq!(settings.printers[0].name, "127.0.0.1");
        assert_eq!(settings.printers[0].address, DEFAULT_MOONRAKER);
        assert_eq!(settings.moonraker, None);
    }

    #[test]
    fn choosing_a_printer_keeps_it_and_uses_it_from_now_on() {
        let mut settings = first_start();

        settings.choose_printer(&endpoint("192.168.40.7:7125"), "sidewinder", Some("abc"));
        settings.choose_printer(&endpoint("http://192.168.40.7:7125"), "other name", None);

        assert_eq!(settings.moonraker.as_deref(), Some("http://192.168.40.7:7125"));
        assert_eq!(settings.printers.len(), 2);
        let saved = settings.printer("http://192.168.40.7:7125").unwrap();
        assert_eq!((saved.name.as_str(), saved.uuid.as_deref()), ("sidewinder", Some("abc")));
    }

    #[test]
    fn a_new_printer_gets_a_name_that_fits_a_section_header_and_no_other_printer_has() {
        let mut settings = first_start();

        settings.choose_printer(&endpoint("192.168.40.7:7125"), "Voron  #2 [left]", None);
        settings.choose_printer(&endpoint("192.168.40.8:7125"), "voron 2 left", None);
        settings.choose_printer(&endpoint("192.168.40.9:7125"), "[]", None);

        let names: Vec<&str> = settings.printers.iter().map(|printer| printer.name.as_str()).collect();
        assert_eq!(names, ["127.0.0.1", "Voron 2 left", "voron 2 left 2", "192.168.40.9"]);
    }

    #[test]
    fn a_printer_with_a_new_address_is_found_by_its_uuid() {
        let mut settings = first_start();
        settings.choose_printer(&endpoint("192.168.40.6:7125"), "voron", Some("29f7"));
        settings.printer_mut("http://192.168.40.6:7125").unwrap().hidden_macros = names(&["PARK"]);

        settings.choose_printer(&endpoint("192.168.40.9:7125"), "voron", Some("29f7"));

        assert_eq!(settings.printers.len(), 2);
        assert_eq!(settings.hidden_macros("http://192.168.40.9:7125"), names(&["PARK"]));
        assert!(settings.printer("http://192.168.40.6:7125").is_none());
    }

    #[test]
    fn forgetting_the_printer_in_use_keeps_using_it() {
        let mut settings = first_start();
        settings.choose_printer(&endpoint("voron.local:7125"), "voron", None);

        assert!(settings.forget_printer("http://voron.local:7125"));
        assert!(!settings.forget_printer("http://voron.local:7125"));

        assert_eq!(settings.moonraker.as_deref(), Some("http://voron.local:7125"));
        assert!(settings.hidden_macros("http://voron.local:7125").is_empty());
    }
}
