//! Calibration and bed mesh commands.

use super::*;

/// Parses `name` if it belongs to this group.
pub(super) fn parse(name: &str, args: &[&str]) -> Result<Option<Command>, CommandError> {
    let command: Result<Command, CommandError> = match name {
        "PROBE_CALIBRATE" => Ok(Command::Calibration(CalibrationCommand::ProbeCalibrate)),
        // The mux commands take the probe by `CHIP` (pe l.112; ldc l.43-74); the tap step takes `TAP` (pe l.340).
        "LDC_CALIBRATE_DRIVE_CURRENT" => {
            Ok(Command::Calibration(CalibrationCommand::EddyDriveCurrent { chip: required_text(args, "CHIP")? }))
        }
        "PROBE_EDDY_CURRENT_CALIBRATE" => {
            Ok(Command::Calibration(CalibrationCommand::EddyCalibrate { chip: required_text(args, "CHIP")? }))
        }
        "PROBE_EDDY_CURRENT_TAP_CALIBRATE" => {
            let step = named_text(args, "TAP").map_or(Ok(TapStep::Guess), |step| TapStep::parse(&step));
            step.map(|step| Command::Calibration(CalibrationCommand::EddyTap(step)))
        }
        "MANUAL_PROBE" => Ok(Command::Calibration(CalibrationCommand::ManualProbe)),
        "TESTZ" => Ok(Command::Calibration(CalibrationCommand::TestZ(TestZ::parse(&required_text(args, "Z")?)?))),
        "ACCEPT" => Ok(Command::Calibration(CalibrationCommand::Accept)),
        "ADJUSTED" => Ok(Command::Calibration(CalibrationCommand::Adjusted)),
        "ABORT" => Ok(Command::Calibration(CalibrationCommand::Abort)),
        "BED_SCREWS_ADJUST" => Ok(Command::Calibration(CalibrationCommand::BedScrewsAdjust)),
        "SCREWS_TILT_CALCULATE" => Ok(Command::Calibration(CalibrationCommand::ScrewsTiltCalculate)),
        "BED_MESH_CALIBRATE" => {
            Ok(Command::Calibration(CalibrationCommand::BedMeshCalibrate { profile: named_text(args, "PROFILE") }))
        }
        // Klipper takes the first of `LOAD`, `SAVE` and `REMOVE` (`bed_mesh.py`).
        "BED_MESH_PROFILE" => Ok(bed_mesh_profile(args).map_or(Command::Ignored, Command::Calibration)),
        "BED_MESH_CLEAR" => Ok(Command::Calibration(CalibrationCommand::BedMeshClear)),
        _ => return Ok(None),
    };
    command.map(Some)
}

fn bed_mesh_profile(args: &[&str]) -> Option<CalibrationCommand> {
    if let Some(profile) = named_text(args, "LOAD") {
        return Some(CalibrationCommand::BedMeshLoad { profile });
    }
    if let Some(profile) = named_text(args, "SAVE") {
        return Some(CalibrationCommand::BedMeshSave { profile });
    }
    named_text(args, "REMOVE").map(|profile| CalibrationCommand::BedMeshRemove { profile })
}
