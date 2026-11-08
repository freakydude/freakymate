//! Spools in the printer: the lanes they sit in. A lane is one place for a spool: a gate of Happy Hare, or the one
//! spool of a printer without MMU (the active spool of Moonraker). How a print compares with them is
//! [`super::filament_match`].

use crate::sources::spoolman::{Spool, SpoolmanState};
use crate::sources::PrinterStatus;

/// A target that is further off than this from the temperature of the spool gets a hint, in °C.
const TEMPERATURE_TOLERANCE: f64 = 5.0;

/// One place for a spool.
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    /// `None` without MMU.
    pub gate: Option<usize>,
    /// First tool that is mapped to the gate.
    pub tool: Option<usize>,
    pub spool_id: Option<u32>,
    /// The spool, once Spoolman has answered.
    pub spool: Option<Spool>,
    /// The filament of this lane is the one in use: the selected gate, or the only spool.
    pub active: bool,
}

/// The lanes of the printer: a gate each while Happy Hare has the say, otherwise the one active spool.
pub fn lanes(status: &PrinterStatus, state: &SpoolmanState) -> Vec<Lane> {
    let spool = |id: Option<u32>| id.and_then(|id| state.spools.get(&id)).cloned();
    match status.happy_hare().filter(|mmu| !mmu.gates.is_empty()) {
        Some(mmu) => mmu
            .gates
            .iter()
            .map(|gate| Lane {
                gate: Some(gate.index),
                tool: gate.tools.first().copied(),
                spool_id: gate.spool_id,
                spool: spool(gate.spool_id),
                active: usize::try_from(mmu.gate) == Ok(gate.index),
            })
            .collect(),
        None => vec![Lane {
            gate: None,
            tool: Some(0),
            spool_id: state.active_spool_id,
            spool: spool(state.active_spool_id),
            active: true,
        }],
    }
}

/// The lane in use, if it has a spool.
pub fn active_spool(lanes: &[Lane]) -> Option<&Spool> {
    lanes.iter().find(|lane| lane.active).and_then(|lane| lane.spool.as_ref())
}

/// The spool for a hint about the temperature: the target that does not match it, if it is off by more than the
/// tolerance. Without target (off) or without a recommendation there is nothing to say.
pub fn temperature_hint(target: f64, recommended: Option<f64>) -> Option<f64> {
    let recommended = recommended?;
    (target > 0.0 && (target - recommended).abs() > TEMPERATURE_TOLERANCE).then_some(recommended)
}

/// The spools for the picker: those that sit in a lane first, then the most recently used.
pub fn picker_order(mut spools: Vec<Spool>, inserted: &[u32]) -> Vec<Spool> {
    spools.sort_by(|left, right| {
        inserted
            .contains(&right.id)
            .cmp(&inserted.contains(&left.id))
            .then_with(|| right.last_used.cmp(&left.last_used))
            .then_with(|| right.id.cmp(&left.id))
    });
    spools
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::happy_hare::{MmuGate, MmuStatus};

    fn spool(id: u32, material: &str, left: f64, color: [u8; 3]) -> Spool {
        Spool {
            id,
            material: material.into(),
            remaining_weight: Some(left),
            initial_weight: Some(1000.0),
            colors: vec![color],
            ..Spool::default()
        }
    }

    fn state_with(spools: &[Spool], active: Option<u32>) -> SpoolmanState {
        SpoolmanState {
            connected: true,
            active_spool_id: active,
            spools: spools.iter().map(|spool| (spool.id, spool.clone())).collect(),
            ..SpoolmanState::default()
        }
    }

    /// A printer with Happy Hare in charge, one gate per entry of `spool_ids`.
    fn mmu_with(spool_ids: &[Option<u32>], selected: i32, tool_to_gate: &[usize]) -> PrinterStatus {
        let mmu = MmuStatus {
            version: "4.1.0".into(),
            enabled: true,
            num_gates: spool_ids.len(),
            gate: selected,
            tool_to_gate: tool_to_gate.to_vec(),
            gates: spool_ids
                .iter()
                .enumerate()
                .map(|(index, spool_id)| MmuGate {
                    index,
                    spool_id: *spool_id,
                    tools: vec![index],
                    ..MmuGate::default()
                })
                .collect(),
            ..MmuStatus::default()
        };
        PrinterStatus { mmu: Some(mmu), ..PrinterStatus::default() }
    }

    fn without_mmu() -> PrinterStatus {
        PrinterStatus::default()
    }

    #[test]
    fn a_printer_without_mmu_has_one_lane_with_the_active_spool() {
        let state = state_with(&[spool(5, "PLA", 800.0, [0; 3])], Some(5));

        let lanes = lanes(&without_mmu(), &state);

        assert_eq!(lanes.len(), 1);
        assert_eq!((lanes[0].gate, lanes[0].spool_id, lanes[0].active), (None, Some(5), true));
        assert_eq!(active_spool(&lanes).map(|spool| spool.id), Some(5));
    }

    #[test]
    fn with_an_mmu_every_gate_is_a_lane_and_the_selected_one_is_active() {
        let state = state_with(&[spool(5, "PLA", 800.0, [0; 3]), spool(6, "PETG", 300.0, [0; 3])], Some(6));
        let status = mmu_with(&[Some(5), Some(6), None], 1, &[0, 1, 2]);

        let lanes = lanes(&status, &state);

        assert_eq!(lanes.iter().map(|lane| lane.spool_id).collect::<Vec<_>>(), [Some(5), Some(6), None]);
        assert_eq!(lanes.iter().map(|lane| lane.active).collect::<Vec<_>>(), [false, true, false]);
        assert_eq!(active_spool(&lanes).map(|spool| spool.id), Some(6));
    }

    #[test]
    fn a_disabled_mmu_leaves_the_one_active_spool() {
        let state = state_with(&[spool(5, "PLA", 800.0, [0; 3])], Some(5));
        let mut status = mmu_with(&[Some(9), None], 0, &[0, 1]);
        status.mmu.as_mut().unwrap().enabled = false;

        let lanes = lanes(&status, &state);

        assert_eq!(lanes.len(), 1);
        assert_eq!((lanes[0].gate, lanes[0].spool_id), (None, Some(5)));
    }

    #[test]
    fn a_spool_that_spoolman_has_not_answered_yet_is_a_lane_without_spool() {
        let lanes = lanes(&mmu_with(&[Some(9)], 0, &[0]), &SpoolmanState::default());

        assert_eq!((lanes[0].spool_id, lanes[0].spool.is_none()), (Some(9), true));
    }

    #[test]
    fn the_target_is_only_flagged_when_it_is_off_by_more_than_the_tolerance() {
        assert_eq!(temperature_hint(210.0, Some(240.0)), Some(240.0));
        assert_eq!(temperature_hint(238.0, Some(240.0)), None);
        assert_eq!(temperature_hint(0.0, Some(240.0)), None);
        assert_eq!(temperature_hint(210.0, None), None);
    }

    #[test]
    fn the_picker_lists_inserted_spools_first_then_the_most_recently_used() {
        let at = |id: u32, last: &str| Spool { id, last_used: last.into(), ..Spool::default() };
        let spools = vec![at(1, "2026-10-01"), at(2, "2026-10-20"), at(3, ""), at(4, "2026-10-10")];

        let order: Vec<u32> = picker_order(spools, &[3]).iter().map(|spool| spool.id).collect();

        assert_eq!(order, [3, 2, 4, 1]);
    }
}
