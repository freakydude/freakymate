//! Turns core values into the row structs of the Slint models, and keeps those models up to date.

use crate::{format, view};
use crate::{
    BarColors, FanItem, FilamentPathItem, HeadItem, HeaterItem, HostInterfaceItem, LedEffectItem, LedItem, PowerItem,
    SensorItem, SpoolItem, ToolMatchItem, TuningItem, UpdateItem,
};
use freakymate_core::color::Rgba;
use freakymate_core::domain::filament_match::{self, Difference, Supply, ToolMatch, Verdict};
use freakymate_core::domain::heads::{HeadRow, PickUp};
use freakymate_core::domain::machine_tree::MachineTree;
use freakymate_core::domain::spools::{self, FilamentColors, Lane, SpoolHolder};
use freakymate_core::sources::happy_hare::{self, FilamentPath, PathSensor};
use freakymate_core::sources::klipper::tuning::TuningValue;
use freakymate_core::sources::klipper::{Fan, Heater, Led, TemperatureSensor};
use freakymate_core::sources::led_effect::LedEffect;
use freakymate_core::sources::moonraker_server::machine::HostInterface;
use freakymate_core::sources::moonraker_server::power::PowerDevice;
use freakymate_core::sources::moonraker_server::update_manager::UpdateInfo;
use freakymate_core::sources::spoolman::Spool;
use freakymate_core::sources::PrinterStatus;
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
            let place = lane.map(spool_place).unwrap_or_default();
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

/// Where a spool sits: the tool changer head, the unit (with several) and the gate or AFC lane, as far as there are
/// (a lane named like its head once); "In use" for the one direct feed.
fn spool_place(lane: &Lane) -> String {
    let slot = lane
        .gate
        .map(|gate| format!("Gate {gate}"))
        .or_else(|| lane.name.clone())
        .filter(|slot| lane.head.as_ref() != Some(slot));
    let parts: Vec<&str> =
        [lane.head.as_deref(), lane.unit_name.as_deref(), slot.as_deref()].into_iter().flatten().collect();
    if parts.is_empty() {
        return "In use".to_owned();
    }
    parts.join(" · ")
}

/// What the spool picker names as the place it assigns a spool to, for a lane kept through its tool macro: the AFC lane
/// (`lane2`) or the head (`T1`) of a tool changer; empty elsewhere, where the picker says what it did before.
pub(crate) fn spool_target(lane: &Lane) -> String {
    match (&lane.head, lane.spool_holder()) {
        (Some(head), Some(SpoolHolder::Tool(_))) => lane.name.clone().unwrap_or_else(|| head.clone()),
        _ => String::new(),
    }
}

/// The place of the spool in use for the picker title, see [`spool_target`].
pub(crate) fn spool_in_use_target(lanes: &[Lane]) -> String {
    lanes.iter().find(|lane| lane.active).map(spool_target).unwrap_or_default()
}

/// The nozzle heaters of the swappable heads as `(label, extruder)`, in head order; empty without heads.
pub(crate) fn head_nozzles(rows: &[HeadRow]) -> Vec<(&str, &str)> {
    rows.iter().filter_map(|row| Some((row.label.as_str(), row.extruder.as_deref()?))).collect()
}

/// The label (`T1`) of the mounted head; `None` without heads or while none is mounted.
pub(crate) fn selected_head(rows: &[HeadRow]) -> Option<&str> {
    rows.iter().find(|row| row.mounted).map(|row| row.label.as_str())
}

/// The printer has swappable heads but none is mounted, so no nozzle takes a target.
pub(crate) fn no_head_selected(tree: &MachineTree) -> bool {
    tree.has_swappable_heads() && tree.selected_extruder().is_none()
}

/// The heaters for the Thermal page: with heads, the nozzles of the heads first in head order, named after their head
/// ("Nozzle T1"), then the others as they come; without heads as they come.
pub(crate) fn heaters_by_head<'a>(
    heaters: &'a [Heater],
    nozzles: &[(&str, &str)],
) -> Vec<(&'a Heater, Option<String>)> {
    let mut rows: Vec<(&Heater, Option<String>)> = nozzles
        .iter()
        .filter_map(|(head, extruder)| {
            let heater = heaters.iter().find(|heater| heater.object == *extruder)?;
            Some((heater, Some(format!("Nozzle {head}"))))
        })
        .collect();
    let rest = heaters.iter().filter(|heater| !nozzles.iter().any(|(_, extruder)| heater.object == *extruder));
    rows.extend(rest.map(|heater| (heater, None)));
    rows
}

/// The row of a head for the Filament page: its nozzle, the spool of its lane in use and how it is picked up.
pub(crate) fn head_item(row: &HeadRow, status: &PrinterStatus) -> HeadItem {
    let nozzle = row.extruder.as_deref().and_then(|object| status.heater(object));
    let spool = row.lane.as_ref().and_then(|lane| lane.spool.as_ref());
    let holder = row.lane.as_ref().and_then(Lane::spool_holder);
    let (pick_up, pick_up_macro) = match row.pick_up {
        Some(PickUp::Macro(number)) => ("macro", format!("T{number}")),
        Some(PickUp::Head) => ("head", String::new()),
        None => ("", String::new()),
    };
    HeadItem {
        name: row.name.as_str().into(),
        label: row.label.as_str().into(),
        subtitle: row.subtitle.as_deref().unwrap_or_default().into(),
        has_nozzle: nozzle.is_some(),
        temperature: nozzle.map_or(0.0, |heater| heater.temperature) as f32,
        target: nozzle.map_or(0.0, |heater| heater.target) as f32,
        mounted: row.mounted,
        detected: row.detected,
        lanes: i32::try_from(row.lane_count).unwrap_or(i32::MAX),
        pick_up: pick_up.into(),
        pick_up_macro: pick_up_macro.into(),
        has_spool: spool.is_some(),
        spool_title: spool.map(Spool::title).unwrap_or_default().into(),
        spool_material: spool.map(|spool| spool.material.as_str()).unwrap_or_default().into(),
        spool_weight: spool.map(view::spool_remaining).unwrap_or_default().into(),
        spool_low: spool.is_some_and(Spool::is_low),
        spool_colors: spool.map(spool_colors).unwrap_or_default(),
        spool_id: id_or_none(spool.map(|spool| spool.id)),
        spool_tool: match holder {
            Some(SpoolHolder::Tool(tool)) => i32::try_from(tool).unwrap_or(-1),
            _ => -1,
        },
        can_pick_spool: matches!(holder, Some(SpoolHolder::Tool(_) | SpoolHolder::ActiveSpool)),
    }
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

/// The row of a heater; the hint says that the target is off from what the spools recommend.
pub(crate) fn heater_item(heater: &Heater, lanes: &[Lane], has_history: bool) -> HeaterItem {
    let recommended = spools::recommended_temperature(lanes, &heater.object);
    let spool_hint = spools::temperature_hint(heater.target, recommended)
        .map_or_else(SharedString::new, |degrees| format!("{degrees:.0}°").into());
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
        nozzle: heater.is_nozzle(),
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

/// `ToolMatchItem.spool_pick`: the tool has no spool of its own to pick.
const SPOOL_PICK_NONE: i32 = -2;
/// `ToolMatchItem.spool_pick`: the spool in use is picked.
const SPOOL_PICK_ACTIVE: i32 = -1;

/// A tool of Happy Hare (`gate` `None` for a tool of the file without gate) with what the file (`matches`) asks of it.
pub(crate) fn tool_match_item(tool: usize, gate: Option<usize>, matches: &[ToolMatch]) -> ToolMatchItem {
    let mut item = ToolMatchItem {
        tool: tool as i32,
        gate: id_or_none(gate),
        gate_row: -1,
        spool_pick: SPOOL_PICK_NONE,
        spool_id: -1,
        ..ToolMatchItem::default()
    };
    let Some(found) = matches.iter().find(|found| found.request.tool == tool) else { return item };
    let request = &found.request;
    item.used = true;
    item.wanted_colors =
        bar_colors(&FilamentColors { colors: request.color.into_iter().collect(), longitudinal: false });
    item.wanted_material = request.material.clone().unwrap_or_default().into();
    item.wanted_weight = request.grams.map(view::planned_weight).unwrap_or_default().into();
    item.wanted_profile = request.profile.clone().unwrap_or_default().into();
    item.needed_weight = found.needed.map(view::planned_weight).unwrap_or_default().into();
    item.shared_tools = gate
        .map(|gate| view::shared_tools_text(&filament_match::tools_of_gate(matches, gate)))
        .unwrap_or_default()
        .into();
    item.verdict = view::verdict_key(&found.verdict).into();
    item.material_differs = view::material_difference(found).is_some();
    item.material_line = view::material_line(found).into();
    item.color_differs = differs(found, |difference| matches!(difference, Difference::Color));
    item.weight_differs = differs(found, |difference| matches!(difference, Difference::Weight { .. }));
    item
}

/// The tools of the file that `keep` lets through, each with what it prints with: for the print page and the question
/// before the start, which show only the file's tools.
pub(crate) fn compared_tool_items(
    matches: &[ToolMatch],
    status: &PrinterStatus,
    lanes: &[Lane],
    keep: impl Fn(&ToolMatch) -> bool,
) -> Vec<ToolMatchItem> {
    matches
        .iter()
        .filter(|found| keep(found))
        .map(|found| {
            let mut item = tool_match_item(found.request.tool, found.gate, matches);
            if let Some(supply) = filament_match::tool_supply(found, status, lanes) {
                fill_loaded(&mut item, &supply, found.reserve);
            }
            fill_spool_pick(&mut item, lanes);
            item
        })
        .collect()
}

/// The tool macro that holds the spool in use, -1 when the spool in use is not kept by a tool macro.
pub(crate) fn spool_tool(lanes: &[Lane]) -> i32 {
    match lanes.iter().find(|lane| lane.active).and_then(Lane::spool_holder) {
        Some(SpoolHolder::Tool(tool)) => id_or_none(Some(tool)),
        _ => -1,
    }
}

/// Which spool a tool without gate can be assigned: the spool of its tool macro or the spool in use; none otherwise.
fn fill_spool_pick(item: &mut ToolMatchItem, lanes: &[Lane]) {
    let lane = usize::try_from(item.tool).ok().and_then(|tool| spools::tool_lane(lanes, tool));
    item.spool_target = lane.map(spool_target).unwrap_or_default().into();
    (item.spool_pick, item.spool_id) = match lane.and_then(Lane::spool_holder) {
        Some(SpoolHolder::Tool(tool)) => (id_or_none(Some(tool)), id_or_none(lane.and_then(|lane| lane.spool_id))),
        Some(SpoolHolder::ActiveSpool) => (SPOOL_PICK_ACTIVE, id_or_none(lane.and_then(|lane| lane.spool_id))),
        Some(SpoolHolder::Gate(_)) | None => (SPOOL_PICK_NONE, -1),
    };
}

fn differs(found: &ToolMatch, kind: impl Fn(&Difference) -> bool) -> bool {
    matches!(&found.verdict, Verdict::Differs(differences) if differences.iter().any(kind))
}

/// What a tool prints with: material, colours, name and the grams left; with an EndlessSpool `reserve` also all the
/// group can still give and what that adds, only when the own grams are known.
pub(crate) fn fill_loaded(item: &mut ToolMatchItem, supply: &Supply, reserve: Option<f64>) {
    item.gate_empty = supply.empty;
    item.loaded_material = supply.material.clone().unwrap_or_default().into();
    item.loaded_colors =
        bar_colors(&FilamentColors { colors: supply.colors.clone(), longitudinal: supply.longitudinal });
    item.loaded_name = supply.name.clone().unwrap_or_default().into();
    item.loaded_weight = supply.left.map(format::filament_weight).unwrap_or_default().into();
    let reserve = reserve.filter(|_| supply.left.is_some());
    item.endless_weight = supply
        .left
        .zip(reserve)
        .map(|(left, reserve)| format!("∞ {}", format::filament_weight(left + reserve)))
        .unwrap_or_default()
        .into();
    item.reserve_weight =
        reserve.map(|grams| format!("+{}", format::filament_weight(grams))).unwrap_or_default().into();
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

    fn lane(gate: Option<usize>, head: Option<&str>, spool_id: Option<u32>) -> Lane {
        Lane {
            gate,
            name: None,
            unit: None,
            unit_name: None,
            head: head.map(str::to_owned),
            extruder: "extruder".into(),
            tool: Some(1),
            spool_id,
            spool_by_tool: head.is_some(),
            spool: None,
            empty: false,
            active: false,
        }
    }

    #[test]
    fn a_spool_names_its_head_its_gate_or_that_it_is_in_use() {
        let spools: Vec<Spool> = (1..=4).map(|id| Spool { id, ..Spool::default() }).collect();
        let lanes = [lane(None, Some("T1"), Some(1)), lane(Some(2), None, Some(2)), lane(None, None, Some(3))];

        let rows = spool_rows(&spools, &lanes);
        let place = |id: i32| rows.iter().find(|row| row.id == id).map(|row| row.place.to_string());

        assert_eq!(place(1).as_deref(), Some("T1"));
        assert_eq!(place(2).as_deref(), Some("Gate 2"));
        assert_eq!(place(3).as_deref(), Some("In use"));
        assert_eq!(place(4).as_deref(), Some(""));
    }

    #[test]
    fn a_spool_names_the_unit_with_several_and_an_afc_lane_once() {
        let spools: Vec<Spool> = (1..=3).map(|id| Spool { id, ..Spool::default() }).collect();
        let unit_gate = Lane { unit_name: Some("Tradrack".into()), ..lane(Some(5), Some("T1"), Some(1)) };
        let afc_lane = Lane { name: Some("lane2".into()), ..lane(None, None, Some(2)) };
        let head_lane = Lane { name: Some("e0".into()), ..lane(None, Some("e0"), Some(3)) };

        let rows = spool_rows(&spools, &[unit_gate, afc_lane, head_lane]);
        let place = |id: i32| rows.iter().find(|row| row.id == id).map(|row| row.place.to_string());

        assert_eq!(place(1).as_deref(), Some("T1 · Tradrack · Gate 5"));
        assert_eq!(place(2).as_deref(), Some("lane2"));
        assert_eq!(place(3).as_deref(), Some("e0"));
    }

    #[test]
    fn a_head_row_names_its_pick_up_and_where_its_spool_is_assigned() {
        let row = |pick_up| HeadRow {
            name: "T1".into(),
            label: "T1".into(),
            subtitle: None,
            number: Some(1),
            extruder: Some("extruder1".into()),
            mounted: false,
            detected: true,
            lane_count: 1,
            lane: Some(lane(None, Some("T1"), None)),
            pick_up,
        };
        let status = PrinterStatus::default();
        let by_macro = head_item(&row(Some(PickUp::Macro(1))), &status);
        assert_eq!((by_macro.pick_up.as_str(), by_macro.pick_up_macro.as_str()), ("macro", "T1"));
        assert_eq!((by_macro.spool_tool, by_macro.can_pick_spool, by_macro.spool_id), (1, true, -1));
        assert!(by_macro.detected && !by_macro.has_nozzle);
        assert_eq!(head_item(&row(Some(PickUp::Head)), &status).pick_up.as_str(), "head");
        assert_eq!(head_item(&row(None), &status).pick_up.as_str(), "");
    }

    fn tree(heads: &[(Option<&str>, bool, &str)]) -> MachineTree {
        use freakymate_core::domain::machine_tree::{Carriage, Head, Nozzle};
        let head = |(name, active, extruder): &(Option<&str>, bool, &str)| Head {
            name: name.map(str::to_owned),
            active: *active,
            nozzles: vec![Nozzle { extruder: (*extruder).into(), lanes: Vec::new() }],
        };
        MachineTree { carriages: vec![Carriage { heads: heads.iter().map(head).collect() }] }
    }

    /// The rows of the swappable heads of `tree`, the head at position `n` answering to `T<n>`.
    fn head_rows(tree: &MachineTree) -> Vec<HeadRow> {
        let heads = tree.carriages.iter().flat_map(|carriage| &carriage.heads);
        heads
            .enumerate()
            .filter_map(|(number, head)| {
                let name = head.name.clone()?;
                Some(HeadRow {
                    label: format!("T{number}"),
                    subtitle: None,
                    name,
                    number: Some(number),
                    extruder: head.nozzles.first().map(|nozzle| nozzle.extruder.clone()),
                    mounted: head.active,
                    detected: false,
                    lane_count: 0,
                    lane: None,
                    pick_up: None,
                })
            })
            .collect()
    }

    fn heater(object: &str) -> Heater {
        Heater {
            object: object.into(),
            label: object.into(),
            temperature: 0.0,
            target: 0.0,
            power: 0.0,
            min_temp: 0.0,
            max_temp: None,
        }
    }

    #[test]
    fn the_picker_names_the_afc_lane_or_the_head_of_a_tool_macro() {
        let head = lane(None, Some("T1"), Some(7));
        assert_eq!(spool_target(&head), "T1");
        let afc = Lane { name: Some("lane2".into()), ..head.clone() };
        assert_eq!(spool_target(&afc), "lane2");
        assert_eq!(spool_target(&lane(Some(5), None, Some(7))), "");
        assert_eq!(spool_target(&lane(None, None, Some(7))), "");
        let mut in_use = head;
        in_use.active = true;
        assert_eq!(spool_in_use_target(&[in_use]), "T1");
    }

    #[test]
    fn the_thermal_page_lists_the_nozzles_by_head_before_the_bed() {
        let heaters = [heater("heater_bed"), heater("extruder1"), heater("extruder")];
        let heads = tree(&[(Some("T0"), false, "extruder"), (Some("T1"), true, "extruder1")]);
        let rows = heaters_by_head(&heaters, &head_nozzles(&head_rows(&heads)));
        let seen: Vec<(&str, Option<&str>)> =
            rows.iter().map(|(heater, name)| (heater.object.as_str(), name.as_deref())).collect();
        assert_eq!(seen, [("extruder", Some("Nozzle T0")), ("extruder1", Some("Nozzle T1")), ("heater_bed", None)]);
        let fixed = tree(&[(None, true, "extruder")]);
        assert!(head_nozzles(&head_rows(&fixed)).is_empty());
        assert_eq!(heaters_by_head(&heaters, &[]).len(), 3);
    }

    #[test]
    fn a_head_is_shown_by_its_number_even_when_its_section_has_another_name() {
        let heads = tree(&[(Some("left"), true, "extruder")]);
        let mut rows = head_rows(&heads);
        rows[0].label = "T4".into();
        assert_eq!(selected_head(&rows), Some("T4"));
        assert_eq!(heaters_by_head(&[heater("extruder")], &head_nozzles(&rows))[0].1.as_deref(), Some("Nozzle T4"));
    }

    #[test]
    fn the_status_bar_follows_the_mounted_head() {
        let mounted = tree(&[(Some("T0"), false, "extruder"), (Some("T1"), true, "extruder1")]);
        assert_eq!(selected_head(&head_rows(&mounted)), Some("T1"));
        assert!(!no_head_selected(&mounted));
        let none = tree(&[(Some("T0"), false, "extruder"), (Some("T1"), false, "extruder1")]);
        assert_eq!(selected_head(&head_rows(&none)), None);
        assert!(no_head_selected(&none));
        let fixed = tree(&[(None, true, "extruder")]);
        assert_eq!(selected_head(&head_rows(&fixed)), None);
        assert!(!no_head_selected(&fixed));
    }

    #[test]
    fn a_tool_picks_the_spool_of_its_macro_the_spool_in_use_or_none() {
        let mut item = tool_match_item(1, None, &[]);
        fill_spool_pick(&mut item, &[lane(None, Some("T1"), Some(7))]);
        assert_eq!((item.spool_pick, item.spool_id), (1, 7));

        let mut in_use = lane(None, None, Some(3));
        in_use.active = true;
        fill_spool_pick(&mut item, &[in_use]);
        assert_eq!((item.spool_pick, item.spool_id), (SPOOL_PICK_ACTIVE, 3));

        fill_spool_pick(&mut item, &[lane(None, None, None)]);
        assert_eq!((item.spool_pick, item.spool_id), (SPOOL_PICK_NONE, -1));
    }

    #[test]
    fn the_spool_in_use_belongs_to_a_tool_only_through_its_macro() {
        let mut head = lane(None, Some("T1"), Some(7));
        assert_eq!(spool_tool(std::slice::from_ref(&head)), -1);
        head.active = true;
        assert_eq!(spool_tool(&[head]), 1);
    }

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
            needed: Some(12.0),
            reserve: None,
            verdict,
        }
    }

    #[test]
    fn a_tool_without_a_match_is_unused_and_a_tool_without_gate_has_minus_one() {
        let item = tool_match_item(4, None, &[]);

        assert_eq!((item.tool, item.gate, item.used), (4, -1, false));
        assert!(item.wanted_material.is_empty() && item.verdict.is_empty());
    }

    #[test]
    fn a_matched_tool_carries_what_the_file_asks_and_the_verdict() {
        let found = tool_match(Verdict::GateEmpty);

        let item = tool_match_item(2, Some(1), std::slice::from_ref(&found));

        assert_eq!((item.tool, item.gate, item.used), (2, 1, true));
        assert_eq!(item.wanted_colors.count, 1);
        assert_eq!(item.wanted_colors.first, color_of([1, 2, 3, 0xFF]));
        assert_eq!((item.wanted_material.as_str(), item.wanted_profile.as_str()), ("PETG", "Generic PETG"));
        assert_eq!(item.verdict.as_str(), "gate-empty");
        assert_eq!((item.wanted_weight.as_str(), item.needed_weight.as_str()), ("~12 g", "~12 g"));
        assert_eq!(item.shared_tools.as_str(), "");
    }

    #[test]
    fn tools_sharing_a_gate_compare_their_weight_together_and_are_named() {
        let mut other = tool_match(Verdict::Fits);
        other.request.tool = 0;
        let matches = [other, tool_match(Verdict::Fits)];

        let item = tool_match_item(2, Some(1), &matches);

        assert_eq!(item.shared_tools.as_str(), "T0 + T2");
    }

    #[test]
    fn each_difference_has_its_own_flag() {
        let found = tool_match(Verdict::Differs(vec![Difference::Weight { needed: 18.0, left: 14.0, reserve: None }]));

        let item = tool_match_item(2, Some(1), &[found]);

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
        let mut item = tool_match_item(1, Some(3), &[]);

        fill_loaded(&mut item, &supply, Some(230.0));

        assert_eq!((item.loaded_material.as_str(), item.loaded_name.as_str()), ("PLA", "Eryone Silk Dual"));
        assert_eq!((item.loaded_weight.as_str(), item.endless_weight.as_str()), ("14 g", "∞ 244 g"));
        assert_eq!(item.reserve_weight.as_str(), "+230 g");
        assert_eq!(item.loaded_colors.count, 2);

        // Without its own grams there is no sum to show, and the reserve alone says nothing.
        fill_loaded(&mut item, &Supply { left: None, ..supply }, Some(230.0));
        assert_eq!((item.loaded_weight.as_str(), item.endless_weight.as_str()), ("", ""));
        assert_eq!(item.reserve_weight.as_str(), "");
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
