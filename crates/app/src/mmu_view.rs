//! The Filament page's view of Happy Hare and AFC: the tiles of the Gates tab (gates or lanes), the load path of the
//! lane last tapped, and for Happy Hare the tools with what the job's file asks of them, the sensors and the state of
//! the MMU.

use crate::items::{
    bar_colors, compared_tool_items, fill_loaded, id_or_none, lane_colors, load_path_item, no_path_item, replace_rows,
    reuse_flags, tool_match_item,
};
use crate::{view, FilamentPathItem, MmuGateItem, MmuGatePage, MmuPageRange, MmuSensorItem, Printer, ToolMatchItem};
use freakymate_core::domain::filament_match::{self, ToolMatch, Verdict};
use freakymate_core::domain::gate_pages::{self, BypassAt, GatePage};
use freakymate_core::domain::heads::HeadRow;
use freakymate_core::domain::load_path::{self, BufferSide, LoadPath};
use freakymate_core::domain::machine_tree::LaneId;
use freakymate_core::domain::spools::{FilamentColors, Lane};
use freakymate_core::sources::happy_hare::{self, LoadedFilament, MmuStatus};
use freakymate_core::sources::spoolman::Spool;
use freakymate_core::sources::PrinterStatus;
use slint::{Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
pub struct MmuView {
    gates: Rc<VecModel<MmuGateItem>>,
    pages: Rc<VecModel<MmuGatePage>>,
    page_ranges: Rc<VecModel<MmuPageRange>>,
    sensors: Rc<VecModel<MmuSensorItem>>,
    tools: Rc<VecModel<ToolMatchItem>>,
    /// The tools of the file prepared for the print page.
    prepared_tools: Rc<VecModel<ToolMatchItem>>,
    /// The lane of each tile of `gates`, by row; what the dialog's actions address.
    rows: RefCell<Vec<LaneId>>,
    /// The lane whose tile was tapped last (or the bypass); the load path card shows it. `None` until a tap.
    tapped: RefCell<Option<LaneId>>,
}

/// What the Filament page shows besides the MMU itself.
pub struct JobFile<'a> {
    /// The name of the job's file; empty without one.
    pub name: String,
    /// How each tool of the file fits; empty without file or metadata.
    pub matches: &'a [ToolMatch],
}

impl MmuView {
    pub fn bind(&self, printer: &Printer) {
        printer.set_mmu_gates(ModelRc::from(self.gates.clone()));
        printer.set_mmu_pages(ModelRc::from(self.pages.clone()));
        printer.set_mmu_page_ranges(ModelRc::from(self.page_ranges.clone()));
        printer.set_mmu_sensors(ModelRc::from(self.sensors.clone()));
        printer.set_mmu_tools(ModelRc::from(self.tools.clone()));
        printer.set_prepared_tools(ModelRc::from(self.prepared_tools.clone()));
    }

    /// The lane of the tile in `row` of the Gates tab, as the commands address it.
    pub fn lane_id(&self, row: usize) -> Option<LaneId> {
        self.rows.borrow().get(row).cloned()
    }

    /// Makes the lane the load path card shows (a tap on its tile, or on the bypass).
    pub fn remember_lane(&self, lane: LaneId) {
        *self.tapped.borrow_mut() = Some(lane);
    }

    /// Happy Hare while it has the say, else the lanes of AFC; an enabled Happy Hare that is too old gets a notice
    /// instead.
    pub fn show(&self, printer: &Printer, status: &PrinterStatus, lanes: &[Lane], file: &JobFile) {
        let in_charge = status.happy_hare();
        printer.set_has_mmu(in_charge.is_some());
        printer.set_mmu_unsupported(status.mmu.as_ref().is_some_and(MmuStatus::is_too_old));
        let tiles = tile_lanes(status, lanes);
        printer.set_has_lanes(in_charge.is_some() || !tiles.is_empty());
        let items = match in_charge {
            Some(mmu) => gate_items(mmu, lanes, file.matches),
            None => lane_items(status, &tiles),
        };
        let ids = match in_charge {
            Some(mmu) => mmu.gates.iter().map(|gate| LaneId::Gate(gate.index)).collect(),
            None => tiles.iter().filter_map(|lane| lane.id()).collect(),
        };
        *self.rows.borrow_mut() = ids;
        replace_rows(&self.gates, items);
        self.show_lane_path(printer, status, lanes);
        let Some(mmu) = in_charge else {
            replace_rows(&self.sensors, Vec::new());
            replace_rows(&self.tools, Vec::new());
            return;
        };

        self.show_tools(printer, mmu, status, lanes, file);
        printer.set_mmu_bypass(mmu.bypass_selected());
        printer.set_mmu_endless_spool(mmu.endless_spool);
        printer.set_mmu_gate_map_local(mmu.gate_map_is_local());
        // The pause shows the selected gate (or the bypass), whatever tile was tapped.
        let selected = usize::try_from(mmu.gate).ok().and_then(|index| mmu.gates.get(index));
        let colors =
            selected.map(|gate| FilamentColors::of_gate(gate, gate_spool(lanes, gate.index))).unwrap_or_default();
        let selected_lane = match mmu.gate {
            happy_hare::BYPASS => Some((LaneId::Bypass, "Bypass".to_owned())),
            gate => usize::try_from(gate).ok().map(|gate| (LaneId::Gate(gate), format!("Gate {gate}"))),
        };
        let path = match selected_lane {
            Some((lane, label)) => load_path::load_path(status, &lane)
                .map_or_else(no_path_item, |path| load_path_item(&path, &label, &colors)),
            None => without_gate_item(&load_path::without_gate_path(mmu)),
        };
        printer.set_mmu_path(path);
        let sensors = mmu
            .listed_sensors()
            .into_iter()
            .map(|sensor| MmuSensorItem {
                label: view::mmu_sensor_label(&sensor.name).into(),
                state: view::mmu_sensor_state(sensor.triggered).into(),
            })
            .collect();
        replace_rows(&self.sensors, sensors);
        show_state(printer, mmu);
    }

    /// The load path card: the lane tapped last while it is still there, else the lane in use.
    fn show_lane_path(&self, printer: &Printer, status: &PrinterStatus, lanes: &[Lane]) {
        let rows = self.rows.borrow();
        let has_bypass = status.happy_hare().is_some();
        let remembered =
            self.tapped.borrow().clone().filter(|lane| rows.contains(lane) || (has_bypass && *lane == LaneId::Bypass));
        let shown = remembered.or_else(|| loaded_lane(status, lanes));
        let Some(lane) = shown else {
            // Happy Hare without a gate still reports the sensors at the head.
            let path = status.happy_hare().map(load_path::without_gate_path);
            printer.set_lane_path(path.map_or_else(no_path_item, |path| without_gate_item(&path)));
            printer.set_lane_buffer("".into());
            return;
        };
        let Some(path) = load_path::load_path(status, &lane) else {
            printer.set_lane_path(no_path_item());
            printer.set_lane_buffer("".into());
            return;
        };
        let (label, colors) = match &lane {
            LaneId::Gate(gate) => (
                format!("Gate {gate}"),
                lanes
                    .iter()
                    .find(|candidate| candidate.gate == Some(*gate))
                    .map(|candidate| lane_colors(status, candidate))
                    .unwrap_or_default(),
            ),
            LaneId::Bypass => ("Bypass".to_owned(), FilamentColors::default()),
            LaneId::Afc(name) => (
                name.clone(),
                lanes
                    .iter()
                    .find(|candidate| candidate.name.as_ref() == Some(name))
                    .map(|candidate| lane_colors(status, candidate))
                    .unwrap_or_default(),
            ),
        };
        printer.set_lane_path(load_path_item(&path, &label, &colors));
        printer.set_lane_buffer(path.buffer.map_or("", buffer_word).into());
    }

    /// The pages of the Gates tab: one view per head (`heads`, in the order of the heads), a single one of all tiles
    /// without heads. Call with the lanes of the tree, whose heads say which gates or lanes a head is fed from.
    pub fn show_pages(&self, status: &PrinterStatus, lanes: &[Lane], heads: &[HeadRow]) {
        let tiles = tile_lanes(status, lanes);
        if tiles.is_empty() {
            replace_rows(&self.pages, Vec::new());
            replace_rows(&self.page_ranges, Vec::new());
            return;
        }
        let loaded = tiles.iter().position(|lane| lane.active);
        let views: Vec<Option<&str>> =
            if heads.is_empty() { vec![None] } else { heads.iter().map(|head| Some(head.name.as_str())).collect() };
        let mut pages = Vec::new();
        let mut ranges = Vec::new();
        for head in views {
            let bypass = status.happy_hare().map_or(BypassAt::Nowhere, |mmu| bypass_at(mmu, &tiles, head));
            let view = gate_pages::gate_pages(&tiles, head, bypass, loaded, SLOTS_PER_PAGE);
            ranges.push(MmuPageRange {
                first: pages.len() as i32,
                count: view.pages.len() as i32,
                start: view.start as i32,
            });
            pages.extend(view.pages.iter().map(|page| page_item(page, heads)));
        }
        replace_rows(&self.pages, pages);
        replace_rows(&self.page_ranges, ranges);
    }

    /// The tools of the prepared file with what each prints with; none without a prepared file. Call after `show`, so
    /// that each tool finds its row on the Tools tab.
    pub fn show_prepared_tools(&self, matches: &[ToolMatch], status: &PrinterStatus, lanes: &[Lane]) {
        let mut items = compared_tool_items(matches, status, lanes, |_| true);
        for item in &mut items {
            item.tools_row = self.tools_row(item);
        }
        replace_rows(&self.prepared_tools, items);
    }

    /// The row of the tool on the Tools tab: Happy Hare's tools come first in order, the file's tools without a gate
    /// after them; -1 when it has none.
    fn tools_row(&self, item: &ToolMatchItem) -> i32 {
        let has_gate = item.gate >= 0;
        self.tools
            .iter()
            .position(|row| row.tool == item.tool && (row.gate >= 0) == has_gate)
            .and_then(|row| i32::try_from(row).ok())
            .unwrap_or(-1)
    }

    /// The tools of Happy Hare with what the job's file asks of them, then the file's tools that have no gate; and the
    /// line about the file above them.
    fn show_tools(&self, printer: &Printer, mmu: &MmuStatus, status: &PrinterStatus, lanes: &[Lane], file: &JobFile) {
        let matches = file.matches;
        let busy = filament_match::busy_gates(matches);
        let mapped = mmu
            .tool_to_gate
            .iter()
            .enumerate()
            .map(|(tool, gate)| (tool, Some(*gate), matches.iter().find(|found| found.request.tool == tool)));
        let without_gate = matches
            .iter()
            .filter(|found| found.verdict == Verdict::NoGate)
            .map(|found| (found.request.tool, None, Some(found)));
        let tools = mapped
            .chain(without_gate)
            .enumerate()
            .map(|(row, (tool, gate, found))| {
                let mut item = tool_match_item(tool, gate, matches);
                let gate_row = gate.and_then(|gate| mmu.gates.iter().position(|state| state.index == gate));
                item.gate_row = id_or_none(gate_row);
                if let Some(state) = gate_row.map(|row| &mmu.gates[row]) {
                    // A tool of the file shares the reserve with the other tools of its group.
                    let reserve = found.map_or_else(
                        || filament_match::endless_spool_reserve(state.index, &busy, status, lanes),
                        |found| found.reserve,
                    );
                    fill_loaded(&mut item, &filament_match::gate_supply(state, lanes), reserve);
                }
                let fitting = found.is_some().then(|| filament_match::fitting_gates(matches, tool, status, lanes));
                item.fitting_gates = reuse_flags(self.tools.row_data(row).map(|old| old.fitting_gates), fitting);
                item
            })
            .collect();
        replace_rows(&self.tools, tools);
        printer.set_mmu_match_file(if matches.is_empty() { String::new() } else { file.name.clone() }.into());
        printer.set_mmu_match_summary(view::match_summary(matches).into());
        printer.set_mmu_match_verdict(view::combined_verdict(matches.iter().map(|found| &found.verdict)).into());
    }
}

/// The lanes that are tiles of the Gates tab: the gates of Happy Hare while it has the say, else the lanes of AFC.
fn tile_lanes(status: &PrinterStatus, lanes: &[Lane]) -> Vec<Lane> {
    if status.happy_hare().is_some() {
        // The tree has a lane for every gate, in the order of the gates.
        lanes.iter().filter(|lane| lane.gate.is_some()).cloned().collect()
    } else if status.afc.is_some() {
        lanes.iter().filter(|lane| lane.gate.is_none() && lane.name.is_some()).cloned().collect()
    } else {
        Vec::new()
    }
}

/// The bypass is the machine's, not a head's: it sits on the unit that has it, whichever head that is on. Where the
/// units do not say, it goes to the last unit.
fn bypass_at(mmu: &MmuStatus, tiles: &[Lane], head: Option<&str>) -> BypassAt {
    let last_head = mmu
        .gates
        .last()
        .and_then(|last| tiles.iter().find(|lane| lane.gate == Some(last.index)))
        .and_then(|lane| lane.head.as_deref());
    match mmu.bypass_unit() {
        Some(unit) => BypassAt::Unit(unit),
        None if head.is_none() || head == last_head => BypassAt::LastUnit,
        None => BypassAt::Nowhere,
    }
}

/// The lane in use: the selected gate or bypass of Happy Hare, the lane loaded in AFC.
fn loaded_lane(status: &PrinterStatus, lanes: &[Lane]) -> Option<LaneId> {
    if let Some(mmu) = status.happy_hare() {
        return match mmu.gate {
            _ if mmu.bypass_selected() => Some(LaneId::Bypass),
            gate => usize::try_from(gate).ok().map(LaneId::Gate),
        };
    }
    tile_lanes(status, lanes).iter().find(|lane| lane.active).and_then(Lane::id)
}

/// The card of Happy Hare with no gate selected; the filament's colour is not known.
fn without_gate_item(path: &LoadPath) -> FilamentPathItem {
    load_path_item(path, "No gate", &FilamentColors::default())
}

/// The side the buffer leans to as the word the UI maps (`MmuTexts.buffer`).
fn buffer_word(side: BufferSide) -> &'static str {
    match side {
        BufferSide::Tension => "tension",
        BufferSide::Neutral => "neutral",
        BufferSide::Compression => "compressed",
        BufferSide::Unknown => "unknown",
    }
}

/// Tiles on a page of the Gates tab, the bypass included.
const SLOTS_PER_PAGE: usize = 5;

/// The caption names the head by its label (`T1`), not by its section.
fn page_item(page: &GatePage, heads: &[HeadRow]) -> MmuGatePage {
    let head = page.head.as_ref().map(|name| heads.iter().find(|row| &row.name == name).map_or(name, |row| &row.label));
    let (first, last) = page.gates.map_or((-1, -1), |(first, last)| (first as i32, last as i32));
    MmuGatePage {
        first_row: page.first_row as i32,
        count: page.count as i32,
        bypass: page.bypass,
        unit: page.unit.clone().unwrap_or_default().into(),
        head: head.cloned().unwrap_or_default().into(),
        first_gate: first,
        last_gate: last,
    }
}

fn gate_items(mmu: &MmuStatus, lanes: &[Lane], matches: &[ToolMatch]) -> Vec<MmuGateItem> {
    mmu.gates
        .iter()
        .map(|gate| {
            let spool = gate_spool(lanes, gate.index);
            let selected = usize::try_from(mmu.gate) == Ok(gate.index);
            MmuGateItem {
                title: format!("Gate {}", gate.index).into(),
                loaded: selected && mmu.filament != LoadedFilament::Unloaded,
                spool_text: spool.map(view::spool_remaining).unwrap_or_default().into(),
                spool_low: spool.is_some_and(Spool::is_low),
                spool_id: id_or_none(gate.spool_id),
                index: gate.index as i32,
                availability: view::mmu_availability(gate.availability).into(),
                name: gate.name.as_str().into(),
                material: gate.material.as_str().into(),
                colors: bar_colors(&FilamentColors::of_gate(gate, spool)),
                has_color: gate.color.is_some(),
                tools: view::gate_tools_text(&gate.tools).into(),
                file_verdict: view::combined_verdict(
                    matches.iter().filter(|found| found.gate == Some(gate.index)).map(|found| &found.verdict),
                )
                .into(),
                group: id_or_none(gate.endless_spool_group),
                selected,
            }
        })
        .collect()
}

/// The lanes of AFC as tiles: what AFC reports of the lane, the spool Spoolman knows in it.
fn lane_items(status: &PrinterStatus, tiles: &[Lane]) -> Vec<MmuGateItem> {
    tiles
        .iter()
        .map(|lane| {
            let spool = lane.spool.as_ref();
            let name = lane.name.clone().unwrap_or_default();
            let reported = status.afc.as_ref().and_then(|afc| afc.lane(&name));
            MmuGateItem {
                title: name.as_str().into(),
                index: -1,
                spool_text: spool.map(view::spool_remaining).unwrap_or_default().into(),
                spool_low: spool.is_some_and(Spool::is_low),
                spool_id: id_or_none(lane.spool_id),
                availability: if lane.empty { "empty" } else { "available" }.into(),
                name: spool.map(Spool::title).unwrap_or_default().into(),
                material: reported.map(|lane| lane.material.as_str()).unwrap_or_default().into(),
                colors: bar_colors(&lane_colors(status, lane)),
                has_color: reported.is_some_and(|lane| lane.color.is_some()),
                tools: lane.tool.map(|tool| view::gate_tools_text(&[tool])).unwrap_or_default().into(),
                file_verdict: "".into(),
                group: -1,
                selected: lane.active,
                loaded: lane.active,
            }
        })
        .collect()
}

/// The state of the MMU: what it holds, does and reports.
fn show_state(printer: &Printer, mmu: &MmuStatus) {
    printer.set_mmu_filament(view::mmu_filament(mmu.filament).into());
    printer.set_mmu_action(mmu.action.as_str().into());
    printer.set_mmu_gate(mmu.gate.max(-1));
    printer.set_mmu_tool(mmu.tool.max(-1));
    printer.set_mmu_paused(mmu.paused);
    printer.set_mmu_locked(mmu.locked);
    printer.set_mmu_homed(mmu.homed);
    printer.set_mmu_servo(view::mmu_servo(mmu.servo).into());
    printer.set_mmu_has_buffer(mmu.sync_feedback.is_some());
    let feedback = mmu.sync_feedback.unwrap_or_default();
    printer.set_mmu_buffer_state(view::mmu_buffer_state(feedback.state).into());
    printer.set_mmu_buffer_bias(feedback.bias as f32);
    let flowguard = mmu.flowguard.clone().unwrap_or_default();
    printer.set_mmu_flowguard_state(view::mmu_flowguard_state(flowguard.state()).into());
    let detail: Vec<&str> =
        [flowguard.trigger.as_str(), flowguard.reason.as_str()].into_iter().filter(|part| !part.is_empty()).collect();
    printer.set_mmu_flowguard_detail(detail.join(": ").into());
    printer.set_mmu_flow_rate(mmu.flow_rate.map_or(-1.0, |rate| rate.value as f32));
}

/// The spool Spoolman knows in a gate.
fn gate_spool(lanes: &[Lane], gate: usize) -> Option<&Spool> {
    lanes.iter().find(|lane| lane.gate == Some(gate)).and_then(|lane| lane.spool.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(tool: i32, gate: i32) -> ToolMatchItem {
        ToolMatchItem { tool, gate, ..ToolMatchItem::default() }
    }

    fn tile(name: &str, active: bool) -> Lane {
        Lane {
            gate: None,
            name: Some(name.into()),
            unit: Some(0),
            unit_name: None,
            head: None,
            extruder: "extruder".into(),
            tool: Some(1),
            spool_id: None,
            spool_by_tool: false,
            spool: None,
            empty: false,
            active,
        }
    }

    fn afc_status() -> PrinterStatus {
        let lane = |name: &str, in_tool| freakymate_core::sources::afc::AfcLane {
            name: name.into(),
            in_tool,
            filled: true,
            material: "PLA".into(),
            color: Some([255, 0, 0, 255]),
            ..Default::default()
        };
        PrinterStatus {
            afc: Some(freakymate_core::sources::afc::AfcStatus {
                lanes: vec![lane("lane1", false), lane("lane2", true)],
                ..Default::default()
            }),
            ..PrinterStatus::default()
        }
    }

    #[test]
    fn lanes_of_afc_are_tiles_named_by_their_lane() {
        let status = afc_status();
        let lanes = [tile("lane1", false), tile("lane2", true)];
        let tiles = tile_lanes(&status, &lanes);
        let items = lane_items(&status, &tiles);
        assert_eq!(items.iter().map(|item| item.title.to_string()).collect::<Vec<_>>(), ["lane1", "lane2"]);
        assert_eq!((items[0].loaded, items[1].loaded), (false, true));
        assert_eq!(
            (items[0].index, items[0].tools.to_string(), items[0].material.to_string()),
            (-1, "T1".into(), "PLA".into())
        );
        assert_eq!(loaded_lane(&status, &lanes), Some(LaneId::Afc("lane2".into())));
    }

    #[test]
    fn the_card_shows_the_lane_tapped_last_while_it_exists() {
        let view = MmuView::default();
        *view.rows.borrow_mut() = vec![LaneId::Afc("lane1".into()), LaneId::Afc("lane2".into())];
        view.remember_lane(LaneId::Afc("lane1".into()));
        assert_eq!(view.lane_id(1), Some(LaneId::Afc("lane2".into())));
        assert_eq!(view.lane_id(2), None);
        assert!(view.rows.borrow().contains(view.tapped.borrow().as_ref().unwrap()));
    }

    #[test]
    fn a_buffer_side_maps_to_the_words_of_the_ui() {
        assert_eq!(buffer_word(BufferSide::Compression), "compressed");
        assert_eq!(buffer_word(BufferSide::Unknown), "unknown");
    }

    #[test]
    fn a_tool_finds_its_row_on_the_tools_tab() {
        let view = MmuView::default();
        // Happy Hare's tools in order, then a tool of the file without a gate.
        replace_rows(&view.tools, vec![item(0, 2), item(1, 0), item(5, -1)]);
        assert_eq!(view.tools_row(&item(1, 0)), 1);
        assert_eq!(view.tools_row(&item(5, -1)), 2);
        assert_eq!(view.tools_row(&item(7, -1)), -1);
    }
}
