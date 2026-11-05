//! Pure display derivations from the printer state; free of Slint types and therefore testable.

use crate::format;
use chrono::{DateTime, Local};
use freakymate_core::domain::estimates;
use freakymate_core::domain::file_catalog::FileInfo;
use freakymate_core::domain::job_end;
use freakymate_core::domain::pause::{Interruption, InterruptionFact};
use freakymate_core::domain::spools::PrintHint;
use freakymate_core::sources::happy_hare::{
    path_sensor_names, FlowGuardState, GateAvailability, LoadedFilament, MmuSensor, ServoPosition, SyncFeedbackState,
};
use freakymate_core::sources::klipper::tuning::TuningRange;
use freakymate_core::sources::klipper::{FanKind, PrintJob, PrintState};
use freakymate_core::sources::mainsail_config::{self, PlannedLayerPause};
use freakymate_core::sources::moonraker_server::files::FileMetadata;
use freakymate_core::sources::moonraker_server::history::{FinishedJob, LastPrint};
use freakymate_core::sources::spoolman::Spool;
use freakymate_core::{Provenance, Sourced};

const NO_VALUE: &str = "-";
const ORIGIN_PACE_ONLY: &str = "measured pace only";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTexts {
    pub name: String,
    pub elapsed: String,
    pub remaining: String,
    pub eta: String,
    pub layers: String,
    /// Height of the current layer and of the object, for example `2.40 / 24.0 mm`; empty if neither is known.
    pub heights: String,
    pub filament: String,
    /// Source of the expected print time behind the remaining time while a job runs; for the log.
    pub expectation_origin: Option<&'static str>,
}

pub fn job_texts(
    job: &PrintJob,
    metadata: Option<&FileMetadata>,
    last_print: Option<&LastPrint>,
    now: DateTime<Local>,
) -> JobTexts {
    let metadata = metadata.filter(|metadata| metadata.filename == job.filename);
    let expected = if job.state.is_active() { estimates::expected_print_seconds(metadata, last_print) } else { None };
    let remaining = job
        .state
        .is_active()
        .then(|| estimates::remaining_print_seconds(job, expected.map(|expected| expected.value)))
        .flatten();
    JobTexts {
        name: if job.filename.is_empty() { String::new() } else { format::job_name(&job.filename) },
        elapsed: if job.filename.is_empty() { NO_VALUE.to_owned() } else { format::duration(job.print_duration) },
        remaining: remaining.map_or_else(|| NO_VALUE.to_owned(), format::duration),
        eta: remaining.map_or_else(|| NO_VALUE.to_owned(), |seconds| format::finish_time(now, seconds)),
        layers: layers(job, metadata),
        heights: heights(job, metadata),
        filament: filament(job, metadata),
        expectation_origin: job
            .state
            .is_active()
            .then(|| expected.map_or(ORIGIN_PACE_ONLY, |expected| expected.origin)),
    }
}

fn layers(job: &PrintJob, metadata: Option<&FileMetadata>) -> String {
    // Layers derived from the object height are a prediction and carry a `~`.
    let total = estimates::total_layers(job, metadata).map(|total| show(total, |layers| layers.to_string()));
    match (job.current_layer, total) {
        (Some(current), Some(total)) => format!("{current} / {total}"),
        (Some(current), None) => current.to_string(),
        (None, Some(total)) if job.state == PrintState::Standby => total,
        _ => NO_VALUE.to_owned(),
    }
}

/// The current height shows each layer step (0.12 or 0.28 mm); for the object height one decimal is enough.
fn heights(job: &PrintJob, metadata: Option<&FileMetadata>) -> String {
    let object = metadata.and_then(|metadata| metadata.object_height).filter(|height| *height > 0.0);
    match (job.height, object) {
        (Some(current), Some(object)) => format!("{current:.2} / {object:.1} mm"),
        (Some(current), None) => format!("{current:.2} mm"),
        (None, Some(object)) => format!("{object:.1} mm"),
        (None, None) => String::new(),
    }
}

/// The height a layer ahead will have, for example `~10.00 mm`; empty if it cannot be told.
pub fn height_at_layer(job: &PrintJob, metadata: Option<&FileMetadata>, layer: u32) -> String {
    let metadata = metadata.filter(|metadata| metadata.filename == job.filename);
    estimates::height_at_layer(job, metadata, layer)
        .map(|height| show(height, |mm| format!("{mm:.2} mm")))
        .unwrap_or_default()
}

/// The notice after planning a pause at `layer`, named as the dialog's button names it.
pub fn layer_pause_notice(layer: u32, current_layer: Option<u32>) -> String {
    if current_layer.is_some_and(|current| mainsail_config::is_next_layer(layer, current)) {
        "Pause at the next layer sent".to_owned()
    } else {
        format!("Pause at layer {layer} sent")
    }
}

/// The planned pause in a word for the Layer tile: `next` or the layer; empty if nothing is planned or not known.
pub fn planned_layer_pause(plan: Option<PlannedLayerPause>) -> String {
    match plan {
        Some(PlannedLayerPause { next_layer: true, .. }) => "next".to_owned(),
        Some(PlannedLayerPause { at_layer: Some(layer), .. }) => layer.to_string(),
        _ => String::new(),
    }
}

fn filament(job: &PrintJob, metadata: Option<&FileMetadata>) -> String {
    let total = metadata.and_then(|metadata| metadata.filament_total);
    match (job.state, total) {
        (PrintState::Standby, Some(total)) => format::filament_length(total),
        (_, Some(total)) if job.filament_used > 0.0 => {
            format!("{} / {}", format::filament_meters(job.filament_used), format::filament_length(total))
        }
        _ if job.filament_used > 0.0 => format::filament_length(job.filament_used),
        _ => NO_VALUE.to_owned(),
    }
}

/// When the shown job ended, from the job history; `-` while it runs or if the history does not know.
pub fn ended_text(job: &PrintJob, newest: Option<&FinishedJob>, now: DateTime<Local>) -> String {
    newest
        .and_then(|newest| job_end::ended_at(job, newest))
        .map_or_else(|| NO_VALUE.to_owned(), |end| format::unix_moment(now, end))
}

/// What the sources state about a pause or an error, one fact per line, and where the print stands (layer and
/// height); empty if nothing is known.
pub fn interruption_text(interruption: &Interruption, job: &PrintJob) -> String {
    interruption
        .facts
        .iter()
        .map(|fact| match fact {
            InterruptionFact::KlipperMessage(text)
            | InterruptionFact::MmuReason(text)
            | InterruptionFact::MmuHint(text) => text.clone(),
            InterruptionFact::SensorEmpty { label } => format!("{label}: no filament"),
        })
        .chain(stands_at(job))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `Layer 50 · 10.00 mm`, or the part that is known.
fn stands_at(job: &PrintJob) -> Option<String> {
    let layer = job.current_layer.map(|layer| format!("Layer {layer}"));
    let height = job.height.map(|height| format!("{height:.2} mm"));
    let parts: Vec<String> = layer.into_iter().chain(height).collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What is left of a spool, for example `613 g`; empty without a weight.
pub fn spool_remaining(spool: &Spool) -> String {
    spool.remaining_weight.map(format::filament_weight).unwrap_or_default()
}

/// The spool as a preset on the temperature page, for example `ABS+  240/120`.
pub fn spool_preset_text(spool: &Spool) -> Option<String> {
    let nozzle = spool.extruder_temp?;
    let bed = spool.bed_temp.map_or_else(String::new, |bed| format!("/{bed:.0}"));
    Some(format!("{}  {nozzle:.0}{bed}", spool.material))
}

/// A hint before the print as a line of text. `show_tool` names the tool when there are several lanes.
pub fn print_hint_text(hint: &PrintHint, show_tool: bool) -> String {
    let tool = |tool: usize| if show_tool { format!("T{tool}: ") } else { String::new() };
    match hint {
        PrintHint::Material { tool: number, expected, loaded } => {
            format!("{}the file is for {expected}, the spool is {loaded}", tool(*number))
        }
        PrintHint::Weight { tool: number, needed, left } => {
            format!(
                "{}needs ~{}, the spool has {} left",
                tool(*number),
                format::filament_weight(*needed),
                format::filament_weight(*left)
            )
        }
        PrintHint::Color { tool: number } => format!("{}the colour differs from the file", tool(*number)),
    }
}

/// What a row of the file list shows; an empty text is a fact the file does not have.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileFacts {
    pub modified: String,
    /// The slicer's filament type; files for several tools list one per tool (`PLA;PETG`).
    pub material: String,
    pub weight: String,
    pub nozzle: String,
    pub time: String,
    pub extruder_temperature: String,
    pub bed_temperature: String,
    pub chamber_temperature: String,
}

/// The facts of a file for its row. Values of the last completed print win over what the slicer predicted;
/// predictions are marked with `~`. The temperatures are the slicer's settings, not predictions.
pub fn file_facts(info: &FileInfo) -> FileFacts {
    let mut facts = FileFacts { modified: format::unix_date(info.entry.modified), ..FileFacts::default() };
    if let Some(metadata) = info.metadata {
        facts.material = metadata.filament_type.clone().unwrap_or_default();
        facts.weight = estimates::filament_weight_grams(metadata, info.last_print, info.spool)
            .map_or_else(String::new, |grams| show(grams, format::filament_weight));
        facts.nozzle =
            metadata.nozzle_diameter.filter(|diameter| *diameter > 0.0).map_or_else(String::new, |d| d.to_string());
        facts.time = file_print_time(metadata, info.last_print).unwrap_or_default();
        facts.extruder_temperature = degrees(metadata.first_layer_extr_temp);
        facts.bed_temperature = degrees(metadata.first_layer_bed_temp);
        facts.chamber_temperature = degrees(metadata.chamber_temp);
    }
    facts
}

/// A temperature of 0 means that the slicer sets none.
fn degrees(temperature: Option<f64>) -> String {
    temperature.filter(|degrees| *degrees > 0.0).map_or_else(String::new, |degrees| format!("{degrees:.0}°"))
}

fn file_print_time(metadata: &FileMetadata, last_print: Option<&LastPrint>) -> Option<String> {
    estimates::expected_print_seconds(Some(metadata), last_print).map(|seconds| show(seconds, format::duration))
}

/// A prediction carries a `~` so that it is not mistaken for a measurement.
fn show<T>(value: Sourced<T>, format: impl FnOnce(T) -> String) -> String {
    let text = format(value.value);
    match value.provenance {
        Provenance::Measured => text,
        Provenance::Estimated => format!("~{text}"),
    }
}

/// Names of the MMU sensors as the operator reads them; unknown ones lose the `mmu_` prefix and get spaces.
pub fn mmu_sensor_label(name: &str) -> String {
    let known = match name {
        "mmu_entry" => "Entry",
        "mmu_exit" => "Exit",
        "mmu_shared_exit" => "Shared exit",
        "encoder" => "Encoder",
        "extruder" => "Extruder",
        "toolhead" => "Toolhead",
        "filament_compression" => "Compression",
        "filament_tension" => "Tension",
        "filament_proportional" => "Proportional",
        "mmu_gear_touch" => "Gear touch",
        "mmu_ext_touch" => "Extruder touch",
        "mmu_sel_touch" => "Selector touch",
        "mmu_sel_home" => "Selector home",
        "mmu_nfc" => "NFC",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    let words = name.strip_prefix("mmu_").unwrap_or(name).replace('_', " ");
    let mut characters = words.chars();
    characters.next().map_or_else(String::new, |first| first.to_uppercase().chain(characters).collect())
}

/// The sensors of the MMU for the chip list. The three buffer sensors are left out when a buffer gauge shows them, and
/// those that the filament path draws.
pub fn mmu_chip_sensors(sensors: &[MmuSensor], has_buffer: bool) -> Vec<&MmuSensor> {
    const BUFFER_SENSORS: [&str; 3] = ["filament_compression", "filament_tension", "filament_proportional"];
    let on_path = path_sensor_names(sensors);
    sensors
        .iter()
        .filter(|sensor| !(has_buffer && BUFFER_SENSORS.contains(&sensor.name.as_str())))
        .filter(|sensor| !on_path.contains(&sensor.name.as_str()))
        .collect()
}

/// The tools of a gate as the tile shows them, e.g. `T0 T3`; empty without one.
pub fn mmu_tools(tools: &[usize]) -> String {
    tools.iter().map(|tool| format!("T{tool}")).collect::<Vec<_>>().join(" ")
}

/// The distance on the filament path, e.g. `412.6 mm`.
pub fn path_distance(millimetres: f64) -> String {
    format!("{millimetres:.1} mm")
}

/// `triggered` means filament is present at the sensor; `None` is a disabled sensor.
pub fn mmu_sensor_state(triggered: Option<bool>) -> &'static str {
    match triggered {
        None => "disabled",
        Some(false) => "open",
        Some(true) => "triggered",
    }
}

pub fn mmu_availability(availability: GateAvailability) -> &'static str {
    match availability {
        GateAvailability::Unknown => "unknown",
        GateAvailability::Empty => "empty",
        GateAvailability::Available => "available",
        GateAvailability::Buffered => "buffered",
    }
}

pub fn mmu_filament(filament: LoadedFilament) -> &'static str {
    match filament {
        LoadedFilament::Loaded => "loaded",
        LoadedFilament::Unloaded => "unloaded",
        LoadedFilament::Unknown => "unknown",
    }
}

/// Empty when the selector has no servo.
pub fn mmu_servo(servo: Option<ServoPosition>) -> &'static str {
    match servo {
        None => "",
        Some(ServoPosition::Up) => "up",
        Some(ServoPosition::Down) => "down",
        Some(ServoPosition::Move) => "move",
        Some(ServoPosition::Unknown) => "unknown",
    }
}

pub fn mmu_buffer_state(state: SyncFeedbackState) -> &'static str {
    match state {
        SyncFeedbackState::Compressed => "compressed",
        SyncFeedbackState::Neutral => "neutral",
        SyncFeedbackState::Tension => "tension",
        SyncFeedbackState::Inactive => "inactive",
        SyncFeedbackState::Disabled => "disabled",
    }
}

pub fn mmu_flowguard_state(state: FlowGuardState) -> &'static str {
    match state {
        FlowGuardState::Off => "off",
        FlowGuardState::Triggered => "triggered",
        FlowGuardState::Boosting => "boosting",
        FlowGuardState::Monitoring => "monitoring",
        FlowGuardState::Ready => "ready",
    }
}

pub fn fan_kind(kind: FanKind) -> &'static str {
    match kind {
        FanKind::Part => "part",
        FanKind::Generic => "generic",
        FanKind::Heater => "heater",
        FanKind::Controller => "controller",
    }
}

/// Measured fan speed, empty without a tachometer.
pub fn fan_rpm(rpm: Option<f64>) -> String {
    rpm.map(|rpm| format!("{rpm:.0} rpm")).unwrap_or_default()
}

/// A value of the expert tab with its unit, e.g. `5000 mm/s²` or `0.040`.
pub fn tuning_value(value: f64, range: &TuningRange) -> String {
    format!("{value:.*}{}", range.decimals, range.unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use freakymate_core::sources::moonraker_server::files::FileEntry;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 21, 12, 0, 0).unwrap()
    }

    fn metadata(filename: &str) -> FileMetadata {
        FileMetadata {
            filename: filename.to_owned(),
            estimated_time: Some(3600.0),
            filament_total: Some(5000.0),
            layer_height: Some(0.2),
            object_height: Some(20.0),
            ..FileMetadata::default()
        }
    }

    #[test]
    fn print_on_the_expected_pace_counts_down_the_slicer_estimate() {
        let job = PrintJob {
            state: PrintState::Printing,
            filename: "dir/cube.gcode".into(),
            progress: 0.1,
            print_duration: 360.0,
            filament_used: 1000.0,
            current_layer: Some(3),
            ..PrintJob::default()
        };

        let texts = job_texts(&job, Some(&metadata("dir/cube.gcode")), None, now());

        assert_eq!(texts.name, "cube");
        assert_eq!(texts.elapsed, "6m 00s");
        assert_eq!(texts.remaining, "54m 00s");
        assert_eq!(texts.eta, "12:54");
        assert_eq!(texts.layers, "3 / ~100");
        assert_eq!(texts.filament, "1.00 / 5.00 m");
    }

    #[test]
    fn last_measured_print_is_the_expectation_before_the_slicer_estimate() {
        let job = PrintJob {
            state: PrintState::Printing,
            filename: "a.gcode".into(),
            progress: 0.1,
            print_duration: 420.0,
            ..PrintJob::default()
        };
        let last_print = LastPrint { print_duration: 4200.0, filament_used: 4800.0, ..LastPrint::default() };

        let texts = job_texts(&job, Some(&metadata("a.gcode")), Some(&last_print), now());

        assert_eq!(texts.remaining, "1h 03m");
        assert_eq!(texts.expectation_origin, Some(estimates::ORIGIN_LAST_PRINT));
    }

    #[test]
    fn without_expectation_the_pace_is_extrapolated_once_progress_exists() {
        let job = PrintJob {
            state: PrintState::Printing,
            filename: "a.gcode".into(),
            progress: 0.5,
            print_duration: 1800.0,
            ..PrintJob::default()
        };

        let texts = job_texts(&job, None, None, now());

        assert_eq!(texts.remaining, "30m 00s");
        assert_eq!(texts.layers, "-");
        assert_eq!(texts.expectation_origin, Some(ORIGIN_PACE_ONLY));
    }

    #[test]
    fn standby_shows_file_totals_and_no_eta() {
        let job = PrintJob { filename: "a.gcode".into(), ..PrintJob::default() };

        let texts = job_texts(&job, Some(&metadata("a.gcode")), None, now());

        assert_eq!(texts.remaining, "-");
        assert_eq!(texts.eta, "-");
        assert_eq!(texts.expectation_origin, None);
        assert_eq!(texts.layers, "~100");
        assert_eq!(texts.filament, "5.00 m");
    }

    #[test]
    fn layers_klipper_reports_carry_no_tilde() {
        let job = PrintJob {
            state: PrintState::Printing,
            filename: "a.gcode".into(),
            current_layer: Some(3),
            total_layer: Some(120),
            ..PrintJob::default()
        };

        assert_eq!(job_texts(&job, Some(&metadata("a.gcode")), None, now()).layers, "3 / 120");
    }

    #[test]
    fn heights_show_the_current_layer_finer_than_the_object() {
        let file = metadata("a.gcode");
        let printing = PrintJob {
            state: PrintState::Printing,
            filename: "a.gcode".into(),
            current_layer: Some(13),
            height: Some(2.6),
            ..PrintJob::default()
        };
        let ended = PrintJob { state: PrintState::Complete, height: None, ..printing.clone() };

        assert_eq!(job_texts(&printing, Some(&file), None, now()).heights, "2.60 / 20.0 mm");
        assert_eq!(job_texts(&printing, None, None, now()).heights, "2.60 mm");
        assert_eq!(job_texts(&ended, Some(&file), None, now()).heights, "20.0 mm");
        assert_eq!(job_texts(&PrintJob::default(), None, None, now()).heights, "");
        assert_eq!(height_at_layer(&printing, Some(&file), 20), "~4.00 mm");
    }

    #[test]
    fn the_notice_names_the_next_layer_as_the_dialog_does() {
        assert_eq!(layer_pause_notice(13, Some(12)), "Pause at the next layer sent");
        assert_eq!(layer_pause_notice(40, Some(12)), "Pause at layer 40 sent");
        assert_eq!(layer_pause_notice(13, None), "Pause at layer 13 sent");
    }

    #[test]
    fn the_planned_pause_shows_in_a_word() {
        assert_eq!(planned_layer_pause(None), "");
        assert_eq!(planned_layer_pause(Some(PlannedLayerPause::default())), "");
        assert_eq!(planned_layer_pause(Some(PlannedLayerPause { next_layer: false, at_layer: Some(49) })), "49");
        assert_eq!(planned_layer_pause(Some(PlannedLayerPause { next_layer: true, at_layer: Some(49) })), "next");
    }

    #[test]
    fn metadata_of_other_file_is_ignored() {
        let job = PrintJob { filename: "a.gcode".into(), ..PrintJob::default() };

        assert_eq!(job_texts(&job, Some(&metadata("b.gcode")), None, now()).filament, "-");
    }

    #[test]
    fn the_end_of_a_finished_job_comes_from_the_history() {
        let job = PrintJob { state: PrintState::Complete, filename: "a.gcode".into(), ..PrintJob::default() };
        let end = Local.with_ymd_and_hms(2026, 10, 21, 11, 32, 0).unwrap().timestamp() as f64;
        let newest = FinishedJob { filename: "a.gcode".into(), status: "completed".into(), end_time: end };

        assert_eq!(ended_text(&job, Some(&newest), now()), "11:32");
        assert_eq!(ended_text(&job, None, now()), "-");
    }

    #[test]
    fn every_stated_fact_of_an_interruption_is_a_line() {
        let interruption = Interruption {
            failed: false,
            facts: vec![
                InterruptionFact::KlipperMessage("Filament runout".into()),
                InterruptionFact::SensorEmpty { label: "Toolhead Runout".into() },
            ],
        };

        assert_eq!(
            interruption_text(&interruption, &PrintJob::default()),
            "Filament runout\nToolhead Runout: no filament"
        );
        let job = PrintJob { current_layer: Some(50), height: Some(10.0), ..PrintJob::default() };
        assert_eq!(interruption_text(&Interruption { failed: false, facts: Vec::new() }, &job), "Layer 50 · 10.00 mm");
    }

    fn file_metadata() -> FileMetadata {
        FileMetadata {
            filename: "a.gcode".into(),
            estimated_time: Some(3600.0),
            filament_total: Some(10_000.0),
            filament_weight_total: Some(30.0),
            filament_type: Some("PLA".into()),
            nozzle_diameter: Some(0.4),
            ..FileMetadata::default()
        }
    }

    fn facts_of(metadata: Option<&FileMetadata>, last_print: Option<&LastPrint>) -> FileFacts {
        let entry = FileEntry { path: "a.gcode".into(), modified: 1_700_000_000.0, size: 100 };
        file_facts(&FileInfo { entry: &entry, metadata, last_print, spool: None })
    }

    fn slicer_settings() -> FileMetadata {
        FileMetadata {
            first_layer_extr_temp: Some(215.0),
            first_layer_bed_temp: Some(60.0),
            chamber_temp: Some(45.0),
            ..file_metadata()
        }
    }

    #[test]
    fn unprinted_file_shows_slicer_values_marked_as_estimates() {
        let facts = facts_of(Some(&slicer_settings()), None);

        assert_eq!(
            facts,
            FileFacts {
                modified: format::unix_date(1_700_000_000.0),
                material: "PLA".into(),
                weight: "~30 g".into(),
                nozzle: "0.4".into(),
                time: "~1h 00m".into(),
                extruder_temperature: "215°".into(),
                bed_temperature: "60°".into(),
                chamber_temperature: "45°".into(),
            }
        );
    }

    #[test]
    fn printed_file_shows_measured_time_and_scaled_weight() {
        let last_print = LastPrint { print_duration: 3300.0, filament_used: 9_000.0, ..LastPrint::default() };

        let facts = facts_of(Some(&file_metadata()), Some(&last_print));

        assert_eq!((facts.material.as_str(), facts.weight.as_str(), facts.time.as_str()), ("PLA", "27 g", "55m 00s"));
    }

    #[test]
    fn file_without_metadata_shows_only_its_date() {
        let facts = facts_of(None, None);

        assert_eq!(facts, FileFacts { modified: format::unix_date(1_700_000_000.0), ..FileFacts::default() });
    }

    #[test]
    fn missing_slicer_fields_and_zero_temperatures_are_left_out() {
        let metadata = FileMetadata {
            filename: "a.gcode".into(),
            nozzle_diameter: Some(0.6),
            chamber_temp: Some(0.0),
            first_layer_bed_temp: Some(0.0),
            ..FileMetadata::default()
        };

        let facts = facts_of(Some(&metadata), None);

        assert_eq!(facts.nozzle, "0.6");
        assert_eq!((facts.material.as_str(), facts.weight.as_str(), facts.time.as_str()), ("", "", ""));
        assert_eq!(
            (facts.extruder_temperature.as_str(), facts.bed_temperature.as_str(), facts.chamber_temperature.as_str()),
            ("", "", "")
        );
    }

    #[test]
    fn mmu_sensors_get_readable_names() {
        assert_eq!(mmu_sensor_label("mmu_shared_exit"), "Shared exit");
        assert_eq!(mmu_sensor_label("filament_tension"), "Tension");
        assert_eq!(mmu_sensor_label("mmu_future_probe"), "Future probe");
        assert_eq!(mmu_sensor_label("hotend"), "Hotend");
        assert_eq!(mmu_sensor_label(""), "");
    }

    #[test]
    fn buffer_sensors_are_hidden_only_when_the_gauge_shows_them() {
        let sensors = ["mmu_entry", "mmu_exit", "filament_tension", "filament_proportional", "toolhead"]
            .map(|name| MmuSensor { name: name.into(), triggered: Some(false) });
        let names = |has_buffer| -> Vec<String> {
            mmu_chip_sensors(&sensors, has_buffer).iter().map(|sensor| sensor.name.clone()).collect()
        };

        assert!(names(true).is_empty(), "entry, exit and toolhead are on the filament path, the buffer has its gauge");
        assert_eq!(names(false), ["filament_tension", "filament_proportional"]);
        assert_eq!(path_distance(412.63), "412.6 mm");
    }

    #[test]
    fn mmu_states_become_words_for_the_ui() {
        assert_eq!([None, Some(false), Some(true)].map(mmu_sensor_state), ["disabled", "open", "triggered"]);
        assert_eq!(
            [
                FlowGuardState::Off,
                FlowGuardState::Triggered,
                FlowGuardState::Boosting,
                FlowGuardState::Monitoring,
                FlowGuardState::Ready
            ]
            .map(mmu_flowguard_state),
            ["off", "triggered", "boosting", "monitoring", "ready"]
        );
        assert_eq!(
            [None, Some(ServoPosition::Up), Some(ServoPosition::Down), Some(ServoPosition::Unknown)].map(mmu_servo),
            ["", "up", "down", "unknown"]
        );
    }

    #[test]
    fn fan_and_tuning_texts() {
        assert_eq!(fan_rpm(Some(2799.6)), "2800 rpm");
        assert_eq!(fan_rpm(None), "");
        assert_eq!(fan_kind(FanKind::Generic), "generic");
        let accel = freakymate_core::sources::klipper::tuning::TuningKey::MaxAccel.range();
        assert_eq!(tuning_value(5000.0, &accel), "5000 mm/s²");
        let advance = freakymate_core::sources::klipper::tuning::TuningKey::PressureAdvance.range();
        assert_eq!(tuning_value(0.04, &advance), "0.040");
    }
}
