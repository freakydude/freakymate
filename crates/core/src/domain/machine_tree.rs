//! The machine as a tree, from the bed inwards: carriage → head → nozzle → lane (design: `docs/multi-tool.md`).
//!
//! Every level is a pass-through until a source says there are several. Today that is only the lane level: Happy
//! Hare brings a lane per gate (labelled with its unit), a printer without MMU has the one direct feed. The
//! Klipper extruder is the joint that attaches the lanes to a nozzle.

use crate::sources::happy_hare::MmuStatus;
use crate::sources::spoolman::{Spool, SpoolmanState};
use crate::sources::PrinterStatus;

/// The extruder of a printer with one nozzle, as Klipper names it.
const MAIN_EXTRUDER: &str = "extruder";

/// One place for a spool: a gate of Happy Hare, or the one spool of a printer without MMU (the active spool of
/// Moonraker).
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    /// Index of the gate across all units; `None` for the direct feed without MMU.
    pub gate: Option<usize>,
    /// The unit that carries the gate (for grouping and its own state); `None` without MMU.
    pub unit: Option<usize>,
    /// The Klipper extruder whose nozzle the lane feeds.
    pub extruder: String,
    /// First tool that is mapped to the gate.
    pub tool: Option<usize>,
    pub spool_id: Option<u32>,
    /// The spool, once Spoolman has answered; empty in the tree itself.
    pub spool: Option<Spool>,
    /// The filament of this lane is the one in use: the selected gate, or the only spool.
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Nozzle {
    pub extruder: String,
    pub lanes: Vec<Lane>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    pub nozzles: Vec<Nozzle>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Carriage {
    pub heads: Vec<Head>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MachineTree {
    pub carriages: Vec<Carriage>,
}

impl Default for MachineTree {
    /// A printer about which nothing is known yet: one carriage, head and nozzle with the direct feed.
    fn default() -> Self {
        Self::build(&PrinterStatus::default())
    }
}

impl MachineTree {
    /// The tree of what `status` reports: Happy Hare's gates while it has the say, otherwise the direct feed.
    pub fn build(status: &PrinterStatus) -> Self {
        let lanes = match status.happy_hare().filter(|mmu| !mmu.gates.is_empty()) {
            Some(mmu) => gate_lanes(mmu),
            None => vec![direct_lane()],
        };
        let nozzle = Nozzle { extruder: MAIN_EXTRUDER.to_owned(), lanes };
        Self { carriages: vec![Carriage { heads: vec![Head { nozzles: vec![nozzle] }] }] }
    }

    /// All lanes in tree order, with the spools Spoolman knows. A direct feed without spool of its own holds the
    /// active spool of Moonraker.
    pub fn lanes(&self, spoolman: &SpoolmanState) -> Vec<Lane> {
        let lanes = self.carriages.iter().flat_map(|carriage| &carriage.heads).flat_map(|head| &head.nozzles);
        lanes
            .flat_map(|nozzle| &nozzle.lanes)
            .map(|lane| {
                let spool_id =
                    lane.spool_id.or_else(|| lane.gate.is_none().then_some(spoolman.active_spool_id).flatten());
                Lane { spool_id, spool: spool_id.and_then(|id| spoolman.spools.get(&id)).cloned(), ..lane.clone() }
            })
            .collect()
    }
}

fn direct_lane() -> Lane {
    Lane {
        gate: None,
        unit: None,
        extruder: MAIN_EXTRUDER.to_owned(),
        tool: Some(0),
        spool_id: None,
        spool: None,
        active: true,
    }
}

/// A lane per gate. Happy Hare numbers gates across all units from 0 on, so the position in the list is the gate
/// index; code that looks a gate up still compares `Lane::gate`.
fn gate_lanes(mmu: &MmuStatus) -> Vec<Lane> {
    mmu.gates
        .iter()
        .map(|gate| Lane {
            gate: Some(gate.index),
            unit: unit_of(mmu, gate.index),
            extruder: MAIN_EXTRUDER.to_owned(),
            tool: gate.tools.first().copied(),
            spool_id: gate.spool_id,
            spool: None,
            active: usize::try_from(mmu.gate) == Ok(gate.index),
        })
        .collect()
}

/// The unit of a gate; without reported units all gates form unit 0.
fn unit_of(mmu: &MmuStatus, gate: usize) -> Option<usize> {
    if mmu.units.is_empty() {
        return Some(0);
    }
    mmu.units.iter().position(|unit| unit.contains(gate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::happy_hare::{MmuGate, MmuUnit};

    fn mmu(units: &[(usize, usize)], selected: i32) -> PrinterStatus {
        let total: usize = units.iter().map(|(_, count)| count).sum();
        let mmu = MmuStatus {
            version: "4.1.0".into(),
            enabled: true,
            num_gates: total,
            gate: selected,
            units: units.iter().map(|&(first_gate, num_gates)| MmuUnit { first_gate, num_gates }).collect(),
            gates: (0..total)
                .map(|index| MmuGate {
                    index,
                    spool_id: Some(10 + index as u32),
                    tools: vec![index],
                    ..MmuGate::default()
                })
                .collect(),
            ..MmuStatus::default()
        };
        PrinterStatus { mmu: Some(mmu), ..PrinterStatus::default() }
    }

    #[test]
    fn a_printer_without_mmu_is_pass_through_to_one_direct_lane() {
        let tree = MachineTree::build(&PrinterStatus::default());

        assert_eq!(tree.carriages.len(), 1);
        assert_eq!(tree.carriages[0].heads.len(), 1);
        let nozzle = &tree.carriages[0].heads[0].nozzles[0];
        assert_eq!(nozzle.extruder, "extruder");
        assert_eq!(nozzle.lanes.len(), 1);
        assert_eq!((nozzle.lanes[0].gate, nozzle.lanes[0].unit, nozzle.lanes[0].active), (None, None, true));
        assert_eq!(tree, MachineTree::default());
    }

    #[test]
    fn the_direct_lane_takes_the_active_spool_of_moonraker() {
        let state = SpoolmanState { active_spool_id: Some(7), ..SpoolmanState::default() };

        let lanes = MachineTree::default().lanes(&state);

        assert_eq!(lanes[0].spool_id, Some(7));
    }

    #[test]
    fn gates_of_all_units_are_lanes_of_the_one_nozzle_labelled_with_their_unit() {
        let tree = MachineTree::build(&mmu(&[(0, 2), (2, 3)], 3));

        let lanes = tree.lanes(&SpoolmanState::default());

        assert_eq!(tree.carriages[0].heads[0].nozzles.len(), 1);
        assert_eq!(lanes.iter().map(|lane| lane.gate).collect::<Vec<_>>(), (0..5).map(Some).collect::<Vec<_>>());
        assert_eq!(
            lanes.iter().map(|lane| lane.unit).collect::<Vec<_>>(),
            [Some(0), Some(0), Some(1), Some(1), Some(1)]
        );
        assert_eq!(lanes.iter().map(|lane| lane.active).collect::<Vec<_>>(), [false, false, false, true, false]);
    }

    #[test]
    fn gates_without_reported_units_form_unit_zero() {
        let mut status = mmu(&[(0, 3)], 0);
        status.mmu.as_mut().unwrap().units.clear();

        let lanes = MachineTree::build(&status).lanes(&SpoolmanState::default());

        assert_eq!(lanes.iter().map(|lane| lane.unit).collect::<Vec<_>>(), [Some(0); 3]);
    }
}
