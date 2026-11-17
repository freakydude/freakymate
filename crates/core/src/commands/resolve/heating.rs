//! Heater targets and the tuning values.

use crate::commands::{Command, CommandError};
use crate::domain::machine_tree::MachineTree;
use crate::sources::PrinterStatus;
use crate::sources::klipper::tuning::TuningKey;
use crate::sources::klipper::{BED_HEATER, Setup};

/// A heater target; never below zero, which is off.
pub(super) fn target(heater: &str, target: f64) -> Command {
    Command::SetHeaterTarget { heater: heater.to_owned(), target: target.max(0.0) }
}

/// Heats the nozzle in use (that of the mounted head; with no head mounted none) and, if there is one, the bed.
pub(super) fn preset(status: &PrinterStatus, extruder: f64, bed: f64) -> Vec<Command> {
    let tree = MachineTree::build(status);
    let mut commands: Vec<Command> =
        tree.selected_extruder().map(|nozzle| target(nozzle, extruder)).into_iter().collect();
    if status.heater(BED_HEATER).is_some() {
        commands.push(target(BED_HEATER, bed));
    }
    commands
}

pub(super) fn cool_down(status: &PrinterStatus) -> Vec<Command> {
    status.heaters.iter().map(|heater| target(&heater.object, 0.0)).collect()
}

pub(super) fn step(status: &PrinterStatus, heater: &str, step: f64) -> Result<Vec<Command>, CommandError> {
    let current = status.heater(heater).ok_or_else(|| CommandError::UnknownHeater(heater.to_owned()))?;
    Ok(vec![target(heater, current.stepped_target(step))])
}

/// The target of a tuning must be one Klipper takes and not off (`heaters.py` l.110-113 refuses the rest).
pub(super) fn pid_calibrate(status: &PrinterStatus, heater: String, target: f64) -> Result<Vec<Command>, CommandError> {
    let known = status.heater(&heater).filter(|known| !known.cools_only);
    let known = known.ok_or_else(|| CommandError::UnknownHeater(heater.clone()))?;
    if target <= 0.0 || !known.accepts_target(target) {
        let maximum = known.max_temp.map_or(String::from("no maximum"), |maximum| format!("up to {maximum} °C"));
        return Err(CommandError::TargetOutsideRange(
            target.to_string(),
            format!("from {} °C, {maximum}", known.min_temp),
        ));
    }
    Ok(vec![Command::PidCalibrate { heater, target }])
}

pub(super) fn step_tuning(
    status: &PrinterStatus,
    key: TuningKey,
    direction: f64,
) -> Result<Vec<Command>, CommandError> {
    let current = key.current(status).ok_or_else(|| CommandError::NoTuningValue(key.id().to_owned()))?;
    Ok(vec![Command::SetTuning { key, value: key.stepped(current, direction) }])
}

pub(super) fn reset_tuning(setup: &Setup, key: TuningKey) -> Result<Vec<Command>, CommandError> {
    let value = setup.tuning.get(key).ok_or_else(|| CommandError::TuningNotConfigured(key.id().to_owned()))?;
    Ok(vec![Command::SetTuning { key, value }])
}
