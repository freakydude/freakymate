//! The sync-feedback buffer, its flow rate and FlowGuard.

use super::sensors::{has_proportional_sensor, MmuSensor};
use crate::sourced::{prefer_measured, Sourced};
use crate::sources::fields::{flag, text};
use serde_json::Value;

pub const ORIGIN_ENCODER: &str = "encoder";
const COMPRESSION_SENSOR: &str = "filament_compression";
const TENSION_SENSOR: &str = "filament_tension";
pub const ORIGIN_SYNC_FEEDBACK: &str = "sync feedback with proportional sensor";

/// Sync-feedback state as Happy Hare reports it (`get_sync_feedback_string`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncFeedbackState {
    Compressed,
    Neutral,
    Tension,
    /// Enabled, but the controller is not active (for example while not printing).
    Inactive,
    #[default]
    Disabled,
}

impl SyncFeedbackState {
    pub(super) fn parse(text: &str) -> Self {
        match text {
            "compressed" => Self::Compressed,
            "neutral" => Self::Neutral,
            "tension" => Self::Tension,
            "inactive" => Self::Inactive,
            _ => Self::Disabled,
        }
    }
}

/// The sides of the buffer a sensor measures: Happy Hare's sensor types CO (compression only), TO (tension only), and
/// D (both switches) or P (proportional) for both (`_get_sensor_type`, `unit/mmu_sync_feedback.py` L880-896 at
/// c176ecc). A one-sided switch reports `neutral` while it is free and its bias never leaves its half; FlowGuard only
/// infers the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BufferSides {
    #[default]
    Both,
    CompressionOnly,
    TensionOnly,
}

/// Buffer of the gear stepper: `bias` runs from -1 (tension) over 0 (neutral) to +1 (compressed).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SyncFeedback {
    pub state: SyncFeedbackState,
    pub bias: f64,
    pub sides: BufferSides,
}

/// Clog and tangle protection; it works with the buffer controller or with an encoder.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlowGuard {
    pub enabled: bool,
    pub active: bool,
    /// Tangle prevention raises the gear current right now: it is on, a proportional sensor reports the tension and
    /// the tension is above its threshold.
    pub boosting: bool,
    /// The tension (0 to 1 of the buffer's bias) from which tangle prevention raises the gear current; `None` while
    /// it is off or cannot work.
    pub tangle_threshold: Option<f64>,
    /// The bias (0 to 1) from which the proportional sensor counts as at its end: FlowGuard's EKF takes it as its
    /// extreme threshold and the virtual switches trigger there (`analog_sensor_threshold` of the buffer's section).
    /// `None` without a proportional sensor or while the configuration does not tell.
    pub extreme_threshold: Option<f64>,
    pub trigger: String,
    pub reason: String,
    /// How close FlowGuard is to a trigger: 0 at rest, +1 a clog (the buffer stays compressed), -1 a tangle (it stays
    /// under tension).
    pub level: f64,
}

/// What an encoder leaves before FlowGuard trips on it: the filament may lag `headroom` mm more behind the extruder,
/// out of the `detection_length` mm it watches (`unit/mmu_encoder.py` L471-481 at c176ecc). The encoder cannot tell a
/// clog from a tangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncoderHeadroom {
    pub headroom: f64,
    pub detection_length: f64,
}

/// One word for the operator: a trigger wins over the boost of tangle prevention, which wins over monitoring, which
/// wins over ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowGuardState {
    Off,
    Triggered,
    Boosting,
    Monitoring,
    Ready,
}

impl FlowGuard {
    pub fn state(&self) -> FlowGuardState {
        if !self.enabled {
            FlowGuardState::Off
        } else if !self.trigger.is_empty() {
            FlowGuardState::Triggered
        } else if self.boosting {
            FlowGuardState::Boosting
        } else if self.active {
            FlowGuardState::Monitoring
        } else {
            FlowGuardState::Ready
        }
    }
}

/// Happy Hare keeps `sync_feedback_flow_rate` at 100 for every buffer and only estimates it when an enabled
/// proportional sensor exists (`mmu_sync_feedback.py`), so the field alone says nothing without that sensor.
pub(super) fn read_flow_rate(mmu: &Value, sensors: &[MmuSensor]) -> Option<Sourced<f64>> {
    let measured = mmu
        .get("encoder")
        .and_then(|encoder| encoder.get("flow_rate"))
        .and_then(Value::as_f64)
        .map(|rate| Sourced::measured(rate, ORIGIN_ENCODER));
    let estimated = mmu
        .get("sync_feedback_flow_rate")
        .and_then(Value::as_f64)
        .filter(|_| has_proportional_sensor(sensors))
        .map(|rate| Sourced::estimated(rate, ORIGIN_SYNC_FEEDBACK));
    prefer_measured(measured, estimated)
}

/// Only while the encoder is armed (`enabled`, which Happy Hare sets while printing, and a detection mode other than
/// 0, off) and reports a length to watch.
pub(super) fn read_encoder_headroom(mmu: &Value) -> Option<EncoderHeadroom> {
    let encoder = mmu.get("encoder").filter(|value| value.is_object())?;
    let armed = flag(encoder, "enabled") && encoder.get("detection_mode").and_then(Value::as_i64) != Some(0);
    let headroom = encoder.get("headroom").and_then(Value::as_f64)?;
    let detection_length = encoder.get("detection_length").and_then(Value::as_f64).filter(|length| *length > 0.0)?;
    armed.then_some(EncoderHeadroom { headroom: headroom.clamp(0.0, detection_length), detection_length })
}

/// Only present with a buffer; the modelled bias is the better value while printing.
pub(super) fn read_sync_feedback(mmu: &Value, sensors: &[MmuSensor]) -> Option<SyncFeedback> {
    let state = mmu.get("sync_feedback_state").and_then(Value::as_str)?;
    let bias = mmu
        .get("sync_feedback_bias_modelled")
        .and_then(Value::as_f64)
        .or_else(|| mmu.get("sync_feedback_bias_raw").and_then(Value::as_f64))
        .unwrap_or_default();
    Some(SyncFeedback { state: SyncFeedbackState::parse(state), bias: bias.clamp(-1.0, 1.0), sides: sides(sensors) })
}

/// A proportional sensor measures both sides (and adds virtual switches for them); else the fitted switches say.
fn sides(sensors: &[MmuSensor]) -> BufferSides {
    let fitted = |name: &str| sensors.iter().any(|sensor| sensor.name == name && sensor.triggered.is_some());
    if has_proportional_sensor(sensors) {
        return BufferSides::Both;
    }
    match (fitted(COMPRESSION_SENSOR), fitted(TENSION_SENSOR)) {
        (true, false) => BufferSides::CompressionOnly,
        (false, true) => BufferSides::TensionOnly,
        _ => BufferSides::Both,
    }
}

/// Tangle prevention counts only with a proportional sensor, as Happy Hare arms it only then.
pub(super) fn read_flowguard(mmu: &Value, sensors: &[MmuSensor]) -> Option<FlowGuard> {
    let flowguard = mmu.get("flowguard").filter(|value| value.is_object())?;
    let tangle_prevention = mmu.get("tangle_prevention").filter(|value| value.is_object());
    let armed = tangle_prevention.filter(|prevention| flag(prevention, "enabled") && has_proportional_sensor(sensors));
    let boosting = armed.is_some_and(|prevention| flag(prevention, "boosted"));
    let tangle_threshold = armed
        .and_then(|prevention| prevention.get("threshold"))
        .and_then(Value::as_f64)
        .filter(|threshold| (0.0..=1.0).contains(threshold));
    Some(FlowGuard {
        enabled: flag(flowguard, "enabled"),
        active: flag(flowguard, "active"),
        boosting,
        tangle_threshold,
        extreme_threshold: None,
        trigger: text(flowguard, "trigger").to_owned(),
        reason: text(flowguard, "reason").to_owned(),
        level: flowguard.get("level").and_then(Value::as_f64).unwrap_or_default().clamp(-1.0, 1.0),
    })
}

/// `analog_sensor_threshold` of the section `[mmu_buffer <buffer>]` (lowercased like every section in
/// `configfile.settings`); Happy Hare allows 0.5 to 1.0 (`unit/mmu_buffer.py` L200). Only a proportional sensor has it.
pub(super) fn read_extreme_threshold(settings: &Value, buffer: &str, sensors: &[MmuSensor]) -> Option<f64> {
    if !has_proportional_sensor(sensors) {
        return None;
    }
    settings
        .get(format!("mmu_buffer {}", buffer.to_lowercase()))?
        .get("analog_sensor_threshold")?
        .as_f64()
        .filter(|threshold| (0.5..=1.0).contains(threshold))
}
