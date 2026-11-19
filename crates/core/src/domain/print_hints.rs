//! What the file asks of the machine itself, besides the filament: the nozzle diameter of the slicer against the
//! nozzle of the extruder that prints it, and the first layer temperatures against the limits (`max_temp`) of the
//! heaters. Only hints: Klipper refuses a target above `max_temp` anyway, we point it out before the start. A fact one
//! side does not have is not compared.

use super::filament_match::ToolMatch;
use super::machine_tree::Lane;
use crate::sources::PrinterStatus;
use crate::sources::klipper::{BED_HEATER, NOZZLE_HEATER};
use crate::sources::moonraker_server::files::FileMetadata;

/// Diameters closer than this (mm) are the same nozzle; slicers and Klipper write 0.4, 0.40 and 0.400.
const DIAMETER_TOLERANCE_MM: f64 = 0.005;

/// One thing of the file that does not fit the machine.
#[derive(Debug, Clone, PartialEq)]
pub enum PrintHint {
    /// The slicer planned for another nozzle than the one of `extruder`.
    NozzleDiameter { extruder: String, file: f64, printer: f64 },
    /// The first layer temperature of the nozzle is above `max_temp` of `extruder`.
    NozzleTemperature { extruder: String, file: f64, max: f64 },
    /// The first layer temperature of the bed is above `max_temp` of the bed heater.
    BedTemperature { file: f64, max: f64 },
}

/// The extruders the tools of the file print with, each once in tool order: the nozzle of the gate (Happy Hare) or of the
/// lane mapped to the tool, else Klipper's `extruder`.
pub fn tool_extruders(matches: &[ToolMatch], lanes: &[Lane]) -> Vec<String> {
    let mut extruders: Vec<String> = Vec::new();
    for found in matches {
        let lane = lanes.iter().find(|lane| match found.gate {
            Some(gate) => lane.gate == Some(gate),
            None => lane.tool == Some(found.request.tool),
        });
        let extruder = lane.map_or(NOZZLE_HEATER, |lane| lane.extruder.as_str());
        if !extruders.iter().any(|known| known == extruder) {
            extruders.push(extruder.to_owned());
        }
    }
    extruders
}

/// The hints of `metadata` for the printer; `extruders` are those of [`tool_extruders`]. Empty when everything fits or
/// nothing can be compared.
pub fn print_hints(metadata: &FileMetadata, status: &PrinterStatus, extruders: &[String]) -> Vec<PrintHint> {
    let mut hints = Vec::new();
    let file_diameter = metadata.nozzle_diameter.filter(|diameter| *diameter > 0.0);
    let file_nozzle_temp = metadata.first_layer_extr_temp.filter(|degrees| *degrees > 0.0);
    for extruder in extruders {
        let Some(heater) = status.heater(extruder) else { continue };
        if let (Some(file), Some(printer)) = (file_diameter, heater.nozzle_diameter)
            && (file - printer).abs() > DIAMETER_TOLERANCE_MM
        {
            hints.push(PrintHint::NozzleDiameter { extruder: extruder.clone(), file, printer });
        }
        if let (Some(file), Some(max)) = (file_nozzle_temp, heater.max_temp)
            && file > max
        {
            hints.push(PrintHint::NozzleTemperature { extruder: extruder.clone(), file, max });
        }
    }
    let bed_max = status.heater(BED_HEATER).and_then(|heater| heater.max_temp);
    if let (Some(file), Some(max)) = (metadata.first_layer_bed_temp.filter(|degrees| *degrees > 0.0), bed_max)
        && file > max
    {
        hints.push(PrintHint::BedTemperature { file, max });
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::klipper::Heater;

    fn nozzle(object: &str, diameter: Option<f64>, max_temp: Option<f64>) -> Heater {
        Heater { object: object.into(), nozzle_diameter: diameter, max_temp, ..Heater::default() }
    }

    fn printer(heaters: Vec<Heater>) -> PrinterStatus {
        PrinterStatus { heaters, ..PrinterStatus::default() }
    }

    fn file(nozzle: Option<f64>, extruder_temp: Option<f64>, bed_temp: Option<f64>) -> FileMetadata {
        FileMetadata {
            nozzle_diameter: nozzle,
            first_layer_extr_temp: extruder_temp,
            first_layer_bed_temp: bed_temp,
            ..FileMetadata::default()
        }
    }

    fn standard_printer() -> PrinterStatus {
        printer(vec![
            nozzle("extruder", Some(0.4), Some(300.0)),
            Heater { object: "heater_bed".into(), max_temp: Some(120.0), ..Heater::default() },
        ])
    }

    fn main_extruder() -> Vec<String> {
        vec!["extruder".to_owned()]
    }

    #[test]
    fn a_file_that_fits_has_no_hints() {
        let hints = print_hints(&file(Some(0.4), Some(215.0), Some(60.0)), &standard_printer(), &main_extruder());
        assert_eq!(hints, []);
    }

    #[test]
    fn another_nozzle_diameter_is_a_hint() {
        let hints = print_hints(&file(Some(0.6), None, None), &standard_printer(), &main_extruder());
        assert_eq!(hints, [PrintHint::NozzleDiameter { extruder: "extruder".into(), file: 0.6, printer: 0.4 }]);
    }

    #[test]
    fn a_rounding_difference_of_the_diameter_is_the_same_nozzle() {
        let hints = print_hints(&file(Some(0.4000001), None, None), &standard_printer(), &main_extruder());
        assert_eq!(hints, []);
    }

    #[test]
    fn temperatures_above_the_limits_are_hints_and_the_limit_itself_is_not() {
        let hot = print_hints(&file(None, Some(310.0), Some(130.0)), &standard_printer(), &main_extruder());
        assert_eq!(
            hot,
            [
                PrintHint::NozzleTemperature { extruder: "extruder".into(), file: 310.0, max: 300.0 },
                PrintHint::BedTemperature { file: 130.0, max: 120.0 },
            ]
        );
        assert_eq!(print_hints(&file(None, Some(300.0), Some(120.0)), &standard_printer(), &main_extruder()), []);
    }

    #[test]
    fn what_a_side_does_not_have_is_not_compared() {
        let unknown =
            printer(vec![nozzle("extruder", None, None), Heater { object: "heater_bed".into(), ..Heater::default() }]);
        assert_eq!(print_hints(&file(Some(0.6), Some(999.0), Some(999.0)), &unknown, &main_extruder()), []);
        assert_eq!(print_hints(&file(None, Some(0.0), Some(0.0)), &standard_printer(), &main_extruder()), []);
        assert_eq!(print_hints(&file(Some(0.6), None, None), &PrinterStatus::default(), &main_extruder()), []);
    }

    #[test]
    fn each_extruder_of_the_file_is_compared_with_its_own_nozzle() {
        let status = printer(vec![nozzle("extruder", Some(0.4), None), nozzle("extruder1", Some(0.6), None)]);
        let both = vec!["extruder".to_owned(), "extruder1".to_owned()];
        let hints = print_hints(&file(Some(0.4), None, None), &status, &both);
        assert_eq!(hints, [PrintHint::NozzleDiameter { extruder: "extruder1".into(), file: 0.4, printer: 0.6 }]);
    }
}
