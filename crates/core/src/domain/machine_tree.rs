//! The machine as a tree, from the bed inwards: carriage → head → nozzle → lane (design: `docs/multi-tool.md`).
//!
//! Every level is a pass-through until a source says there are several. A tool changer brings a head per tool, each
//! with the nozzle of its extruder and the one direct feed; Happy Hare brings a lane per gate (labelled with its unit)
//! on the one nozzle. The Klipper extruder is the joint that attaches the lanes to a nozzle.

use crate::sources::happy_hare::MmuStatus;
use crate::sources::spoolman::{Spool, SpoolmanState};
use crate::sources::tool_macros::ToolSpool;
use crate::sources::toolchanger::ToolHead;
use crate::sources::PrinterStatus;

/// The extruder of a printer with one nozzle, as Klipper names it.
const MAIN_EXTRUDER: &str = "extruder";

/// One place for a spool: a gate of Happy Hare, the head of a tool changer, or the one spool of a printer without
/// either (the active spool of Moonraker).
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    /// Index of the gate across all units; `None` for a direct feed.
    pub gate: Option<usize>,
    /// The unit that carries the gate (for grouping and its own state); `None` without MMU.
    pub unit: Option<usize>,
    /// The swappable head the lane feeds (its name, such as `T1`); `None` on a fixed head.
    pub head: Option<String>,
    /// The Klipper extruder whose nozzle the lane feeds.
    pub extruder: String,
    /// First tool that is mapped to the lane.
    pub tool: Option<usize>,
    pub spool_id: Option<u32>,
    /// The tool macro of [`Self::tool`] declares `spool_id`, so that is where the spool of this direct feed lives.
    pub tool_macro_spool: bool,
    /// The spool, once Spoolman has answered; empty in the tree itself.
    pub spool: Option<Spool>,
    /// The filament of this lane is the one in use: the selected gate, the mounted head, or the only spool.
    pub active: bool,
}

/// Where the spool of a lane is assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpoolHolder {
    /// The gate in Happy Hare's gate map.
    Gate(usize),
    /// The variable `spool_id` of the tool macro `T<n>`.
    ToolMacro(usize),
    /// Moonraker's active spool.
    ActiveSpool,
}

impl Lane {
    /// Where a spool for this lane is assigned; `None` for a head that is not in use and keeps no spool of its own.
    pub fn spool_holder(&self) -> Option<SpoolHolder> {
        match (self.gate, self.tool) {
            (Some(gate), _) => Some(SpoolHolder::Gate(gate)),
            (None, Some(tool)) if self.tool_macro_spool => Some(SpoolHolder::ToolMacro(tool)),
            _ => self.active.then_some(SpoolHolder::ActiveSpool),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Nozzle {
    pub extruder: String,
    pub lanes: Vec<Lane>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    /// The name of a swappable head; `None` for the fixed head.
    pub name: Option<String>,
    /// The head is mounted (always for the fixed head).
    pub active: bool,
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
    /// The tree of what `status` reports. Happy Hare's gates feed the one nozzle while it has the say (with a tool
    /// changer as well, until combinations are built); otherwise a tool changer brings its heads, and without either
    /// the fixed head has the direct feed.
    pub fn build(status: &PrinterStatus) -> Self {
        let tool_heads =
            status.toolchanger.as_ref().map(|toolchanger| toolchanger.heads.as_slice()).unwrap_or_default();
        let heads = match status.happy_hare().filter(|mmu| !mmu.gates.is_empty()) {
            Some(mmu) => vec![fixed_head(gate_lanes(mmu))],
            None if !tool_heads.is_empty() => {
                tool_heads.iter().map(|head| swappable_head(head, &status.tool_spools)).collect()
            }
            None => vec![fixed_head(vec![direct_lane(MAIN_EXTRUDER, Some(0), &status.tool_spools)])],
        };
        Self { carriages: vec![Carriage { heads }] }
    }

    /// The spools the lanes keep themselves (gates, tool macros), sorted and without duplicates: those Spoolman has to
    /// be asked for besides the active spool.
    pub fn own_spool_ids(&self) -> Vec<u32> {
        let nozzles = self.carriages.iter().flat_map(|carriage| &carriage.heads).flat_map(|head| &head.nozzles);
        let mut ids: Vec<u32> = nozzles.flat_map(|nozzle| &nozzle.lanes).filter_map(|lane| lane.spool_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// All lanes in tree order, with the spools Spoolman knows. A direct feed in use without a spool of its own (no
    /// `spool_id` in its tool macro) holds the active spool of Moonraker.
    pub fn lanes(&self, spoolman: &SpoolmanState) -> Vec<Lane> {
        let lanes = self.carriages.iter().flat_map(|carriage| &carriage.heads).flat_map(|head| &head.nozzles);
        lanes
            .flat_map(|nozzle| &nozzle.lanes)
            .map(|lane| {
                let spool_id = match lane.spool_holder() {
                    Some(SpoolHolder::ActiveSpool) => spoolman.active_spool_id,
                    _ => lane.spool_id,
                };
                Lane { spool_id, spool: spool_id.and_then(|id| spoolman.spools.get(&id)).cloned(), ..lane.clone() }
            })
            .collect()
    }
}

fn fixed_head(lanes: Vec<Lane>) -> Head {
    Head { name: None, active: true, nozzles: vec![Nozzle { extruder: MAIN_EXTRUDER.to_owned(), lanes }] }
}

/// A head of the tool changer with the nozzle of its extruder and the direct feed; a head without extruder (a probe,
/// say) has no nozzle.
fn swappable_head(head: &ToolHead, tool_spools: &[ToolSpool]) -> Head {
    let nozzles = head.extruder.iter().map(|extruder| {
        let lane = Lane {
            head: Some(head.name.clone()),
            active: head.active,
            ..direct_lane(extruder, head.number, tool_spools)
        };
        Nozzle { extruder: extruder.clone(), lanes: vec![lane] }
    });
    Head { name: Some(head.name.clone()), active: head.active, nozzles: nozzles.collect() }
}

/// The direct feed of `extruder` for `tool`, with the spool its tool macro declares.
fn direct_lane(extruder: &str, tool: Option<usize>, tool_spools: &[ToolSpool]) -> Lane {
    let declared = tool.and_then(|tool| tool_spools.iter().find(|spool| spool.tool == tool));
    Lane {
        gate: None,
        unit: None,
        head: None,
        extruder: extruder.to_owned(),
        tool,
        spool_id: declared.and_then(|spool| spool.spool_id),
        tool_macro_spool: declared.is_some(),
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
            head: None,
            extruder: MAIN_EXTRUDER.to_owned(),
            tool: gate.tools.first().copied(),
            spool_id: gate.spool_id,
            tool_macro_spool: false,
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
    use crate::sources::toolchanger::ToolchangerStatus;

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

    /// Three heads on `extruder`, `extruder1`, `extruder2`, the one with `active` mounted; `T0` declares spool 1, `T1`
    /// declares none, `T2` has no variable.
    fn toolchanger(active: usize) -> PrinterStatus {
        let head = |number: usize, extruder: &str| ToolHead {
            name: format!("T{number}"),
            number: Some(number),
            extruder: Some(extruder.into()),
            active: number == active,
        };
        PrinterStatus {
            toolchanger: Some(ToolchangerStatus {
                status: "ready".into(),
                heads: vec![head(0, "extruder"), head(1, "extruder1"), head(2, "extruder2")],
            }),
            tool_spools: vec![
                ToolSpool { tool: 0, macro_name: "T0".into(), spool_id: Some(1) },
                ToolSpool { tool: 1, macro_name: "T1".into(), spool_id: None },
            ],
            ..PrinterStatus::default()
        }
    }

    #[test]
    fn a_tool_changer_has_a_head_per_tool_with_the_nozzle_of_its_extruder() {
        let tree = MachineTree::build(&toolchanger(1));

        let heads = &tree.carriages[0].heads;
        assert_eq!(
            heads.iter().map(|head| head.name.as_deref()).collect::<Vec<_>>(),
            [Some("T0"), Some("T1"), Some("T2")]
        );
        assert_eq!(heads.iter().map(|head| head.active).collect::<Vec<_>>(), [false, true, false]);
        assert_eq!(heads[2].nozzles[0].extruder, "extruder2");
        let lanes = tree.lanes(&SpoolmanState::default());
        assert_eq!(lanes.iter().map(|lane| lane.tool).collect::<Vec<_>>(), [Some(0), Some(1), Some(2)]);
        assert_eq!(lanes.iter().map(|lane| lane.active).collect::<Vec<_>>(), [false, true, false]);
        assert_eq!(lanes[1].head.as_deref(), Some("T1"));
    }

    #[test]
    fn a_head_keeps_the_spool_of_its_tool_macro_and_only_the_mounted_one_falls_back_to_the_active_spool() {
        let state = SpoolmanState { active_spool_id: Some(9), ..SpoolmanState::default() };

        let spools = |active| {
            MachineTree::build(&toolchanger(active)).lanes(&state).iter().map(|lane| lane.spool_id).collect::<Vec<_>>()
        };

        assert_eq!(spools(2), [Some(1), None, Some(9)]);
        assert_eq!(spools(1), [Some(1), None, None], "T1 declares no spool, T2 is not in use");
        assert_eq!(spools(0), [Some(1), None, None]);
    }

    #[test]
    fn the_spool_of_a_lane_is_assigned_where_it_lives() {
        let lanes = MachineTree::build(&toolchanger(2)).lanes(&SpoolmanState::default());
        let gate_lane = MachineTree::build(&mmu(&[(0, 2)], 0)).lanes(&SpoolmanState::default());

        assert_eq!(
            lanes.iter().map(Lane::spool_holder).collect::<Vec<_>>(),
            [Some(SpoolHolder::ToolMacro(0)), Some(SpoolHolder::ToolMacro(1)), Some(SpoolHolder::ActiveSpool)]
        );
        assert_eq!(MachineTree::build(&toolchanger(0)).lanes(&SpoolmanState::default())[2].spool_holder(), None);
        assert_eq!(gate_lane[1].spool_holder(), Some(SpoolHolder::Gate(1)));
        assert_eq!(
            MachineTree::default().lanes(&SpoolmanState::default())[0].spool_holder(),
            Some(SpoolHolder::ActiveSpool)
        );
    }

    #[test]
    fn spoolman_is_asked_for_the_spools_the_lanes_keep_themselves() {
        assert_eq!(MachineTree::build(&toolchanger(2)).own_spool_ids(), [1]);
        assert_eq!(MachineTree::build(&mmu(&[(0, 3)], 0)).own_spool_ids(), [10, 11, 12]);
        assert_eq!(MachineTree::default().own_spool_ids(), Vec::<u32>::new());
    }

    #[test]
    fn a_head_without_extruder_has_no_nozzle() {
        let mut status = toolchanger(0);
        status.toolchanger.as_mut().unwrap().heads[2].extruder = None;

        let tree = MachineTree::build(&status);

        assert!(tree.carriages[0].heads[2].nozzles.is_empty());
        assert_eq!(tree.lanes(&SpoolmanState::default()).len(), 2);
    }

    #[test]
    fn happy_hares_gates_stay_on_the_one_nozzle_beside_a_tool_changer() {
        let mut status = mmu(&[(0, 2)], 0);
        status.toolchanger = toolchanger(0).toolchanger;

        let tree = MachineTree::build(&status);

        assert_eq!(tree.carriages[0].heads.len(), 1);
        assert_eq!(tree.lanes(&SpoolmanState::default()).len(), 2);
    }
}
