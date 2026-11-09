//! Calibration and bed mesh commands.

use super::*;

/// Parses `name` if it belongs to this group.
pub(super) fn parse(name: &str, args: &[&str]) -> Result<Option<Command>, CommandError> {
    let command: Result<Command, CommandError> = match name {
        "PROBE_CALIBRATE" => Ok(Command::Calibration(CalibrationCommand::ProbeCalibrate)),
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
