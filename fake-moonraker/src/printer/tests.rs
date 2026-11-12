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

fn toolchanger_printer() -> Printer {
    Printer::with_profile(1.0, MachineProfile::toolchanger())
}

#[test]
fn toolchanger_profile_lists_its_objects() {
    let names = toolchanger_printer().object_names();

    for expected in [
        "extruder",
        "extruder1",
        "extruder2",
        "toolchanger",
        "tool T0",
        "tool T1",
        "tool T2",
        "gcode_macro T0",
        "gcode_macro T1",
        "gcode_macro T2",
        "save_variables",
    ] {
        assert!(names.iter().any(|name| name == expected), "missing {expected}");
    }
    assert!(!Printer::with_profile(1.0, MachineProfile::voron()).object_names().contains(&"toolchanger".to_owned()));
}

#[test]
fn toolchanger_starts_uninitialized_with_tool_0_on_the_carriage_and_known_spools() {
    let status = toolchanger_printer().status();

    // `_handle_connect`: status `uninitialized`, no active tool; the detection still sees T0.
    assert_eq!(status["toolchanger"]["status"], "uninitialized");
    assert_eq!(status["toolchanger"]["tool"], Value::Null);
    assert_eq!(status["toolchanger"]["tool_number"], -1);
    assert_eq!(status["toolchanger"]["detected_tool"], "tool T0");
    assert_eq!(status["toolchanger"]["detected_tool_number"], 0);
    assert_eq!(status["configfile"]["settings"]["toolchanger"]["initialize_on"], "first-use");
    assert_eq!(status["configfile"]["settings"]["toolchanger"]["require_tool_present"], false);
    assert!(status["configfile"]["settings"]["toolchanger"].get("error_gcode").is_none());
    assert_eq!(status["toolchanger"]["tool_names"], json!(["tool T0", "tool T1", "tool T2"]));
    assert_eq!(status["tool T0"]["detect_state"], "mounted");
    assert_eq!(status["tool T1"]["gcode_z_offset"], 0.3);
    assert_eq!(status["toolhead"]["extruder"], "extruder");
    assert_eq!(status["gcode_macro T0"]["spool_id"], 1);
    assert_eq!(status["gcode_macro T1"]["spool_id"], 3);
    assert_eq!(status["gcode_macro T2"]["spool_id"], Value::Null);
}

#[test]
fn tool_macro_switches_the_tool_and_the_toolhead_extruder() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "T1").unwrap();

    let status = printer.status();
    assert_eq!(status["toolchanger"]["tool"], "tool T1");
    assert_eq!(status["toolchanger"]["detected_tool_number"], 1);
    assert_eq!(status["tool T1"]["active"], true);
    assert_eq!(status["tool T1"]["detect_state"], "mounted");
    assert_eq!(status["tool T0"]["detect_state"], "absent");
    assert_eq!(status["toolhead"]["extruder"], "extruder1");
}

#[test]
fn select_tool_accepts_number_and_name_and_rejects_unknown_tools() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "SELECT_TOOL T=2").unwrap();
    assert_eq!(printer.status()["toolchanger"]["tool_number"], 2);
    run(&mut printer, "SELECT_TOOL T=1").unwrap();
    assert_eq!(printer.status()["toolchanger"]["tool_number"], 1);

    assert_eq!(run(&mut printer, "SELECT_TOOL T=7").unwrap_err().message(), "Select tool: T7 not found");
    assert_eq!(run(&mut printer, "SELECT_TOOL TOOL=T1").unwrap_err().message(), "Unknown config object 'T1'");
    assert_eq!(
        run(&mut printer, "SELECT_TOOL").unwrap_err().message(),
        "Select tool: Either TOOL or T needs to be specified"
    );
}

#[test]
fn unselect_tool_leaves_no_tool_but_keeps_the_toolhead_extruder() {
    let mut printer = toolchanger_printer();
    run(&mut printer, "T2\nUNSELECT_TOOL").unwrap();

    let status = printer.status();
    assert_eq!(status["toolchanger"]["tool"], Value::Null);
    assert_eq!(status["toolchanger"]["tool_number"], -1);
    assert_eq!(status["tool T2"]["detect_state"], "absent");
    assert_eq!(status["toolhead"]["extruder"], "extruder2");
}

#[test]
fn m104_with_t_heats_that_extruder_whichever_is_active() {
    let mut printer = toolchanger_printer();
    run(&mut printer, "T1\nM104 T0 S190\nM104 T2 S180").unwrap();

    let status = printer.status();
    assert_eq!(status["extruder"]["target"], 190.0);
    assert_eq!(status["extruder1"]["target"], 0.0);
    assert_eq!(status["extruder2"]["target"], 180.0);
    assert!(run(&mut printer, "M104 T5 S180").is_err());
}

#[test]
fn heating_follows_the_active_extruder() {
    let mut printer = toolchanger_printer();
    run(&mut printer, "T1\nM104 S200").unwrap();
    printer.advance(Duration::from_secs(10));

    let status = printer.status();
    assert_eq!(status["extruder1"]["target"], 200.0);
    assert_eq!(status["extruder"]["target"], 0.0);
    assert!(status["extruder1"]["temperature"].as_f64().unwrap() > 30.0);

    run(&mut printer, "SET_HEATER_TEMPERATURE HEATER=extruder2 TARGET=180").unwrap();
    assert_eq!(printer.status()["extruder2"]["target"], 180.0);
}

#[test]
fn set_gcode_variable_changes_a_macro_variable() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "SET_GCODE_VARIABLE MACRO=T2 VARIABLE=spool_id VALUE=5").unwrap();
    assert_eq!(printer.status()["gcode_macro T2"]["spool_id"], 5);

    run(&mut printer, "SET_GCODE_VARIABLE MACRO=T1 VARIABLE=spool_id VALUE=None").unwrap();
    assert_eq!(printer.status()["gcode_macro T1"]["spool_id"], Value::Null);
}

#[test]
fn set_gcode_variable_rejects_unknown_macros_and_variables() {
    let mut printer = toolchanger_printer();

    let macro_error = run(&mut printer, "SET_GCODE_VARIABLE MACRO=T9 VARIABLE=spool_id VALUE=5").unwrap_err();
    let variable_error = run(&mut printer, "SET_GCODE_VARIABLE MACRO=T0 VARIABLE=colour VALUE=5").unwrap_err();

    assert_eq!(macro_error.message(), "The value 'T9' is not valid for MACRO. Options: 'T0', 'T1', 'T2'");
    assert_eq!(variable_error.message(), "Unknown gcode_macro variable 'colour'");
    assert!(run(&mut printer, "SET_GCODE_VARIABLE MACRO=T0 VARIABLE=SPOOL_ID VALUE=5").is_err(), "names are exact");
    assert_eq!(
        run(&mut printer, "SAVE_VARIABLE VARIABLE=T0__spool_id VALUE=5").unwrap_err().message(),
        "VARIABLE must not contain upper case"
    );
}

#[test]
fn save_variable_shows_in_save_variables() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "SAVE_VARIABLE VARIABLE=t0__spool_id VALUE=4").unwrap();

    assert_eq!(printer.status()["save_variables"]["variables"]["t0__spool_id"], 4);
}

#[test]
fn activate_extruder_sets_the_toolhead_extruder() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "ACTIVATE_EXTRUDER EXTRUDER=extruder2").unwrap();

    assert_eq!(printer.status()["toolhead"]["extruder"], "extruder2");
    assert!(run(&mut printer, "ACTIVATE_EXTRUDER EXTRUDER=extruder9").is_err());
}

#[test]
fn toolchanger_heads_have_fan_generic_fans_and_no_printer_fan() {
    let status = toolchanger_printer().status();

    assert_eq!(status["tool T0"]["fan"], "T0_partfan");
    assert_eq!(status["tool T2"]["fan"], "T2_partfan");
    assert_eq!(status["fan_generic T1_partfan"], json!({ "speed": 0.0, "rpm": null }));
    assert!(status.get("fan").is_none());
    assert!(status["configfile"]["settings"].get("fan").is_none());
    assert!(status["configfile"]["settings"].get("fan_generic T0_partfan").is_some());
}

#[test]
fn m106_drives_the_fan_of_the_active_tool_and_a_tool_change_moves_the_speed() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "T0").unwrap();
    run(&mut printer, "M106 S127.5").unwrap();
    assert_eq!(printer.status()["fan_generic T0_partfan"]["speed"], 0.5);

    run(&mut printer, "T1").unwrap();
    let status = printer.status();
    assert_eq!(status["fan_generic T0_partfan"]["speed"], 0.0);
    assert_eq!(status["fan_generic T1_partfan"]["speed"], 0.5);

    run(&mut printer, "M107").unwrap();
    assert_eq!(printer.status()["fan_generic T1_partfan"]["speed"], 0.0);
    run(&mut printer, "SET_FAN_SPEED FAN=T2_partfan SPEED=0.25").unwrap();
    assert_eq!(printer.status()["fan_generic T2_partfan"]["speed"], 0.25);
}

#[test]
fn m106_without_an_active_tool_waits_for_the_first_tool() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "M106 S255").unwrap();
    assert_eq!(printer.status()["fan_generic T0_partfan"]["speed"], 0.0);

    run(&mut printer, "T2").unwrap();
    assert_eq!(printer.status()["fan_generic T2_partfan"]["speed"], 1.0);
}

#[test]
fn other_profiles_keep_the_printer_fan() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::voron());

    run(&mut printer, "M106 S255").unwrap();

    assert_eq!(printer.status()["fan"]["speed"], 1.0);
}

#[test]
fn other_profiles_report_no_tool_changer_details() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::voron());

    run(&mut printer, "SELECT_TOOL T=1").unwrap();

    let status = printer.status();
    assert!(status["toolhead"].get("extruder").is_none());
    assert!(status.get("extruder1").is_none());
    assert!(status.get("save_variables").is_none());
}

#[test]
fn toolchanger_status_follows_the_fake_state_hook() {
    let mut printer = toolchanger_printer();
    assert_eq!(printer.status()["toolchanger"]["status"], "uninitialized");

    run(&mut printer, "FAKE_TOOLCHANGER_STATE STATE=ready").unwrap();
    assert_eq!(printer.status()["toolchanger"]["status"], "ready");
    run(&mut printer, "FAKE_TOOLCHANGER_STATE STATE=error").unwrap();
    assert_eq!(printer.status()["toolchanger"]["status"], "error");
    run(&mut printer, "FAKE_TOOLCHANGER_STATE STATE=uninitialized").unwrap();
    assert_eq!(printer.status()["toolchanger"]["status"], "uninitialized");
    run(&mut printer, "FAKE_TOOLCHANGER_STATE STATE=ready").unwrap();
    assert_eq!(printer.status()["toolchanger"]["status"], "ready");
    assert!(run(&mut printer, "FAKE_TOOLCHANGER_STATE STATE=bogus").is_err());
}

#[test]
fn first_use_initializes_on_the_first_tool_selection() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "T1").unwrap();

    let status = printer.status();
    assert_eq!(status["toolchanger"]["status"], "ready");
    assert_eq!(status["toolchanger"]["tool_number"], 1);
}

#[test]
fn an_error_refuses_tool_changes_with_its_reason_until_initialize_toolchanger_heals() {
    let mut printer = toolchanger_printer();
    run(&mut printer, "FAKE_TOOLCHANGER_STATE STATE=ready\nT1\nFAKE_TOOLCHANGER_STATE STATE=error").unwrap();

    let error = run(&mut printer, "T2").unwrap_err();

    assert_eq!(
        error.message(),
        "Cannot select tool, toolchanger status is error, reason: Expected tool tool T1 but active is None"
    );
    // The rejected command reset the plugin (`gcode:command_error`); the reason stays.
    assert_eq!(printer.status()["toolchanger"]["status"], "uninitialized");
    assert_eq!(printer.status()["toolchanger"]["tool_number"], -1);
    // `first-use` initialises by itself at the next selection.
    run(&mut printer, "T2").unwrap();
    assert_eq!(printer.status()["toolchanger"]["tool_number"], 2);
}

#[test]
fn any_rejected_command_resets_the_plugin() {
    let mut printer = toolchanger_printer();
    run(&mut printer, "T1").unwrap();

    assert!(run(&mut printer, "SET_HEATER_TEMPERATURE HEATER=nothing TARGET=1").is_err());

    let status = printer.status();
    assert_eq!(status["toolchanger"]["status"], "uninitialized");
    assert_eq!(status["toolchanger"]["tool_number"], -1);
    assert_eq!(status["toolchanger"]["detected_tool_number"], 1);
}

#[test]
fn a_manual_changer_refuses_tool_changes_until_it_is_initialized() {
    let mut printer = Printer::with_profile(
        1.0,
        MachineProfile { initialize_on: crate::machine::InitializeOn::Manual, ..MachineProfile::toolchanger() },
    );

    let error = run(&mut printer, "T1").unwrap_err();
    assert_eq!(error.message(), "Cannot select tool, toolchanger status is uninitialized, reason: ");
    run(&mut printer, "INITIALIZE_TOOLCHANGER").unwrap();
    assert_eq!(printer.status()["toolchanger"]["tool_number"], 0);
    run(&mut printer, "T1").unwrap();
    assert_eq!(printer.status()["toolchanger"]["tool_number"], 1);
}

#[test]
fn unselect_tool_without_an_active_tool_does_nothing() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "UNSELECT_TOOL").unwrap();

    assert_eq!(printer.status()["toolchanger"]["status"], "uninitialized");
}

#[test]
fn initialize_toolchanger_makes_the_detected_tool_active() {
    let mut printer = toolchanger_printer();

    run(&mut printer, "INITIALIZE_TOOLCHANGER").unwrap();

    let status = printer.status();
    assert_eq!(status["toolchanger"]["status"], "ready");
    assert_eq!(status["toolchanger"]["tool_number"], 0);
}

#[test]
fn initialize_toolchanger_with_recover_heals_an_error() {
    let mut printer = toolchanger_printer();
    run(&mut printer, "FAKE_TOOLCHANGER_STATE STATE=error\nINITIALIZE_TOOLCHANGER RECOVER=1").unwrap();

    assert_eq!(printer.status()["toolchanger"]["status"], "ready");
}

#[test]
fn recover_without_a_tool_fails() {
    let mut printer = toolchanger_printer();
    run(&mut printer, "T1\nUNSELECT_TOOL\nFAKE_TOOLCHANGER_STATE STATE=error").unwrap();

    let error = run(&mut printer, "INITIALIZE_TOOLCHANGER RECOVER=1").unwrap_err();

    assert_eq!(error.message(), "Cannot recover, no tool");
}

#[test]
fn an_error_pauses_a_running_print_only_with_error_gcode_pause() {
    let file = Catalog::demo().find("benchy.gcode").unwrap().clone();
    for (profile, expected) in
        [(MachineProfile::toolchanger(), "printing"), (MachineProfile::toolchanger_required(), "paused")]
    {
        let mut printer = Printer::with_profile(1.0, profile);
        printer.start_print(&file).unwrap();

        run(&mut printer, "FAKE_TOOLCHANGER_STATE STATE=error").unwrap();

        assert_eq!(printer.status()["print_stats"]["state"], expected);
        assert_eq!(printer.status()["toolchanger"]["status"], "error");
    }
}

#[test]
fn require_tool_present_removes_unselect_tool() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::toolchanger_required());

    let error = run(&mut printer, "UNSELECT_TOOL").unwrap_err();

    assert_eq!(error.message(), "Unknown command:\"UNSELECT_TOOL\"");
    run(&mut printer, "G28").unwrap();
    assert_eq!(printer.status()["toolchanger"]["tool_number"], 0);
    run(&mut printer, "T1").unwrap();
}

#[test]
fn initialize_on_home_waits_for_the_homing() {
    let mut printer = Printer::with_profile(1.0, MachineProfile::toolchanger_required());
    let settings = printer.status()["configfile"]["settings"]["toolchanger"].clone();
    assert_eq!(settings["initialize_on"], "home");
    assert_eq!(settings["require_tool_present"], true);
    assert_eq!(settings["error_gcode"], "PAUSE");

    let error = run(&mut printer, "T1").unwrap_err();
    assert_eq!(error.message(), "Cannot select tool, toolchanger status is uninitialized, reason: ");

    run(&mut printer, "G28").unwrap();
    assert_eq!(printer.status()["toolchanger"]["status"], "ready");
    assert_eq!(printer.status()["toolchanger"]["tool_number"], 0);
}
