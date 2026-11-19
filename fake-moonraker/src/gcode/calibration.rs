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
        "BED_MESH_CALIBRATE" => bed_mesh_calibrate(args).map(Command::Calibration),
        // Klipper takes the first of `LOAD`, `SAVE` and `REMOVE` (`bed_mesh.py`).
        "BED_MESH_PROFILE" => Ok(bed_mesh_profile(args).map_or(Command::Ignored, Command::Calibration)),
        "BED_MESH_CLEAR" => Ok(Command::Calibration(CalibrationCommand::BedMeshClear)),
        _ => return Ok(None),
    };
    command.map(Some)
}

/// `PROBE_COUNT=<x>,<y>` for rectangular beds, `ROUND_PROBE_COUNT` for round ones (`bed_mesh.py` `update_config`).
fn bed_mesh_calibrate(args: &[&str]) -> Result<CalibrationCommand, CommandError> {
    let probe_count = named_text(args, "PROBE_COUNT").map(|text| parse_count_pair(&text)).transpose()?;
    Ok(CalibrationCommand::BedMeshCalibrate {
        profile: named_text(args, "PROFILE"),
        probe_count,
        round_probe_count: named_index(args, "ROUND_PROBE_COUNT")?,
    })
}

/// `30,40` or a single `30` for both axes (`parse_gcmd_pair`).
fn parse_count_pair(text: &str) -> Result<(usize, usize), CommandError> {
    let count = |part: &str| {
        part.trim().parse::<usize>().map_err(|_| CommandError::new(format!("Unable to parse PROBE_COUNT '{text}'")))
    };
    match text.split_once(',') {
        Some((x, y)) => Ok((count(x)?, count(y)?)),
        None => count(text).map(|both| (both, both)),
    }
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
