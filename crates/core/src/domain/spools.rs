//! Spools in the printer: the lanes they sit in (see [`super::machine_tree`]). How a print compares with them is
//! [`super::filament_match`].

pub use super::machine_tree::Lane;
use crate::color::Rgba;
use crate::commands::Command;
use crate::domain::machine_tree::MachineTree;
use crate::sources::happy_hare::MmuGate;
use crate::sources::klipper::BED_HEATER;
use crate::sources::spoolman::{Spool, SpoolmanState};
use crate::sources::PrinterStatus;

/// A target that is further off than this from the temperature of the spool gets a hint, in °C.
const TEMPERATURE_TOLERANCE: f64 = 5.0;

/// The colours a filament shows and whether they follow each other along it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FilamentColors {
    pub colors: Vec<Rgba>,
    pub longitudinal: bool,
}

impl FilamentColors {
    /// The colours of a gate: a multi-colour spool of Spoolman shows all of its colours; otherwise the gate has the
    /// one colour Happy Hare reports.
    pub fn of_gate(gate: &MmuGate, spool: Option<&Spool>) -> Self {
        match spool.filter(|spool| spool.is_multi_color()) {
            Some(spool) => Self { colors: spool.colors.clone(), longitudinal: spool.has_longitudinal_colors() },
            None => Self { colors: gate.color.into_iter().collect(), longitudinal: false },
        }
    }
}

/// The lanes of the printer, for code that has no tree at hand.
pub fn lanes(status: &PrinterStatus, state: &SpoolmanState) -> Vec<Lane> {
    MachineTree::build(status).lanes(state)
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

/// Heats to what the spool recommends; `None` without a nozzle temperature. The bed keeps its target if Spoolman has
/// no temperature for it.
pub fn preset(spool: &Spool, status: &PrinterStatus) -> Option<Command> {
    let extruder = spool.extruder_temp?;
    let bed = spool.bed_temp.or_else(|| status.heater(BED_HEATER).map(|heater| heater.target)).unwrap_or(0.0);
    Some(Command::ApplyPreset { extruder, bed })
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
    use crate::sources::happy_hare::MmuStatus;
    use crate::sources::klipper::Heater;
    use crate::sources::spoolman::test_spool;

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
        let state = state_with(&[test_spool(5, "PLA", 800.0, [0; 3])], Some(5));

        let lanes = lanes(&without_mmu(), &state);

        assert_eq!(lanes.len(), 1);
        assert_eq!((lanes[0].gate, lanes[0].spool_id, lanes[0].active), (None, Some(5), true));
        assert_eq!(active_spool(&lanes).map(|spool| spool.id), Some(5));
    }

    #[test]
    fn with_an_mmu_every_gate_is_a_lane_and_the_selected_one_is_active() {
        let state = state_with(&[test_spool(5, "PLA", 800.0, [0; 3]), test_spool(6, "PETG", 300.0, [0; 3])], Some(6));
        let status = mmu_with(&[Some(5), Some(6), None], 1, &[0, 1, 2]);

        let lanes = lanes(&status, &state);

        assert_eq!(lanes.iter().map(|lane| lane.spool_id).collect::<Vec<_>>(), [Some(5), Some(6), None]);
        assert_eq!(lanes.iter().map(|lane| lane.active).collect::<Vec<_>>(), [false, true, false]);
        assert_eq!(active_spool(&lanes).map(|spool| spool.id), Some(6));
    }

    #[test]
    fn a_disabled_mmu_leaves_the_one_active_spool() {
        let state = state_with(&[test_spool(5, "PLA", 800.0, [0; 3])], Some(5));
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

    #[test]
    fn a_multi_colour_spool_shows_its_colours_instead_of_the_colour_of_the_gate() {
        let (red, green, blue) = ([0xff, 0, 0, 0xff], [0, 0xff, 0, 0xff], [0, 0, 0xff, 0xff]);
        let gate = MmuGate { color: Some(red), ..MmuGate::default() };
        let spool = |colors: Vec<Rgba>| Spool { colors, longitudinal: true, ..Spool::default() };

        assert_eq!(
            FilamentColors::of_gate(&gate, Some(&spool(vec![green, blue]))),
            FilamentColors { colors: vec![green, blue], longitudinal: true }
        );
        assert_eq!(
            FilamentColors::of_gate(&gate, Some(&spool(vec![green]))),
            FilamentColors { colors: vec![red], longitudinal: false }
        );
        assert_eq!(FilamentColors::of_gate(&gate, None), FilamentColors { colors: vec![red], longitudinal: false });
        assert_eq!(FilamentColors::of_gate(&MmuGate::default(), None), FilamentColors::default());
    }

    #[test]
    fn the_preset_heats_to_the_spool_and_the_bed_keeps_its_target_without_a_bed_temperature() {
        let bed = Heater { object: BED_HEATER.into(), target: 55.0, ..Heater::default() };
        let with_bed = PrinterStatus { heaters: vec![bed], ..PrinterStatus::default() };
        let spool = |extruder, bed| Spool { extruder_temp: extruder, bed_temp: bed, ..Spool::default() };

        assert_eq!(
            preset(&spool(Some(240.0), Some(100.0)), &with_bed),
            Some(Command::ApplyPreset { extruder: 240.0, bed: 100.0 })
        );
        assert_eq!(
            preset(&spool(Some(240.0), None), &with_bed),
            Some(Command::ApplyPreset { extruder: 240.0, bed: 55.0 })
        );
        assert_eq!(
            preset(&spool(Some(240.0), None), &without_mmu()),
            Some(Command::ApplyPreset { extruder: 240.0, bed: 0.0 })
        );
        assert_eq!(preset(&spool(None, Some(100.0)), &with_bed), None);
    }
}
