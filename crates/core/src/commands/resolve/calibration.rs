//! Levelling, Z height and the commands within a calibration mode.

use crate::commands::{Command, CommandError};
use crate::sources::PrinterStatus;
use crate::sources::klipper::{CalibrationMode, CentreOf, Leveling, Setup, centre_target};

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
    // An Eddy probe has no `PROBE_CALIBRATE` (`probe_eddy_current.py` l.1055, `can_set_z_offset=False`; `probe.py`
    // l.56-60): its own command measures the Z heights.
    if let Some(chip) = &setup.eddy_probe {
        Ok(vec![Command::EddyCalibrate { chip: chip.clone() }])
    } else if setup.probe {
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
    if waiting { vec![command] } else { Vec::new() }
}

pub(super) fn eddy_drive_current(setup: &Setup) -> Result<Vec<Command>, CommandError> {
    let chip = setup.eddy_probe.clone().ok_or(CommandError::NoEddyProbe)?;
    Ok(vec![Command::EddyDriveCurrent { chip }])
}

pub(super) fn move_to_bed_centre(status: &PrinterStatus, over: CentreOf) -> Result<Vec<Command>, CommandError> {
    let probe = status.calibration.eddy.as_ref().ok_or(CommandError::NoEddyProbe)?;
    let [x, y] = centre_target(&status.motion, probe, over).ok_or(CommandError::NoAxisRange)?;
    Ok(vec![Command::MoveXy { x, y }])
}
