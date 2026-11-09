//! Pure display derivations from the printer state; free of Slint types and therefore testable.

use crate::format;
use chrono::{DateTime, Local};
use freakymate_core::domain::estimates;
use freakymate_core::domain::filament_match::{self, Difference, PrintHint, PrintHintKind, Remedy, ToolMatch, Verdict};
use freakymate_core::domain::file_catalog::FileInfo;
use freakymate_core::domain::job_end;
use freakymate_core::domain::pause::{Interruption, InterruptionFact};
use freakymate_core::sources::happy_hare::{
    FlowGuardState, GateAvailability, LoadedFilament, ServoPosition, SyncFeedbackState,
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
    interruption.facts.iter().map(fact_text).chain(stands_at(job)).collect::<Vec<_>>().join("\n")
}

/// The first fact of a pause or an error for the status bar, without where the print stands; empty if none is stated.
pub fn interruption_headline(interruption: &Interruption) -> String {
    interruption.facts.first().map(fact_text).unwrap_or_default()
}

fn fact_text(fact: &InterruptionFact) -> String {
    match fact {
        InterruptionFact::KlipperMessage(text)
        | InterruptionFact::MmuReason(text)
        | InterruptionFact::MmuHint(text) => text.clone(),
        InterruptionFact::SensorEmpty { label } => format!("{label}: no filament"),
    }
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
    let tool = if show_tool { format!("T{}: ", hint.tool) } else { String::new() };
    let spool = "The spool";
    match &hint.kind {
        PrintHintKind::Differs(difference) => format!("{tool}{}", difference_sentence(spool, spool, difference)),
        PrintHintKind::GateEmpty => format!("{tool}{}", empty_gate_sentence("The gate")),
        PrintHintKind::NoGate => no_gate_sentence(hint.tool),
    }
}

/// One difference between the file and the filament as a sentence, for the print page and the question before the
/// start alike. `source` names where the tool prints from, `spool` the spool for the weight.
fn difference_sentence(source: &str, spool: &str, difference: &Difference) -> String {
    match difference {
        Difference::Material { loaded, .. } => format!("{source} holds {loaded}."),
        Difference::Color => format!("{source} has another colour."),
        Difference::Weight { needed, left } => format!(
            "{spool} has {} left, the print needs ~{}.",
            format::filament_weight(*left),
            format::filament_weight(*needed)
        ),
    }
}

fn empty_gate_sentence(source: &str) -> String {
    format!("{source} is empty.")
}

fn no_gate_sentence(tool: usize) -> String {
    format!("The printer has no T{tool}.")
}

/// The key of a verdict for the UI (`ToolMatchItem.verdict`): "fits", "differs", "gate-empty", "no-gate" or "".
pub fn verdict_key(verdict: &Verdict) -> &'static str {
    match verdict {
        Verdict::Fits => "fits",
        Verdict::Differs(_) => "differs",
        Verdict::GateEmpty => "gate-empty",
        Verdict::NoGate => "no-gate",
        Verdict::Unknown => "",
    }
}

/// Several verdicts as one key, see [`Verdict::combine`]: "fits", "" or "differs" for any that does not fit.
pub fn combined_verdict<'a>(verdicts: impl IntoIterator<Item = &'a Verdict>) -> &'static str {
    match Verdict::combine(verdicts) {
        Verdict::Fits => "fits",
        Verdict::Unknown => "",
        Verdict::Differs(_) | Verdict::GateEmpty | Verdict::NoGate => "differs",
    }
}

/// How many tools of the file fit, e.g. "3 of 4 tools fit".
pub fn match_summary(matches: &[ToolMatch]) -> String {
    let fitting = matches.iter().filter(|found| found.verdict == Verdict::Fits).count();
    format!("{fitting} of {} tools fit", matches.len())
}

/// What does not fit for a tool, one sentence per line, naming the gate it prints from.
pub fn tool_differences(found: &ToolMatch) -> String {
    // Without a gate (no MMU) the tool prints from the spool in use.
    let source = found.gate.map_or_else(|| "The spool".to_owned(), |gate| format!("Gate {gate}"));
    let spool = found.gate.map_or_else(|| "The spool".to_owned(), |gate| format!("The spool in gate {gate}"));
    match &found.verdict {
        Verdict::Fits | Verdict::Unknown => String::new(),
        Verdict::GateEmpty => empty_gate_sentence(&source),
        Verdict::NoGate => no_gate_sentence(found.request.tool),
        Verdict::Differs(differences) => differences
            .iter()
            .map(|difference| difference_sentence(&source, &spool, difference))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The material the file asks of the tool and the one where it prints from, when they differ.
pub fn material_difference(found: &ToolMatch) -> Option<(&str, &str)> {
    let Verdict::Differs(differences) = &found.verdict else { return None };
    differences.iter().find_map(|difference| match difference {
        Difference::Material { expected, loaded } => Some((expected.as_str(), loaded.as_str())),
        _ => None,
    })
}

/// The material of a tool in a short line: `PLA → ASA` when the filament differs, else what the file asks for.
pub fn material_line(found: &ToolMatch) -> String {
    match material_difference(found) {
        Some((expected, loaded)) => format!("{expected} → {loaded}"),
        None => found.request.material.clone().unwrap_or_default(),
    }
}

/// What can make a tool fit, one sentence per remedy and line.
pub fn remedy_text(remedies: &[Remedy]) -> String {
    remedies.iter().map(remedy_sentence).collect::<Vec<_>>().join("\n")
}

fn remedy_sentence(remedy: &Remedy) -> String {
    match remedy {
        Remedy::UseGate(gates) if gates.len() == 1 => {
            format!("Gate {} holds what the file wants: choose it below.", gates[0])
        }
        Remedy::UseGate(gates) => format!("Gates {} hold what the file wants: choose one of them.", enumeration(gates)),
        Remedy::LoadGate(gate) => format!(
            "Load fitting filament into gate {gate}. If it holds it already, correct its spool, material or colour."
        ),
        Remedy::LoadSpool => "Load fitting filament, or choose the spool that holds it.".to_owned(),
        Remedy::Reslice => "Slice the file for the tools of this printer.".to_owned(),
    }
}

/// Numbers as an enumeration: `0, 2 and 5`.
fn enumeration(numbers: &[usize]) -> String {
    let texts: Vec<String> = numbers.iter().map(ToString::to_string).collect();
    match texts.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => texts.concat(),
    }
}

/// The gate whose filament a remedy changes, for a button that opens it.
pub fn gate_to_load(remedies: &[Remedy]) -> Option<usize> {
    remedies.iter().find_map(|remedy| match remedy {
        Remedy::LoadGate(gate) => Some(*gate),
        _ => None,
    })
}

/// What a row of the file list shows; an empty text is a fact the file does not have.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileFacts {
    pub modified: String,
    /// The slicer's materials of the tools the print uses, each once (`PLA, PETG`).
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
        facts.material = file_materials(metadata);
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

/// Each material of the used tools once, in tool order.
fn file_materials(metadata: &FileMetadata) -> String {
    let mut materials: Vec<String> = Vec::new();
    for material in filament_match::tool_requests(metadata).into_iter().filter_map(|request| request.material) {
        if !materials.contains(&material) {
            materials.push(material);
        }
    }
    materials.join(", ")
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

/// The tools of a gate as the tile shows them, e.g. `T0 T3`; empty without one.
pub fn gate_tools_text(tools: &[usize]) -> String {
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

    #[test]
    fn the_headline_of_an_interruption_is_its_first_fact_without_the_layer() {
        let interruption = Interruption {
            failed: false,
            facts: vec![
                InterruptionFact::KlipperMessage("Filament runout".into()),
                InterruptionFact::SensorEmpty { label: "Toolhead Runout".into() },
            ],
        };

        assert_eq!(interruption_headline(&interruption), "Filament runout");
        assert_eq!(interruption_headline(&Interruption { failed: false, facts: Vec::new() }), "");
    }

    fn file_metadata() -> FileMetadata {
        FileMetadata {
            filename: "a.gcode".into(),
            estimated_time: Some(3600.0),
            filament_total: Some(10_000.0),
            filament_weight_total: Some(30.0),
            filament_types: vec!["PLA".into()],
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
    fn a_multicolour_file_names_each_material_of_its_used_tools_once() {
        let metadata = FileMetadata {
            filament_types: vec!["PLA".into(), "PLA".into(), "TPU".into(), "PETG".into()],
            filament_weights: vec![5.0, 5.0, 0.0, 2.0],
            ..file_metadata()
        };

        // Tool 2 (TPU) has no weight: the print does not use it.
        assert_eq!(facts_of(Some(&metadata), None).material, "PLA, PETG");
    }

    fn tool_match(tool: usize, gate: Option<usize>, verdict: Verdict) -> ToolMatch {
        let request = filament_match::ToolRequest { tool, ..filament_match::ToolRequest::default() };
        ToolMatch { request, gate, verdict }
    }

    #[test]
    fn tools_that_do_not_fit_win_over_those_without_facts() {
        let differs = Verdict::Differs(vec![Difference::Color]);

        assert_eq!(combined_verdict([&Verdict::Fits, &Verdict::Fits]), "fits");
        assert_eq!(combined_verdict([&Verdict::Fits, &Verdict::Unknown]), "");
        assert_eq!(combined_verdict([&Verdict::Unknown, &differs]), "differs");
        assert_eq!(combined_verdict([&Verdict::NoGate]), "differs");
        assert_eq!(combined_verdict([]), "");
    }

    #[test]
    fn the_summary_counts_the_fitting_tools() {
        let matches = [tool_match(0, Some(0), Verdict::Fits), tool_match(1, Some(1), Verdict::GateEmpty)];

        assert_eq!(match_summary(&matches), "1 of 2 tools fit");
    }

    #[test]
    fn each_difference_of_a_tool_is_a_sentence_about_its_gate() {
        let differs = Verdict::Differs(vec![
            Difference::Material { expected: "PLA".into(), loaded: "PETG".into() },
            Difference::Color,
            Difference::Weight { needed: 30.0, left: 20.0 },
        ]);

        assert_eq!(
            tool_differences(&tool_match(3, Some(1), differs)),
            "Gate 1 holds PETG.\nGate 1 has another colour.\nThe spool in gate 1 has 20 g left, the print needs ~30 g."
        );
        assert_eq!(tool_differences(&tool_match(1, Some(3), Verdict::GateEmpty)), "Gate 3 is empty.");
        assert_eq!(tool_differences(&tool_match(4, None, Verdict::NoGate)), "The printer has no T4.");
        assert_eq!(tool_differences(&tool_match(0, Some(0), Verdict::Fits)), "");
        let without_mmu = Verdict::Differs(vec![
            Difference::Material { expected: "PLA".into(), loaded: "ABS".into() },
            Difference::Weight { needed: 52.0, left: 38.0 },
        ]);
        assert_eq!(
            tool_differences(&tool_match(0, None, without_mmu)),
            "The spool holds ABS.\nThe spool has 38 g left, the print needs ~52 g."
        );
    }

    #[test]
    fn the_hints_before_the_print_word_differences_like_the_print_page() {
        let hint = |tool, difference| PrintHint { tool, kind: PrintHintKind::Differs(difference) };
        let material = hint(1, Difference::Material { expected: "PLA".into(), loaded: "PETG".into() });
        assert_eq!(print_hint_text(&material, true), "T1: The spool holds PETG.");
        assert_eq!(print_hint_text(&hint(0, Difference::Color), false), "The spool has another colour.");
        assert_eq!(
            print_hint_text(&hint(0, Difference::Weight { needed: 52.0, left: 38.0 }), false),
            "The spool has 38 g left, the print needs ~52 g."
        );
    }

    #[test]
    fn missing_gates_and_empty_gates_are_hints_before_the_print() {
        assert_eq!(
            print_hint_text(&PrintHint { tool: 1, kind: PrintHintKind::GateEmpty }, true),
            "T1: The gate is empty."
        );
        assert_eq!(
            print_hint_text(&PrintHint { tool: 3, kind: PrintHintKind::NoGate }, false),
            "The printer has no T3."
        );
    }

    #[test]
    fn the_material_line_names_both_materials_when_they_differ() {
        let differs = Verdict::Differs(vec![Difference::Material { expected: "PLA".into(), loaded: "ASA".into() }]);
        let mut found = tool_match(0, Some(0), differs);
        found.request.material = Some("PLA".into());

        assert_eq!(material_line(&found), "PLA → ASA");
        found.verdict = Verdict::Differs(vec![Difference::Color]);
        assert_eq!(material_line(&found), "PLA");
        assert_eq!(material_difference(&found), None);
    }

    #[test]
    fn remedies_read_as_sentences_and_name_the_gate_to_open() {
        let remedies = [Remedy::UseGate(vec![0, 2, 5]), Remedy::LoadGate(3)];

        assert_eq!(
            remedy_text(&remedies),
            "Gates 0, 2 and 5 hold what the file wants: choose one of them.\nLoad fitting filament into gate 3. If it \
             holds it already, correct its spool, material or colour."
        );
        assert_eq!(remedy_text(&[Remedy::UseGate(vec![1])]), "Gate 1 holds what the file wants: choose it below.");
        assert_eq!(gate_to_load(&remedies), Some(3));
        assert_eq!(gate_to_load(&[Remedy::Reslice]), None);
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
    fn the_path_distance_has_one_decimal() {
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
