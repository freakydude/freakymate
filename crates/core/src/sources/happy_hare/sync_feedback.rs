//! The sync-feedback buffer, its flow rate and FlowGuard.

use super::sensors::{has_proportional_sensor, MmuSensor};
use crate::sourced::{prefer_measured, Sourced};
use crate::sources::fields::{flag, text};
use serde_json::Value;

pub const ORIGIN_ENCODER: &str = "encoder";
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

/// Buffer of the gear stepper: `bias` runs from -1 (tension) over 0 (neutral) to +1 (compressed).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SyncFeedback {
    pub state: SyncFeedbackState,
    pub bias: f64,
}

/// Clog and tangle protection; it works with the buffer controller or with an encoder.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlowGuard {
    pub enabled: bool,
    pub active: bool,
    /// Tangle prevention raises the gear current right now: it is on, a proportional sensor reports the tension and
    /// the tension is above its threshold.
    pub boosting: bool,
    pub trigger: String,
    pub reason: String,
    pub level: f64,
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

/// Only present with a buffer; the modelled bias is the better value while printing.
pub(super) fn read_sync_feedback(mmu: &Value) -> Option<SyncFeedback> {
    let state = mmu.get("sync_feedback_state").and_then(Value::as_str)?;
    let bias = mmu
        .get("sync_feedback_bias_modelled")
        .and_then(Value::as_f64)
        .or_else(|| mmu.get("sync_feedback_bias_raw").and_then(Value::as_f64))
        .unwrap_or_default();
    Some(SyncFeedback { state: SyncFeedbackState::parse(state), bias: bias.clamp(-1.0, 1.0) })
}

/// Tangle prevention counts only with a proportional sensor, as Happy Hare arms it only then.
pub(super) fn read_flowguard(mmu: &Value, sensors: &[MmuSensor]) -> Option<FlowGuard> {
    let flowguard = mmu.get("flowguard").filter(|value| value.is_object())?;
    let tangle_prevention = mmu.get("tangle_prevention").filter(|value| value.is_object());
    let boosting = tangle_prevention
        .is_some_and(|prevention| flag(prevention, "enabled") && flag(prevention, "boosted"))
        && has_proportional_sensor(sensors);
    Some(FlowGuard {
        enabled: flag(flowguard, "enabled"),
        active: flag(flowguard, "active"),
        boosting,
        trigger: text(flowguard, "trigger").to_owned(),
        reason: text(flowguard, "reason").to_owned(),
        level: flowguard.get("level").and_then(Value::as_f64).unwrap_or_default(),
    })
}
