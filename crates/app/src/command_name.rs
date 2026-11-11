//! Names of commands for messages to people ("Pause Print: not possible while ..."). Fixed per command, so
//! renaming a variant does not change what the display shows.

use freakymate_core::Command;

/// Short name of a command for feedback in the UI; a macro or a G-code script shows what it runs.
pub fn command_name(command: &Command) -> String {
    match command {
        Command::StartPrint { .. } => "Start Print".to_owned(),
        Command::DeleteFile { .. } => "Delete File".to_owned(),
        Command::PausePrint => "Pause Print".to_owned(),
        Command::ResumePrint => "Resume Print".to_owned(),
        Command::CancelPrint => "Cancel Print".to_owned(),
        Command::PauseAtLayer { .. } => "Pause At Layer".to_owned(),
        Command::ClearLayerPause => "Clear Layer Pause".to_owned(),
        Command::Home { .. } => "Home".to_owned(),
        Command::Jog { .. } => "Jog".to_owned(),
        Command::DisableMotors => "Disable Motors".to_owned(),
        Command::Level => "Level".to_owned(),
        Command::QuadGantryLevel => "Quad Gantry Level".to_owned(),
        Command::ZTiltAdjust => "Z Tilt Adjust".to_owned(),
        Command::SetHeaterTarget { .. } => "Set Heater Target".to_owned(),
        Command::StepHeater { .. } => "Step Heater".to_owned(),
        Command::ApplyPreset { .. } => "Apply Preset".to_owned(),
        Command::CoolDown => "Cool Down".to_owned(),
        Command::PickUpHead { .. } => "Pick Up Head".to_owned(),
        Command::DropHead => "Drop Head".to_owned(),
        Command::InitializeToolchanger { .. } => "Initialize Tool Changer".to_owned(),
        Command::SetFanSpeed { .. } => "Set Fan Speed".to_owned(),
        Command::SetLight { .. } => "Set Light".to_owned(),
        Command::ToggleLight => "Toggle Light".to_owned(),
        Command::SetLed { .. } => "Set Led".to_owned(),
        Command::SetLedEffect { .. } => "Set Led Effect".to_owned(),
        Command::AdjustZOffset { .. } => "Adjust Z Offset".to_owned(),
        Command::ResetZOffset => "Reset Z Offset".to_owned(),
        Command::ApplyZOffset => "Apply Z Offset".to_owned(),
        Command::ApplyZOffsetToProbe => "Apply Z Offset To Probe".to_owned(),
        Command::ApplyZOffsetToEndstop => "Apply Z Offset To Endstop".to_owned(),
        Command::SetSpeedFactor { .. } => "Set Speed Factor".to_owned(),
        Command::SetTuning { .. } => "Set Tuning".to_owned(),
        Command::StepTuning { .. } => "Step Tuning".to_owned(),
        Command::ResetTuning { .. } => "Reset Tuning".to_owned(),
        Command::SaveConfig => "Save Config".to_owned(),
        Command::CalibrateBedMesh => "Calibrate Bed Mesh".to_owned(),
        Command::LoadBedMesh { .. } => "Load Bed Mesh".to_owned(),
        Command::ClearBedMesh => "Clear Bed Mesh".to_owned(),
        Command::SaveBedMesh { .. } => "Save Bed Mesh".to_owned(),
        Command::RemoveBedMesh { .. } => "Remove Bed Mesh".to_owned(),
        Command::CalculateScrewsTilt => "Calculate Screws Tilt".to_owned(),
        Command::AdjustBedScrews => "Adjust Bed Screws".to_owned(),
        Command::CalibrateZ => "Calibrate Z".to_owned(),
        Command::ProbeCalibrate => "Probe Calibrate".to_owned(),
        Command::ZEndstopCalibrate => "Z Endstop Calibrate".to_owned(),
        Command::TestZ { .. } => "Test Z".to_owned(),
        Command::AcceptCalibration => "Accept Calibration".to_owned(),
        Command::ScrewAdjusted => "Screw Adjusted".to_owned(),
        Command::AbortCalibration => "Abort Calibration".to_owned(),
        Command::Extrude { .. } => "Extrude".to_owned(),
        Command::LoadFilament => "Load Filament".to_owned(),
        Command::UnloadFilament => "Unload Filament".to_owned(),
        Command::RunMacro { name } => name.clone(),
        Command::RunGcode { script } => script.lines().next().unwrap_or_default().to_owned(),
        Command::ExcludeObject { .. } => "Exclude Object".to_owned(),
        Command::MmuSelectGate { .. } => "Mmu Select Gate".to_owned(),
        Command::MmuLoad => "Mmu Load".to_owned(),
        Command::MmuLoadGate { .. } => "Mmu Load Gate".to_owned(),
        Command::MmuUnload => "Mmu Unload".to_owned(),
        Command::MmuEject { .. } => "Mmu Eject".to_owned(),
        Command::MmuHome => "Mmu Home".to_owned(),
        Command::MmuRecover => "Mmu Recover".to_owned(),
        Command::MmuUnlock => "Mmu Unlock".to_owned(),
        Command::MmuMotorsOff => "Mmu Motors Off".to_owned(),
        Command::MmuCheckGates { .. } => "Mmu Check Gates".to_owned(),
        Command::MmuPreload { .. } => "Mmu Preload".to_owned(),
        Command::MmuServo { .. } => "Mmu Servo".to_owned(),
        Command::MmuSelectBypass => "Mmu Select Bypass".to_owned(),
        Command::MmuLoadBypass => "Mmu Load Bypass".to_owned(),
        Command::MmuMapTool { .. } => "Mmu Map Tool".to_owned(),
        Command::MmuResetToolMap => "Mmu Reset Tool Map".to_owned(),
        Command::MmuEndlessSpool { .. } => "Mmu Endless Spool".to_owned(),
        Command::SetEndlessSpoolGroup { .. } => "Set Endless Spool Group".to_owned(),
        Command::AssignGateSpool { .. } => "Assign Gate Spool".to_owned(),
        Command::SetGateFilament { .. } => "Set Gate Filament".to_owned(),
        Command::LoadLane { .. } => "Load Lane".to_owned(),
        Command::UnloadLane { .. } => "Unload Lane".to_owned(),
        Command::EjectLane { .. } => "Eject Lane".to_owned(),
        Command::AssignLaneSpool { .. } => "Assign Lane Spool".to_owned(),
        Command::SetLaneFilament { .. } => "Set Lane Filament".to_owned(),
        Command::MapToolToLane { .. } => "Map Tool To Lane".to_owned(),
        Command::ResetToolMap => "Reset Tool Map".to_owned(),
        Command::AfcResetFailure => "AFC Reset Failure".to_owned(),
        Command::AfcResetLane { .. } => "AFC Reset Lane".to_owned(),
        Command::SetActiveSpool { .. } => "Set Active Spool".to_owned(),
        Command::AssignToolSpool { .. } => "Assign Tool Spool".to_owned(),
        Command::MeasureSpool { .. } => "Measure Spool".to_owned(),
        Command::EmergencyStop => "Emergency Stop".to_owned(),
        Command::RestartKlipper => "Restart Klipper".to_owned(),
        Command::FirmwareRestart => "Firmware Restart".to_owned(),
        Command::SetPowerDevice { .. } => "Set Power Device".to_owned(),
        Command::SetWledStrip { .. } => "Set Wled Strip".to_owned(),
        Command::RestartService { .. } => "Restart Service".to_owned(),
        Command::RebootHost => "Reboot Host".to_owned(),
        Command::ShutdownHost => "Shutdown Host".to_owned(),
        Command::RefreshUpdates => "Refresh Updates".to_owned(),
        Command::Upgrade { name } => format!("Upgrade {name}"),
    }
}

/// What a running command is called in the status bar: the first line of its G-code script, as in the console,
/// otherwise the command's name.
pub fn running_label(command: &Command, script_line: Option<&str>) -> String {
    script_line.map_or_else(|| command_name(command), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use freakymate_core::sources::klipper::Axis;

    #[test]
    fn names_commands_briefly() {
        assert_eq!(command_name(&Command::PausePrint), "Pause Print");
        assert_eq!(command_name(&Command::RefreshUpdates), "Refresh Updates");
        assert_eq!(command_name(&Command::Jog { axis: Axis::X, distance: 1.0 }), "Jog");
        assert_eq!(command_name(&Command::RunMacro { name: "PARK".into() }), "PARK");
        assert_eq!(command_name(&Command::Upgrade { name: "klipper".into() }), "Upgrade klipper");
    }

    #[test]
    fn a_running_script_is_named_by_its_first_line() {
        assert_eq!(running_label(&Command::Home { x: true, y: true, z: true }, Some("G28")), "G28");
        assert_eq!(running_label(&Command::RebootHost, None), "Reboot Host");
    }
}
