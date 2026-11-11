//! Filament on this printer: what Load and Unload run, and which filament sensors belong to the printer rather
//! than to Happy Hare.

use super::load_path;
use crate::sources::happy_hare::is_shown_by_mmu;
use crate::sources::klipper::FilamentSensor;
use crate::sources::macro_conventions::FilamentMacros;
use crate::sources::{PrinterStatus, Sources};

/// Where Load and Unload come from, see [`handling`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilamentHandling {
    Mmu,
    Macros(FilamentMacros),
    None,
}

/// Happy Hare while it has the say, otherwise the macros `LOAD_FILAMENT` and `UNLOAD_FILAMENT` (as KlipperScreen
/// does), otherwise nothing. There is no generic fallback: the sequence depends on the length of the bowden tube and
/// the temperatures of the machine.
pub fn handling(sources: &Sources, status: &PrinterStatus) -> FilamentHandling {
    if status.happy_hare().is_some() {
        return FilamentHandling::Mmu;
    }
    sources.filament_macros.map_or(FilamentHandling::None, FilamentHandling::Macros)
}

/// The filament sensors of the printer itself. The sensors Happy Hare (while in charge) and AFC create belong to their
/// load paths (`load_path`); other sensors stay, and so do Happy Hare's toolhead sensors that no head can claim.
pub fn printer_sensors(status: &PrinterStatus) -> impl Iterator<Item = &FilamentSensor> {
    let mmu = status.happy_hare();
    let afc = status.afc.as_ref();
    status.filament_sensors.iter().filter(move |sensor| {
        let by_mmu = mmu.is_some_and(|mmu| {
            is_shown_by_mmu(&sensor.object) && !load_path::is_untied_toolhead_sensor(mmu, &sensor.object)
        });
        let by_afc = afc.is_some_and(|afc| sensor_name(&sensor.object).is_some_and(|name| afc.owns_sensor(name)));
        !(by_mmu || by_afc)
    })
}

/// The name of a filament switch without its kind.
fn sensor_name(object: &str) -> Option<&str> {
    object.split_once(' ').map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::happy_hare::MmuStatus;

    fn happy_hare(enabled: bool, version: &str) -> MmuStatus {
        MmuStatus { enabled, version: version.into(), ..MmuStatus::default() }
    }

    fn status_with(mmu: Option<MmuStatus>) -> PrinterStatus {
        let sensor = |object: &str| FilamentSensor { object: object.into(), ..FilamentSensor::default() };
        PrinterStatus {
            mmu,
            filament_sensors: vec![
                sensor("filament_switch_sensor mmu_gate_0"),
                sensor("filament_switch_sensor toolhead"),
                sensor("filament_switch_sensor spool_runout"),
            ],
            ..PrinterStatus::default()
        }
    }

    #[test]
    fn load_and_unload_prefer_happy_hare_then_macros() {
        let macros =
            Sources { filament_macros: Some(FilamentMacros { load: true, unload: false }), ..Sources::default() };

        assert_eq!(handling(&macros, &status_with(Some(happy_hare(true, "4.1.0")))), FilamentHandling::Mmu);
        assert_eq!(
            handling(&macros, &status_with(None)),
            FilamentHandling::Macros(FilamentMacros { load: true, unload: false })
        );
        assert_eq!(handling(&Sources::default(), &status_with(None)), FilamentHandling::None);
    }

    #[test]
    fn a_disabled_or_too_old_happy_hare_leaves_loading_to_the_macros() {
        let macros =
            Sources { filament_macros: Some(FilamentMacros { load: true, unload: true }), ..Sources::default() };

        for mmu in [happy_hare(false, "4.1.0"), happy_hare(true, "4.0.2")] {
            assert!(matches!(handling(&macros, &status_with(Some(mmu))), FilamentHandling::Macros(_)));
        }
    }

    #[test]
    fn happy_hare_keeps_its_own_sensors() {
        let names = |status: &PrinterStatus| -> Vec<String> {
            printer_sensors(status).map(|sensor| sensor.object.clone()).collect()
        };

        assert_eq!(names(&status_with(Some(happy_hare(true, "4.1.0")))), ["filament_switch_sensor spool_runout"]);
        assert_eq!(names(&status_with(Some(happy_hare(false, "4.1.0")))).len(), 3);
        assert_eq!(names(&status_with(None)).len(), 3);
    }

    /// AFC's switches belong to its lanes, hub, buffer and toolhead; a sensor of the printer stays.
    #[test]
    fn afc_keeps_its_own_sensors() {
        use crate::sources::afc::{AfcBuffer, AfcExtruder, AfcHub, AfcLane, AfcStatus};
        let afc = AfcStatus {
            lanes: vec![AfcLane { name: "lane1".into(), ..AfcLane::default() }],
            hubs: vec![AfcHub { name: "Turtle_1".into(), switch: Some(false) }],
            buffers: vec![AfcBuffer { name: "TN".into(), ..AfcBuffer::default() }],
            extruders: vec![AfcExtruder { name: "extruder".into(), ..AfcExtruder::default() }],
            ..AfcStatus::default()
        };
        let sensor = |name: &str| FilamentSensor {
            object: format!("filament_switch_sensor {name}"),
            ..FilamentSensor::default()
        };
        let names = ["lane1_prep", "lane1_load", "Turtle_1_Hub", "TN_expanded", "extruder_tool_end", "lane1_runout"];
        let status = PrinterStatus {
            afc: Some(afc),
            filament_sensors: names.into_iter().map(sensor).collect(),
            ..PrinterStatus::default()
        };

        let left: Vec<&str> = printer_sensors(&status).map(|sensor| sensor.object.as_str()).collect();

        assert_eq!(left, ["filament_switch_sensor lane1_runout"]);
    }
}
