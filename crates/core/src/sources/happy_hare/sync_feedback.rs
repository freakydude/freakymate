//! The sync-feedback buffer, its flow rate and FlowGuard.

use super::sensors::{MmuSensor, has_proportional_sensor};
use crate::sourced::{Sourced, prefer_measured};
use crate::sources::fields::{flag, section, text};
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

/// From this level on FlowGuard counts as rising towards a trigger; smaller values come and go while the controller
/// works.
pub const RISING_LEVEL: f64 = 0.1;

/// How much a part of the FlowGuard line stands out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    Calm,
    Attention,
    Error,
}

/// What the FlowGuard line says, most important first: a trigger, tangle prevention at work on a rising tangle, a
/// rising level, tangle prevention alone, and otherwise nothing of its own (the buffer's state fills the line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowGuardStatement {
    Triggered,
    TangleBoost,
    Clog,
    Tangle,
    Boosting,
    Quiet,
}

/// FlowGuard as the display needs it: the statement, the level in percent and how much each part stands out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowGuardReading {
    pub statement: FlowGuardStatement,
    /// Distance to a trigger, 0 to 100.
    pub percent: u32,
    /// The line itself.
    pub line: Emphasis,
    /// The label of the tangle end (tension) of the scale, and of the clog end (compression).
    pub tangle_end: Emphasis,
    pub clog_end: Emphasis,
    /// The strip growing from an end of the scale.
    pub strip: Emphasis,
}

impl FlowGuard {
    /// The level that counts: FlowGuard that is off has none.
    pub fn active_level(&self) -> f64 {
        if self.enabled { self.level } else { 0.0 }
    }

    pub fn reading(&self) -> FlowGuardReading {
        let level = self.active_level();
        let triggered = self.state() == FlowGuardState::Triggered;
        let rising = !triggered && level.abs() >= RISING_LEVEL;
        let statement = match (triggered, rising, self.boosting) {
            (true, _, _) => FlowGuardStatement::Triggered,
            (_, true, true) => FlowGuardStatement::TangleBoost,
            (_, true, false) if level > 0.0 => FlowGuardStatement::Clog,
            (_, true, false) => FlowGuardStatement::Tangle,
            (_, false, true) => FlowGuardStatement::Boosting,
            _ => FlowGuardStatement::Quiet,
        };
        let line = if triggered {
            Emphasis::Error
        } else if rising || self.boosting {
            Emphasis::Attention
        } else {
            Emphasis::Calm
        };
        FlowGuardReading {
            statement,
            percent: (level.abs() * 100.0).round() as u32,
            line,
            tangle_end: if level <= -RISING_LEVEL { line } else { Emphasis::Calm },
            clog_end: if level >= RISING_LEVEL { line } else { Emphasis::Calm },
            strip: if triggered { Emphasis::Error } else { Emphasis::Attention },
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
    section(settings, "mmu_buffer ", buffer)?
        .get("analog_sensor_threshold")?
        .as_f64()
        .filter(|threshold| (0.5..=1.0).contains(threshold))
}

#[cfg(test)]
mod reading_tests {
    use super::*;

    fn guard(level: f64, boosting: bool, trigger: &str) -> FlowGuard {
        FlowGuard { enabled: true, boosting, trigger: trigger.into(), level, ..FlowGuard::default() }
    }

    #[test]
    fn a_trigger_outranks_everything() {
        let reading = guard(0.6, true, "clog").reading();
        assert_eq!(reading.statement, FlowGuardStatement::Triggered);
        assert_eq!(
            (reading.line, reading.strip, reading.clog_end),
            (Emphasis::Error, Emphasis::Error, Emphasis::Error)
        );
    }

    #[test]
    fn a_rising_tangle_with_boost_comes_before_the_level_alone() {
        let reading = guard(-0.35, true, "").reading();
        assert_eq!(reading.statement, FlowGuardStatement::TangleBoost);
        assert_eq!(reading.percent, 35);
        assert_eq!((reading.tangle_end, reading.clog_end), (Emphasis::Attention, Emphasis::Calm));
    }

    #[test]
    fn the_sign_tells_clog_from_tangle() {
        assert_eq!(guard(0.2, false, "").reading().statement, FlowGuardStatement::Clog);
        assert_eq!(guard(-0.2, false, "").reading().statement, FlowGuardStatement::Tangle);
    }

    #[test]
    fn small_levels_are_not_rising_and_boost_alone_still_counts() {
        assert_eq!(guard(0.05, false, "").reading().statement, FlowGuardStatement::Quiet);
        let boosting = guard(0.05, true, "").reading();
        assert_eq!((boosting.statement, boosting.line), (FlowGuardStatement::Boosting, Emphasis::Attention));
    }

    #[test]
    fn flowguard_that_is_off_has_no_level() {
        let off = FlowGuard { enabled: false, level: 0.9, ..FlowGuard::default() };
        assert_eq!((off.reading().statement, off.reading().percent), (FlowGuardStatement::Quiet, 0));
    }
}
