use super::*;
use crate::files::Catalog;
use crate::gcode::parse_script;

fn run(printer: &mut Printer, script: &str) -> Result<(), CommandError> {
    parse_script(script)?.into_iter().try_for_each(|command| printer.apply(command))
}

fn printer() -> Printer {
    Printer::new(1.0)
}

#[test]
fn extruder_heats_up_to_target_and_stops() {
    let mut printer = printer();
    run(&mut printer, "M104 S60").unwrap();

    printer.advance(Duration::from_secs(5));
    assert_eq!(printer.status()["extruder"]["temperature"], 39.0);

    printer.advance(Duration::from_secs(60));
    assert_eq!(printer.status()["extruder"]["temperature"], 60.0);
}

#[test]
fn cools_down_to_ambient_temperature() {
    let mut printer = printer();
    run(&mut printer, "M140 S40").unwrap();
    printer.advance(Duration::from_secs(60));
    run(&mut printer, "M140 S0").unwrap();

    printer.advance(Duration::from_secs(3600));

    assert_eq!(printer.status()["heater_bed"]["temperature"], 24.0);
}

#[test]
fn rejects_temperature_above_limit() {
    let error = run(&mut printer(), "M104 S350").unwrap_err();

    assert_eq!(error.message(), "Requested temperature (350.0) out of range (10.0:300.0)");
}

#[test]
fn idle_timeout_goes_from_busy_to_ready_to_idle_and_switches_off() {
    let mut printer = printer();
    assert_eq!(printer.status()["idle_timeout"]["state"], "Ready");

    run(&mut printer, "M104 S200\nG28").unwrap();
    assert_eq!(printer.status()["idle_timeout"]["state"], "Printing");

    printer.advance(Duration::from_secs(10));
    assert_eq!(printer.status()["idle_timeout"]["state"], "Ready");

    printer.advance(Duration::from_secs(600));
    let status = printer.status();
    assert_eq!(status["idle_timeout"]["state"], "Idle");
    assert_eq!(status["extruder"]["target"], 0.0);
    assert_eq!(status["toolhead"]["homed_axes"], "");
}

#[test]
fn moving_requires_homing() {
    let error = run(&mut printer(), "G90\nG0 X10").unwrap_err();

    assert_eq!(error.message(), "Must home axis first");
}

#[test]
fn moves_after_homing_and_reports_homed_axes() {
    let mut printer = printer();

    run(&mut printer, "G28 X Y\nG90\nG0 X10 Y20").unwrap();

    let status = printer.status();
    assert_eq!(status["toolhead"]["homed_axes"], "xy");
    assert_eq!(status["toolhead"]["position"], json!([10.0, 20.0, 0.0, 0.0]));
}

#[test]
fn rejects_moves_out_of_range() {
    let mut printer = printer();
    run(&mut printer, "G28").unwrap();

    let error = run(&mut printer, "G0 X500").unwrap_err();

    assert_eq!(error.message(), "Move out of range");
}

#[test]
fn extrusion_needs_a_hot_nozzle() {
    let mut printer = printer();
    assert!(run(&mut printer, "G91\nG1 E10").is_err());

    run(&mut printer, "M104 S200").unwrap();
    printer.advance(Duration::from_secs(120));

    run(&mut printer, "G1 E10").unwrap();
    assert_eq!(printer.status()["extruder"]["can_extrude"], true);
}

#[test]
fn gcode_offset_adjusts_homing_origin() {
    let mut printer = printer();

    run(&mut printer, "SET_GCODE_OFFSET Z_ADJUST=0.05\nSET_GCODE_OFFSET Z_ADJUST=0.05").unwrap();

    assert_eq!(printer.status()["gcode_move"]["homing_origin"], json!([0.0, 0.0, 0.1, 0.0]));
}

#[test]
fn print_runs_to_completion_and_switches_heaters_off() {
    let mut printer = printer();
    let file = Catalog::demo().find("calibration_cube.gcode").unwrap().clone();

    printer.start_print(&file).unwrap();
    assert_eq!(printer.print_state(), PrintState::Printing);
    assert_eq!(printer.status()["extruder"]["target"], 210.0);

    printer.advance(Duration::from_secs(750));
    assert_eq!(printer.status()["display_status"]["progress"], 0.5);
    assert_eq!(printer.status()["print_stats"]["info"]["current_layer"], 51);

    printer.advance(Duration::from_secs(750));
    assert_eq!(printer.print_state(), PrintState::Complete);
    assert_eq!(printer.status()["extruder"]["target"], 0.0);
}

#[test]
fn a_planned_layer_pause_pauses_the_print_at_that_layer() {
    let mut printer = printer();
    let file = Catalog::demo().find("calibration_cube.gcode").unwrap().clone();
    printer.start_print(&file).unwrap();
    run(&mut printer, "SET_PAUSE_AT_LAYER LAYER=30").unwrap();
    assert_eq!(printer.status()["gcode_macro SET_PRINT_STATS_INFO"]["pause_at_layer"]["enable"], true);

    printer.advance(Duration::from_secs(750));

    assert_eq!(printer.print_state(), PrintState::Paused);
    assert_eq!(printer.status()["gcode_macro SET_PRINT_STATS_INFO"]["pause_at_layer"]["enable"], false);
}

#[test]
fn a_print_error_ends_the_job_with_klippers_message() {
    let mut printer = printer();
    let file = Catalog::demo().find("benchy.gcode").unwrap().clone();
    printer.start_print(&file).unwrap();

    run(&mut printer, "FAKE_PRINT_ERROR").unwrap();

    let status = printer.status();
    assert_eq!(status["print_stats"]["state"], "error");
    assert_eq!(status["print_stats"]["message"], PRINT_ERROR_MESSAGE);
    assert!(run(&mut printer, "FAKE_PRINT_ERROR").is_err());
}

#[test]
fn paused_print_does_not_progress() {
    let mut printer = printer();
    let file = Catalog::demo().find("benchy.gcode").unwrap().clone();
    printer.start_print(&file).unwrap();

    printer.pause_print().unwrap();
    printer.advance(Duration::from_secs(600));

    let status = printer.status();
    assert_eq!(status["print_stats"]["state"], "paused");
    assert_eq!(status["print_stats"]["print_duration"], 0.0);
    assert_eq!(status["print_stats"]["total_duration"], 600.0);
}

#[test]
fn cancel_requires_an_active_print() {
    let mut printer = printer();

    assert!(printer.cancel_print().is_err());
}

#[test]
fn rejects_second_print_while_one_is_running() {
    let mut printer = printer();
    let file = Catalog::demo().find("benchy.gcode").unwrap().clone();
    printer.start_print(&file).unwrap();

    let error = printer.start_print(&file).unwrap_err();

    assert_eq!(error.message(), "Print job already in progress");
}

#[test]
fn emergency_stop_shuts_down_until_restart() {
    let mut printer = printer();
    let file = Catalog::demo().find("benchy.gcode").unwrap().clone();
    printer.start_print(&file).unwrap();

    printer.emergency_stop();

    assert_eq!(printer.status()["webhooks"]["state"], "shutdown");
    assert_eq!(printer.print_state(), PrintState::Paused);
    assert!(run(&mut printer, "G28").is_err());

    printer.restart();
    assert_eq!(printer.status()["webhooks"]["state"], "ready");
    assert!(run(&mut printer, "G28").is_ok());
}

fn names(printer: &Printer) -> Vec<String> {
    printer.object_names()
}

#[test]
fn basic_profile_has_no_optional_modules() {
    let printer = Printer::with_profile(1.0, MachineProfile::basic());

    let names = names(&printer);

    assert!(names.contains(&"extruder".to_owned()));
    assert!(!names.contains(&"quad_gantry_level".to_owned()));
    assert!(!names.iter().any(|name| name.starts_with("mmu")));
    assert!(printer.status()["configfile"]["settings"].get("quad_gantry_level").is_none());
}

#[test]
fn voron_profile_adds_quad_gantry_level() {
    let printer = Printer::with_profile(1.0, MachineProfile::voron());

    assert!(names(&printer).contains(&"quad_gantry_level".to_owned()));
    assert_eq!(printer.status()["configfile"]["settings"]["quad_gantry_level"]["max_adjust"], 10);
}

#[test]
fn exclude_object_lists_objects_and_skips_them() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::voron());
    assert!(names(&printer).contains(&"exclude_object".to_owned()));
    assert_eq!(printer.status()["exclude_object"]["objects"], json!([]), "no file has defined objects yet");
    assert_eq!(printer.status()["exclude_object"]["current_object"], Value::Null, "nothing printed in standby");
    printer.start_print(&Catalog::demo().find("benchy.gcode").unwrap().clone()).unwrap();
    assert_eq!(printer.status()["exclude_object"]["objects"].as_array().unwrap().len(), print_objects::count());

    run(&mut printer, "EXCLUDE_OBJECT NAME=benchy_1\nEXCLUDE_OBJECT NAME=BENCHY_1").unwrap();
    assert_eq!(printer.status()["exclude_object"]["excluded_objects"], json!(["BENCHY_1"]));

    run(&mut printer, "EXCLUDE_OBJECT NAME=unknown").unwrap();
    run(&mut printer, "EXCLUDE_OBJECT NAME=BENCHY_0").unwrap();
    assert_eq!(printer.status()["exclude_object"]["excluded_objects"], json!(["BENCHY_0", "BENCHY_1", "UNKNOWN"]));

    run(&mut printer, "EXCLUDE_OBJECT").unwrap();
    assert_eq!(printer.status()["exclude_object"]["excluded_objects"].as_array().unwrap().len(), 3);
}

#[test]
fn a_print_reports_the_object_being_printed() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::voron());
    let file = Catalog::demo().find("benchy.gcode").unwrap().clone();
    printer.start_print(&file).unwrap();
    run(&mut printer, "EXCLUDE_OBJECT NAME=BENCHY_0").unwrap();
    let current = printer.status()["exclude_object"]["current_object"].clone();
    assert!(current.is_string() && current != "BENCHY_0", "an excluded object is not printed: {current}");
}

#[test]
fn a_file_without_object_labels_defines_no_objects() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::voron());
    printer.start_print(&Catalog::demo().find("calibration_cube.gcode").unwrap().clone()).unwrap();
    assert_eq!(printer.status()["exclude_object"]["objects"], json!([]));
    assert_eq!(printer.status()["exclude_object"]["current_object"], Value::Null);
}

#[test]
fn basic_profile_has_no_exclude_object() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::basic());
    assert!(!names(&printer).contains(&"exclude_object".to_owned()));
    assert!(run(&mut printer, "EXCLUDE_OBJECT NAME=BENCHY_0").is_ok());
}

#[test]
fn happy_hare_profile_registers_mmu_objects_and_config() {
    let printer = Printer::with_profile(1.0, MachineProfile::happy_hare(6));

    let status = printer.status();

    assert!(names(&printer).contains(&"mmu".to_owned()));
    assert_eq!(status["mmu"]["num_gates"], 6);
    assert_eq!(status["mmu_machine"]["num_gates"], 6);
    assert!(status["configfile"]["settings"].get("mmu_machine").is_some());
}

#[test]
fn raw_config_holds_the_same_sections_as_strings() {
    let status = printer().status();

    assert_eq!(status["configfile"]["settings"]["extruder"]["max_temp"], 300.0);
    assert_eq!(status["configfile"]["config"]["extruder"]["max_temp"], "300.0");
}

#[test]
fn mmu_commands_are_unknown_without_happy_hare() {
    let error = run(&mut printer(), "MMU_HOME").unwrap_err();

    assert_eq!(error.message(), "Unknown command:\"MMU_HOME\"");
}

#[test]
fn tool_change_runs_through_the_printer_with_happy_hare() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::happy_hare(4));

    run(&mut printer, "MMU_HOME\nT1").unwrap();

    let status = printer.status();
    assert_eq!(status["mmu"]["tool"], 1);
    assert_eq!(status["mmu"]["filament"], "Loaded");
}

#[test]
fn restart_keeps_the_machine_profile() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::happy_hare(4));
    run(&mut printer, "MMU_HOME").unwrap();

    printer.restart();

    assert_eq!(printer.status()["mmu"]["is_homed"], false);
}
