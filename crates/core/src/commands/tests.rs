use super::*;
use crate::sources::klipper::tuning::TuningKey;
use crate::sources::klipper::{Axis, CalibrationMode, Motion, PrintJob, WhiteChannel};
use crate::sources::{PrinterStatus, Sources};
use serde_json::json;

fn motion_at(position: [f64; 4]) -> Motion {
    Motion {
        position,
        axis_minimum: [0.0, 0.0, -2.0, 0.0],
        axis_maximum: [300.0, 300.0, 250.0, 0.0],
        ..Motion::default()
    }
}

fn script(command: Command) -> String {
    let call = command.to_call(&motion_at([10.0, 10.0, 5.0, 0.0])).unwrap();
    assert_eq!(call.method, "printer.gcode.script");
    call.params.unwrap()["script"].as_str().unwrap().to_owned()
}

#[test]
fn each_command_reaches_its_source() {
    let motion = Motion::default();
    let method = |command: Command| command.to_call(&motion).unwrap().method;

    assert_eq!(script(Command::Jog { axis: Axis::Z, distance: 0.1 }), "G90\nG1 Z5.1 F600");
    assert_eq!(script(Command::MmuEject { gate: Some(1) }), "MMU_EJECT GATE=1");
    assert_eq!(script(Command::MmuCheckGates { gates: mmu::Gates::All }), "MMU_CHECK_GATE ALL=1");
    assert_eq!(
        script(Command::SetLedEffect { effect: "rainbow".into(), on: false }),
        "SET_LED_EFFECT EFFECT=rainbow STOP=1"
    );
    assert_eq!(script(Command::RunMacro { name: "PARK".into() }), "PARK");
    assert_eq!(method(Command::StartPrint { filename: "a.gcode".into() }), "printer.print.start");
    assert_eq!(method(Command::DeleteFile { path: "a.gcode".into() }), "server.files.delete_file");
    assert_eq!(method(Command::SetActiveSpool { spool_id: None }), "server.spoolman.post_spool_id");
    assert_eq!(
        method(Command::SetPowerDevice { device: "printer".into(), on: true }),
        "machine.device_power.post_device"
    );
    assert_eq!(method(Command::RebootHost), "machine.reboot");
    assert_eq!(method(Command::Upgrade { name: "klipper".into() }), "machine.update.upgrade");
}

#[test]
fn commands_without_effect_are_not_sent() {
    let at_minimum = motion_at([0.0; 4]);
    assert_eq!(Command::Jog { axis: Axis::X, distance: -1.0 }.to_call(&at_minimum), None);
    assert_eq!(Command::ExcludeObject { name: "Part 2".into() }.to_call(&at_minimum), None);
    assert_eq!(Command::SetFanSpeed { fan: "heater_fan hotend".into(), speed: 1.0 }.to_call(&at_minimum), None);
}

fn printer(objects: &[&str]) -> Sources {
    let objects: Vec<String> = objects.iter().map(|name| (*name).to_owned()).collect();
    Sources::detect(&objects, Default::default())
}

fn status_with_targets(targets: &[(&str, f64)]) -> PrinterStatus {
    PrinterStatus {
        heaters: targets
            .iter()
            .map(|(object, target)| crate::sources::klipper::Heater {
                object: (*object).to_owned(),
                target: *target,
                min_temp: 10.0,
                ..crate::sources::klipper::Heater::default()
            })
            .collect(),
        ..PrinterStatus::default()
    }
}

fn resolved(command: Command, objects: &[&str]) -> Result<Vec<Command>, CommandError> {
    command.resolve(&printer(objects), &PrinterStatus::default())
}

fn set(heater: &str, target: f64) -> Command {
    Command::SetHeaterTarget { heater: heater.into(), target }
}

#[test]
fn leveling_uses_quad_gantry_level_before_z_tilt() {
    assert_eq!(resolved(Command::Level, &["quad_gantry_level", "z_tilt"]), Ok(vec![Command::QuadGantryLevel]));
    assert_eq!(resolved(Command::Level, &["z_tilt"]), Ok(vec![Command::ZTiltAdjust]));
    assert!(resolved(Command::Level, &["extruder"]).is_err());
}

#[test]
fn layer_pauses_call_mainsails_macros_with_documented_parameters_only() {
    let both = ["gcode_macro SET_PAUSE_NEXT_LAYER", "gcode_macro SET_PAUSE_AT_LAYER"];
    let gcode = |script: &str| Command::RunGcode { script: script.into() };
    let at_layer_10 =
        PrinterStatus { job: PrintJob { current_layer: Some(10), ..PrintJob::default() }, ..PrinterStatus::default() };
    let pause_at =
        |layer: u32, objects: &[&str]| Command::PauseAtLayer { layer }.resolve(&printer(objects), &at_layer_10);

    assert_eq!(pause_at(11, &both), Ok(vec![gcode("SET_PAUSE_NEXT_LAYER ENABLE=1")]));
    assert_eq!(pause_at(42, &both), Ok(vec![gcode("SET_PAUSE_AT_LAYER ENABLE=1 LAYER=42")]));
    assert!(pause_at(42, &["gcode_macro SET_PAUSE_NEXT_LAYER"]).is_err());
    assert!(resolved(Command::PauseAtLayer { layer: 42 }, &both).is_err());
    assert_eq!(
        resolved(Command::ClearLayerPause, &both),
        Ok(vec![gcode("SET_PAUSE_NEXT_LAYER ENABLE=0"), gcode("SET_PAUSE_AT_LAYER ENABLE=0")])
    );
    assert_eq!(
        resolved(Command::ClearLayerPause, &["gcode_macro SET_PAUSE_AT_LAYER"]),
        Ok(vec![gcode("SET_PAUSE_AT_LAYER ENABLE=0")])
    );
    assert!(resolved(Command::ClearLayerPause, &["extruder"]).is_err());
}

#[test]
fn the_z_offset_goes_to_the_probe_if_there_is_one() {
    assert_eq!(resolved(Command::ApplyZOffset, &["probe"]), Ok(vec![Command::ApplyZOffsetToProbe]));
    assert_eq!(resolved(Command::ApplyZOffset, &["extruder"]), Ok(vec![Command::ApplyZOffsetToEndstop]));
    assert_eq!(script(Command::ApplyZOffsetToProbe), "Z_OFFSET_APPLY_PROBE");
    assert_eq!(script(Command::ApplyZOffsetToEndstop), "Z_OFFSET_APPLY_ENDSTOP");
}

#[test]
fn the_z_height_is_calibrated_at_the_probe_else_at_the_endstop() {
    assert_eq!(resolved(Command::CalibrateZ, &["probe"]), Ok(vec![Command::ProbeCalibrate]));
    assert!(resolved(Command::CalibrateZ, &["extruder"]).is_err());
    let mut sources = printer(&["extruder"]);
    sources.klipper.calibration.z_endstop = true;
    let resolved = Command::CalibrateZ.resolve(&sources, &PrinterStatus::default());
    assert_eq!(resolved, Ok(vec![Command::ZEndstopCalibrate]));
    assert_eq!(script(Command::ZEndstopCalibrate), "Z_ENDSTOP_CALIBRATE");
}

#[test]
fn mode_commands_reach_klipper_only_while_it_waits() {
    let in_mode = |mode: Option<CalibrationMode>, command: Command| {
        let mut status = PrinterStatus::default();
        status.calibration.mode = mode;
        command.resolve(&printer(&["probe"]), &status).map(|commands| commands.len())
    };
    let probing = || Some(CalibrationMode::ManualProbe { z: 1.0, lower: None, upper: None });
    let screws =
        || Some(CalibrationMode::BedScrews { screw: "front left".into(), fine: false, accepted: 0, screws: 4 });
    let test_z = || Command::TestZ { step: TestZ::By(0.1) };

    assert_eq!(in_mode(probing(), test_z()), Ok(1));
    assert_eq!(in_mode(screws(), test_z()), Ok(0));
    assert_eq!(in_mode(screws(), Command::ScrewAdjusted), Ok(1));
    assert_eq!(in_mode(probing(), Command::ScrewAdjusted), Ok(0));
    assert_eq!(in_mode(probing(), Command::AbortCalibration), Ok(1));
    assert_eq!(in_mode(None, Command::AbortCalibration), Ok(0));
    assert_eq!(in_mode(None, Command::AcceptCalibration), Ok(0));
    assert_eq!(script(Command::AbortCalibration), "ABORT");
}

#[test]
fn a_preset_heats_the_bed_only_if_there_is_one() {
    let preset = Command::ApplyPreset { extruder: 210.0, bed: 60.0 };

    let with_bed = status_with_targets(&[("extruder", 0.0), ("heater_bed", 0.0)]);
    let without_bed = status_with_targets(&[("extruder", 0.0)]);

    assert_eq!(
        preset.clone().resolve(&printer(&[]), &with_bed),
        Ok(vec![set("extruder", 210.0), set("heater_bed", 60.0)])
    );
    assert_eq!(preset.resolve(&printer(&[]), &without_bed), Ok(vec![set("extruder", 210.0)]));
}

#[test]
fn cooling_down_switches_every_heater_off() {
    let status = status_with_targets(&[("extruder", 210.0), ("heater_bed", 60.0), ("heater_generic chamber", 40.0)]);

    let commands = Command::CoolDown.resolve(&printer(&[]), &status);

    assert_eq!(commands, Ok(vec![set("extruder", 0.0), set("heater_bed", 0.0), set("heater_generic chamber", 0.0)]));
}

#[test]
fn a_step_starts_from_the_target_shown_now() {
    let status = status_with_targets(&[("extruder", 200.0)]);
    let step = |step| Command::StepHeater { heater: "extruder".into(), step };

    assert_eq!(step(5.0).resolve(&printer(&[]), &status), Ok(vec![set("extruder", 205.0)]));
    assert_eq!(step(-5.0).resolve(&printer(&[]), &status), Ok(vec![set("extruder", 195.0)]));
    assert!(Command::StepHeater { heater: "nozzle".into(), step: 5.0 }.resolve(&printer(&[]), &status).is_err());
}

#[test]
fn loading_prefers_happy_hare_then_the_macros() {
    let macro_named = |name: &str| Command::RunMacro { name: name.into() };
    let happy_hare = PrinterStatus {
        mmu: Some(crate::sources::happy_hare::MmuStatus {
            version: "4.1.0".into(),
            enabled: true,
            ..Default::default()
        }),
        ..PrinterStatus::default()
    };
    let with_happy_hare = |command: Command| command.resolve(&printer(&["gcode_macro LOAD_FILAMENT"]), &happy_hare);

    assert_eq!(with_happy_hare(Command::LoadFilament), Ok(vec![Command::MmuLoad]));
    assert_eq!(with_happy_hare(Command::UnloadFilament), Ok(vec![Command::MmuUnload]));
    assert_eq!(resolved(Command::LoadFilament, &["gcode_macro LOAD_FILAMENT"]), Ok(vec![macro_named("LOAD_FILAMENT")]));
    assert_eq!(
        resolved(Command::UnloadFilament, &["gcode_macro UNLOAD_FILAMENT"]),
        Ok(vec![macro_named("UNLOAD_FILAMENT")])
    );
    assert!(resolved(Command::LoadFilament, &["gcode_macro UNLOAD_FILAMENT"]).is_err());
    assert!(resolved(Command::UnloadFilament, &["extruder"]).is_err());
}

#[test]
fn a_gate_is_loaded_after_selecting_it_in_the_same_script() {
    assert_eq!(script(Command::MmuLoadGate { gate: 2 }), "MMU_SELECT GATE=2\nMMU_LOAD");
}

#[test]
fn the_filament_of_a_gate_is_set_unless_happy_hare_takes_it_from_spoolman() {
    let with_support = |spoolman_support: &str| PrinterStatus {
        mmu: Some(crate::sources::happy_hare::MmuStatus {
            spoolman_support: spoolman_support.into(),
            gates: vec![crate::sources::happy_hare::MmuGate { temperature: Some(215.0), ..Default::default() }],
            ..Default::default()
        }),
        ..PrinterStatus::default()
    };
    let red = |gate| Command::SetGateFilament { gate, material: None, color: Some([255, 0, 0]) };

    assert_eq!(
        red(0).resolve(&printer(&[]), &with_support("off")),
        Ok(vec![Command::RunGcode { script: "MMU_GATE_MAP GATE=0 COLOR=ff0000 TEMP=215".into() }])
    );
    assert!(red(0).resolve(&printer(&[]), &with_support("pull")).is_err());
    assert!(red(1).resolve(&printer(&[]), &with_support("off")).is_err());
    assert!(red(0).resolve(&printer(&[]), &PrinterStatus::default()).is_err());
}

#[test]
fn bypass_tool_map_and_endless_spool_are_happy_hare_gcode() {
    assert_eq!(script(Command::MmuSelectBypass), "MMU_SELECT BYPASS=1");
    assert_eq!(script(Command::MmuLoadBypass), "MMU_SELECT BYPASS=1\nMMU_LOAD");
    assert_eq!(script(Command::MmuMapTool { tool: 3, gate: 1 }), "MMU_TTG_MAP TOOL=3 GATE=1");
    assert_eq!(script(Command::MmuResetToolMap), "MMU_TTG_MAP RESET=1");
    assert_eq!(script(Command::MmuEndlessSpool { enabled: true }), "MMU_ENDLESS_SPOOL ENABLE=1");
}

#[test]
fn an_endless_spool_group_is_sent_with_the_groups_of_all_other_gates() {
    let with_groups = |groups: &[Option<u32>]| PrinterStatus {
        mmu: Some(crate::sources::happy_hare::MmuStatus {
            gates: groups
                .iter()
                .enumerate()
                .map(|(index, group)| crate::sources::happy_hare::MmuGate {
                    index,
                    endless_spool_group: *group,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
        ..PrinterStatus::default()
    };
    let move_gate = |gate| Command::SetEndlessSpoolGroup { gate, group: 0 };
    let groups = [Some(0), Some(1), Some(2)];

    assert_eq!(
        move_gate(2).resolve(&printer(&[]), &with_groups(&groups)),
        Ok(vec![Command::RunGcode { script: "MMU_ENDLESS_SPOOL GROUPS=0,1,0".into() }])
    );
    assert!(move_gate(3).resolve(&printer(&[]), &with_groups(&groups)).is_err());
    assert!(move_gate(0).resolve(&printer(&[]), &with_groups(&[None, None])).is_err());
}

#[test]
fn typed_gcode_is_trimmed_and_blank_input_is_dropped() {
    let gcode = |script: &str| Command::RunGcode { script: script.into() };

    assert_eq!(resolved(gcode("  G28 \n"), &[]), Ok(vec![gcode("G28")]));
    assert_eq!(resolved(gcode("   "), &[]), Ok(Vec::new()));
}

#[test]
fn other_commands_stay_as_they_are() {
    assert_eq!(resolved(Command::PausePrint, &[]), Ok(vec![Command::PausePrint]));
}

#[test]
fn intents_are_no_calls_of_their_own() {
    assert_eq!(Command::Level.to_call(&Motion::default()), None);
    assert_eq!(Command::CoolDown.to_call(&Motion::default()), None);
}

#[test]
fn spool_commands_make_the_spools_be_read_again() {
    assert!(Command::MeasureSpool { spool_id: 1, weight: 1.0 }.changes_spools());
    assert!(Command::AssignGateSpool { gate: 0, spool_id: None }.changes_spools());
    assert!(!Command::PausePrint.changes_spools());
}

#[test]
fn light_uses_white_where_there_is_one_and_toggles_the_main_light() {
    let mut sources = printer(&["neopixel sb", "neopixel chamber"]);
    sources.klipper.led_white =
        vec![("neopixel chamber".into(), WhiteChannel::All), ("neopixel sb".into(), WhiteChannel::Some)];
    let mut status = PrinterStatus {
        leds: vec![crate::sources::klipper::Led {
            object: "neopixel chamber".into(),
            label: "Chamber".into(),
            on: false,
            level: Some(0.0),
        }],
        ..PrinterStatus::default()
    };

    let on = Command::ToggleLight.resolve(&sources, &status).unwrap();
    assert_eq!(script(on[0].clone()), "SET_LED LED=chamber RED=0.00 GREEN=0.00 BLUE=0.00 WHITE=1.00 SYNC=0");
    status.leds[0].on = true;
    let off = Command::ToggleLight.resolve(&sources, &status).unwrap();
    assert_eq!(script(off[0].clone()), "SET_LED LED=chamber RED=0.00 GREEN=0.00 BLUE=0.00 WHITE=0.00 SYNC=0");
    let mixed = Command::SetLight { led: "neopixel sb".into(), level: 0.5 }.resolve(&sources, &status).unwrap();
    assert_eq!(script(mixed[0].clone()), "SET_LED LED=sb RED=0.50 GREEN=0.50 BLUE=0.50 WHITE=0.50 SYNC=0");
    let rgb = Command::SetLight { led: "neopixel status".into(), level: 0.5 }.resolve(&sources, &status).unwrap();
    assert_eq!(script(rgb[0].clone()), "SET_LED LED=status RED=0.50 GREEN=0.50 BLUE=0.50 WHITE=0.00 SYNC=0");
    assert!(Command::ToggleLight.resolve(&printer(&[]), &status).is_err());
}

#[test]
fn a_wled_strip_is_switched_and_dimmed_by_moonraker() {
    let mut sources = printer(&["neopixel sb"]);
    sources.wled_strips = vec!["wled chamber".into()];
    let strip = wled::WledStrip { name: "chamber".into(), on: false, brightness: None };
    let mut status = PrinterStatus { leds: vec![strip.led()], ..PrinterStatus::default() };
    let resolve = |command: Command, status: &PrinterStatus| command.resolve(&sources, status).unwrap();
    let wled = |action| vec![Command::SetWledStrip { strip: "chamber".into(), action }];
    let dim = Command::SetLight { led: "wled chamber".into(), level: 0.5 };

    assert_eq!(resolve(Command::ToggleLight, &status), wled(StripAction::On));
    assert_eq!(resolve(dim.clone(), &status), wled(StripAction::OnAt(0.5)));
    status.leds[0].on = true;
    assert_eq!(resolve(Command::ToggleLight, &status), wled(StripAction::Off));
    assert_eq!(resolve(dim, &status), wled(StripAction::Dim(0.5)));
    let call = Command::SetWledStrip { strip: "chamber".into(), action: StripAction::Off }
        .to_call(&Motion::default())
        .unwrap();
    assert_eq!(call.method, "machine.wled.post_strip");
}

#[test]
fn tuning_steps_from_the_current_value_and_resets_to_the_configuration() {
    let mut sources = printer(&[]);
    sources.apply_settings(Some(&json!({ "printer": { "max_accel": 5000.0 } })));
    let mut status = PrinterStatus::default();
    status.motion.max_accel = 3000.0;

    let step = Command::StepTuning { key: TuningKey::MaxAccel, direction: 1.0 }.resolve(&sources, &status).unwrap();
    assert_eq!(step, [Command::SetTuning { key: TuningKey::MaxAccel, value: 3500.0 }]);
    let reset = Command::ResetTuning { key: TuningKey::MaxAccel }.resolve(&sources, &status).unwrap();
    assert_eq!(script(reset[0].clone()), "SET_VELOCITY_LIMIT ACCEL=5000");
    assert!(Command::ResetTuning { key: TuningKey::SmoothTime }.resolve(&sources, &status).is_err());
    assert!(Command::StepTuning { key: TuningKey::RetractLength, direction: 1.0 }.resolve(&sources, &status).is_err());
}

#[test]
fn a_heater_target_is_never_negative() {
    assert_eq!(resolved(set("heater_bed", -5.0), &["heater_bed"]), Ok(vec![set("heater_bed", 0.0)]));
}
