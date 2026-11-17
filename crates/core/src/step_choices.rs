//! The step sizes the UI offers, configurable in the `[step_choices]` table of the settings file. A missing key means
//! the defaults; a list written by hand is sorted, cleaned and checked here, so the UI only shows what fits.

use serde::{Deserialize, Serialize};

/// Most steps one row of the UI has room for at 800x480.
const MAX_STEPS: usize = 6;
/// Most steps per side for the manual probe, whose row shows the minus and the plus side.
const MAX_PROBE_STEPS: usize = 4;

pub const DEFAULT_MOVE_MM: [f64; 5] = [0.1, 1.0, 10.0, 50.0, 100.0];
pub const DEFAULT_Z_OFFSET_MM: [f64; 5] = [0.005, 0.01, 0.025, 0.05, 0.1];
pub const DEFAULT_TEMPERATURE: [f64; 4] = [1.0, 5.0, 10.0, 25.0];
/// Lengths in mm the Filament page offers for extruding by hand, before the limit of the printer cuts them.
pub const DEFAULT_EXTRUDE_MM: [f64; 5] = [5.0, 10.0, 25.0, 50.0, 100.0];
pub const DEFAULT_EXTRUDE_SPEED: [f64; 4] = [1.0, 2.0, 5.0, 10.0];
pub const DEFAULT_MANUAL_PROBE_MM: [f64; 4] = [0.01, 0.05, 0.1, 1.0];

/// The lists as written in the settings file; `None` is a key that is not there. Kept as written, so saving the
/// settings does not rearrange what someone typed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepChoices {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_mm: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z_offset_mm: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extrude_mm: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extrude_speed: Option<Vec<f64>>,
    /// One side only, the steps of the manual probe are offered in both directions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_probe_mm: Option<Vec<f64>>,
}

/// The lists the UI offers: each ascending, without duplicates, positive and within the maximum count.
#[derive(Debug, Clone, PartialEq)]
pub struct StepLists {
    pub move_mm: Vec<f64>,
    pub z_offset_mm: Vec<f64>,
    pub temperature: Vec<f64>,
    pub extrude_mm: Vec<f64>,
    pub extrude_speed: Vec<f64>,
    pub manual_probe_mm: Vec<f64>,
}

impl StepChoices {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn lists(&self) -> StepLists {
        StepLists {
            move_mm: normalise("move_mm", self.move_mm.as_deref(), &DEFAULT_MOVE_MM, MAX_STEPS),
            z_offset_mm: normalise("z_offset_mm", self.z_offset_mm.as_deref(), &DEFAULT_Z_OFFSET_MM, MAX_STEPS),
            temperature: normalise("temperature", self.temperature.as_deref(), &DEFAULT_TEMPERATURE, MAX_STEPS),
            extrude_mm: normalise("extrude_mm", self.extrude_mm.as_deref(), &DEFAULT_EXTRUDE_MM, MAX_STEPS),
            extrude_speed: normalise("extrude_speed", self.extrude_speed.as_deref(), &DEFAULT_EXTRUDE_SPEED, MAX_STEPS),
            manual_probe_mm: normalise(
                "manual_probe_mm",
                self.manual_probe_mm.as_deref(),
                &DEFAULT_MANUAL_PROBE_MM,
                MAX_PROBE_STEPS,
            ),
        }
    }
}

impl Default for StepLists {
    fn default() -> Self {
        StepChoices::default().lists()
    }
}

/// Sorts the configured values ascending and drops duplicates and values that are not finite or not above zero. If
/// nothing is left or there are more than `max`, the defaults apply; each case logs one warning naming the key.
fn normalise(key: &str, configured: Option<&[f64]>, default: &[f64], max: usize) -> Vec<f64> {
    let Some(configured) = configured else {
        return default.to_vec();
    };
    let mut values: Vec<f64> = configured.iter().copied().filter(|value| value.is_finite() && *value > 0.0).collect();
    if values.len() != configured.len() {
        log::warn!("settings: step_choices.{key} has values that are not above zero; they are left out");
    }
    values.sort_by(f64::total_cmp);
    values.dedup();
    if values.is_empty() {
        log::warn!("settings: step_choices.{key} has no usable value; using the defaults");
        return default.to_vec();
    }
    if values.len() > max {
        log::warn!("settings: step_choices.{key} has {} values, at most {max} fit; using the defaults", values.len());
        return default.to_vec();
    }
    values
}

/// The value of `list` closest to `value` (the smaller one on a tie); `value` itself for an empty list.
pub fn nearest_step(list: &[f64], value: f64) -> f64 {
    list.iter().copied().min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs())).unwrap_or(value)
}

/// A row for both directions: the steps negated and descending, then the steps ascending.
pub fn mirrored_row(list: &[f64]) -> Vec<f64> {
    list.iter().rev().map(|step| -step).chain(list.iter().copied()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices(move_mm: &[f64]) -> StepChoices {
        StepChoices { move_mm: Some(move_mm.to_vec()), ..StepChoices::default() }
    }

    #[test]
    fn without_entries_the_lists_are_the_defaults() {
        let lists = StepChoices::default().lists();

        assert_eq!(lists.move_mm, DEFAULT_MOVE_MM);
        assert_eq!(lists.z_offset_mm, DEFAULT_Z_OFFSET_MM);
        assert_eq!(lists.temperature, DEFAULT_TEMPERATURE);
        assert_eq!(lists.extrude_mm, DEFAULT_EXTRUDE_MM);
        assert_eq!(lists.extrude_speed, DEFAULT_EXTRUDE_SPEED);
        assert_eq!(lists.manual_probe_mm, DEFAULT_MANUAL_PROBE_MM);
    }

    #[test]
    fn a_list_is_sorted_and_loses_duplicates() {
        assert_eq!(choices(&[10.0, 0.5, 10.0, 2.0]).lists().move_mm, [0.5, 2.0, 10.0]);
    }

    #[test]
    fn values_that_are_not_finite_or_not_above_zero_are_dropped() {
        let list = choices(&[0.0, -1.0, f64::NAN, f64::INFINITY, 5.0, 1.0]).lists().move_mm;

        assert_eq!(list, [1.0, 5.0]);
    }

    #[test]
    fn nothing_usable_means_the_defaults() {
        assert_eq!(choices(&[]).lists().move_mm, DEFAULT_MOVE_MM);
        assert_eq!(choices(&[0.0, -2.0]).lists().move_mm, DEFAULT_MOVE_MM);
    }

    #[test]
    fn too_many_values_mean_the_defaults() {
        assert_eq!(choices(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).lists().move_mm, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(choices(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]).lists().move_mm, DEFAULT_MOVE_MM);
        // Duplicates do not count.
        assert_eq!(choices(&[1.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).lists().move_mm, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn the_probe_row_has_room_for_four_per_side() {
        let probe = |values: &[f64]| {
            StepChoices { manual_probe_mm: Some(values.to_vec()), ..StepChoices::default() }.lists().manual_probe_mm
        };

        assert_eq!(probe(&[0.5, 0.1, 1.0, 0.01]), [0.01, 0.1, 0.5, 1.0]);
        assert_eq!(probe(&[0.01, 0.02, 0.05, 0.1, 1.0]), DEFAULT_MANUAL_PROBE_MM);
    }

    #[test]
    fn the_nearest_step_is_chosen_and_the_smaller_one_on_a_tie() {
        let list = [1.0, 5.0, 10.0];

        assert_eq!(nearest_step(&list, 5.0), 5.0);
        assert_eq!(nearest_step(&list, 7.0), 5.0);
        assert_eq!(nearest_step(&list, 8.0), 10.0);
        assert_eq!(nearest_step(&list, 3.0), 1.0);
        assert_eq!(nearest_step(&list, 500.0), 10.0);
        assert_eq!(nearest_step(&[], 4.0), 4.0);
    }

    #[test]
    fn the_probe_row_mirrors_the_steps() {
        assert_eq!(mirrored_row(&[0.01, 0.1, 1.0]), [-1.0, -0.1, -0.01, 0.01, 0.1, 1.0]);
    }

    #[test]
    fn the_table_is_read_from_toml_and_only_written_when_there_is_an_entry() {
        let text = "[step_choices]\nmove_mm = [100, 1, 0.5]\n";
        let settings: crate::Settings = toml::from_str(text).unwrap();

        assert_eq!(settings.step_choices.move_mm, Some(vec![100.0, 1.0, 0.5]));
        assert_eq!(settings.step_choices.lists().move_mm, [0.5, 1.0, 100.0]);
        assert!(toml::to_string(&settings).unwrap().contains("move_mm = [100.0, 1.0, 0.5]"));
        assert!(!toml::to_string(&crate::Settings::default()).unwrap().contains("step_choices"));
    }
}
