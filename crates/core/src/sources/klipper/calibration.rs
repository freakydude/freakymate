//! Calibrating the bed and the Z height: the bed mesh, the bed screws (measured with the probe or by paper test), the
//! Z height of the probe or the Z endstop, the interactive modes Klipper waits in (`manual_probe`, `bed_screws`) and
//! what `SAVE_CONFIG` would write. Commands and fields follow Klipper's G-Codes.md and Status_Reference.md.

use super::bed_mesh::{self, BedMesh};
use crate::sources::fields::{count, number, text};
use crate::sources::objects::ObjectStore;
use serde_json::Value;

/// What this printer offers for calibration, from `printer.objects.list` and its configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalibrationSetup {
    /// `[bed_mesh]`: `BED_MESH_CALIBRATE`, `BED_MESH_PROFILE`, `BED_MESH_CLEAR`.
    pub bed_mesh: bool,
    /// `[screws_tilt_adjust]`: `SCREWS_TILT_CALCULATE` measures each screw with the probe.
    pub screws_tilt: bool,
    /// `[bed_screws]`: `BED_SCREWS_ADJUST`, the paper test at each screw.
    pub bed_screws: bool,
    /// `[stepper_z]` with `position_endstop`: only then Klipper registers `Z_ENDSTOP_CALIBRATE` (`manual_probe.py`).
    pub z_endstop: bool,
}

impl CalibrationSetup {
    /// Notes the objects of `printer.objects.list` that bring calibration commands.
    pub(super) fn note_object(&mut self, name: &str) {
        match name {
            "bed_mesh" => self.bed_mesh = true,
            "screws_tilt_adjust" => self.screws_tilt = true,
            "bed_screws" => self.bed_screws = true,
            _ => {}
        }
    }

    pub(super) fn apply_settings(&mut self, settings: Option<&Value>) {
        self.z_endstop = settings.and_then(|settings| settings.pointer("/stepper_z/position_endstop")).is_some();
    }
}

/// The objects calibration reads; the subscription leaves out those the printer does not have.
pub(super) const OBJECTS: [&str; 4] = ["manual_probe", "bed_mesh", "screws_tilt_adjust", "bed_screws"];

/// An interactive mode Klipper waits in until it gets `ACCEPT` or `ABORT`. Started here or elsewhere (Mainsail, a
/// macro); only `manual_probe` and `bed_screws` report it.
#[derive(Debug, Clone, PartialEq)]
pub enum CalibrationMode {
    /// `manual_probe`: `PROBE_CALIBRATE`, `Z_ENDSTOP_CALIBRATE`, `MANUAL_PROBE` and every `METHOD=manual`. `lower` and
    /// `upper` are the nearest heights tried below and above.
    ManualProbe { z: f64, lower: Option<f64>, upper: Option<f64> },
    /// `bed_screws`: the screw to turn by paper test; `fine` after a first round, if fine adjust points are configured.
    BedScrews { screw: String, fine: bool, accepted: u32, screws: usize },
}

/// One screw from the last `SCREWS_TILT_CALCULATE`, in the order of the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrewAdjustment {
    pub name: String,
    pub z: f64,
    /// The screw the others are measured against; it is not turned.
    pub is_base: bool,
    pub clockwise: bool,
    /// Turns as Klipper reports them, `"HH:MM"` like a clock face: `01:15` is one and a quarter turns.
    pub turns: String,
}

/// A screw that needs less than this many clock minutes (`00:10`, a sixth of a turn) counts as level.
const LEVEL_BELOW_MINUTES: u32 = 10;

impl ScrewAdjustment {
    /// The turn in clock minutes (`01:15` is 75); `None` if the text is not `HH:MM`.
    fn turn_minutes(&self) -> Option<u32> {
        let (hours, minutes) = self.turns.split_once(':')?;
        Some(hours.parse::<u32>().ok()? * 60 + minutes.parse::<u32>().ok()?)
    }

    /// Whether the screw needs so little that it counts as done: under `00:10`. The base screw is never turned.
    pub fn is_level(&self) -> bool {
        !self.is_base && self.turn_minutes().is_some_and(|minutes| minutes < LEVEL_BELOW_MINUTES)
    }
}

/// The bed mesh in use and the saved profiles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BedMeshState {
    /// Empty if no mesh is active.
    pub active: String,
    pub profiles: Vec<String>,
    /// The points of the active mesh.
    pub mesh: Option<BedMesh>,
}

/// A section `SAVE_CONFIG` would write: options with their new values, or `None` if it removes the section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSection {
    pub section: String,
    pub options: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CalibrationState {
    pub mode: Option<CalibrationMode>,
    pub bed_mesh: Option<BedMeshState>,
    pub screws: Vec<ScrewAdjustment>,
    pub pending: Vec<PendingSection>,
}

impl CalibrationState {
    pub(super) fn read(store: &ObjectStore) -> Self {
        let settings = store.object("configfile").and_then(|configfile| configfile.get("settings"));
        let section = |name: &str| settings.and_then(|settings| settings.get(name));
        Self {
            mode: read_manual_probe(store.object("manual_probe"))
                .or_else(|| read_bed_screws(store.object("bed_screws"), section("bed_screws"))),
            bed_mesh: store.object("bed_mesh").map(|object| read_bed_mesh(object, section("bed_mesh"))),
            screws: read_screws(store.object("screws_tilt_adjust"), section("screws_tilt_adjust")),
            pending: read_pending(store.object("configfile")),
        }
    }
}

fn read_manual_probe(object: Option<&Value>) -> Option<CalibrationMode> {
    let object = object.filter(|object| is_active(object))?;
    let height = |field: &str| object.get(field).and_then(Value::as_f64);
    Some(CalibrationMode::ManualProbe {
        z: number(object, "z_position"),
        lower: height("z_position_lower"),
        upper: height("z_position_upper"),
    })
}

fn read_bed_screws(object: Option<&Value>, settings: Option<&Value>) -> Option<CalibrationMode> {
    let object = object.filter(|object| is_active(object))?;
    let fine = text(object, "state") == "fine";
    let current = count::<usize>(object, "current_screw").unwrap_or_default();
    let screws = configured_screws(settings);
    // Like `bed_screws.py`: the fine round only visits the screws with a fine adjust point.
    let round: Vec<&Screw> = screws.iter().filter(|screw| !fine || screw.fine_adjust).collect();
    Some(CalibrationMode::BedScrews {
        screw: round.get(current).map_or_else(|| default_name(current), |screw| screw.name.clone()),
        fine,
        accepted: count(object, "accepted_screws").unwrap_or_default(),
        screws: screws.len(),
    })
}

fn is_active(object: &Value) -> bool {
    object.get("is_active").and_then(Value::as_bool).unwrap_or_default()
}

fn read_bed_mesh(object: &Value, settings: Option<&Value>) -> BedMeshState {
    let profiles = object.get("profiles").and_then(Value::as_object).map(|profiles| profiles.keys().cloned().collect());
    BedMeshState {
        active: text(object, "profile_name").to_owned(),
        profiles: profiles.unwrap_or_default(),
        mesh: bed_mesh::read(object, settings),
    }
}

fn read_screws(object: Option<&Value>, settings: Option<&Value>) -> Vec<ScrewAdjustment> {
    let Some(results) = object.and_then(|object| object.get("results")).and_then(Value::as_object) else {
        return Vec::new();
    };
    // `results` is keyed `screw1`, `screw2`, … in the order of the configuration.
    configured_screws(settings)
        .into_iter()
        .enumerate()
        .filter_map(|(index, screw)| {
            let result = results.get(&format!("screw{}", index + 1))?;
            Some(ScrewAdjustment {
                name: screw.name,
                z: number(result, "z"),
                is_base: result.get("is_base").and_then(Value::as_bool).unwrap_or_default(),
                clockwise: text(result, "sign") == "CW",
                turns: text(result, "adjust").to_owned(),
            })
        })
        .collect()
}

fn read_pending(configfile: Option<&Value>) -> Vec<PendingSection> {
    let Some(items) = configfile.and_then(|configfile| configfile.get("save_config_pending_items")) else {
        return Vec::new();
    };
    let Some(sections) = items.as_object() else { return Vec::new() };
    sections
        .iter()
        .map(|(section, options)| PendingSection {
            section: section.clone(),
            options: options.as_object().map(|options| {
                options
                    .iter()
                    .map(|(option, value)| (option.clone(), value.as_str().unwrap_or_default().to_owned()))
                    .collect()
            }),
        })
        .collect()
}

struct Screw {
    name: String,
    fine_adjust: bool,
}

/// `screw1`, `screw2`, … of `[bed_screws]` or `[screws_tilt_adjust]`, until the first gap, as Klipper reads them.
fn configured_screws(settings: Option<&Value>) -> Vec<Screw> {
    let Some(settings) = settings else { return Vec::new() };
    (0..)
        .map_while(|index| {
            let prefix = format!("screw{}", index + 1);
            settings.get(&prefix)?;
            let name = settings.get(format!("{prefix}_name")).and_then(Value::as_str);
            Some(Screw {
                name: name.map_or_else(|| default_name(index), str::to_owned),
                fine_adjust: settings.get(format!("{prefix}_fine_adjust")).is_some(),
            })
        })
        .collect()
}

/// Klipper names an unnamed screw by its position; without the configuration only the number is known.
fn default_name(index: usize) -> String {
    format!("screw {}", index + 1)
}

/// `TESTZ` within `manual_probe`: a relative move, or halfway towards the nearest height tried above or below.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TestZ {
    By(f64),
    BisectUp,
    BisectDown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(status: Value) -> CalibrationState {
        let mut store = ObjectStore::default();
        store.replace(&status);
        CalibrationState::read(&store)
    }

    const SCREWS: &str = r#"{ "screw1": [27.0, 272.5], "screw1_name": "back left",
        "screw2": [27.0, 72.5], "screw2_name": "front left", "screw2_fine_adjust": [30.0, 70.0],
        "screw3": [227.0, 72.5] }"#;

    #[test]
    fn detects_commands_from_objects_and_settings() {
        let mut setup = CalibrationSetup::default();
        for name in ["bed_mesh", "bed_screws", "probe"] {
            setup.note_object(name);
        }
        setup.apply_settings(Some(&json!({ "stepper_z": { "position_endstop": 0.5 } })));
        assert!(setup.bed_mesh && setup.bed_screws && setup.z_endstop && !setup.screws_tilt);
        setup.apply_settings(Some(&json!({ "stepper_z": { "endstop_pin": "probe:z_virtual_endstop" } })));
        assert!(!setup.z_endstop);
    }

    #[test]
    fn reads_manual_probe_mode() {
        let calibration = state(json!({
            "manual_probe": { "is_active": true, "z_position": 0.35, "z_position_lower": 0.3, "z_position_upper": null }
        }));
        assert_eq!(calibration.mode, Some(CalibrationMode::ManualProbe { z: 0.35, lower: Some(0.3), upper: None }));
        let idle = state(json!({ "manual_probe": { "is_active": false, "z_position": null } }));
        assert_eq!(idle.mode, None);
    }

    #[test]
    fn names_the_bed_screw_of_each_round() {
        let settings: Value = serde_json::from_str(SCREWS).unwrap();
        let mode = |state_name: &str, current: u32| {
            state(json!({
                "bed_screws": { "is_active": true, "state": state_name, "current_screw": current, "accepted_screws": 1 },
                "configfile": { "settings": { "bed_screws": settings } }
            }))
            .mode
        };
        let screw = |name: &str, fine: bool| {
            Some(CalibrationMode::BedScrews { screw: name.to_owned(), fine, accepted: 1, screws: 3 })
        };
        assert_eq!(mode("adjust", 2), screw("screw 3", false));
        assert_eq!(mode("fine", 0), screw("front left", true));
    }

    #[test]
    fn a_screw_under_ten_clock_minutes_is_level_but_the_base_screw_is_not() {
        let screw = |is_base: bool, turns: &str| ScrewAdjustment {
            name: "front left".into(),
            z: 2.5,
            is_base,
            clockwise: true,
            turns: turns.into(),
        };
        assert!(screw(false, "00:00").is_level());
        assert!(screw(false, "00:09").is_level());
        assert!(!screw(false, "00:10").is_level());
        assert!(!screw(false, "01:00").is_level());
        assert!(!screw(false, "").is_level());
        assert!(!screw(true, "00:00").is_level());
    }

    #[test]
    fn reads_screw_results_in_configured_order() {
        let settings: Value = serde_json::from_str(SCREWS).unwrap();
        let calibration = state(json!({
            "screws_tilt_adjust": { "error": false, "max_deviation": null, "results": {
                "screw2": { "z": 2.4, "sign": "CCW", "adjust": "00:18", "is_base": false },
                "screw1": { "z": 2.5, "sign": "CW", "adjust": "00:00", "is_base": true }
            } },
            "configfile": { "settings": { "screws_tilt_adjust": settings } }
        }));
        let names: Vec<_> = calibration.screws.iter().map(|screw| screw.name.as_str()).collect();
        assert_eq!(names, ["back left", "front left"]);
        assert!(calibration.screws[0].is_base);
        assert!(!calibration.screws[1].clockwise);
        assert_eq!(calibration.screws[1].turns, "00:18");
    }

    #[test]
    fn reads_bed_mesh_and_pending_items() {
        let calibration = state(json!({
            "bed_mesh": { "profile_name": "default", "profiles": { "default": {}, "hot": {} } },
            "configfile": { "save_config_pending_items": {
                "probe": { "z_offset": "0.550" },
                "bed_mesh old": null
            } }
        }));
        let mesh = calibration.bed_mesh.unwrap();
        assert_eq!(mesh.active, "default");
        assert_eq!(mesh.profiles, ["default", "hot"]);
        assert!(calibration.pending.contains(&PendingSection {
            section: "probe".to_owned(),
            options: Some(vec![("z_offset".to_owned(), "0.550".to_owned())]),
        }));
        assert!(calibration.pending.contains(&PendingSection { section: "bed_mesh old".to_owned(), options: None }));
    }
}
