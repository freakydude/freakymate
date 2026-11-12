//! Connects the UI's `Actions` callbacks to printer commands.

use crate::presenter::Presenter;
use crate::settings_controller::SettingsController;
use crate::{Actions, AppWindow};
use freakymate_core::domain::machine_tree::LaneId;
use freakymate_core::sources::happy_hare::commands::{Gates, ServoCommand};
use freakymate_core::sources::klipper::tuning::TuningKey;
use freakymate_core::sources::klipper::{Axis, TestZ};
use freakymate_core::{Command, PrinterService, Query};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::rc::Rc;
use std::time::Duration;

pub fn bind(
    window: &AppWindow,
    service: &PrinterService,
    presenter: &Rc<Presenter>,
    settings: &Rc<SettingsController>,
) {
    let actions = window.global::<Actions>();
    bind_motion(&actions, service);
    bind_temperatures(&actions, service, presenter, settings);
    bind_calibration(&actions, service, presenter);
    bind_filament(&actions, service);
    bind_mmu(&actions, service);
    bind_lanes(&actions, service, presenter);
    bind_spools(&actions, service, presenter);
    bind_files_and_print(&actions, service, presenter);
    bind_layer_pause(&actions, service, presenter);
    bind_system(&actions, service);
    bind_console_and_macros(&actions, service, presenter, settings);
    bind_status_messages(&actions, presenter);

    let key_window = window.as_weak();
    actions.on_press_key(move |text| press_key(&key_window, text));
}

fn bind_motion(actions: &Actions, service: &PrinterService) {
    let home_service = service.clone();
    actions.on_home(move |x, y, z| home_service.send(Command::Home { x, y, z }));
    let jog_service = service.clone();
    actions.on_jog(move |axis, distance| {
        if let Some(axis) = parse_axis(&axis) {
            jog_service.send(Command::Jog { axis, distance: f64::from(distance) });
        }
    });
    actions.on_disable_motors(send(service, || Command::DisableMotors));
    actions.on_level(send(service, || Command::Level));
    actions.on_adjust_z_offset(with(service, |service, delta: f32| {
        service.send(Command::AdjustZOffset { delta: round_offset(delta) })
    }));
    actions.on_reset_z_offset(send(service, || Command::ResetZOffset));
    actions.on_apply_z_offset(send(service, || Command::ApplyZOffset));
    actions.on_save_config(send(service, || Command::SaveConfig));
    actions.on_set_speed_factor(with(service, |service, percent: f32| {
        service.send(Command::SetSpeedFactor { percent: f64::from(percent) })
    }));
}

/// Heaters, fans, lights, the tuning values and the history of a temperature.
fn bind_temperatures(
    actions: &Actions,
    service: &PrinterService,
    presenter: &Rc<Presenter>,
    settings: &Rc<SettingsController>,
) {
    let heater_service = service.clone();
    actions.on_set_heater(move |heater, target| {
        heater_service.send(Command::SetHeaterTarget { heater: heater.into(), target: f64::from(target) });
    });
    let step_service = service.clone();
    actions.on_step_heater(move |heater, step| {
        step_service.send(Command::StepHeater { heater: heater.into(), step: f64::from(step) });
    });
    let (preset_presenter, preset_settings) = (Rc::clone(presenter), Rc::clone(settings));
    actions.on_apply_preset(move |index| {
        let Some(preset) = usize::try_from(index).ok().and_then(|index| preset_settings.preset(index)) else { return };
        let command = Command::ApplyPreset { extruder: preset.extruder, bed: preset.bed };
        preset_presenter.send_preset(command, &preset_presenter.snapshot());
    });
    actions.on_cool_down(send(service, || Command::CoolDown));
    let fan_service = service.clone();
    actions.on_set_fan(move |fan, percent| {
        fan_service.send(Command::SetFanSpeed { fan: fan.into(), speed: f64::from(percent) / 100.0 });
    });
    let light_service = service.clone();
    actions.on_set_light(move |led, percent| {
        light_service.send(Command::SetLight { led: led.into(), level: f64::from(percent) / 100.0 });
    });
    actions.on_toggle_light(send(service, || Command::ToggleLight));
    let effect_service = service.clone();
    actions.on_set_led_effect(move |effect, on| {
        effect_service.send(Command::SetLedEffect { effect: effect.into(), on });
    });
    let tuning_service = service.clone();
    actions.on_step_tuning(move |id, direction| {
        if let Some(key) = TuningKey::parse(&id) {
            tuning_service.send(Command::StepTuning { key, direction: f64::from(direction) });
        }
    });
    actions.on_reset_tuning(with(service, |service, id: SharedString| {
        if let Some(key) = TuningKey::parse(&id) {
            service.send(Command::ResetTuning { key });
        }
    }));
    actions
        .on_open_history(with_presenter(presenter, |presenter, object: SharedString| presenter.open_history(&object)));
    actions.on_close_history(run_presenter(presenter, Presenter::close_history));
    let target_presenter = Rc::clone(presenter);
    actions.on_heater_target_outside(move |heater, typed| target_presenter.heater_target_outside(&heater, &typed));
}

/// Bed mesh, screws and Z calibration.
fn bind_calibration(actions: &Actions, service: &PrinterService, presenter: &Rc<Presenter>) {
    actions.on_calibrate_bed_mesh(send(service, || Command::CalibrateBedMesh));
    actions.on_load_bed_mesh(with(service, |service, profile: SharedString| {
        service.send(Command::LoadBedMesh { profile: profile.into() })
    }));
    actions.on_clear_bed_mesh(send(service, || Command::ClearBedMesh));
    actions.on_remove_bed_mesh(with(service, |service, profile: SharedString| {
        service.send(Command::RemoveBedMesh { profile: profile.into() })
    }));
    actions.on_calculate_screws_tilt(send(service, || Command::CalculateScrewsTilt));
    actions.on_adjust_bed_screws(send(service, || Command::AdjustBedScrews));
    actions.on_calibrate_z(send(service, || Command::CalibrateZ));
    actions.on_test_z(with(service, |service, distance: f32| {
        service.send(Command::TestZ { step: TestZ::By(round_offset(distance)) })
    }));
    actions.on_bisect_z(with(service, |service, direction: i32| {
        let step = if direction > 0 { TestZ::BisectUp } else { TestZ::BisectDown };
        service.send(Command::TestZ { step })
    }));
    actions.on_accept_calibration(send(service, || Command::AcceptCalibration));
    actions.on_screw_adjusted(send(service, || Command::ScrewAdjusted));
    actions.on_abort_calibration(send(service, || Command::AbortCalibration));
    actions.on_bed_mesh_name_hint(with_presenter(presenter, |presenter, name: SharedString| {
        SharedString::from(presenter.bed_mesh_name_hint(&name))
    }));
    actions.on_save_bed_mesh(with_presenter(presenter, |presenter, name: SharedString| {
        presenter.save_bed_mesh(name.trim())
    }));
}

/// The Filament page without MMU: extruding, the load and unload macros.
fn bind_filament(actions: &Actions, service: &PrinterService) {
    let extrude_service = service.clone();
    actions.on_extrude(move |length, speed| {
        extrude_service.send(Command::Extrude { length: f64::from(length), speed: f64::from(speed) });
    });
    actions.on_load_filament(send(service, || Command::LoadFilament));
    actions.on_unload_filament(send(service, || Command::UnloadFilament));
    actions.on_pick_up_head(with(service, |service, head: SharedString| {
        service.send(Command::PickUpHead { head: head.into() })
    }));
    actions.on_drop_head(send(service, || Command::DropHead));
    let initialize_service = service.clone();
    actions.on_initialize_toolchanger(move |recover| {
        initialize_service.send(Command::InitializeToolchanger { recover });
    });
}

/// Happy Hare: gates, tools, endless spool, bypass and the filament of a gate.
fn bind_mmu(actions: &Actions, service: &PrinterService) {
    actions.on_mmu_select_gate(with_gate(service, |gate| Command::MmuSelectGate { gate }));
    actions.on_mmu_load(send(service, || Command::MmuLoad));
    actions.on_mmu_unload(send(service, || Command::MmuUnload));
    actions.on_mmu_home(send(service, || Command::MmuHome));
    actions.on_mmu_recover(send(service, || Command::MmuRecover));
    actions.on_mmu_unlock(send(service, || Command::MmuUnlock));
    actions.on_mmu_motors_off(send(service, || Command::MmuMotorsOff));
    actions.on_mmu_check_all_gates(send(service, || Command::MmuCheckGates { gates: Gates::All }));
    actions.on_mmu_check_gate(with_gate(service, |gate| Command::MmuCheckGates { gates: Gates::One(gate) }));
    actions.on_mmu_preload(with_gate(service, |gate| Command::MmuPreload { gate }));
    actions.on_mmu_servo_up(send(service, || Command::MmuServo { position: ServoCommand::Up }));
    actions.on_mmu_servo_down(send(service, || Command::MmuServo { position: ServoCommand::Down }));
    actions.on_mmu_select_bypass(send(service, || Command::MmuSelectBypass));
    actions.on_mmu_load_bypass(send(service, || Command::MmuLoadBypass));
    let tool_map_service = service.clone();
    actions.on_mmu_map_tool(move |tool, gate| {
        if let (Ok(tool), Ok(gate)) = (usize::try_from(tool), usize::try_from(gate)) {
            tool_map_service.send(Command::MmuMapTool { tool, gate });
        }
    });
    // The source in charge resets its map: Happy Hare (`MMU_TTG_MAP RESET=1`), else AFC (`RESET_AFC_MAPPING`).
    actions.on_mmu_reset_tool_map(send(service, || Command::ResetToolMap));
    actions.on_afc_reset_failure(send(service, || Command::AfcResetFailure));
    actions.on_afc_reset_lane(with(service, |service, lane: SharedString| {
        service.send(Command::AfcResetLane { lane: lane.into() })
    }));
    actions.on_mmu_set_endless_spool(with(service, |service, enabled: bool| {
        service.send(Command::MmuEndlessSpool { enabled })
    }));
    let group_service = service.clone();
    actions.on_mmu_set_endless_spool_group(move |gate, group| {
        if let (Ok(gate), Ok(group)) = (usize::try_from(gate), u32::try_from(group)) {
            group_service.send(Command::SetEndlessSpoolGroup { gate, group });
        }
    });
}

/// The tiles of the Gates tab, a gate of Happy Hare or a lane of AFC: the core resolves each intent for its source.
fn bind_lanes(actions: &Actions, service: &PrinterService, presenter: &Rc<Presenter>) {
    actions.on_lane_tapped(with_presenter(presenter, |presenter, row: i32| {
        if let Some(lane) = lane_of_row(presenter, row) {
            presenter.show_lane(lane);
        }
    }));
    actions.on_bypass_tapped(run_presenter(presenter, |presenter| presenter.show_lane(LaneId::Bypass)));
    let (service_for_map, presenter_for_map) = (service.clone(), Rc::clone(presenter));
    actions.on_map_tool_to_lane(move |tool, row| {
        if let (Ok(tool), Some(lane)) = (usize::try_from(tool), lane_of_row(&presenter_for_map, row)) {
            service_for_map.send(Command::MapToolToLane { tool, lane });
        }
    });
    actions.on_lane_load(with_lane(service, presenter, |lane| Command::LoadLane { lane }));
    actions.on_lane_unload(with_lane(service, presenter, |lane| Command::UnloadLane { lane }));
    actions.on_lane_eject(with_lane(service, presenter, |lane| Command::EjectLane { lane }));
    let (service_for_spool, presenter_for_spool) = (service.clone(), Rc::clone(presenter));
    actions.on_assign_lane_spool(move |row, id| {
        if let Some(lane) = lane_of_row(&presenter_for_spool, row) {
            service_for_spool.send(Command::AssignLaneSpool { lane, spool_id: u32::try_from(id).ok() });
        }
    });
    let (service_for_material, presenter_for_material) = (service.clone(), Rc::clone(presenter));
    actions.on_set_lane_material(move |row, material| {
        if let Some(lane) = lane_of_row(&presenter_for_material, row) {
            service_for_material.send(Command::SetLaneFilament { lane, material: Some(material.into()), color: None });
        }
    });
    let (service_for_color, presenter_for_color) = (service.clone(), Rc::clone(presenter));
    actions.on_set_lane_color(move |row, color| {
        if let Some(lane) = lane_of_row(&presenter_for_color, row) {
            let color = Some([color.red(), color.green(), color.blue()]);
            service_for_color.send(Command::SetLaneFilament { lane, material: None, color });
        }
    });
}

/// Spoolman: the spool in use, the picker, weighing and the spool's temperatures.
fn bind_spools(actions: &Actions, service: &PrinterService, presenter: &Rc<Presenter>) {
    actions.on_load_spools(run_presenter(presenter, Presenter::load_spools));
    actions.on_set_active_spool(with(service, |service, id: i32| {
        service.send(Command::SetActiveSpool { spool_id: u32::try_from(id).ok() });
    }));
    let assign_service = service.clone();
    actions.on_assign_gate_spool(move |gate, id| {
        if let Ok(gate) = usize::try_from(gate) {
            assign_service.send(Command::AssignGateSpool { gate, spool_id: u32::try_from(id).ok() });
        }
    });
    let tool_service = service.clone();
    actions.on_assign_tool_spool(move |tool, id| {
        if let Ok(tool) = usize::try_from(tool) {
            tool_service.send(Command::AssignToolSpool { tool, spool_id: u32::try_from(id).ok() });
        }
    });
    let (measure_service, measure_presenter) = (service.clone(), Rc::clone(presenter));
    actions.on_measure_spool(move |id, text| match (u32::try_from(id), parse_weight(&text)) {
        (Ok(spool_id), Some(weight)) => measure_service.send(Command::MeasureSpool { spool_id, weight }),
        _ => measure_presenter.show_notice("Enter the weight of the spool in grams", true),
    });
    actions.on_apply_spool_preset(run_presenter(presenter, Presenter::apply_spool_preset));
}

/// The file list, the file prepared for the print page and the print itself.
fn bind_files_and_print(actions: &Actions, service: &PrinterService, presenter: &Rc<Presenter>) {
    actions.on_start_tools(with_presenter(presenter, |presenter, path: SharedString| {
        ModelRc::new(VecModel::from(presenter.start_tools(&path)))
    }));
    actions.on_refresh_files(run_presenter(presenter, Presenter::request_files));
    actions.on_toggle_file_mark(with_presenter(presenter, |presenter, path: SharedString| {
        presenter.toggle_file_mark(&path)
    }));
    actions.on_clear_file_marks(run_presenter(presenter, Presenter::clear_file_marks));
    actions.on_prepare_file(with_presenter(presenter, |presenter, path: SharedString| presenter.prepare_file(&path)));
    let (delete_presenter, delete_service) = (Rc::clone(presenter), service.clone());
    actions.on_delete_marked_files(move || {
        for path in delete_presenter.take_marked_files() {
            delete_service.send(Command::DeleteFile { path });
        }
    });
    actions.on_start_print(with(service, |service, filename: SharedString| {
        service.send(Command::StartPrint { filename: filename.into() });
    }));
    actions.on_pause_print(send(service, || Command::PausePrint));
    actions.on_resume_print(send(service, || Command::ResumePrint));
    actions.on_cancel_print(send(service, || Command::CancelPrint));
    let map_presenter = Rc::clone(presenter);
    actions.on_print_object_at(move |x_share, y_share, reach_share| {
        map_presenter
            .print_object_at(f64::from(x_share), f64::from(y_share), f64::from(reach_share))
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(-1)
    });
    actions.on_exclude_object(with(service, |service, name: SharedString| {
        service.send(Command::ExcludeObject { name: name.into() })
    }));
}

/// Login, emergency stop, restarts, power devices, updates and the host.
fn bind_system(actions: &Actions, service: &PrinterService) {
    let login_service = service.clone();
    actions.on_log_in(move |username, password| login_service.log_in(&username, &password));
    let logout_service = service.clone();
    actions.on_log_out(move || logout_service.log_out());
    actions.on_emergency_stop(send(service, || Command::EmergencyStop));
    actions.on_restart_klipper(send(service, || Command::RestartKlipper));
    actions.on_firmware_restart(send(service, || Command::FirmwareRestart));
    let system_service = service.clone();
    actions.on_refresh_system(move || {
        system_service.query(Query::PowerDevices);
        system_service.query(Query::Updates);
    });
    let power_service = service.clone();
    actions.on_set_power(move |device, on| power_service.send(Command::SetPowerDevice { device: device.into(), on }));
    actions.on_restart_service(with(service, |service, name: SharedString| {
        service.send(Command::RestartService { service: name.into() })
    }));
    actions.on_reboot_host(send(service, || Command::RebootHost));
    actions.on_shutdown_host(send(service, || Command::ShutdownHost));
    let updates_service = service.clone();
    actions.on_refresh_updates(move || {
        updates_service.send(Command::RefreshUpdates);
        updates_service.query(Query::Updates);
    });
    actions
        .on_upgrade(with(service, |service, name: SharedString| service.send(Command::Upgrade { name: name.into() })));
}

/// The console, the G-code line and the macros with their parameter form.
fn bind_console_and_macros(
    actions: &Actions,
    service: &PrinterService,
    presenter: &Rc<Presenter>,
    settings: &Rc<SettingsController>,
) {
    actions.on_run_gcode(with(service, |service, script: SharedString| {
        service.send(Command::RunGcode { script: script.into() })
    }));
    actions.on_run_macro(with(service, |service, name: SharedString| {
        service.send(Command::RunMacro { name: name.into() })
    }));
    let console_service = service.clone();
    actions.on_load_console(move || console_service.query(Query::ConsoleHistory));
    actions.on_clear_console(run_presenter(presenter, Presenter::clear_console));
    let macro_settings = settings.clone();
    actions.on_set_macro_visible(move |name, visible| macro_settings.set_macro_visible(name.as_str(), visible));
    actions.on_macro_has_params(with_presenter(presenter, |presenter, name: SharedString| {
        presenter.macro_has_params(&name)
    }));
    actions.on_open_macro_form(with_presenter(presenter, |presenter, name: SharedString| {
        presenter.open_macro_form(&name)
    }));
    let param_presenter = Rc::clone(presenter);
    actions.on_set_macro_param(move |index, value| {
        if let Ok(index) = usize::try_from(index) {
            param_presenter.set_macro_param(index, &value);
        }
    });
    actions.on_macro_form_script(run_presenter(presenter, |presenter| presenter.macro_form_script().into()));
}

/// The flyout of the status bar.
fn bind_status_messages(actions: &Actions, presenter: &Rc<Presenter>) {
    actions.on_messages_opened(run_presenter(presenter, Presenter::open_messages));
    actions.on_messages_closed(run_presenter(presenter, Presenter::close_messages));
    actions.on_messages_clear(run_presenter(presenter, Presenter::clear_messages));
}

/// Planning a pause at a layer. A short notice confirms what was sent; the plan itself shows on the Layer tile once
/// the macros' variables report it.
fn bind_layer_pause(actions: &Actions, service: &PrinterService, presenter: &Rc<Presenter>) {
    let (pause_service, pause_presenter) = (service.clone(), Rc::clone(presenter));
    actions.on_pause_at_layer(move |layer| {
        let Some(layer) = u32::try_from(layer).ok().filter(|layer| *layer > 0) else { return };
        pause_service.send(Command::PauseAtLayer { layer });
        pause_presenter.show_notice(&pause_presenter.layer_pause_notice(layer), false);
    });
    let (clear_service, clear_presenter) = (service.clone(), Rc::clone(presenter));
    actions.on_clear_layer_pause(move || {
        clear_service.send(Command::ClearLayerPause);
        clear_presenter.show_notice("Planned pauses cleared", false);
    });
    let height_presenter = Rc::clone(presenter);
    actions.on_height_at_layer(move |layer| height_presenter.height_at_layer_text(layer).into());
}

/// Types a key of the on-screen keyboard into the focused field, as a physical keyboard would. Deferred, because the
/// tap that caused it is still being handled.
fn press_key(window: &slint::Weak<AppWindow>, text: SharedString) {
    let window = window.clone();
    slint::Timer::single_shot(Duration::ZERO, move || {
        let Some(window) = window.upgrade() else { return };
        window.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.window().dispatch_event(WindowEvent::KeyReleased { text });
    });
}

/// A weight as typed, in grams; a comma as decimal separator is fine. Zero and negative are no weights.
fn parse_weight(text: &str) -> Option<f64> {
    text.trim().replace(',', ".").parse::<f64>().ok().filter(|weight| weight.is_finite() && *weight > 0.0)
}

/// Callback without arguments that sends the command.
fn send(service: &PrinterService, command: fn() -> Command) -> impl Fn() + 'static {
    let service = service.clone();
    move || service.send(command())
}

/// Callback with a gate number that sends the command for that gate; a negative number sends nothing.
fn with_gate(service: &PrinterService, command: fn(usize) -> Command) -> impl Fn(i32) + 'static {
    with(service, move |service, gate: i32| {
        if let Ok(gate) = usize::try_from(gate) {
            service.send(command(gate));
        }
    })
}

/// The lane of the tile in `row` of the Gates tab.
fn lane_of_row(presenter: &Presenter, row: i32) -> Option<LaneId> {
    usize::try_from(row).ok().and_then(|row| presenter.lane_id(row))
}

/// Callback with the row of a tile that sends the command for its lane; a row without lane sends nothing.
fn with_lane(
    service: &PrinterService,
    presenter: &Rc<Presenter>,
    command: fn(LaneId) -> Command,
) -> impl Fn(i32) + 'static {
    let (service, presenter) = (service.clone(), Rc::clone(presenter));
    move |row| {
        if let Some(lane) = lane_of_row(&presenter, row) {
            service.send(command(lane));
        }
    }
}

/// Callback with one argument that receives the presenter.
fn with_presenter<A, R, F>(presenter: &Rc<Presenter>, action: F) -> impl Fn(A) -> R + 'static
where
    F: Fn(&Presenter, A) -> R + 'static,
{
    let presenter = Rc::clone(presenter);
    move |argument| action(&presenter, argument)
}

/// Callback without arguments that receives the presenter.
fn run_presenter<R>(presenter: &Rc<Presenter>, action: impl Fn(&Presenter) -> R + 'static) -> impl Fn() -> R + 'static {
    let presenter = Rc::clone(presenter);
    move || action(&presenter)
}

/// Callback with arguments that receives a cloned service.
fn with<A, F>(service: &PrinterService, action: F) -> impl Fn(A) + 'static
where
    F: Fn(&PrinterService, A) + 'static,
{
    let service = service.clone();
    move |arguments| action(&service, arguments)
}

fn parse_axis(axis: &str) -> Option<Axis> {
    match axis {
        "x" | "X" => Some(Axis::X),
        "y" | "Y" => Some(Axis::Y),
        "z" | "Z" => Some(Axis::Z),
        _ => None,
    }
}

/// Slint computes in f32; small steps like 0.005 are smoothed to five decimal places.
fn round_offset(value: f32) -> f64 {
    (f64::from(value) * 100_000.0).round() / 100_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_names_and_rounding() {
        assert_eq!(parse_axis("y"), Some(Axis::Y));
        assert_eq!(parse_axis("e"), None);
        assert_eq!(round_offset(0.005_f32), 0.005);
        assert_eq!(round_offset(-0.025_f32), -0.025);
    }

    #[test]
    fn weights_are_read_with_a_point_or_a_comma_and_must_be_positive() {
        assert_eq!(parse_weight("793.5"), Some(793.5));
        assert_eq!(parse_weight(" 793,5 "), Some(793.5));
        assert_eq!(parse_weight("0"), None);
        assert_eq!(parse_weight("-3"), None);
        assert_eq!(parse_weight("abc"), None);
        assert_eq!(parse_weight(""), None);
    }
}
