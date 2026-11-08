//! Happy Hare on the Filament page: the gates, the tools with what the job's file asks of them, the sensors, the way of
//! the filament and the state of the MMU.

use crate::items::{bar_colors, filament_path_item, id_or_none, replace_rows, reuse_flags, tool_match_item};
use crate::{view, MmuGateItem, MmuSensorItem, Printer, ToolMatchItem};
use freakymate_core::domain::filament_match::{self, ToolMatch, Verdict};
use freakymate_core::domain::spools::{FilamentColors, Lane};
use freakymate_core::sources::happy_hare::MmuStatus;
use freakymate_core::sources::spoolman::Spool;
use freakymate_core::sources::PrinterStatus;
use slint::{Model, ModelRc, VecModel};
use std::rc::Rc;

#[derive(Default)]
pub struct MmuView {
    gates: Rc<VecModel<MmuGateItem>>,
    sensors: Rc<VecModel<MmuSensorItem>>,
    tools: Rc<VecModel<ToolMatchItem>>,
    /// The tools of the file prepared for the print page.
    prepared_tools: Rc<VecModel<ToolMatchItem>>,
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
        printer.set_mmu_sensors(ModelRc::from(self.sensors.clone()));
        printer.set_mmu_tools(ModelRc::from(self.tools.clone()));
        printer.set_prepared_tools(ModelRc::from(self.prepared_tools.clone()));
    }

    /// Happy Hare while it has the say; an enabled one that is too old gets a notice instead.
    pub fn show(&self, printer: &Printer, status: &PrinterStatus, lanes: &[Lane], file: &JobFile) {
        let in_charge = status.happy_hare();
        printer.set_has_mmu(in_charge.is_some());
        printer.set_mmu_unsupported(status.mmu.as_ref().is_some_and(MmuStatus::is_too_old));
        let Some(mmu) = in_charge else {
            replace_rows(&self.gates, Vec::new());
            replace_rows(&self.sensors, Vec::new());
            replace_rows(&self.tools, Vec::new());
            return;
        };

        replace_rows(&self.gates, gate_items(mmu, lanes, file.matches));
        self.show_tools(printer, mmu, status, lanes, file);
        printer.set_mmu_bypass(mmu.bypass_selected());
        printer.set_mmu_endless_spool(mmu.endless_spool);
        printer.set_mmu_gate_map_local(mmu.gate_map_is_local());
        let selected = usize::try_from(mmu.gate).ok().and_then(|index| mmu.gates.get(index));
        let colors =
            selected.map(|gate| FilamentColors::of_gate(gate, gate_spool(lanes, gate.index))).unwrap_or_default();
        printer.set_mmu_path(filament_path_item(&mmu.filament_path(), mmu.gate, &colors));
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

    /// The tools of the prepared file; none without a prepared file.
    pub fn show_prepared_tools(&self, matches: &[ToolMatch]) {
        let tools = matches.iter().map(|found| tool_match_item(found.request.tool, found.gate, Some(found))).collect();
        replace_rows(&self.prepared_tools, tools);
    }

    /// The tools of Happy Hare with what the job's file asks of them, then the file's tools that have no gate; and the
    /// line about the file above them.
    fn show_tools(&self, printer: &Printer, mmu: &MmuStatus, status: &PrinterStatus, lanes: &[Lane], file: &JobFile) {
        let matches = file.matches;
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
                let mut item = tool_match_item(tool, gate, found);
                let fitting = found.map(|found| filament_match::fitting_gates(&found.request, status, lanes));
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

fn gate_items(mmu: &MmuStatus, lanes: &[Lane], matches: &[ToolMatch]) -> Vec<MmuGateItem> {
    mmu.gates
        .iter()
        .map(|gate| {
            let spool = gate_spool(lanes, gate.index);
            MmuGateItem {
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
                selected: usize::try_from(mmu.gate) == Ok(gate.index),
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
