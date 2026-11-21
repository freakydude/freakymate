//! Pure display derivations from the printer state; free of Slint types and therefore testable.

use crate::{Severity, format};
use chrono::{DateTime, Local};
use sailhand_core::domain::estimates;
use sailhand_core::domain::filament_match::{self, Difference, ToolMatch, Verdict};
use sailhand_core::domain::file_catalog::FileInfo;
use sailhand_core::domain::job_end::{self, JobLoss};
use sailhand_core::domain::pause::{Interruption, InterruptionFact};
use sailhand_core::domain::print_hints::PrintHint;
use sailhand_core::sources::happy_hare::{
    BufferSides, Emphasis, EncoderHeadroom, FlowGuardState, FlowGuardStatement, GateAvailability, LoadedFilament,
    ServoPosition, SyncFeedbackState,
};
use sailhand_core::sources::klipper::tuning::TuningRange;
use sailhand_core::sources::klipper::{FanKind, PrintJob, PrintState};
use sailhand_core::sources::mainsail_config::{self, PlannedLayerPause};
use sailhand_core::sources::moonraker_server::files::FileMetadata;
use sailhand_core::sources::moonraker_server::history::{FinishedJob, LastPrint};
use sailhand_core::sources::spoolman::Spool;
use sailhand_core::{Provenance, Sourced};

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
    /// Time from the start to the end, with heating and pauses.
    pub total: String,
    pub filament: String,
    /// Since when the print is paused and for how long (`since 14:03 · 12 min`); empty if not paused or not seen
    /// beginning.
    pub paused_since: String,
    /// Source of the expected print time behind the remaining time while a job runs; for the log.
    pub expectation_origin: Option<&'static str>,
}

pub fn job_texts(
    job: &PrintJob,
    metadata: Option<&FileMetadata>,
    last_print: Option<&LastPrint>,
    spool: Option<&Spool>,
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
        total: if job.filename.is_empty() { NO_VALUE.to_owned() } else { format::duration(job.total_duration) },
        remaining: remaining.map_or_else(|| NO_VALUE.to_owned(), format::duration),
        eta: remaining.map_or_else(|| NO_VALUE.to_owned(), |seconds| format::finish_time(now, seconds)),
        layers: layers(job, metadata),
        heights: heights(job, metadata),
        filament: filament(job, metadata, spool),
        paused_since: paused_since(job, now),
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
        // A complete job printed every layer: the pair would say the same twice.
        (_, Some(total)) if job.state == PrintState::Complete => total,
        (Some(current), Some(total)) => format!("{current} / {total}"),
        (Some(current), None) => current.to_string(),
        (None, Some(total)) if job.state == PrintState::Standby => total,
        _ => NO_VALUE.to_owned(),
    }
}

fn heights(job: &PrintJob, metadata: Option<&FileMetadata>) -> String {
    let object = metadata.and_then(|metadata| metadata.object_height).filter(|height| *height > 0.0);
    let reached = job.height.filter(|_| job.state != PrintState::Complete || object.is_none());
    match (reached, object) {
        (Some(current), Some(object)) => format::height_of(current, object),
        (Some(height), None) | (None, Some(height)) => format::height(height),
        (None, None) => String::new(),
    }
}

/// The height a layer ahead will have, for example `~10.00 mm`; empty if it cannot be told.
pub fn height_at_layer(job: &PrintJob, metadata: Option<&FileMetadata>, layer: u32) -> String {
    let metadata = metadata.filter(|metadata| metadata.filename == job.filename);
    estimates::height_at_layer(job, metadata, layer).map(|height| show(height, format::height)).unwrap_or_default()
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

/// Filament as weight where it is known, otherwise as length: before the print what the file plans, afterwards what
/// a complete job used, else what the job used of the plan.
fn filament(job: &PrintJob, metadata: Option<&FileMetadata>, spool: Option<&Spool>) -> String {
    let planned = metadata.and_then(|metadata| metadata.filament_weight_total).filter(|grams| *grams > 0.0);
    let used = Some(job.filament_used)
        .filter(|length| *length > 0.0)
        .and_then(|length| estimates::used_filament_grams(metadata, length, spool));
    match (job.state, used, planned) {
        (PrintState::Standby, _, Some(planned)) => {
            show(Sourced::estimated(planned, estimates::ORIGIN_SLICER_ESTIMATE), format::filament_weight)
        }
        (PrintState::Complete, Some(used), _) | (_, Some(used), None) => show(used, format::filament_weight),
        (_, Some(used), Some(planned)) => format::filament_weight_progress(used.value, planned),
        _ => filament_length(job, metadata),
    }
}

fn filament_length(job: &PrintJob, metadata: Option<&FileMetadata>) -> String {
    let total = metadata.and_then(|metadata| metadata.filament_total);
    match (job.state, total) {
        (PrintState::Standby, Some(total)) => format::filament_length(total),
        (PrintState::Complete, _) if job.filament_used > 0.0 => format::filament_length(job.filament_used),
        (_, Some(total)) if job.filament_used > 0.0 => format::filament_progress(job.filament_used, total),
        _ if job.filament_used > 0.0 => format::filament_length(job.filament_used),
        _ => NO_VALUE.to_owned(),
    }
}

/// Shorter pauses show only since when: `0 min` would read odd, seconds would change the banner every second.
const PAUSE_DURATION_SHOWN_FROM_SECONDS: f64 = 60.0;

/// `since 14:03 · 12 min` while paused, if the pause was seen beginning; in its first minute only `since 14:03`.
fn paused_since(job: &PrintJob, now: DateTime<Local>) -> String {
    let Some(since) = job.paused_since.filter(|_| job.state == PrintState::Paused) else { return String::new() };
    let start = format!("since {}", format::unix_moment(now, since));
    let paused_for = now.timestamp() as f64 - since;
    if paused_for < PAUSE_DURATION_SHOWN_FROM_SECONDS {
        return start;
    }
    format!("{start} · {}", format::duration_minutes(paused_for))
}

/// When the shown job ended, from the job history; empty while it runs or if the history does not know (Moonraker
/// without its `history` component).
pub fn ended_text(job: &PrintJob, newest: Option<&FinishedJob>, now: DateTime<Local>) -> String {
    newest
        .and_then(|newest| job_end::ended_at(job, newest))
        .map_or_else(String::new, |end| format::unix_moment(now, end))
}

/// What the history states about a job Klipper lost.
pub fn job_loss_text(loss: JobLoss) -> String {
    match loss {
        JobLoss::Shutdown => "Klipper shut down during the print".to_owned(),
        JobLoss::Disconnect => "Klipper stopped during the print".to_owned(),
    }
}

/// What the sources state about a pause or an error, one fact per line; empty if nothing is stated.
pub fn interruption_text(interruption: &Interruption) -> String {
    interruption.facts.iter().map(fact_text).collect::<Vec<_>>().join("\n")
}

/// Where the print stands at a pause or an error (`Layer 50 · 10.00 mm`, or the part that is known); empty if unknown.
/// Kept apart from the facts, so the banner can show it beside its title and long facts have the room.
pub fn interruption_position(job: &PrintJob) -> String {
    stands_at(job).unwrap_or_default()
}

/// The first line of the first fact of a pause or an error for the status bar, without where the print stands; empty
/// if none is stated. Happy Hare's `reason_for_pause` can span several lines; the bar has room for one.
pub fn interruption_headline(interruption: &Interruption) -> String {
    let first_fact = interruption.facts.first().map(fact_text).unwrap_or_default();
    first_fact.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_owned()
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
    let height = job.height.map(format::height);
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

/// A weight the slicer plans, an estimate: `~54 g`.
pub fn planned_weight(grams: f64) -> String {
    format!("~{}", format::filament_weight(grams))
}

/// The tools of the file that share a gate, `T0 + T2`; empty for one tool alone.
pub fn shared_tools_text(tools: &[usize]) -> String {
    if tools.len() < 2 {
        return String::new();
    }
    tools.iter().map(|tool| format!("T{tool}")).collect::<Vec<_>>().join(" + ")
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

/// What the prepared file's hints show: one line each, and which tiles of the facts they mark.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreparedHints {
    pub lines: Vec<String>,
    pub nozzle_differs: bool,
    pub temperature_differs: bool,
}

/// The hints of the prepared file in words; an extruder other than Klipper's `extruder` is named.
pub fn prepared_hints(hints: &[PrintHint]) -> PreparedHints {
    let named = |extruder: &str| if extruder == "extruder" { String::new() } else { format!(" ({extruder})") };
    let mut shown = PreparedHints::default();
    for hint in hints {
        let line = match hint {
            PrintHint::NozzleDiameter { extruder, file, printer } => {
                shown.nozzle_differs = true;
                let (file, printer) = (format::nozzle_diameter(*file), format::nozzle_diameter(*printer));
                format!("File plans a {file} nozzle, the printer has {printer}{}", named(extruder))
            }
            PrintHint::NozzleTemperature { extruder, file, max } => {
                shown.temperature_differs = true;
                let (file, max) = (format::degrees(*file), format::degrees(*max));
                format!("Nozzle {file} is above the printer's maximum of {max}{}", named(extruder))
            }
            PrintHint::BedTemperature { file, max } => {
                shown.temperature_differs = true;
                format!("Bed {} is above the printer's maximum of {}", format::degrees(*file), format::degrees(*max))
            }
        };
        shown.lines.push(line);
    }
    shown
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
    temperature.filter(|degrees| *degrees > 0.0).map_or_else(String::new, format::degrees)
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

/// The sides the buffer measures, as the word the UI maps (`BufferScale`).
pub fn mmu_buffer_sides(sides: BufferSides) -> &'static str {
    match sides {
        BufferSides::Both => "both",
        BufferSides::CompressionOnly => "compression",
        BufferSides::TensionOnly => "tension",
    }
}

/// What the encoder leaves before FlowGuard trips, of the length it watches, e.g. `7.5 / 12.0 mm`.
pub fn encoder_headroom(encoder: EncoderHeadroom) -> String {
    format::encoder_headroom(encoder.headroom, encoder.detection_length)
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

pub fn flowguard_statement(statement: FlowGuardStatement) -> &'static str {
    match statement {
        FlowGuardStatement::Triggered => "triggered",
        FlowGuardStatement::TangleBoost => "tangle-boost",
        FlowGuardStatement::Clog => "clog",
        FlowGuardStatement::Tangle => "tangle",
        FlowGuardStatement::Boosting => "boosting",
        FlowGuardStatement::Quiet => "quiet",
    }
}

pub fn emphasis_severity(emphasis: Emphasis) -> Severity {
    match emphasis {
        Emphasis::Calm => Severity::Calm,
        Emphasis::Attention => Severity::Attention,
        Emphasis::Error => Severity::Error,
    }
}

/// The marker of the buffer scale: FlowGuard's emphasis while it has one; otherwise neutral for a measuring buffer
/// (neutral is the resting state, leaning to a side is the controller at work, not a deviation).
pub fn buffer_marker_severity(line: Emphasis, buffer: SyncFeedbackState) -> Severity {
    match (line, buffer) {
        (Emphasis::Calm, SyncFeedbackState::Neutral | SyncFeedbackState::Tension | SyncFeedbackState::Compressed) => {
            Severity::Neutral
        }
        _ => emphasis_severity(line),
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
    use sailhand_core::sources::moonraker_server::files::FileEntry;

    #[test]
    fn a_buffer_names_the_sides_a_sensor_measures() {
        assert_eq!(mmu_buffer_sides(BufferSides::Both), "both");
        assert_eq!(mmu_buffer_sides(BufferSides::CompressionOnly), "compression");
        assert_eq!(mmu_buffer_sides(BufferSides::TensionOnly), "tension");
    }

    #[test]
    fn the_encoder_headroom_reads_as_used_of_watched_length() {
        assert_eq!(encoder_headroom(EncoderHeadroom { headroom: 7.5, detection_length: 12.0 }), "7.5 / 12.0 mm");
    }

    #[test]
    fn the_buffer_marker_follows_flowguard_before_the_buffer() {
        assert_eq!(buffer_marker_severity(Emphasis::Error, SyncFeedbackState::Neutral), Severity::Error);
        assert_eq!(buffer_marker_severity(Emphasis::Calm, SyncFeedbackState::Neutral), Severity::Neutral);
        assert_eq!(buffer_marker_severity(Emphasis::Calm, SyncFeedbackState::Tension), Severity::Neutral);
        assert_eq!(buffer_marker_severity(Emphasis::Calm, SyncFeedbackState::Inactive), Severity::Calm);
    }

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

        let texts = job_texts(&job, Some(&metadata("dir/cube.gcode")), None, None, now());

        assert_eq!(texts.name, "cube");
        assert_eq!(texts.elapsed, "6 min 00 s");
        assert_eq!(texts.remaining, "54 min 00 s");
        assert_eq!(texts.eta, "12:54");
        assert_eq!(texts.layers, "3 / ~100");
        assert_eq!(texts.filament, "1.0 / 5.0 m");
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

        let texts = job_texts(&job, Some(&metadata("a.gcode")), Some(&last_print), None, now());

        assert_eq!(texts.remaining, "1 h 03 min");
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

        let texts = job_texts(&job, None, None, None, now());

        assert_eq!(texts.remaining, "30 min 00 s");
        assert_eq!(texts.layers, "-");
        assert_eq!(texts.expectation_origin, Some(ORIGIN_PACE_ONLY));
    }

    #[test]
    fn standby_shows_file_totals_and_no_eta() {
        let job = PrintJob { filename: "a.gcode".into(), ..PrintJob::default() };

        let texts = job_texts(&job, Some(&metadata("a.gcode")), None, None, now());

        assert_eq!(texts.remaining, "-");
        assert_eq!(texts.eta, "-");
        assert_eq!(texts.expectation_origin, None);
        assert_eq!(texts.layers, "~100");
        assert_eq!(texts.filament, "5.0 m");
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

        assert_eq!(job_texts(&job, Some(&metadata("a.gcode")), None, None, now()).layers, "3 / 120");
    }

    #[test]
    fn heights_show_each_layer_step_in_one_unit() {
        let file = metadata("a.gcode");
        let printing = PrintJob {
            state: PrintState::Printing,
            filename: "a.gcode".into(),
            current_layer: Some(13),
            height: Some(2.6),
            ..PrintJob::default()
        };
        let ended = PrintJob { state: PrintState::Complete, height: None, ..printing.clone() };

        assert_eq!(job_texts(&printing, Some(&file), None, None, now()).heights, "2.60 / 20.00 mm");
        assert_eq!(job_texts(&printing, None, None, None, now()).heights, "2.60 mm");
        assert_eq!(job_texts(&ended, Some(&file), None, None, now()).heights, "20.00 mm");
        assert_eq!(job_texts(&PrintJob::default(), None, None, None, now()).heights, "");
        assert_eq!(height_at_layer(&printing, Some(&file), 20), "~4.00 mm");
    }

    #[test]
    fn an_ended_job_shows_the_height_it_reached_and_a_complete_one_its_object() {
        let file = metadata("a.gcode");
        let cancelled = PrintJob {
            state: PrintState::Cancelled,
            filename: "a.gcode".into(),
            current_layer: Some(7),
            total_layer: Some(100),
            height: Some(1.4),
            ..PrintJob::default()
        };
        let complete =
            PrintJob { state: PrintState::Complete, current_layer: Some(100), height: Some(30.0), ..cancelled.clone() };

        let cancelled_texts = job_texts(&cancelled, Some(&file), None, None, now());
        let complete_texts = job_texts(&complete, Some(&file), None, None, now());

        assert_eq!((cancelled_texts.heights, cancelled_texts.layers), ("1.40 / 20.00 mm".into(), "7 / 100".into()));
        assert_eq!((complete_texts.heights, complete_texts.layers), ("20.00 mm".into(), "100".into()));
    }

    #[test]
    fn filament_shows_as_weight_where_the_file_tells_it() {
        let file = FileMetadata { filament_weight_total: Some(15.0), ..metadata("a.gcode") };
        let cancelled = PrintJob {
            state: PrintState::Cancelled,
            filename: "a.gcode".into(),
            filament_used: 1000.0,
            total_duration: 900.0,
            ..PrintJob::default()
        };
        let complete = PrintJob { state: PrintState::Complete, filament_used: 5000.0, ..cancelled.clone() };
        let standby = PrintJob { filename: "a.gcode".into(), ..PrintJob::default() };

        assert_eq!(job_texts(&cancelled, Some(&file), None, None, now()).filament, "3.0 / 15 g");
        assert_eq!(job_texts(&complete, Some(&file), None, None, now()).filament, "15 g");
        assert_eq!(job_texts(&standby, Some(&file), None, None, now()).filament, "~15 g");
        assert_eq!(job_texts(&complete, None, None, None, now()).filament, "5.0 m");
        assert_eq!(job_texts(&cancelled, Some(&file), None, None, now()).total, "15 min 00 s");
    }

    #[test]
    fn a_pause_seen_beginning_tells_since_when_and_how_long() {
        let since = now().timestamp() as f64 - 12.0 * 60.0;
        let paused = PrintJob {
            state: PrintState::Paused,
            filename: "a.gcode".into(),
            paused_since: Some(since),
            ..PrintJob::default()
        };
        let unseen = PrintJob { paused_since: None, ..paused.clone() };
        let fresh = PrintJob { paused_since: Some(now().timestamp() as f64 - 40.0), ..paused.clone() };

        assert_eq!(job_texts(&paused, None, None, None, now()).paused_since, "since 11:48 · 12 min");
        assert_eq!(job_texts(&fresh, None, None, None, now()).paused_since, "since 11:59");
        assert_eq!(job_texts(&unseen, None, None, None, now()).paused_since, "");
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

        assert_eq!(job_texts(&job, Some(&metadata("b.gcode")), None, None, now()).filament, "-");
    }

    #[test]
    fn the_end_of_a_finished_job_comes_from_the_history() {
        let job = PrintJob { state: PrintState::Complete, filename: "a.gcode".into(), ..PrintJob::default() };
        let end = Local.with_ymd_and_hms(2026, 10, 21, 11, 32, 0).unwrap().timestamp() as f64;
        let newest = FinishedJob {
            filename: "a.gcode".into(),
            status: "completed".into(),
            end_time: end,
            ..FinishedJob::default()
        };

        assert_eq!(ended_text(&job, Some(&newest), now()), "11:32");
        assert_eq!(ended_text(&job, None, now()), "");
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

        assert_eq!(interruption_text(&interruption), "Filament runout\nToolhead Runout: no filament");
        let job = PrintJob { current_layer: Some(50), height: Some(10.0), ..PrintJob::default() };
        assert_eq!(interruption_position(&job), "Layer 50 · 10.00 mm");
        assert_eq!(interruption_position(&PrintJob::default()), "");
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

    #[test]
    fn the_headline_of_a_multi_line_mmu_reason_is_its_first_line() {
        let interruption = Interruption {
            failed: false,
            facts: vec![InterruptionFact::MmuReason(
                "\nFailure during check gate T0:\nNo filament detected\nGate marked as empty".into(),
            )],
        };

        assert_eq!(interruption_headline(&interruption), "Failure during check gate T0:");
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
                time: "~1 h 00 min".into(),
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

        assert_eq!(
            (facts.material.as_str(), facts.weight.as_str(), facts.time.as_str()),
            ("PLA", "27 g", "55 min 00 s")
        );
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
        ToolMatch { request, gate, needed: None, reserve: None, verdict, color_elsewhere: false }
    }

    #[test]
    fn tools_that_do_not_fit_win_over_those_without_facts() {
        let differs = Verdict::Differs(vec![Difference::Weight { needed: 20.0, left: 5.0, reserve: None }]);

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
    fn planned_weights_are_estimates_and_shared_gates_name_their_tools() {
        assert_eq!(planned_weight(54.0), "~54 g");
        assert_eq!(shared_tools_text(&[0, 2]), "T0 + T2");
        assert_eq!(shared_tools_text(&[1]), "");
    }

    #[test]
    fn the_material_line_names_both_materials_when_they_differ() {
        let differs = Verdict::Differs(vec![Difference::Material { expected: "PLA".into(), loaded: "ASA".into() }]);
        let mut found = tool_match(0, Some(0), differs);
        found.request.material = Some("PLA".into());

        assert_eq!(material_line(&found), "PLA → ASA");
        found.verdict = Verdict::Differs(vec![Difference::Weight { needed: 20.0, left: 5.0, reserve: None }]);
        assert_eq!(material_line(&found), "PLA");
        assert_eq!(material_difference(&found), None);
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
    fn hints_of_the_prepared_file_read_as_lines_and_mark_their_tiles() {
        let hints = [
            PrintHint::NozzleDiameter { extruder: "extruder1".into(), file: 0.6, printer: 0.4 },
            PrintHint::BedTemperature { file: 130.0, max: 120.0 },
        ];

        let shown = prepared_hints(&hints);

        assert_eq!(
            shown.lines,
            [
                "File plans a 0.6 mm nozzle, the printer has 0.4 mm (extruder1)",
                "Bed 130° is above the printer's maximum of 120°"
            ]
        );
        assert!(shown.nozzle_differs && shown.temperature_differs);
        assert_eq!(prepared_hints(&[]), PreparedHints::default());
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
        let accel = sailhand_core::sources::klipper::tuning::TuningKey::MaxAccel.range();
        assert_eq!(tuning_value(5000.0, &accel), "5000 mm/s²");
        let advance = sailhand_core::sources::klipper::tuning::TuningKey::PressureAdvance.range();
        assert_eq!(tuning_value(0.04, &advance), "0.040");
    }
}
