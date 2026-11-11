//! The profiles with Happy Hare units and with AFC: objects, key values, settings and one state change each.

use super::*;
use crate::gcode::parse_script;

fn run(printer: &mut Printer, script: &str) -> Result<(), CommandError> {
    parse_script(script)?.into_iter().try_for_each(|command| printer.apply(command))
}

fn with(profile: MachineProfile) -> Printer {
    Printer::with_profile(1.0, profile)
}

fn homed(profile: MachineProfile) -> Printer {
    let mut printer = with(profile);
    run(&mut printer, "G28").unwrap();
    printer
}

fn names(printer: &Printer) -> Vec<String> {
    printer.object_names()
}

fn settings(printer: &Printer) -> Value {
    printer.status()["configfile"]["settings"].clone()
}

// Happy Hare with two units on one extruder

#[test]
fn happy_hare_units_lists_one_wrapper_per_extruder_and_one_gear_per_unit() {
    let names = names(&with(MachineProfile::happy_hare_units()));

    for expected in
        ["mmu", "mmu_machine", "mmu_extruder extruder", "mmu_stepper ercf_gear", "mmu_stepper tradrack_gear"]
    {
        assert!(names.contains(&expected.to_owned()), "missing {expected}");
    }
    assert!(!names.contains(&"mmu_extruder extruder1".to_owned()));
    assert!(!names.contains(&"mmu_stepper unit0_gear".to_owned()));
}

#[test]
fn happy_hare_units_number_the_gates_across_the_units() {
    let status = with(MachineProfile::happy_hare_units()).status();
    let machine = &status["mmu_machine"];

    assert_eq!(machine["num_units"], 2);
    assert_eq!(machine["num_gates"], 8);
    assert_eq!(machine["unit_0"]["name"], "ercf");
    assert_eq!(machine["unit_0"]["first_gate"], 0);
    assert_eq!(machine["unit_0"]["has_bypass"], true);
    assert_eq!(machine["unit_1"]["name"], "tradrack");
    assert_eq!(machine["unit_1"]["display_name"], "Tradrack");
    assert_eq!(machine["unit_1"]["vendor"], "Tradrack");
    assert_eq!(machine["unit_1"]["first_gate"], 4);
    assert_eq!(machine["unit_1"]["num_gates"], 4);
    assert_eq!(machine["unit_1"]["has_bypass"], false);
    assert_eq!(status["mmu"]["num_gates"], 8);
    assert_eq!(status["mmu"]["ttg_map"], json!([0, 1, 2, 3, 4, 5, 6, 7]));
    assert_eq!(status["mmu"]["gate_status"].as_array().unwrap().len(), 8);
    assert_eq!(status["mmu"]["unit"], 0);
}

#[test]
fn happy_hare_units_settings_name_the_units_and_their_extruders() {
    let settings = settings(&with(MachineProfile::happy_hare_units()));

    assert_eq!(settings["mmu_machine"]["units"], json!(["ercf", "tradrack"]));
    assert_eq!(settings["mmu_unit tradrack"]["extruder_name"], "extruder");
    assert_eq!(settings["mmu_unit tradrack"]["num_gates"], 4);
    assert_eq!(settings["mmu_unit ercf"]["display_name"], "ERCF");
}

#[test]
fn selecting_a_gate_of_the_second_unit_makes_it_the_active_unit() {
    let mut printer = with(MachineProfile::happy_hare_units());

    run(&mut printer, "MMU_HOME\nMMU_SELECT GATE=5").unwrap();
    let status = printer.status();

    assert_eq!(status["mmu"]["unit"], 1);
    assert_eq!(status["mmu"]["gate"], 5);
    assert_eq!(status["mmu"]["extruder_name"], "extruder");
    run(&mut printer, "MMU_SELECT GATE=1").unwrap();
    assert_eq!(printer.status()["mmu"]["unit"], 0);
    // The ERCF shows the bypass, the Tradrack does not.
    assert_eq!(printer.status()["mmu"]["selector"]["has_bypass"], true);
}

// Tool changer with Happy Hare units on its heads

#[test]
fn toolchanger_mmu_has_the_objects_of_both() {
    let names = names(&with(MachineProfile::toolchanger_mmu()));

    for expected in [
        "toolchanger",
        "tool T0",
        "tool T1",
        "extruder1",
        "mmu",
        "mmu_machine",
        "mmu_extruder extruder",
        "mmu_extruder extruder1",
        "mmu_stepper tradrack_gear",
    ] {
        assert!(names.contains(&expected.to_owned()), "missing {expected}");
    }
    assert!(!names.contains(&"tool T2".to_owned()));
}

#[test]
fn toolchanger_mmu_settings_say_which_extruder_each_unit_feeds() {
    let settings = settings(&with(MachineProfile::toolchanger_mmu()));

    assert_eq!(settings["mmu_unit ercf"]["extruder_name"], "extruder");
    assert_eq!(settings["mmu_unit tradrack"]["extruder_name"], "extruder1");
    assert_eq!(settings["mmu_machine"]["units"], json!(["ercf", "tradrack"]));
    assert!(settings.get("extruder1").is_some());
}

#[test]
fn a_gate_of_the_second_unit_does_not_move_the_head_but_select_tool_does() {
    let mut printer = with(MachineProfile::toolchanger_mmu());

    run(&mut printer, "MMU_HOME\nMMU_SELECT GATE=5").unwrap();
    let status = printer.status();
    assert_eq!(status["mmu"]["unit"], 1);
    assert_eq!(status["mmu"]["extruder_name"], "extruder1");
    assert_eq!(status["toolhead"]["extruder"], "extruder");
    assert_eq!(status["toolchanger"]["detected_tool"], "tool T0");

    run(&mut printer, "SELECT_TOOL T=1").unwrap();
    let status = printer.status();
    assert_eq!(status["toolhead"]["extruder"], "extruder1");
    assert_eq!(status["toolchanger"]["tool"], "tool T1");
    assert_eq!(status["mmu"]["unit"], 1);
}

#[test]
fn tool_numbers_beyond_the_heads_are_gates_of_the_mmu() {
    let mut printer = with(MachineProfile::toolchanger_mmu());

    run(&mut printer, "MMU_HOME\nT5").unwrap();

    let status = printer.status();
    assert_eq!(status["mmu"]["gate"], 5);
    assert_eq!(status["mmu"]["unit"], 1);
    assert_eq!(status["toolhead"]["extruder"], "extruder");
}

// AFC with a Box Turtle

#[test]
fn afc_lists_the_objects_of_a_box_turtle() {
    let names = names(&with(MachineProfile::afc()));

    for expected in [
        "AFC",
        "AFC_BoxTurtle Turtle_1",
        "AFC_stepper lane1",
        "AFC_stepper lane4",
        "AFC_hub Turtle_1",
        "AFC_extruder extruder",
    ] {
        assert!(names.contains(&expected.to_owned()), "missing {expected}");
    }
    assert!(!names.iter().any(|name| name.starts_with("AFC_lane") || name.starts_with("mmu")));
    assert!(!names.contains(&"toolchanger".to_owned()));
}

#[test]
fn afc_reports_the_box_turtle_with_lane_two_in_the_toolhead() {
    let status = with(MachineProfile::afc()).status();

    assert_eq!(status["AFC"]["current_load"], "lane2");
    assert_eq!(status["AFC"]["current_state"], "Idle");
    assert_eq!(status["AFC"]["units"], json!(["Box_Turtle Turtle_1"]));
    assert_eq!(status["AFC"]["lanes"], json!(["lane1", "lane2", "lane3", "lane4"]));
    assert_eq!(status["AFC"]["maps"], json!(["T0", "T1", "T2", "T3"]));
    assert_eq!(status["AFC"]["extruders"], json!(["extruder"]));
    assert_eq!(status["AFC"]["hubs"], json!(["Turtle_1"]));
    let lane = &status["AFC_stepper lane2"];
    assert_eq!(lane["unit"], "Turtle_1");
    assert_eq!(lane["hub"], "Turtle_1");
    assert_eq!(lane["extruder"], "extruder");
    assert_eq!(lane["lane"], 2);
    assert_eq!(lane["map"], "T1");
    assert_eq!(lane["tool_loaded"], true);
    assert_eq!(lane["status"], "Tooled");
    assert_eq!(lane["filament_status"], "In Tool");
    assert_eq!(lane["spool_id"], 4);
    assert_eq!(lane["material"], "ABS");
    assert_eq!(lane["color"], "#1A1A1A");
    let empty = &status["AFC_stepper lane4"];
    assert_eq!(empty["prep"], false);
    assert_eq!(empty["load"], false);
    assert_eq!(empty["status"], "None");
    assert_eq!(empty["spool_id"], Value::Null);
    assert_eq!(empty["color"], "");
    assert_eq!(status["AFC_extruder extruder"]["lane_loaded"], "lane2");
    assert_eq!(status["AFC_extruder extruder"]["on_shuttle"], true);
    assert_eq!(status["AFC_BoxTurtle Turtle_1"]["lanes"], json!(["lane1", "lane2", "lane3", "lane4"]));
    assert_eq!(status["AFC_hub Turtle_1"]["state"], true);
}

#[test]
fn afc_settings_name_the_klipper_extruder_of_the_afc_extruder() {
    let settings = settings(&with(MachineProfile::afc()));

    assert_eq!(settings["afc_extruder extruder"]["extruder_name"], "extruder");
    assert!(settings.get("afc_stepper lane3").is_some());
    assert!(settings.get("afc_boxturtle turtle_1").is_some());
}

#[test]
fn t2_loads_lane_three_and_unloads_lane_two() {
    let mut printer = homed(MachineProfile::afc());

    run(&mut printer, "T2").unwrap();

    let status = printer.status();
    assert_eq!(status["AFC"]["current_load"], "lane3");
    assert_eq!(status["AFC_stepper lane3"]["tool_loaded"], true);
    assert_eq!(status["AFC_stepper lane3"]["status"], "Tooled");
    assert_eq!(status["AFC_stepper lane2"]["tool_loaded"], false);
    assert_eq!(status["AFC_stepper lane2"]["status"], "None");
    assert_eq!(status["AFC_extruder extruder"]["lane_loaded"], "lane3");
    assert_eq!(printer.selected_gate_spool(), Some(3));
}

#[test]
fn a_tool_change_needs_a_homed_printer() {
    let mut printer = with(MachineProfile::afc());

    let error = run(&mut printer, "T2").unwrap_err();

    assert_eq!(error.message(), "Please home printer before doing a tool load");
    assert_eq!(printer.status()["AFC"]["current_load"], "lane2");
}

#[test]
fn set_spool_id_takes_the_filament_of_the_spool() {
    let mut printer = with(MachineProfile::afc());

    run(&mut printer, "SET_SPOOL_ID LANE=lane4 SPOOL_ID=6\nSET_COLOR LANE=lane1 COLOR=FF0000").unwrap();

    let status = printer.status();
    assert_eq!(status["AFC_stepper lane4"]["spool_id"], 6);
    assert_eq!(status["AFC_stepper lane4"]["material"], "PETG");
    assert_eq!(status["AFC_stepper lane4"]["weight"], 720.0);
    assert_eq!(status["AFC_stepper lane1"]["color"], "#FF0000");
}

// AFC on a tool changer

#[test]
fn afc_toolchanger_lists_afc_objects_and_no_viesturz_objects() {
    let names = names(&with(MachineProfile::afc_toolchanger()));

    for expected in [
        "AFC",
        "AFC_Toolchanger Tools",
        "AFC_extruder e0",
        "AFC_extruder e3",
        "AFC_lane e0",
        "AFC_lane e3",
        "extruder1",
        "extruder3",
    ] {
        assert!(names.contains(&expected.to_owned()), "missing {expected}");
    }
    assert!(!names.iter().any(|name| name.starts_with("AFC_stepper") || name.starts_with("AFC_hub")));
    assert!(!names.iter().any(|name| name == "toolchanger" || name.starts_with("tool T")));
    assert!(!names.iter().any(|name| name.starts_with("gcode_macro T")));
}

#[test]
fn afc_toolchanger_starts_with_head_one_on_the_shuttle() {
    let status = with(MachineProfile::afc_toolchanger()).status();

    assert_eq!(status["toolhead"]["extruder"], "extruder1");
    assert_eq!(status["AFC"]["current_load"], "e1");
    assert_eq!(status["AFC"]["units"], json!(["Toolchanger Tools"]));
    assert_eq!(status["AFC"]["hubs"], json!([]));
    assert_eq!(status["AFC"]["extruders"], json!(["e0", "e1", "e2", "e3"]));
    assert_eq!(status["AFC_extruder e1"]["on_shuttle"], true);
    assert_eq!(status["AFC_extruder e0"]["on_shuttle"], false);
    assert_eq!(status["AFC_extruder e1"]["lane_loaded"], "e1");
    let lane = &status["AFC_lane e1"];
    assert_eq!(lane["unit"], "Tools");
    assert_eq!(lane["hub"], "direct");
    assert_eq!(lane["extruder"], "e1");
    assert_eq!(lane["map"], "T1");
    assert_eq!(status["AFC_lane e3"]["prep"], false);
    assert_eq!(status["AFC_Toolchanger Tools"]["lanes"], json!(["e0", "e1", "e2", "e3"]));
}

#[test]
fn afc_toolchanger_settings_name_the_klipper_extruder_of_each_head() {
    let settings = settings(&with(MachineProfile::afc_toolchanger()));

    for (head, extruder) in [("e0", "extruder"), ("e1", "extruder1"), ("e2", "extruder2"), ("e3", "extruder3")] {
        assert_eq!(settings[format!("afc_extruder {head}")]["extruder_name"], extruder);
        assert_eq!(settings[format!("afc_extruder {head}")]["toolchanger_unit"], "Tools");
    }
    assert!(settings.get("extruder3").is_some());
}

#[test]
fn t0_moves_the_shuttle_and_the_active_extruder() {
    let mut printer = homed(MachineProfile::afc_toolchanger());

    run(&mut printer, "T0").unwrap();

    let status = printer.status();
    assert_eq!(status["toolhead"]["extruder"], "extruder");
    assert_eq!(status["AFC"]["current_load"], "e0");
    assert_eq!(status["AFC_extruder e0"]["on_shuttle"], true);
    assert_eq!(status["AFC_extruder e1"]["on_shuttle"], false);
    assert_eq!(status["AFC_lane e0"]["tool_loaded"], true);
    assert_eq!(status["AFC_lane e1"]["tool_loaded"], false);

    // `M104` heats the extruder of the head on the shuttle.
    run(&mut printer, "CHANGE_TOOL LANE=e2\nM104 S200").unwrap();
    let status = printer.status();
    assert_eq!(status["toolhead"]["extruder"], "extruder2");
    assert_eq!(status["extruder2"]["target"], 200.0);
    assert_eq!(status["extruder"]["target"], 0.0);
}

#[test]
fn the_empty_lane_of_the_tool_changer_cannot_be_loaded() {
    let mut printer = homed(MachineProfile::afc_toolchanger());

    let error = run(&mut printer, "T3").unwrap_err();

    assert!(error.message().starts_with("Current lane not loaded"));
    assert_eq!(printer.status()["toolhead"]["extruder"], "extruder1");
}

// Filament sensors of Happy Hare units

fn sensor_names(printer: &Printer) -> Vec<String> {
    let mut sensors: Vec<String> = names(printer)
        .into_iter()
        .filter_map(|name| name.strip_prefix("filament_switch_sensor ").map(str::to_owned))
        // `toolhead_runout` belongs to the voron base of the AFC profiles, not to AFC.
        .filter(|name| name != "toolhead_runout")
        .collect();
    sensors.sort();
    sensors
}

fn detected(printer: &Printer, sensor: &str) -> Value {
    printer.status()[format!("filament_switch_sensor {sensor}")]["filament_detected"].clone()
}

#[test]
fn one_unit_lists_its_sensors_without_prefix() {
    let printer = with(MachineProfile::happy_hare(4));

    // The older profiles list the first five of the six sensors of the recorded machine.
    assert_eq!(sensor_names(&printer), ["extruder", "filament_compression", "mmu_entry_0", "mmu_exit_0", "toolhead"]);
}

#[test]
fn two_units_on_one_head_list_gate_sensors_and_prefixed_owner_sensors() {
    let printer = with(MachineProfile::happy_hare_units());

    let mut expected: Vec<String> =
        (0..8).flat_map(|gate| [format!("mmu_entry_{gate}"), format!("mmu_exit_{gate}")]).collect();
    expected.extend(
        [
            "default:extruder",
            "default:toolhead",
            "ercf:filament_compression",
            "ercf:filament_tension",
            "tradrack:filament_compression",
            "tradrack:filament_tension",
            "tradrack:mmu_shared_exit",
        ]
        .map(str::to_owned),
    );
    expected.sort();
    assert_eq!(sensor_names(&printer), expected);
}

#[test]
fn two_units_on_two_heads_have_one_toolhead_each() {
    let printer = with(MachineProfile::toolchanger_mmu());

    let sensors = sensor_names(&printer);
    for expected in ["t0:extruder", "t0:toolhead", "t1:extruder", "t1:toolhead", "tradrack:mmu_shared_exit"] {
        assert!(sensors.contains(&expected.to_owned()), "missing {expected}");
    }
    assert!(!sensors.contains(&"default:toolhead".to_owned()));
    assert!(!sensors.contains(&"ercf:mmu_shared_exit".to_owned()));
}

#[test]
fn the_units_name_their_toolhead_and_buffer_in_the_settings() {
    let one_head = settings(&with(MachineProfile::happy_hare_units()));
    let two_heads = settings(&with(MachineProfile::toolchanger_mmu()));

    assert_eq!(one_head["mmu_unit ercf"]["toolhead"], "default");
    assert_eq!(one_head["mmu_unit tradrack"]["toolhead"], "default");
    assert!(one_head.get("mmu_toolhead default").is_some());
    assert!(one_head.get("mmu_toolhead t0").is_none());
    assert_eq!(two_heads["mmu_unit ercf"]["toolhead"], "t0");
    assert_eq!(two_heads["mmu_unit tradrack"]["toolhead"], "t1");
    assert_eq!(two_heads["mmu_unit tradrack"]["buffer"], "tradrack");
    for section in ["mmu_toolhead t0", "mmu_toolhead t1", "mmu_buffer ercf", "mmu_buffer tradrack"] {
        assert!(two_heads.get(section).is_some(), "missing {section}");
    }
    assert!(two_heads.get("mmu_toolhead default").is_none());
}

#[test]
fn loading_a_gate_of_the_second_unit_triggers_its_sensors_and_the_shared_toolhead() {
    let mut printer = with(MachineProfile::happy_hare_units());
    // Before any load the gates hold filament at their entry; the last gate is empty.
    assert_eq!(detected(&printer, "mmu_entry_5"), true);
    assert_eq!(detected(&printer, "mmu_entry_7"), false);
    assert_eq!(detected(&printer, "mmu_exit_5"), false);

    run(&mut printer, "MMU_HOME\nMMU_SELECT GATE=5\nMMU_LOAD").unwrap();

    assert_eq!(detected(&printer, "mmu_exit_5"), true);
    assert_eq!(detected(&printer, "tradrack:mmu_shared_exit"), true);
    assert_eq!(detected(&printer, "default:extruder"), true);
    assert_eq!(detected(&printer, "default:toolhead"), true);
    assert_eq!(detected(&printer, "mmu_exit_1"), false);
    assert_eq!(detected(&printer, "mmu_entry_1"), true);
}

#[test]
fn only_the_toolhead_of_the_selected_unit_sees_the_filament() {
    let mut printer = with(MachineProfile::toolchanger_mmu());

    run(&mut printer, "MMU_HOME\nMMU_SELECT GATE=5\nMMU_LOAD").unwrap();
    assert_eq!(detected(&printer, "t1:toolhead"), true);
    assert_eq!(detected(&printer, "t1:extruder"), true);
    assert_eq!(detected(&printer, "t0:toolhead"), false);
    assert_eq!(detected(&printer, "t0:extruder"), false);

    run(&mut printer, "MMU_UNLOAD\nMMU_SELECT GATE=1\nMMU_LOAD").unwrap();
    assert_eq!(detected(&printer, "t0:toolhead"), true);
    assert_eq!(detected(&printer, "t1:toolhead"), false);
    assert_eq!(detected(&printer, "tradrack:mmu_shared_exit"), false);
}

// AFC sensors and buffer

#[test]
fn afc_lists_switch_sensors_buffer_and_tool_end_of_the_box_turtle() {
    let printer = with(MachineProfile::afc());

    assert_eq!(
        sensor_names(&printer),
        [
            "TN_compressed",
            "TN_expanded",
            "Turtle_1_Hub",
            "extruder_tool_end",
            "lane1_load",
            "lane1_prep",
            "lane2_load",
            "lane2_prep",
            "lane3_load",
            "lane3_prep",
            "lane4_load",
            "lane4_prep"
        ]
    );
    assert!(names(&printer).contains(&"AFC_buffer TN".to_owned()));
}

#[test]
fn afc_sensors_and_buffer_follow_the_tool_change() {
    let mut printer = homed(MachineProfile::afc());
    let status = printer.status();
    assert_eq!(status["AFC_buffer TN"]["state"], "Advancing");
    assert_eq!(status["AFC_buffer TN"]["active_lane"], "lane2");
    assert_eq!(status["AFC_stepper lane1"]["buffer"], "TN");
    assert_eq!(status["AFC_stepper lane1"]["buffer_status"], "Advancing");
    assert_eq!(status["AFC_extruder extruder"]["tool_end_status"], true);
    assert_eq!(status["AFC"]["buffers"], json!(["TN"]));
    assert_eq!(status["AFC_BoxTurtle Turtle_1"]["buffers"], json!(["TN"]));
    assert_eq!(detected(&printer, "extruder_tool_end"), true);
    assert_eq!(detected(&printer, "Turtle_1_Hub"), true);

    run(&mut printer, "TOOL_UNLOAD").unwrap();
    let status = printer.status();
    assert_eq!(status["AFC_buffer TN"]["state"], "Unknown");
    assert_eq!(status["AFC_buffer TN"]["enabled"], false);
    assert_eq!(status["AFC_extruder extruder"]["tool_end_status"], false);
    assert_eq!(detected(&printer, "extruder_tool_end"), false);
    assert_eq!(detected(&printer, "Turtle_1_Hub"), false);

    run(&mut printer, "CHANGE_TOOL LANE=lane3").unwrap();
    assert_eq!(printer.status()["AFC_buffer TN"]["active_lane"], "lane3");
    assert_eq!(detected(&printer, "extruder_tool_end"), true);
    assert_eq!(detected(&printer, "lane3_prep"), true);
    assert_eq!(detected(&printer, "lane4_prep"), false);
}

#[test]
fn afc_settings_name_the_buffer_and_the_tool_end_pin() {
    let settings = settings(&with(MachineProfile::afc()));

    assert_eq!(settings["afc_buffer tn"]["type"], "switched");
    assert!(settings["afc_buffer tn"]["advance_pin"].is_string());
    assert!(settings["afc_buffer tn"]["trailing_pin"].is_string());
    assert!(settings["afc_extruder extruder"]["pin_tool_end"].is_string());
    assert_eq!(settings["afc_extruder extruder"]["buffer"], "TN");
    assert_eq!(settings["afc_boxturtle turtle_1"]["buffer"], "TN");
}

#[test]
fn the_afc_tool_changer_has_no_switch_sensors_because_the_u1_template_has_no_pins() {
    let printer = with(MachineProfile::afc_toolchanger());

    assert!(sensor_names(&printer).is_empty());
    assert!(!names(&printer).iter().any(|name| name.starts_with("AFC_buffer")));
}
