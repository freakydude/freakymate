//! Turns core values into the row structs of the Slint models, and keeps those models up to date.

use crate::{format, view};
use crate::{
    ConsoleItem, FanItem, FilamentPathItem, HeaterItem, HostInterfaceItem, LedEffectItem, LedItem, PowerItem,
    SensorItem, SpoolItem, TuningItem, UpdateItem,
};
use freakymate_core::color::Rgba;
use freakymate_core::domain::spools::{self, Lane};
use freakymate_core::sources::happy_hare::{FilamentPath, PathSensor};
use freakymate_core::sources::klipper::tuning::TuningValue;
use freakymate_core::sources::klipper::{Fan, Heater, Led, TemperatureSensor};
use freakymate_core::sources::led_effect::LedEffect;
use freakymate_core::sources::moonraker_server::gcode_store::{ConsoleKind, ConsoleLine};
use freakymate_core::sources::moonraker_server::machine::HostInterface;
use freakymate_core::sources::moonraker_server::power::PowerDevice;
use freakymate_core::sources::moonraker_server::update_manager::UpdateInfo;
use freakymate_core::sources::spoolman::Spool;
use freakymate_core::ConnectionState;
use slint::{Model, SharedString, VecModel};

pub(crate) fn connection_name(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Connecting => "connecting",
        ConnectionState::Connected => "connected",
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::LoginRequired => "login-required",
    }
}

/// Brings the rows up to date without rebuilding them: a rebuild (`set_vec`) destroys the buttons in the rows, and a tap
/// that is still going on when the next status arrives would be lost. Only a different number of rows rebuilds the list.
pub(crate) fn replace_rows<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (index, row) in rows.into_iter().enumerate() {
        if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
        }
    }
}

/// The picker rows: spools in a lane first, then the most recently used, each with its place.
pub(crate) fn spool_rows(spools: &[Spool], lanes: &[Lane]) -> Vec<SpoolItem> {
    let inserted: Vec<u32> = lanes.iter().filter_map(|lane| lane.spool_id).collect();
    spools::picker_order(spools.to_vec(), &inserted)
        .iter()
        .map(|spool| {
            let lane = lanes.iter().find(|lane| lane.spool_id == Some(spool.id));
            let place = match lane {
                Some(lane) => lane.gate.map_or_else(|| "In use".to_owned(), |gate| format!("Gate {gate}")),
                None => String::new(),
            };
            let bar = color_bar(&spool.colors);
            SpoolItem {
                id: spool.id as i32,
                title: spool.title().into(),
                material: spool.material.as_str().into(),
                remaining: view::spool_remaining(spool).into(),
                color: bar.0,
                color_2: bar.1,
                color_3: bar.2,
                color_count: bar.3,
                longitudinal: spool.longitudinal && spool.colors.len() > 1,
                place: place.into(),
                low: spool.is_low(),
            }
        })
        .collect()
}

pub(crate) fn color_of([red, green, blue, alpha]: Rgba) -> slint::Color {
    slint::Color::from_argb_u8(alpha, red, green, blue)
}

/// The colours of a bar: up to three, and how many there are.
pub(crate) fn color_bar(colors: &[Rgba]) -> (slint::Color, slint::Color, slint::Color, i32) {
    let color = |index: usize| colors.get(index).copied().map_or_else(slint::Color::default, color_of);
    (color(0), color(1), color(2), colors.len().min(3) as i32)
}

/// The row of a heater; the hint says that the target is off from what the spool in use recommends.
pub(crate) fn heater_item(heater: &Heater, spool: Option<&Spool>, has_history: bool) -> HeaterItem {
    let recommended = spool.and_then(|spool| match heater.object.as_str() {
        "extruder" => spool.extruder_temp,
        "heater_bed" => spool.bed_temp,
        _ => None,
    });
    let spool_hint = spools::temperature_hint(heater.target, recommended)
        .map_or_else(SharedString::new, |degrees| format!("Spool {degrees:.0}°").into());
    HeaterItem {
        spool_hint,
        object: heater.object.as_str().into(),
        label: heater.label.as_str().into(),
        temperature: heater.temperature as f32,
        target: heater.target as f32,
        power: heater.power as f32,
        can_raise: heater.can_raise(),
        target_range: heater.max_temp.map_or_else(SharedString::new, |maximum| {
            format!("Klipper: {:.0}–{maximum:.0} °C", heater.min_temp).into()
        }),
        has_history,
    }
}

/// One row per message line: the console list has fixed row heights, so a line break inside a row would overlap its neighbours.
/// Only the first row of a message shows the time.
pub(crate) fn console_items(line: &ConsoleLine) -> Vec<ConsoleItem> {
    let time = format::unix_clock(line.time);
    let is_command = line.kind == ConsoleKind::Command;
    let is_error = line.is_error();
    let text = line.text();
    let mut rows: Vec<&str> = text.lines().collect();
    if rows.is_empty() {
        rows.push("");
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, text)| ConsoleItem {
            text: text.into(),
            time: if index == 0 { time.as_str().into() } else { "".into() },
            is_command,
            is_error,
        })
        .collect()
}

pub(crate) fn power_item(device: &PowerDevice) -> PowerItem {
    PowerItem { name: device.device.as_str().into(), on: device.is_on(), locked: device.locked_while_printing }
}

pub(crate) fn host_interface_item(interface: &HostInterface) -> HostInterfaceItem {
    HostInterfaceItem {
        name: interface.name.as_str().into(),
        wireless: interface.is_wireless(),
        addresses: interface.addresses.join(", ").into(),
        mac: interface.mac_address.as_str().into(),
    }
}

pub(crate) fn update_item(update: &UpdateInfo) -> UpdateItem {
    let detail = if update.name == "system" {
        format!("{} packages", update.pending)
    } else if update.has_update() {
        format!("{} → {}", update.version, update.remote_version)
    } else {
        update.version.clone()
    };
    UpdateItem {
        name: update.name.as_str().into(),
        version: update.version.as_str().into(),
        detail: detail.into(),
        has_update: update.has_update(),
        up_to_date: update.is_up_to_date(),
    }
}

pub(crate) fn sensor_item(sensor: &TemperatureSensor, has_history: bool) -> SensorItem {
    SensorItem {
        object: sensor.object.as_str().into(),
        label: sensor.label.as_str().into(),
        temperature: sensor.temperature.unwrap_or_default() as f32,
        known: sensor.temperature.is_some(),
        has_history,
    }
}

pub(crate) fn fan_item(fan: &Fan) -> FanItem {
    FanItem {
        object: fan.object.as_str().into(),
        label: fan.label.as_str().into(),
        kind: view::fan_kind(fan.kind).into(),
        controllable: fan.kind.is_controllable(),
        speed: (fan.speed * 100.0) as f32,
        rpm: view::fan_rpm(fan.rpm).into(),
        stalled: fan.stalled,
    }
}

pub(crate) fn led_item(led: &Led) -> LedItem {
    LedItem {
        object: led.object.as_str().into(),
        label: led.label.as_str().into(),
        on: led.on,
        level_known: led.level.is_some(),
        level: (led.level.unwrap_or_default() * 100.0) as f32,
    }
}

pub(crate) fn led_effect_item(effect: &LedEffect) -> LedEffectItem {
    LedEffectItem { name: effect.name.as_str().into(), label: effect.label.as_str().into(), enabled: effect.enabled }
}

pub(crate) fn tuning_item(value: &TuningValue) -> TuningItem {
    let range = value.key.range();
    TuningItem {
        id: value.key.id().into(),
        value: view::tuning_value(value.current, &range).into(),
        configured: value
            .configured
            .map(|configured| view::tuning_value(configured, &range))
            .unwrap_or_default()
            .into(),
        changed: value.is_changed(),
        can_reset: value.is_changed(),
    }
}

/// The way of the filament of the selected gate, in the colours of what is in it (see `gate_colors`).
pub(crate) fn filament_path_item(
    path: &FilamentPath,
    gate: i32,
    colors: &[Rgba],
    longitudinal: bool,
) -> FilamentPathItem {
    let sensor = |sensor: PathSensor| match sensor {
        PathSensor::Absent => -1,
        PathSensor::Free => 0,
        PathSensor::Triggered => 1,
    };
    let bar = color_bar(colors);
    FilamentPathItem {
        position: path.position,
        direction: path.direction,
        distance: view::path_distance(path.distance).into(),
        gate_label: match gate {
            -2 => "Bypass".into(),
            gate if gate >= 0 => format!("Gate {gate}").into(),
            _ => SharedString::new(),
        },
        bowden_progress: path.bowden_progress.map_or(-1.0, |progress| progress as f32),
        gate_entry_sensor: sensor(path.gate_entry_sensor),
        gate_sensor: sensor(path.gate_exit_sensor),
        extruder_sensor: sensor(path.extruder_sensor),
        toolhead_sensor: sensor(path.toolhead_sensor),
        color: bar.0,
        color_2: bar.1,
        color_3: bar.2,
        color_count: bar.3,
        longitudinal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: ConsoleKind, message: &str) -> ConsoleLine {
        ConsoleLine { kind, message: message.to_owned(), time: 0.0 }
    }

    #[test]
    fn a_colour_bar_has_up_to_three_colours() {
        let red = [255, 0, 0, 255];
        assert_eq!(color_bar(&[]).3, 0);
        assert_eq!(color_bar(&[red]).3, 1);
        assert_eq!(color_bar(&[red, [0, 255, 0, 255]]).3, 2);
        assert_eq!(color_bar(&[red; 5]).3, 3);
        assert_eq!(color_bar(&[red, [0, 255, 0, 255]]).1, slint::Color::from_rgb_u8(0, 255, 0));
    }

    #[test]
    fn the_alpha_of_a_colour_reaches_the_ui() {
        assert_eq!(color_of([255, 0, 0, 0x80]), slint::Color::from_argb_u8(0x80, 255, 0, 0));
    }

    #[test]
    fn multi_line_message_becomes_one_row_per_line() {
        let rows = console_items(&line(ConsoleKind::Response, "first\nsecond\r\nthird"));
        let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, ["first", "second", "third"]);
        assert!(rows[1].time.is_empty() && rows[2].time.is_empty());
    }

    #[test]
    fn empty_message_keeps_one_row() {
        let rows = console_items(&line(ConsoleKind::Response, ""));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].text.is_empty());
    }

    #[test]
    fn plugin_markup_does_not_reach_the_console() {
        let rows = console_items(&line(
            ConsoleKind::Response,
            r#"// <b><span style="color:#87CEEB">EMPTY</span></b>&nbsp;gate"#,
        ));
        assert_eq!(rows[0].text, "EMPTY gate");
    }

    #[test]
    fn error_flag_applies_to_every_row() {
        let rows = console_items(&line(ConsoleKind::Response, "!! Lost connection\nsee log"));
        assert_eq!(rows[0].text, "Lost connection");
        assert!(rows.iter().all(|row| row.is_error));
        assert!(!rows.iter().any(|row| row.is_command));
    }
}
