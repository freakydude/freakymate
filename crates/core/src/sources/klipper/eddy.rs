//! Eddy current probes (`[probe_eddy_current <name>]`): the section name that the `CHIP` parameter of their commands
//! takes, and the options the setup steps write (`LDC_CALIBRATE_DRIVE_CURRENT`, `PROBE_EDDY_CURRENT_CALIBRATE`,
//! `PROBE_EDDY_CURRENT_TAP_CALIBRATE`), read from `configfile.config` and `save_config_pending_items`.
//! Source: Klipper master 29d4ec2d, `klippy/extras/probe_eddy_current.py` and `ldc1612.py`, `docs/Eddy_Probe.md`.

use super::Motion;
use super::calibration::PendingSection;
use crate::sources::fields::{number, section};
use serde_json::Value;
use std::ops::RangeInclusive;

/// The object name prefix of the section.
const OBJECT_PREFIX: &str = "probe_eddy_current ";

/// The toolhead heights in mm the guide names for the tap steps (Eddy_Probe.md: nozzle 3 to 10 mm above the bed).
pub const TAP_HEIGHT_MM: RangeInclusive<f64> = 3.0..=10.0;

/// The `tap` calibration steps; Klipper checks their order itself ("Must complete valid 'tap' probe first").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EddyTap {
    Guess,
    Refine,
    Verify,
}

impl EddyTap {
    /// The value of the `TAP` parameter (`probe_eddy_current.py` `cmd_TAP_CALIBRATE`).
    pub fn parameter(self) -> &'static str {
        match self {
            Self::Guess => "guess",
            Self::Refine => "refine",
            Self::Verify => "verify",
        }
    }
}

/// The name of an Eddy probe from an object name (`probe_eddy_current eddy` is `eddy`); helpers starting with `_` and
/// other objects give `None`.
pub fn instance(object: &str) -> Option<&str> {
    object.strip_prefix(OBJECT_PREFIX).filter(|name| !name.is_empty() && !name.starts_with('_'))
}

/// The Eddy probe the configuration has (`configfile.settings` lists every section, lower case), for a Klipper whose
/// object list does not name it: Klipper lists only objects with a `get_status`, and the `probe` object is the helper.
pub(super) fn configured_name(settings: Option<&Value>) -> Option<&str> {
    settings?.as_object()?.keys().find_map(|section| instance(section))
}

/// What one option holds in the configuration file now and what `SAVE_CONFIG` would write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EddyOption<T> {
    pub current: Option<T>,
    pub pending: Option<T>,
}

/// The setup state of the Eddy probe the printer has.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EddyProbe {
    /// The section name after `probe_eddy_current `, the value of `CHIP`.
    pub name: String,
    pub drive_current: EddyOption<String>,
    /// The number of points of the `calibrate` table, not the table itself.
    pub calibration_points: EddyOption<usize>,
    pub tap_threshold: EddyOption<String>,
    /// `x_offset` and `y_offset` in mm: where the sensor sits relative to the nozzle (Klipper lists the defaults, 0).
    pub offset: [f64; 2],
}

impl EddyProbe {
    /// The first Eddy probe among the object names, with its options from `configfile`.
    pub(super) fn read<'a>(names: impl IntoIterator<Item = &'a str>, configfile: Option<&Value>) -> Option<Self> {
        let name = names
            .into_iter()
            .find_map(instance)
            .or_else(|| configured_name(configfile.and_then(|c| c.get("settings"))))?;
        let raw = configfile
            .and_then(|configfile| configfile.get("config"))
            .and_then(|config| section(config, OBJECT_PREFIX, name));
        let pending = pending_options(configfile, name);
        let option = |key: &str| raw.and_then(|raw| raw.get(key)).and_then(Value::as_str).map(str::trim);
        let pending_option = |key: &str| {
            pending.as_ref().and_then(|options| options.iter().find(|(option, _)| option == key)).map(|(_, v)| v.trim())
        };
        let text = |key: &str| EddyOption {
            current: option(key).filter(|value| !value.is_empty()).map(str::to_owned),
            pending: pending_option(key).filter(|value| !value.is_empty()).map(str::to_owned),
        };
        let points = |value: &str| table_points(value);
        Some(Self {
            name: name.to_owned(),
            drive_current: text("reg_drive_current"),
            calibration_points: EddyOption {
                current: option("calibrate").map(points).filter(|count| *count > 0),
                pending: pending_option("calibrate").map(points).filter(|count| *count > 0),
            },
            tap_threshold: text("tap_threshold"),
            offset: configured_offset(configfile, name),
        })
    }

    /// Whether the sensor sits away from the nozzle, so "sensor over the centre" differs from "nozzle over the centre".
    pub fn has_offset(&self) -> bool {
        self.offset != [0.0; 2]
    }

    /// The object name of the section, as shown next to the title.
    pub fn section(&self) -> String {
        format!("{OBJECT_PREFIX}{}", self.name)
    }
}

/// `x_offset` and `y_offset` of `configfile.settings`; 0 where the file does not tell.
fn configured_offset(configfile: Option<&Value>, name: &str) -> [f64; 2] {
    let settings = configfile.and_then(|configfile| configfile.get("settings"));
    let section = settings.and_then(|settings| section(settings, OBJECT_PREFIX, name));
    ["x_offset", "y_offset"].map(|option| section.map_or(0.0, |section| number(section, option)))
}

/// What to put over the middle of the bed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CentreOf {
    Nozzle,
    Sensor,
}

/// Where the toolhead goes so that the nozzle, or the sensor at its offset, is over the middle of the axis ranges
/// (`toolhead.axis_minimum` and `axis_maximum`; the bed mesh may not exist), kept within the axes. `None` if the
/// printer reports no range.
pub fn centre_target(motion: &Motion, probe: &EddyProbe, over: CentreOf) -> Option<[f64; 2]> {
    let [x, y] = [0, 1].map(|axis| {
        let (minimum, maximum) = (motion.axis_minimum[axis], motion.axis_maximum[axis]);
        let offset = if over == CentreOf::Sensor { probe.offset[axis] } else { 0.0 };
        (maximum > minimum).then(|| ((minimum + maximum) / 2.0 - offset).clamp(minimum, maximum))
    });
    Some([x?, y?])
}

/// The pending options of the section of Eddy probe `name`, if `SAVE_CONFIG` would write it.
fn pending_options(configfile: Option<&Value>, name: &str) -> Option<Vec<(String, String)>> {
    let pending = super::calibration::read_pending(configfile);
    let wanted = format!("{OBJECT_PREFIX}{name}");
    pending.into_iter().find(|PendingSection { section, .. }| section.eq_ignore_ascii_case(&wanted))?.options
}

/// The points of a `calibrate` table: `z:frequency` pairs, three per line, separated by commas
/// (`EddyCalibration.apply_calibration`, `probe_eddy_current.py`).
fn table_points(table: &str) -> usize {
    table.split([',', '\n']).filter(|point| point.contains(':')).count()
}

/// Whether the toolhead height suits the tap steps; a hint only, nothing is locked.
pub fn tap_height_fits(z: f64) -> bool {
    TAP_HEIGHT_MM.contains(&z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TABLE: &str = "\n3.000000:3300.123,2.500000:3400.456,2.000000:3500.789\n1.500000:3600.1,1.000000:3700.2";

    fn read(configfile: Value) -> Option<EddyProbe> {
        EddyProbe::read(["probe", "probe_eddy_current eddy"], Some(&configfile))
    }

    #[test]
    fn names_the_probe_after_its_section() {
        assert_eq!(instance("probe_eddy_current eddy"), Some("eddy"));
        assert_eq!(instance("probe_eddy_current _helper"), None);
        assert_eq!(instance("probe"), None);
        assert_eq!(EddyProbe::read(["probe"], None), None);
    }

    #[test]
    fn reads_the_options_of_the_file_and_the_pending_ones() {
        let probe = read(json!({
            "config": { "probe_eddy_current eddy": {
                "reg_drive_current": "15", "calibrate": TABLE, "z_offset": "0.5"
            } },
            "save_config_pending_items": { "probe_eddy_current eddy": { "tap_threshold": "0.582" } }
        }))
        .unwrap();
        assert_eq!(probe.name, "eddy");
        assert_eq!(probe.section(), "probe_eddy_current eddy");
        assert_eq!(probe.drive_current, EddyOption { current: Some("15".into()), pending: None });
        assert_eq!(probe.calibration_points, EddyOption { current: Some(5), pending: None });
        assert_eq!(probe.tap_threshold, EddyOption { current: None, pending: Some("0.582".into()) });
    }

    #[test]
    fn a_long_pending_table_counts_its_points() {
        let probe = read(json!({
            "config": { "probe_eddy_current eddy": {} },
            "save_config_pending_items": { "probe_eddy_current eddy": { "calibrate": TABLE } }
        }))
        .unwrap();
        assert_eq!(probe.calibration_points, EddyOption { current: None, pending: Some(5) });
    }

    #[test]
    fn without_the_file_text_nothing_is_known_but_the_name() {
        let probe = read(json!({ "settings": { "probe_eddy_current eddy": { "tap_threshold": 0.4 } } })).unwrap();
        assert_eq!(probe, EddyProbe { name: "eddy".into(), ..EddyProbe::default() });
    }

    fn motion() -> Motion {
        Motion { axis_minimum: [0.0, 0.0, -2.0, 0.0], axis_maximum: [350.0, 300.0, 330.0, 0.0], ..Motion::default() }
    }

    fn probe_at(offset: [f64; 2]) -> EddyProbe {
        EddyProbe { name: "eddy".into(), offset, ..EddyProbe::default() }
    }

    #[test]
    fn finds_the_probe_in_the_configuration_if_the_object_list_lacks_it() {
        let configfile = json!({ "settings": { "probe_eddy_current eddy": { "x_offset": 3.0 }, "probe": {} } });
        let probe = EddyProbe::read(["probe"], Some(&configfile)).unwrap();
        assert_eq!((probe.name.as_str(), probe.offset), ("eddy", [3.0, 0.0]));
        assert_eq!(configured_name(Some(&json!({ "probe": {} }))), None);
    }

    #[test]
    fn reads_the_offset_of_the_sensor_from_the_settings() {
        let probe = read(json!({ "settings": { "probe_eddy_current eddy": { "x_offset": -21.0, "y_offset": 4.5 } } }));
        assert_eq!(probe.unwrap().offset, [-21.0, 4.5]);
        let none = read(json!({ "settings": { "probe_eddy_current eddy": { "x_offset": 0.0, "y_offset": 0.0 } } }));
        assert!(!none.unwrap().has_offset());
    }

    #[test]
    fn the_nozzle_goes_to_the_middle_and_the_sensor_by_its_offset() {
        let probe = probe_at([-21.0, 4.5]);
        assert_eq!(centre_target(&motion(), &probe, CentreOf::Nozzle), Some([175.0, 150.0]));
        assert_eq!(centre_target(&motion(), &probe, CentreOf::Sensor), Some([196.0, 145.5]));
    }

    #[test]
    fn the_centre_stays_within_the_axes() {
        let near_the_edge = Motion { axis_maximum: [30.0, 300.0, 330.0, 0.0], ..motion() };
        assert_eq!(centre_target(&near_the_edge, &probe_at([-40.0, 0.0]), CentreOf::Sensor), Some([30.0, 150.0]));
        assert_eq!(centre_target(&near_the_edge, &probe_at([40.0, 0.0]), CentreOf::Sensor), Some([0.0, 150.0]));
    }

    #[test]
    fn without_an_axis_range_there_is_no_centre() {
        assert_eq!(centre_target(&Motion::default(), &probe_at([0.0; 2]), CentreOf::Nozzle), None);
    }

    #[test]
    fn the_tap_range_is_three_to_ten_millimetres() {
        assert!(!tap_height_fits(2.9));
        assert!(tap_height_fits(3.0) && tap_height_fits(6.0) && tap_height_fits(10.0));
        assert!(!tap_height_fits(20.0));
    }
}
