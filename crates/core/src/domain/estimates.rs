//! Values derived from several sources. Each rule prefers what the printer measured over what the slicer or
//! Klipper predicted (see [`prefer_measured`]) and says which source it used.

use crate::sourced::{Sourced, prefer_measured};
use crate::sources::klipper::PrintJob;
use crate::sources::moonraker_server::files::FileMetadata;
use crate::sources::moonraker_server::history::LastPrint;
use crate::sources::spoolman::Spool;

/// How much the expected print time counts against what has been observed so far, as a fraction of the expected
/// time. The smaller it is, the sooner the remaining time follows the measured pace.
const PRIOR_WEIGHT: f64 = 0.05;

/// Klipper requires `filament_diameter` in the extruder section, so a missing value means that the configuration is
/// not known (yet): assume the usual 1.75 mm filament.
const ASSUMED_FILAMENT_DIAMETER_MM: f64 = 1.75;

pub const ORIGIN_KLIPPER_REPORT: &str = "reported by Klipper";
pub const ORIGIN_LAYER_HEIGHT: &str = "object height divided by layer height";
pub const ORIGIN_HEIGHT_AHEAD: &str = "current height plus the slicer's layer height per layer ahead";
pub const ORIGIN_CONFIGURED_DIAMETER: &str = "configured filament diameter";
pub const ORIGIN_ASSUMED_DIAMETER: &str = "assumed filament diameter of 1.75 mm";
pub const ORIGIN_LAST_PRINT: &str = "last completed print";
pub const ORIGIN_SLICER_ESTIMATE: &str = "slicer estimate";
pub const ORIGIN_LAST_PRINT_LENGTH: &str = "length of the last print, scaled by the slicer weight";
pub const ORIGIN_SPOOL_DENSITY: &str = "length of the last print, diameter and density of the spool";

/// Total print time of a file: what the last completed print took, else the slicer estimate.
pub fn expected_print_seconds(metadata: Option<&FileMetadata>, last_print: Option<&LastPrint>) -> Option<Sourced<f64>> {
    let measured = last_print
        .map(|last_print| Sourced::measured(last_print.print_duration, ORIGIN_LAST_PRINT))
        .filter(is_positive);
    let estimated = metadata
        .and_then(|metadata| metadata.estimated_time)
        .map(|seconds| Sourced::estimated(seconds, ORIGIN_SLICER_ESTIMATE))
        .filter(is_positive);
    prefer_measured(measured, estimated)
}

fn is_positive(seconds: &Sourced<f64>) -> bool {
    seconds.value > 0.0
}

/// Weight of the filament of a file in grams. Moonraker records only the length a print used. With the one spool of
/// that print the weight follows from its diameter and density; otherwise it is the slicer's weight scaled by the used
/// length (same density and diameter assumed). Without a print there is the slicer's weight, as a prediction.
pub fn filament_weight_grams(
    metadata: &FileMetadata,
    last_print: Option<&LastPrint>,
    spool: Option<&Spool>,
) -> Option<Sourced<f64>> {
    let slicer_weight = metadata.filament_weight_total;
    let from_spool = last_print.zip(spool).and_then(|(last_print, spool)| {
        let weight = spool.weight_of_length(last_print.filament_used)?;
        Some(Sourced::measured(weight, ORIGIN_SPOOL_DENSITY))
    });
    let scaled = last_print.and_then(|last_print| {
        let planned_length = metadata.filament_total.filter(|length| *length > 0.0)?;
        Some(Sourced::measured(slicer_weight? * last_print.filament_used / planned_length, ORIGIN_LAST_PRINT_LENGTH))
    });
    let estimated = slicer_weight.map(|grams| Sourced::estimated(grams, ORIGIN_SLICER_ESTIMATE));
    prefer_measured(from_spool, prefer_measured(scaled, estimated))
}

/// Number of layers of a job: what Klipper reports (`SET_PRINT_STATS_INFO`), else the object height of the file
/// divided by its layer height.
pub fn total_layers(job: &PrintJob, metadata: Option<&FileMetadata>) -> Option<Sourced<u32>> {
    let reported = job.total_layer.map(|layers| Sourced::measured(layers, ORIGIN_KLIPPER_REPORT));
    let derived = metadata.and_then(|metadata| {
        let height = metadata.object_height?;
        let layer = metadata.layer_height.filter(|layer| *layer > 0.0)?;
        Some(Sourced::estimated((height / layer).round() as u32, ORIGIN_LAYER_HEIGHT))
    });
    prefer_measured(reported, derived)
}

/// Volumetric flow in mm³/s from the extruder velocity in mm/s and the configured filament diameter in mm.
pub fn volumetric_flow(extruder_velocity: f64, filament_diameter: Option<f64>) -> Sourced<f64> {
    let configured = filament_diameter.filter(|diameter| *diameter > 0.0);
    let diameter = configured.unwrap_or(ASSUMED_FILAMENT_DIAMETER_MM);
    let flow = extruder_velocity.max(0.0) * std::f64::consts::PI * (diameter / 2.0).powi(2);
    match configured {
        Some(_) => Sourced::measured(flow, ORIGIN_CONFIGURED_DIAMETER),
        None => Sourced::estimated(flow, ORIGIN_ASSUMED_DIAMETER),
    }
}

/// Seconds left of a running print. The expected total time counts like `PRIOR_WEIGHT` of it as observed print
/// time: at the start it is the whole answer, with growing progress the measured pace (`elapsed / progress`) takes
/// over, and nothing jumps in between. Without an expectation only the pace is left, once progress exists.
pub fn remaining_print_seconds(job: &PrintJob, expected: Option<f64>) -> Option<f64> {
    let (progress, elapsed) = (job.progress, job.print_duration);
    let remaining_share = 1.0 - progress.clamp(0.0, 1.0);
    let Some(expected) = expected else {
        return (progress > 0.0 && elapsed > 0.0).then(|| elapsed * remaining_share / progress);
    };
    if progress <= 0.0 {
        return Some(expected);
    }
    let prior_seconds = PRIOR_WEIGHT * expected;
    let pace = (elapsed + prior_seconds) / (progress * expected + prior_seconds);
    Some(remaining_share * expected * pace)
}

/// The height a layer ahead will have: the current height plus the slicer's layer height for each layer in between.
/// Starting from what is printed now leaves out the first layer; a variable layer height still makes it wrong.
pub fn height_at_layer(job: &PrintJob, metadata: Option<&FileMetadata>, layer: u32) -> Option<Sourced<f64>> {
    let layer_height = metadata?.layer_height.filter(|height| *height > 0.0)?;
    let ahead = layer.checked_sub(job.current_layer?)?;
    Some(Sourced::estimated(job.height? + f64::from(ahead) * layer_height, ORIGIN_HEIGHT_AHEAD))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sourced::Provenance;
    use crate::sources::klipper::PrintState;

    fn running(progress: f64, print_duration: f64) -> PrintJob {
        PrintJob { state: PrintState::Printing, progress, print_duration, ..PrintJob::default() }
    }

    fn remaining(expected: Option<f64>, progress: f64, elapsed: f64) -> Option<f64> {
        remaining_print_seconds(&running(progress, elapsed), expected)
    }

    fn metadata() -> FileMetadata {
        FileMetadata {
            estimated_time: Some(3600.0),
            filament_total: Some(10_000.0),
            filament_weight_total: Some(30.0),
            ..FileMetadata::default()
        }
    }

    #[test]
    fn last_completed_print_is_the_expectation_before_the_slicer_estimate() {
        let last_print = LastPrint { print_duration: 4200.0, filament_used: 9_000.0, ..LastPrint::default() };

        let expected = expected_print_seconds(Some(&metadata()), Some(&last_print)).unwrap();

        assert_eq!(expected, Sourced::measured(4200.0, ORIGIN_LAST_PRINT));
    }

    #[test]
    fn slicer_estimate_is_marked_as_an_estimate() {
        let expected = expected_print_seconds(Some(&metadata()), None).unwrap();

        assert_eq!(
            (expected.value, expected.provenance, expected.origin),
            (3600.0, Provenance::Estimated, ORIGIN_SLICER_ESTIMATE)
        );
    }

    #[test]
    fn no_expectation_without_positive_times() {
        let empty = FileMetadata { estimated_time: Some(0.0), ..FileMetadata::default() };

        assert_eq!(expected_print_seconds(Some(&empty), None), None);
        assert_eq!(expected_print_seconds(None, None), None);
    }

    #[test]
    fn weight_of_the_last_print_scales_the_slicer_weight_by_the_used_length() {
        let last_print = LastPrint { print_duration: 3300.0, filament_used: 9_000.0, ..LastPrint::default() };

        let weight = filament_weight_grams(&metadata(), Some(&last_print), None).unwrap();

        assert_eq!(weight, Sourced::measured(27.0, ORIGIN_LAST_PRINT_LENGTH));
    }

    #[test]
    fn the_weight_of_the_last_print_follows_the_spool_it_used() {
        let last_print = LastPrint { print_duration: 3300.0, filament_used: 10_000.0, spool_ids: vec![18] };
        let spool = Spool { id: 18, diameter: Some(1.75), density: Some(1.04), ..Spool::default() };

        let weight = filament_weight_grams(&metadata(), Some(&last_print), Some(&spool)).unwrap();

        // 10 m of 1.75 mm filament at 1.04 g/cm³; the slicer's 30 g for the same length is not used.
        assert!((weight.value - 25.02).abs() < 0.01);
        assert_eq!((weight.provenance, weight.origin), (Provenance::Measured, ORIGIN_SPOOL_DENSITY));
    }

    #[test]
    fn a_spool_without_density_leaves_the_scaled_slicer_weight() {
        let last_print = LastPrint { print_duration: 3300.0, filament_used: 9_000.0, spool_ids: vec![18] };
        let bare = Spool { id: 18, ..Spool::default() };

        let weight = filament_weight_grams(&metadata(), Some(&last_print), Some(&bare)).unwrap();

        assert_eq!(weight.origin, ORIGIN_LAST_PRINT_LENGTH);
    }

    #[test]
    fn weight_falls_back_to_the_slicer_weight() {
        let weight = filament_weight_grams(&metadata(), None, None).unwrap();

        assert_eq!(weight, Sourced::estimated(30.0, ORIGIN_SLICER_ESTIMATE));
    }

    #[test]
    fn weight_without_a_planned_length_stays_the_slicer_weight() {
        let without_length = FileMetadata { filament_total: None, ..metadata() };
        let last_print = LastPrint { print_duration: 3300.0, filament_used: 9_000.0, ..LastPrint::default() };

        let weight = filament_weight_grams(&without_length, Some(&last_print), None).unwrap();

        assert_eq!(weight.provenance, Provenance::Estimated);
    }

    #[test]
    fn print_on_the_expected_pace_counts_down_the_expectation() {
        assert_eq!(remaining(Some(3600.0), 0.1, 360.0), Some(3240.0));
    }

    #[test]
    fn slow_start_lengthens_the_remaining_time_gradually() {
        // Expected 3600 s, 10 % done after 480 s instead of 360 s.
        let early = remaining(Some(3600.0), 0.1, 480.0).unwrap();
        // 50 % done after 2000 s instead of 1800 s.
        let halfway = remaining(Some(3600.0), 0.5, 2000.0).unwrap();

        assert_eq!(early.round(), 3960.0);
        assert_eq!(halfway.round(), 1982.0);
        // The pure extrapolation lies at 4320 s and 2000 s; the estimate approaches it.
        assert!(early < 4320.0 && halfway < 2000.0);
    }

    #[test]
    fn remaining_time_is_continuous_in_progress() {
        let steps: Vec<f64> = (1..=100)
            .map(|percent| remaining(Some(3600.0), f64::from(percent) / 100.0, 50.0 * f64::from(percent)).unwrap())
            .collect();

        assert!(steps.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 200.0));
    }

    #[test]
    fn without_progress_the_expectation_is_shown_whole() {
        assert_eq!(remaining(Some(3600.0), 0.0, 90.0), Some(3600.0));
    }

    #[test]
    fn finished_print_has_nothing_left() {
        assert_eq!(remaining(Some(3600.0), 1.0, 3500.0), Some(0.0));
        assert_eq!(remaining(None, 1.0, 3500.0), Some(0.0));
    }

    #[test]
    fn without_expectation_the_pace_is_extrapolated_once_progress_exists() {
        assert_eq!(remaining(None, 0.5, 1800.0), Some(1800.0));
        assert_eq!(remaining(None, 0.0, 90.0), None);
    }

    #[test]
    fn layers_reported_by_klipper_beat_the_ones_derived_from_the_file() {
        let job = PrintJob { total_layer: Some(120), ..PrintJob::default() };
        let file = FileMetadata { object_height: Some(20.0), layer_height: Some(0.2), ..FileMetadata::default() };

        assert_eq!(total_layers(&job, Some(&file)), Some(Sourced::measured(120, ORIGIN_KLIPPER_REPORT)));
    }

    #[test]
    fn the_height_ahead_counts_from_the_current_layer() {
        let job = PrintJob { current_layer: Some(10), height: Some(2.2), ..PrintJob::default() };
        let file = FileMetadata { layer_height: Some(0.2), ..FileMetadata::default() };

        let height = height_at_layer(&job, Some(&file), 15).unwrap();
        assert!((height.value - 3.2).abs() < 1e-9);
        assert_eq!(height.origin, ORIGIN_HEIGHT_AHEAD);
        assert_eq!(height_at_layer(&job, Some(&file), 9), None);
        assert_eq!(height_at_layer(&job, None, 15), None);
        assert_eq!(height_at_layer(&PrintJob { height: None, ..job }, Some(&file), 15), None);
    }

    #[test]
    fn layers_are_derived_from_object_and_layer_height() {
        let file = FileMetadata { object_height: Some(20.0), layer_height: Some(0.2), ..FileMetadata::default() };

        assert_eq!(total_layers(&PrintJob::default(), Some(&file)), Some(Sourced::estimated(100, ORIGIN_LAYER_HEIGHT)));
        assert_eq!(total_layers(&PrintJob::default(), None), None);
        let flat = FileMetadata { layer_height: Some(0.0), ..file };
        assert_eq!(total_layers(&PrintJob::default(), Some(&flat)), None);
    }

    #[test]
    fn flow_follows_the_configured_filament_diameter() {
        let thin = volumetric_flow(10.0, Some(1.75));
        let thick = volumetric_flow(10.0, Some(2.85));

        assert!((thin.value - 24.05).abs() < 0.01);
        assert!((thick.value - 63.79).abs() < 0.01);
        assert_eq!((thin.provenance, thin.origin), (Provenance::Measured, ORIGIN_CONFIGURED_DIAMETER));
    }

    #[test]
    fn flow_assumes_1_75_mm_without_a_configured_diameter_and_says_so() {
        let flow = volumetric_flow(10.0, None);

        assert!((flow.value - 24.05).abs() < 0.01);
        assert_eq!((flow.provenance, flow.origin), (Provenance::Estimated, ORIGIN_ASSUMED_DIAMETER));
        assert_eq!(volumetric_flow(-1.0, None).value, 0.0);
    }
}
