//! The gates of the MMU: availability, loaded filament, colours and tool mapping.

use crate::color::{parse_hex, Rgba};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateAvailability {
    #[default]
    Unknown,
    Empty,
    Available,
    /// Filament is available from the buffer of the gate.
    Buffered,
}

impl GateAvailability {
    pub(super) fn parse(value: i64) -> Self {
        match value {
            0 => Self::Empty,
            1 => Self::Available,
            2 => Self::Buffered,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoadedFilament {
    Loaded,
    Unloaded,
    #[default]
    Unknown,
}

impl LoadedFilament {
    pub(super) fn parse(text: &str) -> Self {
        match text {
            "Loaded" => Self::Loaded,
            "Unloaded" => Self::Unloaded,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MmuGate {
    pub index: usize,
    pub availability: GateAvailability,
    pub name: String,
    pub material: String,
    /// Only known when the gate has a colour set (`gate_color` is not empty).
    pub color: Option<Rgba>,
    /// Tools that take their filament from this gate (`ttg_map`), ascending; none, one or several.
    pub tools: Vec<usize>,
    /// EndlessSpool group (`endless_spool_groups`); gates of one group stand in for each other on a runout.
    pub endless_spool_group: Option<u32>,
    /// Spoolman spool of the gate (`gate_spool_id`); `None` for none (Happy Hare reports -1).
    pub spool_id: Option<u32>,
    /// Print temperature of the gate in °C (`gate_temperature`), from Spoolman if the gate has a spool.
    pub temperature: Option<f64>,
}

/// `gate` and `tool` while the bypass is selected (`TOOL_GATE_BYPASS` in `mmu_constants.py`).
pub const BYPASS: i32 = -2;

/// Happy Hare reports colours as floats from 0 to 1, a channel of 8 bits runs to 255.
const COLOR_CHANNEL_MAX: f64 = 255.0;

/// The gate of each tool (`ttg_map`).
pub(super) fn read_tool_to_gate(mmu: &Value) -> Vec<usize> {
    mmu.get("ttg_map")
        .and_then(Value::as_array)
        .map(|map| map.iter().filter_map(|gate| gate.as_u64().map(|gate| gate as usize)).collect())
        .unwrap_or_default()
}

pub(super) fn read_gates(mmu: &Value, num_gates: usize, tool_to_gate: &[usize]) -> Vec<MmuGate> {
    let list = |field: &str| mmu.get(field).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let (status, names, materials, colors, rgb, groups, spool_ids, temperatures) = (
        list("gate_status"),
        list("gate_filament_name"),
        list("gate_material"),
        list("gate_color"),
        list("gate_color_rgb"),
        list("endless_spool_groups"),
        list("gate_spool_id"),
        list("gate_temperature"),
    );
    let string_at =
        |values: &[Value], index: usize| values.get(index).and_then(Value::as_str).unwrap_or_default().to_owned();
    (0..num_gates)
        .map(|index| MmuGate {
            index,
            availability: status
                .get(index)
                .and_then(Value::as_i64)
                .map_or(GateAvailability::Unknown, GateAvailability::parse),
            name: string_at(names, index),
            material: string_at(materials, index),
            color: gate_color(&string_at(colors, index), rgb.get(index)),
            tools: tool_to_gate.iter().enumerate().filter(|(_, gate)| **gate == index).map(|(tool, _)| tool).collect(),
            endless_spool_group: groups.get(index).and_then(Value::as_u64).and_then(|group| u32::try_from(group).ok()),
            spool_id: spool_ids.get(index).and_then(Value::as_i64).and_then(|id| u32::try_from(id).ok()),
            temperature: temperatures.get(index).and_then(Value::as_f64).filter(|degrees| *degrees > 0.0),
        })
        .collect()
}

/// The colour of a gate: red, green and blue from `gate_color_rgb` (three floats from 0 to 1, which cuts the alpha
/// channel off), the alpha from `gate_color` when that is `rrggbbaa` (`gate_color` may also be a colour name).
pub(super) fn gate_color(text: &str, rgb: Option<&Value>) -> Option<Rgba> {
    if text.is_empty() {
        return None;
    }
    let [red, green, blue] = color_from_rgb(rgb?)?;
    let alpha = parse_hex(text).map_or(0xff, |[.., alpha]| alpha);
    Some([red, green, blue, alpha])
}

fn color_from_rgb(value: &Value) -> Option<[u8; 3]> {
    let channels = value.as_array()?;
    let channel = |index: usize| {
        channels.get(index).and_then(Value::as_f64).map(|part| (part.clamp(0.0, 1.0) * COLOR_CHANNEL_MAX).round() as u8)
    };
    Some([channel(0)?, channel(1)?, channel(2)?])
}
