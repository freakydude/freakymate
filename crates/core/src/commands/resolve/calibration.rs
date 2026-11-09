//! Levelling, Z height and the commands within a calibration mode.

use crate::commands::{Command, CommandError};
use crate::sources::klipper::{CalibrationMode, Leveling, Setup};

pub(super) fn level(setup: &Setup) -> Result<Vec<Command>, CommandError> {
    match setup.leveling {
        Some(Leveling::QuadGantry) => Ok(vec![Command::QuadGantryLevel]),
        Some(Leveling::ZTilt) => Ok(vec![Command::ZTiltAdjust]),
        None => Err(CommandError::NoLeveling),
    }
}

pub(super) fn apply_z_offset(setup: &Setup) -> Vec<Command> {
    vec![if setup.probe { Command::ApplyZOffsetToProbe } else { Command::ApplyZOffsetToEndstop }]
}

pub(super) fn calibrate_z(setup: &Setup) -> Result<Vec<Command>, CommandError> {
    if setup.probe {
        Ok(vec![Command::ProbeCalibrate])
    } else if setup.calibration.z_endstop {
        Ok(vec![Command::ZEndstopCalibrate])
    } else {
        Err(CommandError::NoZCalibration)
    }
}

/// A command within a waiting mode is dropped unless Klipper is waiting there, since it knows the command only then.
pub(super) fn within_mode(command: Command, mode: Option<&CalibrationMode>) -> Vec<Command> {
    let waiting = match command {
        Command::TestZ { .. } => matches!(mode, Some(CalibrationMode::ManualProbe { .. })),
        Command::ScrewAdjusted => matches!(mode, Some(CalibrationMode::BedScrews { .. })),
        _ => mode.is_some(),
    };
    if waiting {
        vec![command]
    } else {
        Vec::new()
    }
}
