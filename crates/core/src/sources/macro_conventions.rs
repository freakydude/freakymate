//! Macro names that many printer configurations share without a plugin behind them. Like every macro they are black
//! boxes: called by name, nothing assumed about what they do inside.

use super::klipper::Setup;

/// Loads filament into the extruder (as KlipperScreen calls it).
pub const LOAD_FILAMENT_MACRO: &str = "LOAD_FILAMENT";
/// Unloads filament from the extruder.
pub const UNLOAD_FILAMENT_MACRO: &str = "UNLOAD_FILAMENT";

/// Which of the macros `LOAD_FILAMENT` and `UNLOAD_FILAMENT` exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilamentMacros {
    pub load: bool,
    pub unload: bool,
}

impl FilamentMacros {
    /// `None` if neither macro exists.
    pub fn detect(klipper: &Setup) -> Option<Self> {
        let load = klipper.has_macro(LOAD_FILAMENT_MACRO);
        let unload = klipper.has_macro(UNLOAD_FILAMENT_MACRO);
        (load || unload).then_some(Self { load, unload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_macro_is_detected_on_its_own() {
        let detect = |macros: &[&str]| FilamentMacros::detect(&Setup::with_macros(macros));

        assert_eq!(detect(&["PRINT_START"]), None);
        assert_eq!(detect(&["load_filament"]), Some(FilamentMacros { load: true, unload: false }));
        assert_eq!(detect(&["LOAD_FILAMENT", "UNLOAD_FILAMENT"]), Some(FilamentMacros { load: true, unload: true }));
    }
}
