//! The machine as a tree, from the bed inwards: carriage → head → nozzle → lane (design: `docs/multi-tool.md`).
//!
//! Every level is a pass-through until a source says there are several. A tool changer brings a head per tool, each
//! with the nozzle of its extruder; Happy Hare brings a lane per gate (labelled with its unit) on the nozzle of the
//! extruder its unit feeds; AFC a lane per lane of its units, and on its own tool changer the heads. The Klipper extruder is the joint that attaches the lanes to a nozzle; a nozzle without
//! lanes has the one direct feed.

use crate::sources::afc::AfcStatus;
use crate::sources::happy_hare::MmuStatus;
use crate::sources::spoolman::{Spool, SpoolmanState};
use crate::sources::tool_macros::ToolSpool;
use crate::sources::toolchanger::ToolHead;
use crate::sources::PrinterStatus;

/// The extruder of a printer with one nozzle, as Klipper names it.
const MAIN_EXTRUDER: &str = "extruder";
/// Klipper's part cooling fan of `M106`, the fan of a printer without a head level.
const PART_FAN: &str = "fan";

/// One place for a spool: a gate of Happy Hare, a lane of AFC, the head of a tool changer, or the one spool of a
/// printer without either (the active spool of Moonraker).
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    /// Index of the gate across all units; `None` for a direct feed.
    pub gate: Option<usize>,
    /// The name of an AFC lane (`lane1`), which its commands take.
    pub name: Option<String>,
    /// The unit that carries the gate (for grouping and its own state); `None` without MMU.
    pub unit: Option<usize>,
    /// The name of that unit to show, when the machine has several; `None` with one unit or none.
    pub unit_name: Option<String>,
    /// The swappable head the lane feeds (its name, such as `T1`); `None` on a fixed head.
    pub head: Option<String>,
    /// The Klipper extruder whose nozzle the lane feeds.
    pub extruder: String,
    /// First tool that is mapped to the lane.
    pub tool: Option<usize>,
    pub spool_id: Option<u32>,
    /// The spool is assigned through [`Self::tool`]: its tool macro declares `spool_id`, or AFC keeps the spool of the
    /// lane mapped to it.
    pub spool_by_tool: bool,
    /// The spool, once Spoolman has answered; empty in the tree itself.
    pub spool: Option<Spool>,
    /// The source reports no filament in the lane (AFC); Happy Hare's gates say so in their own state.
    pub empty: bool,
    /// The filament of this lane is the one in use: the selected gate, the mounted head, or the only spool.
    pub active: bool,
}

/// Where the spool of a lane is assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpoolHolder {
    /// The gate in Happy Hare's gate map.
    Gate(usize),
    /// Through the tool `T<n>`: the variable `spool_id` of its macro, or the AFC lane mapped to it.
    Tool(usize),
    /// Moonraker's active spool.
    ActiveSpool,
}

/// What the commands of a lane address: a gate of Happy Hare, its bypass, or a lane of AFC by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneId {
    Gate(usize),
    Bypass,
    Afc(String),
}

impl Lane {
    /// The lane as its commands address it; `None` for the direct feed, which has no commands of its own.
    pub fn id(&self) -> Option<LaneId> {
        match (self.gate, &self.name) {
            (Some(gate), _) => Some(LaneId::Gate(gate)),
            (None, Some(name)) => Some(LaneId::Afc(name.clone())),
            (None, None) => None,
        }
    }

    /// Where a spool for this lane is assigned; `None` for a head that is not in use and keeps no spool of its own,
    /// and for an AFC lane without tool (AFC sets the active spool itself).
    pub fn spool_holder(&self) -> Option<SpoolHolder> {
        match (self.gate, self.tool) {
            (Some(gate), _) => Some(SpoolHolder::Gate(gate)),
            (None, Some(tool)) if self.spool_by_tool => Some(SpoolHolder::Tool(tool)),
            _ if self.name.is_some() => None,
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
    /// The Klipper object of the part cooling fan the head states (`tool.fan`); `None` for a head that states none and
    /// for the fixed head.
    pub fan: Option<String>,
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
    /// The tree of what `status` reports. The lanes of a filament changer (Happy Hare's gates while it has the say,
    /// AFC's lanes) attach to the nozzle of the extruder they feed; a nozzle that no lane feeds has the direct feed. A tool changer
    /// (`toolchanger`, else AFC's own) brings the heads with their nozzles; without one the fixed head carries every nozzle the lanes feed (`extruder`
    /// alone without lanes). Lanes on an extruder that no head of the tool changer has stay on a fixed head of their
    /// own, so none goes missing.
    pub fn build(status: &PrinterStatus) -> Self {
        let mut fed = status.happy_hare().map(gate_lanes).unwrap_or_default();
        fed.extend(status.afc.iter().flat_map(afc_lanes));
        let tool_heads = match (&status.toolchanger, &status.afc) {
            (Some(toolchanger), _) if !toolchanger.heads.is_empty() => toolchanger.heads.clone(),
            (_, Some(afc)) => afc_heads(afc),
            _ => Vec::new(),
        };
        let mut heads: Vec<Head> =
            tool_heads.iter().map(|head| swappable_head(head, &mut fed, &status.tool_spools)).collect();
        if heads.is_empty() || !fed.is_empty() {
            heads.push(fixed_head(fed, &status.tool_spools));
        }
        Self { carriages: vec![Carriage { heads }] }
    }

    /// The tree has swappable heads (a tool changer); only then the screens show a head level.
    pub fn has_swappable_heads(&self) -> bool {
        self.heads().any(|head| head.name.is_some())
    }

    /// The Klipper extruder of the nozzle in use: that of the mounted head of a tool changer (`None` while no head
    /// is mounted or the mounted one has no nozzle), else that of the fixed head.
    pub fn selected_extruder(&self) -> Option<&str> {
        self.selected_head().and_then(|head| head.nozzles.first()).map(|nozzle| nozzle.extruder.as_str())
    }

    /// The part cooling fan of the nozzle in use: with swappable heads the one the mounted head states (`None` while
    /// no head is mounted or the mounted one states none), else Klipper's `[fan]`.
    pub fn selected_part_fan(&self) -> Option<&str> {
        if self.has_swappable_heads() {
            self.selected_head()?.fan.as_deref()
        } else {
            Some(PART_FAN)
        }
    }

    /// The mounted head of a tool changer, else the fixed head.
    fn selected_head(&self) -> Option<&Head> {
        if self.has_swappable_heads() {
            self.heads().find(|head| head.name.is_some() && head.active)
        } else {
            self.heads().next()
        }
    }

    fn heads(&self) -> impl Iterator<Item = &Head> {
        self.carriages.iter().flat_map(|carriage| &carriage.heads)
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

/// The fixed head with a nozzle per extruder that `fed` lanes feed, in the order the lanes come; without lanes the
/// nozzle of `extruder` with the direct feed.
fn fixed_head(fed: Vec<Lane>, tool_spools: &[ToolSpool]) -> Head {
    let mut nozzles: Vec<Nozzle> = Vec::new();
    for lane in fed {
        match nozzles.iter_mut().find(|nozzle| nozzle.extruder == lane.extruder) {
            Some(nozzle) => nozzle.lanes.push(lane),
            None => nozzles.push(Nozzle { extruder: lane.extruder.clone(), lanes: vec![lane] }),
        }
    }
    if nozzles.is_empty() {
        let lanes = vec![direct_lane(MAIN_EXTRUDER, Some(0), tool_spools)];
        nozzles.push(Nozzle { extruder: MAIN_EXTRUDER.to_owned(), lanes });
    }
    Head { name: None, active: true, nozzles, fan: None }
}

/// A head of the tool changer with the nozzle of its extruder: the lanes of `fed` that feed it (taken out of `fed`),
/// else the direct feed. A head without extruder (a probe, say) has no nozzle.
fn swappable_head(head: &ToolHead, fed: &mut Vec<Lane>, tool_spools: &[ToolSpool]) -> Head {
    let nozzles = head.extruder.iter().map(|extruder| {
        let (mine, rest): (Vec<Lane>, Vec<Lane>) =
            std::mem::take(fed).into_iter().partition(|lane| &lane.extruder == extruder);
        *fed = rest;
        let lanes = if mine.is_empty() {
            let direct = direct_lane(extruder, head.number, tool_spools);
            vec![Lane { active: head.active, ..direct }]
        } else {
            mine
        };
        let lanes = lanes.into_iter().map(|lane| Lane { head: Some(head.name.clone()), ..lane }).collect();
        Nozzle { extruder: extruder.clone(), lanes }
    });
    Head { name: Some(head.name.clone()), active: head.active, nozzles: nozzles.collect(), fan: head.fan.clone() }
}

/// The direct feed of `extruder` for `tool`, with the spool its tool macro declares.
fn direct_lane(extruder: &str, tool: Option<usize>, tool_spools: &[ToolSpool]) -> Lane {
    let declared = tool.and_then(|tool| tool_spools.iter().find(|spool| spool.tool == tool));
    Lane {
        gate: None,
        name: None,
        unit: None,
        unit_name: None,
        head: None,
        extruder: extruder.to_owned(),
        tool,
        spool_id: declared.and_then(|spool| spool.spool_id),
        spool_by_tool: declared.is_some(),
        spool: None,
        empty: false,
        active: true,
    }
}

/// A lane per gate, on the extruder of its unit. Happy Hare numbers gates across all units from 0 on, so the position
/// in the list is the gate index; code that looks a gate up still compares `Lane::gate`.
fn gate_lanes(mmu: &MmuStatus) -> Vec<Lane> {
    let several_units = mmu.units.len() > 1;
    mmu.gates
        .iter()
        .map(|gate| {
            let unit = mmu.units.iter().position(|unit| unit.contains(gate.index));
            let unit_state = unit.map(|unit| &mmu.units[unit]);
            Lane {
                gate: Some(gate.index),
                name: None,
                // Without reported units all gates form unit 0.
                unit: if mmu.units.is_empty() { Some(0) } else { unit },
                unit_name: unit_state.filter(|_| several_units).map(|unit| unit.display_name.clone()),
                head: None,
                extruder: unit_state.map_or(MAIN_EXTRUDER, |unit| unit.extruder.as_str()).to_owned(),
                tool: gate.tools.first().copied(),
                spool_id: gate.spool_id,
                spool_by_tool: false,
                spool: None,
                empty: false,
                active: usize::try_from(mmu.gate) == Ok(gate.index),
            }
        })
        .collect()
}

/// A lane per lane of AFC, named with its unit when AFC has several. Its spool is assigned through its tool.
fn afc_lanes(afc: &AfcStatus) -> Vec<Lane> {
    let mut units: Vec<&str> = Vec::new();
    afc.lanes
        .iter()
        .map(|lane| Lane {
            gate: None,
            name: Some(lane.name.clone()),
            // AFC's units have no number; their order of first appearance numbers them.
            unit: Some(units.iter().position(|unit| *unit == lane.unit).unwrap_or_else(|| {
                units.push(&lane.unit);
                units.len() - 1
            })),
            unit_name: afc.several_units.then(|| lane.unit.clone()),
            head: None,
            extruder: lane.extruder.clone(),
            tool: lane.tool,
            spool_id: lane.spool_id,
            spool_by_tool: lane.tool.is_some(),
            spool: None,
            empty: !lane.filled,
            active: afc.current_load.as_deref() == Some(lane.name.as_str()),
        })
        .collect()
}

/// The heads of AFC's tool changer, numbered by the tool of the lane on their extruder.
fn afc_heads(afc: &AfcStatus) -> Vec<ToolHead> {
    let tool_of = |extruder: &str| afc.lanes.iter().find(|lane| lane.extruder == extruder).and_then(|lane| lane.tool);
    afc.heads
        .iter()
        .map(|head| ToolHead {
            name: head.name.clone(),
            number: tool_of(&head.extruder),
            extruder: Some(head.extruder.clone()),
            // AFC states no part fan of a head (no `fan` in `AFC_extruder.py`, `AFC_Toolchanger.py`, main 2026-11-12).
            fan: None,
            active: head.mounted,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::afc::{AfcHead, AfcLane};
    use crate::sources::happy_hare::{MmuGate, MmuUnit};
    use crate::sources::toolchanger::ToolchangerStatus;

    fn mmu(units: &[(usize, usize)], selected: i32) -> PrinterStatus {
        let total: usize = units.iter().map(|(_, count)| count).sum();
        let mmu = MmuStatus {
            version: "4.1.0".into(),
            enabled: true,
            num_gates: total,
            gate: selected,
            units: units
                .iter()
                .map(|&(first_gate, num_gates)| MmuUnit {
                    display_name: format!("Unit {first_gate}"),
                    first_gate,
                    num_gates,
                    extruder: "extruder".into(),
                    ..MmuUnit::default()
                })
                .collect(),
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
            fan: None,
            active: number == active,
        };
        PrinterStatus {
            toolchanger: Some(ToolchangerStatus {
                status: "ready".into(),
                heads: vec![head(0, "extruder"), head(1, "extruder1"), head(2, "extruder2")],
                ..ToolchangerStatus::default()
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
            [Some(SpoolHolder::Tool(0)), Some(SpoolHolder::Tool(1)), Some(SpoolHolder::ActiveSpool)]
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
    fn happy_hares_gates_join_the_head_of_their_extruder_beside_a_tool_changer() {
        let mut status = mmu(&[(0, 2)], 0);
        status.toolchanger = toolchanger(0).toolchanger;

        let tree = MachineTree::build(&status);

        let heads = &tree.carriages[0].heads;
        assert_eq!(heads.len(), 3, "no fixed head beside the tool changer");
        let lanes = tree.lanes(&SpoolmanState::default());
        assert_eq!(lanes.iter().map(|lane| lane.gate).collect::<Vec<_>>(), [Some(0), Some(1), None, None]);
        assert_eq!(lanes[0].head.as_deref(), Some("T0"));
        assert_eq!(lanes[3].head.as_deref(), Some("T2"));
    }

    /// Two units on the heads of `extruder` and `extruder1`, as Happy Hare 4 allows with `extruder_name` per unit.
    fn units_on_two_extruders() -> PrinterStatus {
        let mut status = mmu(&[(0, 2), (2, 2)], 2);
        status.mmu.as_mut().unwrap().units[1].extruder = "extruder1".into();
        status
    }

    #[test]
    fn the_units_of_a_tool_changer_feed_the_heads_of_their_extruders() {
        let mut status = units_on_two_extruders();
        status.toolchanger = toolchanger(1).toolchanger;

        let tree = MachineTree::build(&status);

        let heads = &tree.carriages[0].heads;
        let gates = |head: usize| heads[head].nozzles[0].lanes.iter().map(|lane| lane.gate).collect::<Vec<_>>();
        assert_eq!(gates(0), [Some(0), Some(1)]);
        assert_eq!(gates(1), [Some(2), Some(3)]);
        assert_eq!(gates(2), [None], "T2 keeps its direct feed");
        let lanes = tree.lanes(&SpoolmanState::default());
        assert_eq!(
            lanes.iter().map(|lane| lane.unit_name.as_deref()).collect::<Vec<_>>(),
            [Some("Unit 0"), Some("Unit 0"), Some("Unit 2"), Some("Unit 2"), None]
        );
        assert_eq!(lanes.iter().position(|lane| lane.active), Some(2), "the selected gate only; T2 is not mounted");
    }

    #[test]
    fn without_a_tool_changer_the_fixed_head_carries_a_nozzle_per_fed_extruder() {
        let tree = MachineTree::build(&units_on_two_extruders());

        let nozzles = &tree.carriages[0].heads[0].nozzles;
        assert_eq!(
            nozzles.iter().map(|nozzle| nozzle.extruder.as_str()).collect::<Vec<_>>(),
            ["extruder", "extruder1"]
        );
        assert_eq!(nozzles[1].lanes.iter().map(|lane| lane.gate).collect::<Vec<_>>(), [Some(2), Some(3)]);
    }

    #[test]
    fn lanes_on_an_extruder_no_head_has_stay_on_a_fixed_head() {
        let mut status = units_on_two_extruders();
        status.mmu.as_mut().unwrap().units[1].extruder = "extruder9".into();
        status.toolchanger = toolchanger(0).toolchanger;

        let tree = MachineTree::build(&status);

        let heads = &tree.carriages[0].heads;
        assert_eq!(heads.len(), 4);
        assert_eq!((heads[3].name.as_deref(), heads[3].nozzles[0].extruder.as_str()), (None, "extruder9"));
    }

    /// AFC lanes `lane1` … on `extruder` (unit `Turtle_1`), mapped to `T0` …, `lane2` loaded; `AfcStatus` as
    /// `sources::afc` reads it.
    fn box_turtle(lanes: usize) -> PrinterStatus {
        let lane = |index: usize| AfcLane {
            name: format!("lane{}", index + 1),
            unit: "Turtle_1".into(),
            extruder: "extruder".into(),
            tool: Some(index),
            spool_id: Some(20 + index as u32),
            filled: true,
            ..AfcLane::default()
        };
        let afc = AfcStatus {
            current_load: Some("lane2".into()),
            lanes: (0..lanes).map(lane).collect(),
            ..AfcStatus::default()
        };
        PrinterStatus { afc: Some(afc), ..PrinterStatus::default() }
    }

    #[test]
    fn afc_lanes_feed_the_nozzle_of_their_extruder_and_take_their_spool_through_the_tool() {
        let tree = MachineTree::build(&box_turtle(4));

        assert_eq!(tree.carriages[0].heads.len(), 1);
        let lanes = tree.lanes(&SpoolmanState::default());
        assert_eq!(
            lanes.iter().map(|lane| lane.name.as_deref()).collect::<Vec<_>>(),
            [Some("lane1"), Some("lane2"), Some("lane3"), Some("lane4")]
        );
        assert_eq!(lanes.iter().map(|lane| lane.active).collect::<Vec<_>>(), [false, true, false, false]);
        assert_eq!(lanes[2].spool_holder(), Some(SpoolHolder::Tool(2)));
        assert_eq!(lanes[0].unit_name, None, "one unit");
        assert_eq!(tree.own_spool_ids(), [20, 21, 22, 23]);
    }

    #[test]
    fn an_afc_lane_without_tool_has_no_holder_even_in_use() {
        let mut status = box_turtle(2);
        status.afc.as_mut().unwrap().lanes[1].tool = None;

        let lanes = MachineTree::build(&status).lanes(&SpoolmanState::default());

        assert_eq!(lanes[1].spool_holder(), None);
    }

    /// AFC's own tool changer as on the Snapmaker U1 template: a head per `AFC_extruder` with its lane of the same name.
    #[test]
    fn the_heads_of_afcs_tool_changer_carry_their_lanes() {
        let mut status = box_turtle(0);
        let afc = status.afc.as_mut().unwrap();
        for (index, extruder) in ["extruder", "extruder1"].into_iter().enumerate() {
            afc.lanes.push(AfcLane {
                name: format!("e{index}"),
                unit: "Tools".into(),
                extruder: extruder.into(),
                tool: Some(index),
                ..AfcLane::default()
            });
            afc.heads.push(AfcHead { name: format!("e{index}"), extruder: extruder.into(), mounted: index == 1 });
        }
        afc.current_load = Some("e1".into());

        let tree = MachineTree::build(&status);

        let heads = &tree.carriages[0].heads;
        assert_eq!(
            heads.iter().map(|head| (head.name.as_deref(), head.active)).collect::<Vec<_>>(),
            [(Some("e0"), false), (Some("e1"), true)]
        );
        let lanes = tree.lanes(&SpoolmanState::default());
        assert_eq!(
            lanes.iter().map(|lane| (lane.head.as_deref(), lane.name.as_deref())).collect::<Vec<_>>(),
            [(Some("e0"), Some("e0")), (Some("e1"), Some("e1"))]
        );
    }

    #[test]
    fn beside_a_tool_changer_afc_brings_the_lanes_of_each_head() {
        let mut status = box_turtle(4);
        for lane in &mut status.afc.as_mut().unwrap().lanes[2..] {
            lane.extruder = "extruder1".into();
        }
        status.toolchanger = toolchanger(1).toolchanger;

        let tree = MachineTree::build(&status);

        let names = |head: usize| {
            tree.carriages[0].heads[head].nozzles[0].lanes.iter().map(|lane| lane.name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(names(0), [Some("lane1".into()), Some("lane2".into())]);
        assert_eq!(names(1), [Some("lane3".into()), Some("lane4".into())]);
        assert_eq!(names(2), [None], "T2 keeps its direct feed");
    }

    #[test]
    fn a_single_unit_is_not_named_at_the_lane() {
        let lanes = MachineTree::build(&mmu(&[(0, 2)], 0)).lanes(&SpoolmanState::default());

        assert_eq!(lanes[0].unit_name, None);
    }

    #[test]
    fn the_selected_nozzle_is_that_of_the_mounted_head() {
        assert_eq!(MachineTree::build(&toolchanger(1)).selected_extruder(), Some("extruder1"));
    }

    #[test]
    fn no_nozzle_is_selected_while_no_head_is_mounted() {
        let tree = MachineTree::build(&toolchanger(7));

        assert!(tree.has_swappable_heads());
        assert_eq!(tree.selected_extruder(), None);
    }

    #[test]
    fn a_printer_without_heads_heats_the_extruder() {
        let tree = MachineTree::build(&PrinterStatus::default());

        assert!(!tree.has_swappable_heads());
        assert_eq!(tree.selected_extruder(), Some("extruder"));
    }

    #[test]
    fn the_part_fan_is_that_of_the_mounted_head_and_plain_fan_only_without_heads() {
        let with_fans = |active: usize| {
            let mut status = toolchanger(active);
            let heads = &mut status.toolchanger.as_mut().unwrap().heads;
            heads[0].fan = Some("fan_generic T0_partfan".into());
            heads[1].fan = Some("fan_generic T1_partfan".into());
            MachineTree::build(&status)
        };

        assert_eq!(with_fans(1).selected_part_fan(), Some("fan_generic T1_partfan"));
        // A head that states no fan has none, not `[fan]`.
        assert_eq!(with_fans(2).selected_part_fan(), None);
        // No head mounted: no nozzle, no fan.
        assert_eq!(with_fans(7).selected_part_fan(), None);
        assert_eq!(MachineTree::build(&PrinterStatus::default()).selected_part_fan(), Some("fan"));
    }
}
