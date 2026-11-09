//! Turns core values into the row structs of the Slint models, and keeps those models up to date.

use crate::{format, view};
use crate::{
    BarColors, FanItem, FilamentPathItem, HeaterItem, HostInterfaceItem, LedEffectItem, LedItem, PowerItem, SensorItem,
    SpoolItem, ToolMatchItem, TuningItem, UpdateItem,
};
use freakymate_core::color::Rgba;
use freakymate_core::domain::filament_match::{Difference, Supply, ToolMatch, Verdict};
use freakymate_core::domain::spools::{self, FilamentColors, Lane};
use freakymate_core::sources::happy_hare::{self, FilamentPath, PathSensor};
use freakymate_core::sources::klipper::tuning::TuningValue;
use freakymate_core::sources::klipper::{Fan, Heater, Led, TemperatureSensor, BED_HEATER, NOZZLE_HEATER};
use freakymate_core::sources::led_effect::LedEffect;
use freakymate_core::sources::moonraker_server::machine::HostInterface;
use freakymate_core::sources::moonraker_server::power::PowerDevice;
use freakymate_core::sources::moonraker_server::update_manager::UpdateInfo;
use freakymate_core::sources::spoolman::Spool;
use freakymate_core::ConnectionState;
use slint::{Model, ModelRc, SharedString, VecModel};

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
            SpoolItem {
                id: spool.id as i32,
                title: spool.title().into(),
                material: spool.material.as_str().into(),
                remaining: view::spool_remaining(spool).into(),
                colors: spool_colors(spool),
                place: place.into(),
                low: spool.is_low(),
            }
        })
        .collect()
}

pub(crate) fn color_of([red, green, blue, alpha]: Rgba) -> slint::Color {
    slint::Color::from_argb_u8(alpha, red, green, blue)
}

/// Colours a filament shows at most: a rainbow along the filament; a bar shows the first three of them.
const MAX_COLORS: usize = 6;

/// The colours of a bar or swatch: up to six, and how many there are.
pub(crate) fn bar_colors(colors: &FilamentColors) -> BarColors {
    let color = |index: usize| colors.colors.get(index).copied().map_or_else(slint::Color::default, color_of);
    BarColors {
        first: color(0),
        second: color(1),
        third: color(2),
        fourth: color(3),
        fifth: color(4),
        sixth: color(5),
        count: colors.colors.len().min(MAX_COLORS) as i32,
        longitudinal: colors.longitudinal,
    }
}

/// The colours of a spool's bar; none without a spool.
pub(crate) fn spool_colors(spool: &Spool) -> BarColors {
    bar_colors(&FilamentColors { colors: spool.colors.clone(), longitudinal: spool.has_longitudinal_colors() })
}

/// The row of a heater; the hint says that the target is off from what the spool in use recommends.
pub(crate) fn heater_item(heater: &Heater, spool: Option<&Spool>, has_history: bool) -> HeaterItem {
    let recommended = spool.and_then(|spool| match heater.object.as_str() {
        NOZZLE_HEATER => spool.extruder_temp,
        BED_HEATER => spool.bed_temp,
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
    let detail = if update.is_system() {
        format!("{} packages", update.pending)
    } else if update.has_update() {
        format!("{} → {}", update.version, update.remote_version)
    } else {
        update.version.clone()
    };
    UpdateItem {
        // Moonraker looks the section up by its exact name; only the label is capitalised.
        name: update.name.as_str().into(),
        label: format::service_name(&update.name).into(),
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
pub(crate) fn filament_path_item(path: &FilamentPath, gate: i32, colors: &FilamentColors) -> FilamentPathItem {
    let sensor = |sensor: PathSensor| match sensor {
        PathSensor::Absent => -1,
        PathSensor::Free => 0,
        PathSensor::Triggered => 1,
    };
    FilamentPathItem {
        position: path.position,
        direction: path.direction,
        distance: view::path_distance(path.distance).into(),
        gate_label: match gate {
            happy_hare::BYPASS => "Bypass".into(),
            gate if gate >= 0 => format!("Gate {gate}").into(),
            _ => SharedString::new(),
        },
        bowden_progress: path.bowden_progress.map_or(-1.0, |progress| progress as f32),
        gate_entry_sensor: sensor(path.gate_entry_sensor),
        gate_sensor: sensor(path.gate_exit_sensor),
        extruder_sensor: sensor(path.extruder_sensor),
        toolhead_sensor: sensor(path.toolhead_sensor),
        colors: bar_colors(colors),
    }
}

/// An id, index or group number for a Slint model: -1 when there is none or it does not fit.
pub(crate) fn id_or_none<T: TryInto<i32>>(id: Option<T>) -> i32 {
    id.and_then(|id| id.try_into().ok()).unwrap_or(-1)
}

/// A tool of Happy Hare (`gate` `None` for a tool of the file without gate) with what the file asks of it.
pub(crate) fn tool_match_item(tool: usize, gate: Option<usize>, found: Option<&ToolMatch>) -> ToolMatchItem {
    let mut item =
        ToolMatchItem { tool: tool as i32, gate: id_or_none(gate), gate_to_load: -1, ..ToolMatchItem::default() };
    let Some(found) = found else { return item };
    let request = &found.request;
    item.used = true;
    item.wanted_colors =
        bar_colors(&FilamentColors { colors: request.color.into_iter().collect(), longitudinal: false });
    item.wanted_material = request.material.clone().unwrap_or_default().into();
    item.wanted_weight = request.grams.map(format::filament_weight).unwrap_or_default().into();
    item.wanted_profile = request.profile.clone().unwrap_or_default().into();
    item.verdict = view::verdict_key(&found.verdict).into();
    item.differences = view::tool_differences(found).into();
    item.material_differs = view::material_difference(found).is_some();
    item.material_line = view::material_line(found).into();
    item.color_differs = differs(found, |difference| matches!(difference, Difference::Color));
    item.weight_differs = differs(found, |difference| matches!(difference, Difference::Weight { .. }));
    item
}

fn differs(found: &ToolMatch, kind: impl Fn(&Difference) -> bool) -> bool {
    matches!(&found.verdict, Verdict::Differs(differences) if differences.iter().any(kind))
}

/// What the gate of a tool holds: material, colours, name and the grams left; with an EndlessSpool `reserve` also all
/// the group can still give.
pub(crate) fn fill_loaded(item: &mut ToolMatchItem, supply: &Supply, reserve: Option<f64>, empty: bool) {
    item.gate_empty = empty;
    item.loaded_material = supply.material.clone().unwrap_or_default().into();
    item.loaded_colors =
        bar_colors(&FilamentColors { colors: supply.colors.clone(), longitudinal: supply.longitudinal });
    item.loaded_name = supply.name.clone().unwrap_or_default().into();
    item.loaded_weight = supply.left.map(format::filament_weight).unwrap_or_default().into();
    item.endless_weight = reserve
        .map(|reserve| format!("∞ {}", format::filament_weight(supply.left.unwrap_or(0.0) + reserve)))
        .unwrap_or_default()
        .into();
}

/// The flags as a model, the old one while they are unchanged: a new model counts as a changed row and would rebuild
/// the tiles on every status update.
pub(crate) fn reuse_flags(old: Option<ModelRc<bool>>, flags: Option<Vec<bool>>) -> ModelRc<bool> {
    let flags = flags.unwrap_or_default();
    match old {
        Some(old) if old.iter().eq(flags.iter().copied()) => old,
        _ => ModelRc::new(VecModel::from(flags)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freakymate_core::domain::filament_match::ToolRequest;

    #[test]
    fn a_colour_bar_has_up_to_six_colours() {
        let red = [255, 0, 0, 255];
        let bar = |colors: Vec<Rgba>, longitudinal| bar_colors(&FilamentColors { colors, longitudinal });
        assert_eq!(bar(vec![], false).count, 0);
        assert_eq!(bar(vec![red], false).count, 1);
        assert_eq!(bar(vec![red, [0, 255, 0, 255]], false).count, 2);
        assert_eq!(bar(vec![red; 7], false).count, 6);
        assert_eq!(bar(vec![red, [0, 255, 0, 255]], true).second, slint::Color::from_rgb_u8(0, 255, 0));
        assert!(bar(vec![red], true).longitudinal);
    }

    #[test]
    fn the_alpha_of_a_colour_reaches_the_ui() {
        assert_eq!(color_of([255, 0, 0, 0x80]), slint::Color::from_argb_u8(0x80, 255, 0, 0));
    }

    #[test]
    fn a_missing_or_too_large_id_becomes_minus_one() {
        assert_eq!(id_or_none(Some(7_u32)), 7);
        assert_eq!(id_or_none(Some(3_usize)), 3);
        assert_eq!(id_or_none(None::<u32>), -1);
        assert_eq!(id_or_none(Some(u32::MAX)), -1);
    }

    fn tool_match(verdict: Verdict) -> ToolMatch {
        ToolMatch {
            request: ToolRequest {
                tool: 2,
                material: Some("PETG".into()),
                profile: Some("Generic PETG".into()),
                color: Some([1, 2, 3, 0xFF]),
                grams: Some(12.0),
                temperature: None,
            },
            gate: Some(1),
            verdict,
        }
    }

    #[test]
    fn a_tool_without_a_match_is_unused_and_a_tool_without_gate_has_minus_one() {
        let item = tool_match_item(4, None, None);

        assert_eq!((item.tool, item.gate, item.used), (4, -1, false));
        assert!(item.wanted_material.is_empty() && item.verdict.is_empty());
    }

    #[test]
    fn a_matched_tool_carries_what_the_file_asks_and_the_verdict() {
        let found = tool_match(Verdict::GateEmpty);

        let item = tool_match_item(2, Some(1), Some(&found));

        assert_eq!((item.tool, item.gate, item.used), (2, 1, true));
        assert_eq!(item.wanted_colors.count, 1);
        assert_eq!(item.wanted_colors.first, color_of([1, 2, 3, 0xFF]));
        assert_eq!((item.wanted_material.as_str(), item.wanted_profile.as_str()), ("PETG", "Generic PETG"));
        assert_eq!(item.verdict.as_str(), "gate-empty");
        assert_eq!(item.differences.as_str(), "Gate 1 is empty.");
    }

    #[test]
    fn each_difference_has_its_own_flag() {
        let found = tool_match(Verdict::Differs(vec![Difference::Weight { needed: 18.0, left: 14.0 }]));

        let item = tool_match_item(1, Some(3), Some(&found));

        assert!(item.weight_differs && !item.color_differs && !item.material_differs);
    }

    #[test]
    fn the_gate_side_shows_the_spool_and_what_endless_spool_adds() {
        let supply = Supply {
            material: Some("PLA".into()),
            colors: vec![[1, 2, 3, 0xFF], [4, 5, 6, 0xFF]],
            name: Some("Eryone Silk Dual".into()),
            left: Some(14.0),
            ..Supply::default()
        };
        let mut item = tool_match_item(1, Some(3), None);

        fill_loaded(&mut item, &supply, Some(230.0), false);

        assert_eq!((item.loaded_material.as_str(), item.loaded_name.as_str()), ("PLA", "Eryone Silk Dual"));
        assert_eq!((item.loaded_weight.as_str(), item.endless_weight.as_str()), ("14 g", "∞ 244 g"));
        assert_eq!(item.loaded_colors.count, 2);
    }

    #[test]
    fn unchanged_flags_keep_their_model_and_changed_ones_get_a_new_one() {
        let first = reuse_flags(None, Some(vec![true, false]));
        let kept = reuse_flags(Some(first.clone()), Some(vec![true, false]));
        let changed = reuse_flags(Some(first.clone()), Some(vec![true, true]));

        assert!(std::ptr::addr_eq(first.as_any(), kept.as_any()));
        assert!(!std::ptr::addr_eq(first.as_any(), changed.as_any()));
        assert_eq!(changed.iter().collect::<Vec<_>>(), [true, true]);
        assert_eq!(reuse_flags(None, None).row_count(), 0);
    }

    #[test]
    fn update_items_keep_the_section_name_for_the_upgrade_and_capitalise_only_the_label() {
        let update = UpdateInfo { name: "freakymate".into(), version: "0.4.5".into(), ..UpdateInfo::default() };

        let item = update_item(&update);

        assert_eq!(item.name, "freakymate");
        assert_eq!(item.label, "Freakymate");
    }
}
