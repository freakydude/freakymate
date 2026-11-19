//! Klipper's calibration helpers, simplified: `manual_probe` (`PROBE_CALIBRATE`, `MANUAL_PROBE`, `TESTZ`), the paper
//! test of `bed_screws`, `SCREWS_TILT_CALCULATE` with fixed results, the bed mesh with its profiles and what
//! `SAVE_CONFIG` would write (`configfile.save_config_pending_items`). Behaviour follows
//! `klippy/extras/manual_probe.py`, `bed_screws.py`, `bed_mesh.py` and `configfile.py` (read 2026-10-28 and
//! 2026-10-29); measured values are made up.

use crate::bed_mesh::{MeshGrid, MeshPoints};
use crate::eddy_probe::{EddyProbe, TapStep};
use crate::gcode::CommandError;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Largest step of a bisecting `TESTZ`, as in `manual_probe.py`.
const BISECT_MAX: f64 = 0.2;
/// Height the nozzle waits at when a manual probe starts.
const START_HEIGHT: f64 = 2.0;
/// The configured `z_offset` of the simulated probe.
pub const PROBE_Z_OFFSET: f64 = 1.0;
/// The bed screws of a bed slinger (names and positions of a Sidewinder X2).
pub const SCREWS: [(&str, [f64; 2]); 4] = [
    ("back left", [27.0, 272.5]),
    ("front left", [27.0, 72.5]),
    ("front right", [227.0, 72.5]),
    ("back right", [227.0, 272.5]),
];
/// `SCREWS_TILT_CALCULATE` results per screw: sign and turns; the first screw is the base.
const SCREW_TILT: [(&str, &str); 4] = [("CW", "00:00"), ("CW", "00:18"), ("CCW", "01:05"), ("CW", "00:02")];
const DEFAULT_MESH: &str = "default";
/// Profiles saved before the start, each with its own tilt of the made-up surface.
const SAVED_MESHES: [(&str, f64); 2] = [(DEFAULT_MESH, 0.0), ("pei-textured", 0.03)];

#[derive(Debug, Clone, PartialEq)]
pub enum CalibrationCommand {
    ProbeCalibrate,
    EddyDriveCurrent { chip: String },
    EddyCalibrate { chip: String },
    EddyTap(TapStep),
    ManualProbe,
    TestZ(TestZ),
    Accept,
    Adjusted,
    Abort,
    BedScrewsAdjust,
    ScrewsTiltCalculate,
    BedMeshCalibrate { profile: Option<String>, probe_count: Option<(usize, usize)>, round_probe_count: Option<usize> },
    BedMeshLoad { profile: String },
    BedMeshClear,
    BedMeshSave { profile: String },
    BedMeshRemove { profile: String },
}

/// `TESTZ Z=`: a relative move, `+`/`-` halfway to the nearest height tried, `++`/`--` all the way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TestZ {
    By(f64),
    Up { half: bool },
    Down { half: bool },
}

impl TestZ {
    pub fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "+" => Ok(Self::Up { half: true }),
            "++" => Ok(Self::Up { half: false }),
            "-" => Ok(Self::Down { half: true }),
            "--" => Ok(Self::Down { half: false }),
            number => number
                .parse()
                .map(Self::By)
                .map_err(|_| CommandError::new(format!("Unable to parse '{number}' as a float"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbePurpose {
    /// `PROBE_CALIBRATE`: on `ACCEPT` the probe's `z_offset` becomes pending.
    ProbeOffset,
    /// `PROBE_EDDY_CURRENT_CALIBRATE`: on `ACCEPT` the measurement of the Z heights follows and the `calibrate` table
    /// becomes pending (pe l.118-132; Klipper takes minutes, here it is instant).
    EddyHeights,
    /// `MANUAL_PROBE`: the height is only reported.
    Height,
}

#[derive(Debug, Clone, PartialEq)]
struct ManualProbe {
    purpose: ProbePurpose,
    z: f64,
    tried: Vec<f64>,
}

impl ManualProbe {
    fn new(purpose: ProbePurpose) -> Self {
        Self { purpose, z: START_HEIGHT, tried: Vec::new() }
    }

    /// Like `cmd_TESTZ`: remember the current height, then move.
    fn test(&mut self, step: TestZ) {
        let z = self.z;
        if !self.tried.contains(&z) {
            self.tried.push(z);
            self.tried.sort_by(f64::total_cmp);
        }
        self.z = match step {
            TestZ::By(distance) => z + distance,
            TestZ::Up { half } => {
                let above = self.above(z).unwrap_or(f64::MAX);
                (if half { (above + z) / 2.0 } else { above }).min(z + BISECT_MAX)
            }
            TestZ::Down { half } => {
                let below = self.below(z).unwrap_or(f64::MIN);
                (if half { (below + z) / 2.0 } else { below }).max(z - BISECT_MAX)
            }
        };
    }

    fn above(&self, z: f64) -> Option<f64> {
        self.tried.iter().copied().find(|tried| *tried > z)
    }

    fn below(&self, z: f64) -> Option<f64> {
        self.tried.iter().copied().rev().find(|tried| *tried < z)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BedScrews {
    current: usize,
    accepted: i64,
}

/// What the simulated printer has and where its helpers stand.
/// A measured mesh: its grid and the heights.
#[derive(Debug, Clone, PartialEq)]
struct Mesh {
    grid: MeshGrid,
    points: MeshPoints,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Calibration {
    probe: bool,
    /// The probe is an Eddy current probe: Klipper registers no `PROBE_CALIBRATE` for it (pe l.1055,
    /// `can_set_z_offset=False`; probe.py l.56-60).
    eddy: Option<EddyProbe>,
    bed_mesh: bool,
    screws: bool,
    manual_probe: Option<ManualProbe>,
    bed_screws: Option<BedScrews>,
    screw_results: bool,
    /// Name and points; like Klipper, the active mesh stays when its profile is removed.
    active_mesh: Option<(String, Mesh)>,
    meshes: BTreeMap<String, Mesh>,
    /// The mesh of `[bed_mesh]` (and what `BED_MESH_CALIBRATE` starts from).
    mesh_config: Option<MeshGrid>,
    /// `None` for a section `SAVE_CONFIG` removes.
    pending: BTreeMap<String, Option<Map<String, Value>>>,
}

impl Calibration {
    /// `screws` adds both `[bed_screws]` and `[screws_tilt_adjust]`; a bed mesh starts with saved profiles.
    pub fn new(probe: bool, eddy: bool, bed_mesh: bool, screws: bool) -> Self {
        let mut calibration = Self { probe, eddy: eddy.then(EddyProbe::default), bed_mesh, screws, ..Self::default() };
        calibration.configure_mesh(false);
        calibration
    }

    /// A round bed: `[bed_mesh]` has `mesh_radius` instead of `mesh_min`/`mesh_max`, and the saved profiles follow.
    pub fn with_round_mesh(mut self) -> Self {
        self.configure_mesh(true);
        self
    }

    fn configure_mesh(&mut self, round: bool) {
        if !self.bed_mesh {
            return;
        }
        let grid = MeshGrid::configured(round);
        self.mesh_config = Some(grid);
        self.meshes = SAVED_MESHES
            .iter()
            .map(|(name, tilt)| ((*name).to_owned(), Mesh { grid, points: grid.heights(*tilt) }))
            .collect();
    }

    /// `homed`: whether all axes are homed; Klipper refuses to probe or move to a screw otherwise.
    pub fn apply(&mut self, command: CalibrationCommand, homed: bool) -> Result<(), CommandError> {
        match command {
            CalibrationCommand::ProbeCalibrate if self.probe && self.eddy.is_none() => {
                self.start_probe(ProbePurpose::ProbeOffset, homed)?
            }
            CalibrationCommand::EddyDriveCurrent { chip } if self.eddy.is_some() => {
                EddyProbe::check_chip(&chip)?;
                self.set_eddy_pending(EddyProbe::measure_drive_current());
            }
            CalibrationCommand::EddyCalibrate { chip } if self.eddy.is_some() => {
                EddyProbe::check_chip(&chip)?;
                self.start_probe(ProbePurpose::EddyHeights, homed)?
            }
            CalibrationCommand::EddyTap(step) if self.eddy.is_some() => {
                let measured = self.eddy.as_mut().map(|eddy| eddy.tap(step)).transpose()?.flatten();
                if let Some(option) = measured {
                    self.set_eddy_pending(option);
                }
            }
            CalibrationCommand::ManualProbe => self.start_probe(ProbePurpose::Height, homed)?,
            CalibrationCommand::TestZ(step) => self.probing()?.test(step),
            CalibrationCommand::Accept if self.manual_probe.is_some() => self.accept_probe(),
            CalibrationCommand::Accept | CalibrationCommand::Adjusted if self.bed_screws.is_some() => {
                self.accept_screw(command == CalibrationCommand::Adjusted)
            }
            CalibrationCommand::Abort if self.manual_probe.is_some() || self.bed_screws.is_some() => {
                self.manual_probe = None;
                self.bed_screws = None;
            }
            CalibrationCommand::BedScrewsAdjust if self.screws => {
                if self.is_waiting() {
                    return Err(CommandError::new("Already in bed_screws helper; use ABORT to exit"));
                }
                must_be_homed(homed)?;
                self.bed_screws = Some(BedScrews { current: 0, accepted: 0 });
            }
            CalibrationCommand::ScrewsTiltCalculate if self.screws && self.probe => {
                must_be_homed(homed)?;
                self.screw_results = true;
            }
            CalibrationCommand::BedMeshCalibrate { profile, probe_count, round_probe_count }
                if self.bed_mesh && self.probe =>
            {
                must_be_homed(homed)?;
                let profile = profile.unwrap_or_else(|| DEFAULT_MESH.to_owned());
                let grid = self.configured_grid().with_counts(probe_count, round_probe_count);
                let mesh = Mesh { grid, points: grid.heights(-0.01) };
                self.store_mesh(&profile, mesh.clone());
                self.active_mesh = Some((profile, mesh));
            }
            CalibrationCommand::BedMeshLoad { profile } if self.bed_mesh => {
                let Some(mesh) = self.meshes.get(&profile) else {
                    return Err(CommandError::new(format!("bed_mesh: Unknown profile [{profile}]")));
                };
                self.active_mesh = Some((profile, mesh.clone()));
            }
            CalibrationCommand::BedMeshClear if self.bed_mesh => self.active_mesh = None,
            // Klipper only answers with a message when there is no mesh or the name is `default`.
            CalibrationCommand::BedMeshSave { profile } if self.bed_mesh => {
                if let Some((_, mesh)) = self.active_mesh.clone().filter(|_| profile != DEFAULT_MESH) {
                    self.store_mesh(&profile, mesh);
                }
            }
            CalibrationCommand::BedMeshRemove { profile } if self.bed_mesh => {
                if self.meshes.remove(&profile).is_some() {
                    self.pending.insert(format!("bed_mesh {profile}"), None);
                }
            }
            other => return Err(CommandError::new(format!("Unknown command:\"{}\"", command_name(&other)))),
        }
        Ok(())
    }

    /// Puts options of a section on the list of `SAVE_CONFIG` (`configfile.set`), keeping what is there.
    pub fn set_pending(&mut self, section: &str, options: Map<String, Value>) {
        self.pending.entry(section.to_owned()).or_default().get_or_insert_with(Map::new).extend(options);
    }

    fn set_eddy_pending(&mut self, (option, value): (&str, String)) {
        self.pending
            .entry(EddyProbe::section_name().to_owned())
            .or_default()
            .get_or_insert_with(Map::new)
            .insert(option.to_owned(), json!(value));
    }

    fn configured_grid(&self) -> MeshGrid {
        self.mesh_config.unwrap_or_else(|| MeshGrid::configured(false))
    }

    fn store_mesh(&mut self, profile: &str, mesh: Mesh) {
        self.pending.insert(format!("bed_mesh {profile}"), Some(mesh.grid.profile_section(&mesh.points)));
        self.meshes.insert(profile.to_owned(), mesh);
    }

    fn start_probe(&mut self, purpose: ProbePurpose, homed: bool) -> Result<(), CommandError> {
        if self.is_waiting() {
            return Err(CommandError::new("Already in a manual Z probe. Use ABORT to abort it."));
        }
        must_be_homed(homed)?;
        self.manual_probe = Some(ManualProbe::new(purpose));
        Ok(())
    }

    fn probing(&mut self) -> Result<&mut ManualProbe, CommandError> {
        self.manual_probe.as_mut().ok_or_else(|| CommandError::new("Unknown command:\"TESTZ\""))
    }

    fn accept_probe(&mut self) {
        let Some(probe) = self.manual_probe.take() else { return };
        if probe.purpose == ProbePurpose::EddyHeights {
            self.set_eddy_pending(EddyProbe::measured_table());
        }
        if probe.purpose == ProbePurpose::ProbeOffset {
            // The probe triggered at the start height; the nozzle touched the paper at `z`.
            let z_offset = PROBE_Z_OFFSET + START_HEIGHT - probe.z;
            self.pending
                .entry("probe".to_owned())
                .or_default()
                .get_or_insert_with(Map::new)
                .insert("z_offset".to_owned(), json!(format!("{z_offset:.3}")));
        }
    }

    /// Like `bed_screws.py`: all screws in a row without adjustment end the helper; `ADJUSTED` starts the count over.
    fn accept_screw(&mut self, adjusted: bool) {
        let Some(mut screws) = self.bed_screws.take() else { return };
        screws.accepted = if adjusted { 0 } else { screws.accepted + 1 };
        if screws.accepted < SCREWS.len() as i64 {
            screws.current = (screws.current + 1) % SCREWS.len();
            self.bed_screws = Some(screws);
        }
    }

    fn is_waiting(&self) -> bool {
        self.manual_probe.is_some() || self.bed_screws.is_some()
    }

    /// `SAVE_CONFIG` writes everything pending.
    pub fn save(&mut self) {
        let written = self.pending.remove(EddyProbe::section_name()).flatten();
        if let (Some(eddy), Some(options)) = (self.eddy.as_mut(), written) {
            eddy.write(&options);
        }
        self.pending.clear();
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn pending_json(&self) -> Value {
        json!(self.pending)
    }

    pub fn status(&self) -> Map<String, Value> {
        let mut objects = Map::new();
        let probe = self.manual_probe.as_ref();
        let tried = |pick: fn(&ManualProbe, f64) -> Option<f64>| probe.and_then(|probe| pick(probe, probe.z));
        objects.insert(
            "manual_probe".to_owned(),
            json!({
                "is_active": probe.is_some(),
                "z_position": probe.map(|probe| round(probe.z)),
                "z_position_lower": tried(ManualProbe::below),
                "z_position_upper": tried(ManualProbe::above),
            }),
        );
        if self.probe {
            objects.insert(
                "probe".to_owned(),
                json!({ "name": "probe", "last_query": false, "last_probe_position": [0.0, 0.0, 0.0, 0.0] }),
            );
        }
        if self.bed_mesh {
            let profiles: Map<String, Value> = self
                .meshes
                .iter()
                .map(|(name, mesh)| (name.clone(), json!({ "points": mesh.points, "mesh_params": mesh.grid.params() })))
                .collect();
            // Without a mesh Klipper reports an empty name, zero corners and `[[]]`.
            let (name, points, min, max) = match &self.active_mesh {
                Some((name, mesh)) => {
                    let (min, max) = mesh.grid.extent();
                    (name.as_str(), json!(mesh.points), min, max)
                }
                None => ("", json!([[]]), [0.0; 2], [0.0; 2]),
            };
            objects.insert(
                "bed_mesh".to_owned(),
                json!({
                    "profile_name": name,
                    "mesh_min": min,
                    "mesh_max": max,
                    "probed_matrix": points,
                    "mesh_matrix": points,
                    "profiles": profiles,
                }),
            );
        }
        if self.screws {
            let screws = self.bed_screws;
            objects.insert(
                "bed_screws".to_owned(),
                json!({
                    "is_active": screws.is_some(),
                    "state": screws.map(|_| "adjust"),
                    "current_screw": screws.map_or(0, |screws| screws.current),
                    "accepted_screws": screws.map_or(0, |screws| screws.accepted),
                }),
            );
            objects.insert("screws_tilt_adjust".to_owned(), self.screw_tilt_json());
        }
        objects
    }

    /// The raw text of the file for the sections where it differs from `settings` (Eddy: no defaults).
    pub fn raw_config_sections(&self) -> Map<String, Value> {
        let eddy = self.eddy.as_ref().map(|eddy| (EddyProbe::section_name().to_owned(), eddy.raw()));
        eddy.into_iter().collect()
    }

    fn screw_tilt_json(&self) -> Value {
        let results: Map<String, Value> = if self.screw_results {
            SCREW_TILT
                .iter()
                .enumerate()
                .map(|(index, (sign, adjust))| {
                    let z = 2.5 - index as f64 * 0.04;
                    (
                        format!("screw{}", index + 1),
                        json!({ "z": z, "sign": sign, "adjust": adjust, "is_base": index == 0 }),
                    )
                })
                .collect()
        } else {
            Map::new()
        };
        json!({ "error": false, "max_deviation": null, "results": results })
    }

    pub fn config_sections(&self) -> Map<String, Value> {
        let mut sections = Map::new();
        if let Some(eddy) = &self.eddy {
            sections.insert(EddyProbe::section_name().to_owned(), eddy.settings());
        } else if self.probe {
            sections.insert(
                "probe".to_owned(),
                json!({ "x_offset": 23.0, "y_offset": -22.5, "z_offset": PROBE_Z_OFFSET, "speed": 5.0 }),
            );
        }
        if self.bed_mesh {
            sections.insert("bed_mesh".to_owned(), self.configured_grid().config_section());
        }
        if self.screws {
            let mut screws = Map::new();
            for (index, (name, position)) in SCREWS.iter().enumerate() {
                screws.insert(format!("screw{}", index + 1), json!(position));
                screws.insert(format!("screw{}_name", index + 1), json!(name));
            }
            sections.insert("bed_screws".to_owned(), Value::Object(screws.clone()));
            screws.insert("screw_thread".to_owned(), json!("CW-M3"));
            sections.insert("screws_tilt_adjust".to_owned(), Value::Object(screws));
        }
        sections
    }
}

fn must_be_homed(homed: bool) -> Result<(), CommandError> {
    if homed { Ok(()) } else { Err(CommandError::new("Must home axis first")) }
}

fn command_name(command: &CalibrationCommand) -> &'static str {
    match command {
        CalibrationCommand::ProbeCalibrate => "PROBE_CALIBRATE",
        CalibrationCommand::EddyDriveCurrent { .. } => "LDC_CALIBRATE_DRIVE_CURRENT",
        CalibrationCommand::EddyCalibrate { .. } => "PROBE_EDDY_CURRENT_CALIBRATE",
        CalibrationCommand::EddyTap(_) => "PROBE_EDDY_CURRENT_TAP_CALIBRATE",
        CalibrationCommand::ManualProbe => "MANUAL_PROBE",
        CalibrationCommand::TestZ(_) => "TESTZ",
        CalibrationCommand::Accept => "ACCEPT",
        CalibrationCommand::Adjusted => "ADJUSTED",
        CalibrationCommand::Abort => "ABORT",
        CalibrationCommand::BedScrewsAdjust => "BED_SCREWS_ADJUST",
        CalibrationCommand::ScrewsTiltCalculate => "SCREWS_TILT_CALCULATE",
        CalibrationCommand::BedMeshCalibrate { .. } => "BED_MESH_CALIBRATE",
        CalibrationCommand::BedMeshLoad { .. }
        | CalibrationCommand::BedMeshSave { .. }
        | CalibrationCommand::BedMeshRemove { .. } => "BED_MESH_PROFILE",
        CalibrationCommand::BedMeshClear => "BED_MESH_CLEAR",
    }
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bed_mesh::MESH_COUNT;

    fn printer() -> Calibration {
        Calibration::new(true, false, true, true)
    }

    #[test]
    fn manual_probe_bisects_between_heights_tried() {
        let mut calibration = printer();
        calibration.apply(CalibrationCommand::ProbeCalibrate, true).unwrap();
        calibration.apply(CalibrationCommand::TestZ(TestZ::By(-1.0)), true).unwrap();
        calibration.apply(CalibrationCommand::TestZ(TestZ::Up { half: true }), true).unwrap();
        let status = &calibration.status()["manual_probe"];
        assert_eq!(status["z_position"], 1.2);
        assert_eq!(status["z_position_lower"], 1.0);
        assert_eq!(status["z_position_upper"], 2.0);

        calibration.apply(CalibrationCommand::Accept, true).unwrap();
        assert_eq!(calibration.status()["manual_probe"]["is_active"], false);
        assert_eq!(calibration.pending_json()["probe"]["z_offset"], "1.800");
    }

    #[test]
    fn mode_commands_are_unknown_outside_the_mode() {
        let mut calibration = printer();
        assert!(calibration.apply(CalibrationCommand::Accept, true).is_err());
        assert!(calibration.apply(CalibrationCommand::TestZ(TestZ::By(0.1)), true).is_err());
        assert!(calibration.apply(CalibrationCommand::ProbeCalibrate, false).is_err());
    }

    #[test]
    fn bed_screws_end_after_every_screw_was_accepted_in_a_row() {
        let mut calibration = printer();
        calibration.apply(CalibrationCommand::BedScrewsAdjust, true).unwrap();
        calibration.apply(CalibrationCommand::Accept, true).unwrap();
        calibration.apply(CalibrationCommand::Adjusted, true).unwrap();
        assert_eq!(calibration.status()["bed_screws"]["current_screw"], 2);
        assert_eq!(calibration.status()["bed_screws"]["accepted_screws"], 0);
        for _ in SCREWS {
            calibration.apply(CalibrationCommand::Accept, true).unwrap();
        }
        assert_eq!(calibration.status()["bed_screws"]["is_active"], false);
    }

    #[test]
    fn mesh_calibration_is_active_and_pending() {
        let mut calibration = printer();
        calibration
            .apply(
                CalibrationCommand::BedMeshCalibrate { profile: None, probe_count: None, round_probe_count: None },
                true,
            )
            .unwrap();
        assert_eq!(calibration.status()["bed_mesh"]["profile_name"], "default");
        assert!(calibration.has_pending());
        assert_eq!(calibration.status()["bed_mesh"]["probed_matrix"].as_array().unwrap().len(), MESH_COUNT);
        calibration.apply(CalibrationCommand::BedMeshClear, true).unwrap();
        assert_eq!(calibration.status()["bed_mesh"]["profile_name"], "");
        assert_eq!(calibration.status()["bed_mesh"]["probed_matrix"], json!([[]]));
        calibration.save();
        assert!(!calibration.has_pending());
    }

    #[test]
    fn a_dense_scan_reports_its_grid() {
        let mut calibration = printer();

        calibration
            .apply(
                CalibrationCommand::BedMeshCalibrate {
                    profile: Some("dense".into()),
                    probe_count: Some((30, 40)),
                    round_probe_count: Some(31),
                },
                true,
            )
            .unwrap();

        let status = calibration.status();
        assert_eq!(status["bed_mesh"]["probed_matrix"].as_array().unwrap().len(), 40);
        assert_eq!(status["bed_mesh"]["probed_matrix"][0].as_array().unwrap().len(), 30);
        assert_eq!(status["bed_mesh"]["profiles"]["dense"]["mesh_params"]["x_count"], 30);
        assert_eq!(calibration.pending_json()["bed_mesh dense"]["y_count"], "40");
    }

    #[test]
    fn a_round_bed_configures_a_radius_and_scans_a_circle() {
        let mut calibration = Calibration::new(true, false, true, false).with_round_mesh();

        calibration
            .apply(
                CalibrationCommand::BedMeshCalibrate {
                    profile: None,
                    probe_count: Some((30, 40)),
                    round_probe_count: Some(31),
                },
                true,
            )
            .unwrap();

        let settings = calibration.config_sections();
        assert_eq!(settings["bed_mesh"]["mesh_radius"], 140.0);
        assert!(settings["bed_mesh"].get("mesh_min").is_none());
        let status = calibration.status();
        assert_eq!(status["bed_mesh"]["probed_matrix"].as_array().unwrap().len(), 31);
        assert_eq!(status["bed_mesh"]["mesh_min"], json!([10.05, 10.05]));
    }

    #[test]
    fn profiles_are_saved_and_removed_for_the_session() {
        let mut calibration = printer();
        calibration.apply(CalibrationCommand::BedMeshLoad { profile: "pei-textured".into() }, true).unwrap();
        calibration.apply(CalibrationCommand::BedMeshSave { profile: "copy".into() }, true).unwrap();
        calibration.apply(CalibrationCommand::BedMeshSave { profile: DEFAULT_MESH.into() }, true).unwrap();
        calibration.apply(CalibrationCommand::BedMeshRemove { profile: "pei-textured".into() }, true).unwrap();
        let status = calibration.status();
        let profiles = status["bed_mesh"]["profiles"].as_object().unwrap();
        assert_eq!(profiles.keys().collect::<Vec<_>>(), ["copy", "default"]);
        // Saving does not rename the active mesh, removing its profile keeps it.
        assert_eq!(status["bed_mesh"]["profile_name"], "pei-textured");
        assert_eq!(calibration.pending_json()["bed_mesh pei-textured"], Value::Null);
        assert_eq!(calibration.pending_json()["bed_mesh copy"]["x_count"], "5");
    }

    fn eddy_printer() -> Calibration {
        Calibration::new(true, true, true, false)
    }

    fn eddy_chip() -> String {
        "eddy".to_owned()
    }

    #[test]
    fn an_eddy_probe_has_no_probe_calibrate_but_the_setup_steps() {
        let mut calibration = eddy_printer();
        assert!(calibration.apply(CalibrationCommand::ProbeCalibrate, true).is_err());
        calibration.apply(CalibrationCommand::EddyDriveCurrent { chip: eddy_chip() }, true).unwrap();
        assert_eq!(calibration.pending_json()["probe_eddy_current eddy"]["reg_drive_current"], "16");
        assert!(calibration.apply(CalibrationCommand::EddyDriveCurrent { chip: "other".into() }, true).is_err());
        assert!(calibration.config_sections().get("probe").is_none());
    }

    #[test]
    fn the_z_heights_go_through_the_paper_test_and_leave_the_table_pending() {
        let mut calibration = eddy_printer();
        calibration.apply(CalibrationCommand::EddyCalibrate { chip: eddy_chip() }, true).unwrap();
        assert_eq!(calibration.status()["manual_probe"]["is_active"], true);
        assert!(!calibration.has_pending());
        calibration.apply(CalibrationCommand::Accept, true).unwrap();
        let table = calibration.pending_json()["probe_eddy_current eddy"]["calibrate"].clone();
        assert!(table.as_str().unwrap().contains(':'));
        calibration.save();
        assert!(!calibration.has_pending());
        assert_eq!(calibration.raw_config_sections()["probe_eddy_current eddy"]["calibrate"], table);
    }

    #[test]
    fn verify_leaves_the_tap_threshold_pending_and_saving_writes_it_to_the_file() {
        let mut calibration = eddy_printer();
        let raw = |calibration: &Calibration| calibration.raw_config_sections()["probe_eddy_current eddy"].clone();
        assert!(raw(&calibration).get("tap_threshold").is_none());
        for step in [TapStep::Guess, TapStep::Refine] {
            calibration.apply(CalibrationCommand::EddyTap(step), true).unwrap();
            assert!(!calibration.has_pending());
        }
        calibration.apply(CalibrationCommand::EddyTap(TapStep::Verify), true).unwrap();
        assert_eq!(calibration.pending_json()["probe_eddy_current eddy"]["tap_threshold"], "0.582");
        calibration.save();
        assert_eq!(raw(&calibration)["tap_threshold"], "0.582");
    }
}
