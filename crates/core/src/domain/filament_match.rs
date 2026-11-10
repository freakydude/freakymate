//! What a print asks of each tool and how that compares with the filament where the tool prints from. Only facts that
//! both sides have are compared, and everything here is a hint: nothing forbids anything.
//!
//! Where the filament of a tool comes from depends on what the printer has: with Happy Hare the gate of the tool
//! (`ttg_map`), its spool from Spoolman first, otherwise what Happy Hare knows of the gate; with a tool changer the
//! spool of the tool's head; otherwise only tool 0 and the active spool of Spoolman. Without either there is nothing
//! to compare. Tools that share a gate share its spool, so their planned weights add up. Without Happy Hare a tool
//! beyond 0 needs a command `T<n>` in Klipper's command list (a tool changer's or a macro); a file that calls a missing
//! one stops at an unknown command.
//!
//! Slicers know one colour per tool (with alpha), so a multi-colour spool is compared by its first colour. With
//! EndlessSpool on, the other gates of a gate's group add their weight, except those a tool of the file uses; gates of
//! the file in one group share them.

use super::spools::Lane;
use crate::color::{is_clear, parse_hex, rgb, Rgba};
use crate::sources::happy_hare::{GateAvailability, MmuGate};
use crate::sources::klipper::Setup;
use crate::sources::moonraker_server::files::FileMetadata;
use crate::sources::spoolman::Spool;
use crate::sources::PrinterStatus;
use std::collections::HashMap;

/// Colours further apart than this (distance in RGB, at most 441) count as different. Wide enough for "the same red
/// from another maker", too wide to tell close colours apart (orange and red are 81 apart).
const COLOR_DISTANCE: f64 = 120.0;

/// What the file asks of one tool.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolRequest {
    pub tool: usize,
    pub material: Option<String>,
    /// Name of the slicer's filament profile, free text: for display, never compared.
    pub profile: Option<String>,
    /// The slicer's colour with its alpha; fully clear (`#RRGGBB00`) is a clear filament, whose colour is not compared.
    pub color: Option<Rgba>,
    pub grams: Option<f64>,
    pub temperature: Option<f64>,
}

/// One difference between what a tool asks for and what it would print with.
#[derive(Debug, Clone, PartialEq)]
pub enum Difference {
    Material {
        expected: String,
        loaded: String,
    },
    Color,
    /// Planned grams against what the spool has left; tools that share the spool add up. `reserve` is what EndlessSpool
    /// could continue with, not part of `left`.
    Weight {
        needed: f64,
        left: f64,
        reserve: Option<f64>,
    },
}

/// How a tool of the file compares with the printer.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Every fact that both sides have fits; at least one was compared.
    Fits,
    Differs(Vec<Difference>),
    /// Happy Hare reports the gate of the tool as empty; its gate check at print start pauses then.
    GateEmpty,
    /// The printer has no such tool: Happy Hare maps no gate to it, or without Happy Hare Klipper has no command `T<n>`.
    NoGate,
    /// Nothing to compare: one side has no facts.
    Unknown,
}

impl Verdict {
    /// Several verdicts as one: the first that does not fit wins over those without facts; `Fits` only when all fit;
    /// `Unknown` when some cannot be compared or there are none.
    pub fn combine<'a>(verdicts: impl IntoIterator<Item = &'a Verdict>) -> Verdict {
        let mut all_fit = None;
        for verdict in verdicts {
            match verdict {
                Verdict::Fits => all_fit = Some(all_fit.unwrap_or(true)),
                Verdict::Unknown => all_fit = Some(false),
                Verdict::Differs(_) | Verdict::GateEmpty | Verdict::NoGate => return verdict.clone(),
            }
        }
        if all_fit == Some(true) {
            Verdict::Fits
        } else {
            Verdict::Unknown
        }
    }

    /// Something does not fit: a fact differs, the gate is empty or the printer has no such tool.
    pub fn differs(&self) -> bool {
        matches!(self, Verdict::Differs(_) | Verdict::GateEmpty | Verdict::NoGate)
    }
}

/// A tool of the file, the gate it prints from (with Happy Hare) and how it compares.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolMatch {
    pub request: ToolRequest,
    pub gate: Option<usize>,
    /// The planned grams that come from the spool the tool prints from: with the other tools of the file that share
    /// its gate. `None` when the file plans no weight for them.
    pub needed: Option<f64>,
    /// The grams EndlessSpool can continue with for this tool's gate, shared with the other tools of its group.
    pub reserve: Option<f64>,
    pub verdict: Verdict,
}

/// The tools of the file that print from `gate`, in tool order.
pub fn tools_of_gate(matches: &[ToolMatch], gate: usize) -> Vec<usize> {
    matches.iter().filter(|found| found.gate == Some(gate)).map(|found| found.request.tool).collect()
}

/// What the file asks of each tool it uses, in tool order.
pub fn tool_requests(metadata: &FileMetadata) -> Vec<ToolRequest> {
    let tools = used_tools(metadata);
    let only_one = tools.len() == 1;
    tools
        .into_iter()
        .map(|tool| ToolRequest {
            tool,
            material: per_tool(&metadata.filament_types, tool).map(str::to_owned),
            profile: per_tool(&metadata.filament_names, tool).map(str::to_owned),
            color: metadata.filament_colors.get(tool).and_then(|color| parse_hex(color)),
            grams: metadata
                .filament_weights
                .get(tool)
                .copied()
                .filter(|grams| *grams > 0.0)
                .or_else(|| only_one.then_some(metadata.filament_weight_total).flatten()),
            temperature: metadata.filament_temps.get(tool).copied().filter(|degrees| *degrees > 0.0),
        })
        .collect()
}

/// The tools the print uses: what the G-code calls (`referenced_tools`, written by Happy Hare's preprocessor), else
/// the filaments with a planned weight, else tool 0.
fn used_tools(metadata: &FileMetadata) -> Vec<usize> {
    if !metadata.referenced_tools.is_empty() {
        return metadata.referenced_tools.clone();
    }
    let weighed: Vec<usize> =
        (0..metadata.filament_weights.len()).filter(|tool| metadata.filament_weights[*tool] > 0.0).collect();
    if weighed.is_empty() {
        vec![0]
    } else {
        weighed
    }
}

/// The entry of a tool in a list per filament; a single entry stands for every tool.
fn per_tool(list: &[String], tool: usize) -> Option<&str> {
    let entry = if list.len() == 1 { list.first() } else { list.get(tool) };
    entry.map(String::as_str).filter(|text| !text.is_empty())
}

/// Compares every tool the file uses with the filament it would print with; `klipper` tells which tool commands exist.
pub fn match_tools(metadata: &FileMetadata, status: &PrinterStatus, lanes: &[Lane], klipper: &Setup) -> Vec<ToolMatch> {
    let tool_to_gate = status.happy_hare().map(|mmu| mmu.tool_to_gate.as_slice()).unwrap_or_default();
    match_requests(tool_requests(metadata), tool_to_gate, status, lanes, klipper)
}

/// [`match_tools`] with the tool map given, so that a gate can be tried for a tool (see [`fitting_gates`]).
fn match_requests(
    requests: Vec<ToolRequest>,
    tool_to_gate: &[usize],
    status: &PrinterStatus,
    lanes: &[Lane],
    klipper: &Setup,
) -> Vec<ToolMatch> {
    let with_gates = status.happy_hare().is_some_and(|mmu| !mmu.gates.is_empty());
    let gate_of = |tool: usize| tool_to_gate.get(tool).copied().filter(|_| with_gates);
    let needs = needs_per_gate(&requests, gate_of);
    let busy: Vec<usize> = requests.iter().filter_map(|request| gate_of(request.tool)).collect();
    let reserves = shared_reserves(&busy, &needs, status, lanes);
    requests
        .into_iter()
        .map(|request| {
            let gate = gate_of(request.tool);
            let needed = gate.and_then(|gate| needs.get(&gate).copied()).or(request.grams);
            let reserve = gate.and_then(|gate| reserves.get(&gate).copied());
            match_tool(request, gate, needed, reserve, status, lanes, klipper)
        })
        .collect()
}

/// The gates the tools of the file print from (with Happy Hare); see [`endless_spool_reserve`].
pub fn busy_gates(matches: &[ToolMatch]) -> Vec<usize> {
    matches.iter().filter_map(|found| found.gate).collect()
}

/// The grams EndlessSpool can continue with once the gate runs out: what Spoolman knows is left in the other gates of
/// its group. Conservative: a gate that a tool of the file prints from (`busy`) is kept for that tool, and an empty
/// gate or one without known weight adds nothing. `None` when EndlessSpool is off, the gate has no group or no other
/// gate of the group adds anything.
pub fn endless_spool_reserve(gate: usize, busy: &[usize], status: &PrinterStatus, lanes: &[Lane]) -> Option<f64> {
    let mmu = status.happy_hare().filter(|mmu| mmu.endless_spool)?;
    let group = mmu.gates.iter().find(|candidate| candidate.index == gate)?.endless_spool_group?;
    let grams: Vec<f64> = mmu
        .gates
        .iter()
        .filter(|other| other.index != gate && other.endless_spool_group == Some(group))
        .filter(|other| other.availability != GateAvailability::Empty && !busy.contains(&other.index))
        .filter_map(|other| gate_supply(other, lanes).left)
        .collect();
    (!grams.is_empty()).then(|| grams.iter().sum())
}

/// Per gate a tool of the file prints from: the grams EndlessSpool can continue with once its spool runs out. The
/// gates of the group that no tool uses ([`endless_spool_reserve`]) are shared: each gate gets them less what the other
/// gates of its group lack for their tools. So a group fits as long as its gates together lack no more than there is,
/// without guessing which runs out first. An empty gate lacks all it needs, one without known weight nothing.
fn shared_reserves(
    busy: &[usize],
    needs: &HashMap<usize, f64>,
    status: &PrinterStatus,
    lanes: &[Lane],
) -> HashMap<usize, f64> {
    let Some(mmu) = status.happy_hare().filter(|mmu| mmu.endless_spool) else { return HashMap::new() };
    let state_of = |gate: usize| mmu.gates.iter().find(|candidate| candidate.index == gate);
    let group_of = |gate: usize| state_of(gate).and_then(|state| state.endless_spool_group);
    let lacking = |gate: usize| {
        let left = state_of(gate).and_then(|state| match state.availability {
            GateAvailability::Empty => Some(0.0),
            _ => gate_supply(state, lanes).left,
        });
        needs.get(&gate).zip(left).map_or(0.0, |(needed, left)| (needed - left).max(0.0))
    };
    let mut gates = busy.to_vec();
    gates.sort_unstable();
    gates.dedup();
    gates
        .iter()
        .filter_map(|&gate| {
            let pool = endless_spool_reserve(gate, busy, status, lanes)?;
            let others: f64 = gates
                .iter()
                .filter(|&&other| other != gate && group_of(other) == group_of(gate))
                .map(|&other| lacking(other))
                .sum();
            Some((gate, pool - others)).filter(|(_, grams)| *grams > 0.0)
        })
        .collect()
}

/// The planned weight of all tools that print from the same gate, per gate: they all take from its spool.
fn needs_per_gate(requests: &[ToolRequest], gate_of: impl Fn(usize) -> Option<usize>) -> HashMap<usize, f64> {
    let mut needs = HashMap::new();
    for request in requests {
        if let (Some(gate), Some(grams)) = (gate_of(request.tool), request.grams) {
            *needs.entry(gate).or_insert(0.0) += grams;
        }
    }
    needs
}

/// Per gate index of Happy Hare: whether the gate is a good choice for `tool` of the file (`matches`). Each gate is
/// tried as if the tool were mapped to it, by the same rule as the verdict (shared spool, EndlessSpool reserve), so the
/// own gate fits exactly when the tool does. Stricter than the verdict: the gate must know the material and colour the
/// file asks for. An empty gate never fits. Empty without Happy Hare or when the file does not use the tool.
pub fn fitting_gates(matches: &[ToolMatch], tool: usize, status: &PrinterStatus, lanes: &[Lane]) -> Vec<bool> {
    let Some(mmu) = status.happy_hare() else { return Vec::new() };
    let Some(position) = matches.iter().position(|found| found.request.tool == tool) else { return Vec::new() };
    let request = &matches[position].request;
    let mut fits = vec![false; mmu.gates.iter().map(|gate| gate.index + 1).max().unwrap_or(0)];
    if tool >= mmu.tool_to_gate.len() {
        return fits;
    }
    for gate in mmu.gates.iter().filter(|gate| gate.availability != GateAvailability::Empty) {
        let supply = gate_supply(gate, lanes);
        let knows_material = request.material.is_none() || supply.material.is_some();
        let knows_color = !request.color.is_some_and(|color| !is_clear(color)) || !supply.colors.is_empty();
        if knows_material && knows_color {
            let mut tool_to_gate = mmu.tool_to_gate.clone();
            tool_to_gate[tool] = gate.index;
            let requests = matches.iter().map(|found| found.request.clone()).collect();
            // Klipper's tool commands only count without Happy Hare's gates, so its setup is not needed here.
            let tried = match_requests(requests, &tool_to_gate, status, lanes, &Setup::default());
            fits[gate.index] = tried[position].verdict == Verdict::Fits;
        }
    }
    fits
}

/// One tool against where it prints from (`gate` with Happy Hare); `needed` is the planned weight that comes from that
/// spool, `reserve` what EndlessSpool can continue with.
fn match_tool(
    request: ToolRequest,
    gate: Option<usize>,
    needed: Option<f64>,
    reserve: Option<f64>,
    status: &PrinterStatus,
    lanes: &[Lane],
    klipper: &Setup,
) -> ToolMatch {
    let with_gates = status.happy_hare().is_some_and(|mmu| !mmu.gates.is_empty());
    let missing = if with_gates {
        gate.is_none()
    } else {
        request.tool > 0 && klipper.has_tool_command(request.tool) == Some(false)
    };
    if missing {
        return ToolMatch { request, gate: None, needed, reserve: None, verdict: Verdict::NoGate };
    }
    let supply = supply_of(request.tool, gate, status, lanes).unwrap_or_default();
    let verdict = if supply.empty { Verdict::GateEmpty } else { compare(&request, needed, reserve, &supply) };
    ToolMatch { request, gate, needed, reserve, verdict }
}

/// What a tool of the file prints with, as far as it is known: with Happy Hare's gates what its gate holds, without
/// them the spool of the direct feed for that tool (the head of a tool changer, or tool 0 with the spool in use).
/// `None` when there is nothing to show (no gate, no spool, no lane for the tool).
pub fn tool_supply(found: &ToolMatch, status: &PrinterStatus, lanes: &[Lane]) -> Option<Supply> {
    supply_of(found.request.tool, found.gate, status, lanes)
}

fn supply_of(tool: usize, gate: Option<usize>, status: &PrinterStatus, lanes: &[Lane]) -> Option<Supply> {
    match status.happy_hare().filter(|mmu| !mmu.gates.is_empty()) {
        Some(mmu) => {
            let gate = gate?;
            mmu.gates.iter().find(|candidate| candidate.index == gate).map(|state| gate_supply(state, lanes))
        }
        None => super::spools::direct_lane(lanes, tool)?.spool.as_ref().map(Supply::of_spool),
    }
}

/// The filament where a tool prints from, as far as it is known.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Supply {
    pub material: Option<String>,
    /// The colours of the filament with their alpha; several for a multi-colour spool of Spoolman.
    pub colors: Vec<Rgba>,
    /// The colours follow each other along the filament instead of lying side by side.
    pub longitudinal: bool,
    /// Vendor and name of the spool, or the filament name Happy Hare has for the gate.
    pub name: Option<String>,
    /// Grams left on the spool; only Spoolman knows.
    pub left: Option<f64>,
    /// Happy Hare reports the gate as empty.
    pub empty: bool,
}

impl Supply {
    fn of_spool(spool: &Spool) -> Self {
        Self {
            material: Some(spool.material.clone()).filter(|material| !material.is_empty()),
            colors: spool.colors.clone(),
            longitudinal: spool.has_longitudinal_colors(),
            name: Some(spool.title()).filter(|title| !title.is_empty()),
            left: spool.remaining_weight,
            empty: false,
        }
    }
}

/// What a gate holds: its spool from Spoolman first, otherwise what Happy Hare knows of the gate (no weight there).
pub fn gate_supply(gate: &MmuGate, lanes: &[Lane]) -> Supply {
    let lane = lanes.iter().find(|lane| lane.gate == Some(gate.index));
    let empty = gate.availability == GateAvailability::Empty;
    if let Some(spool) = lane.and_then(|lane| lane.spool.as_ref()) {
        return Supply { empty, ..Supply::of_spool(spool) };
    }
    Supply {
        material: Some(gate.material.clone()).filter(|material| !material.is_empty()),
        colors: gate.color.into_iter().collect(),
        longitudinal: false,
        name: Some(gate.name.clone()).filter(|name| !name.is_empty()),
        left: None,
        empty,
    }
}

fn compare(request: &ToolRequest, needed: Option<f64>, reserve: Option<f64>, supply: &Supply) -> Verdict {
    let mut compared = false;
    let mut differences = Vec::new();
    if let (Some(expected), Some(loaded)) = (&request.material, &supply.material) {
        compared = true;
        if !same_material(expected, loaded) {
            differences.push(Difference::Material { expected: expected.clone(), loaded: loaded.clone() });
        }
    }
    // Slicers know one colour per tool, so a multi-colour spool is compared by its first colour; a clear one on either
    // side is nothing to compare.
    let wanted = request.color.filter(|color| !is_clear(*color));
    let loaded = supply.colors.first().copied().filter(|color| !is_clear(*color));
    if let (Some(wanted), Some(loaded)) = (wanted, loaded) {
        compared = true;
        if color_distance(rgb(wanted), rgb(loaded)) > COLOR_DISTANCE {
            differences.push(Difference::Color);
        }
    }
    if let (Some(needed), Some(left)) = (needed, supply.left) {
        compared = true;
        if needed > left + reserve.unwrap_or(0.0) {
            differences.push(Difference::Weight { needed, left, reserve });
        }
    }
    match (differences.is_empty(), compared) {
        (false, _) => Verdict::Differs(differences),
        (true, true) => Verdict::Fits,
        (true, false) => Verdict::Unknown,
    }
}

/// `ABS+` is `ABS`, case does not matter.
fn same_material(left: &str, right: &str) -> bool {
    let normalized = |text: &str| text.trim().trim_end_matches('+').to_ascii_uppercase();
    normalized(left) == normalized(right)
}

fn color_distance(left: [u8; 3], right: [u8; 3]) -> f64 {
    left.iter().zip(right).map(|(a, b)| (f64::from(*a) - f64::from(b)).powi(2)).sum::<f64>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::spools::lanes;
    use crate::sources::happy_hare::MmuStatus;
    use crate::sources::moonraker_server::files::parse_metadata;
    use crate::sources::spoolman::test_spool;
    use crate::sources::spoolman::{Spool, SpoolmanState};

    /// The metadata Moonraker reported for the sliced test files (OrcaSlicer 2.5, Voron with Happy Hare 4.2.0).
    fn fixture(name: &str) -> FileMetadata {
        let text = match name {
            "4 tools" => include_str!("../../tests/fixtures/multicolor/cb-mc4-used4-all_pla-t0-t3.metadata.json"),
            "two of four" => {
                include_str!("../../tests/fixtures/multicolor/cb-mc4-used2-all_pla-t0_and_t3.metadata.json")
            }
            "white to black" => {
                include_str!(
                    "../../tests/fixtures/multicolor/cb-mc4-used4-white_to_black-all_pla-t0_and_t3.metadata.json"
                )
            }
            "only T2" => include_str!("../../tests/fixtures/multicolor/cb-mc4-used4-all_pla-t2.metadata.json"),
            "PETG support" => {
                include_str!("../../tests/fixtures/multicolor/cb-mc4-used4-all_pla_support_petg-t0-t3.metadata.json")
            }
            "transparent" => include_str!(
                "../../tests/fixtures/multicolor/cb-mc4-used4_1-is-transparent-all_pla-t0_and_t3.metadata.json"
            ),
            "11 tools" => {
                include_str!("../../tests/fixtures/multicolor/cb-mc11-used11-11color-all_pla-t0_and_t10.metadata.json")
            }
            _ => unreachable!("no fixture {name}"),
        };
        let response: serde_json::Value = serde_json::from_str(text).unwrap();
        parse_metadata(&response["result"]).unwrap()
    }

    fn spoolman(spools: &[Spool], active: Option<u32>) -> SpoolmanState {
        SpoolmanState {
            connected: true,
            active_spool_id: active,
            spools: spools.iter().map(|spool| (spool.id, spool.clone())).collect(),
            ..SpoolmanState::default()
        }
    }

    /// A gate of Happy Hare with what it knows of its filament, without spool.
    fn gate(material: &str, color: Option<[u8; 3]>) -> MmuGate {
        let color = color.map(|[red, green, blue]| [red, green, blue, 0xff]);
        MmuGate { availability: GateAvailability::Available, material: material.into(), color, ..MmuGate::default() }
    }

    /// A printer with Happy Hare in charge; gate `n` is `gates[n]`, tool `t` prints from gate `tool_to_gate[t]`.
    fn happy_hare(gates: Vec<MmuGate>, tool_to_gate: &[usize]) -> PrinterStatus {
        let gates = gates.into_iter().enumerate().map(|(index, gate)| MmuGate { index, ..gate }).collect::<Vec<_>>();
        let mmu = MmuStatus {
            version: "4.2.0".into(),
            enabled: true,
            num_gates: gates.len(),
            tool_to_gate: tool_to_gate.to_vec(),
            gates,
            ..MmuStatus::default()
        };
        PrinterStatus { mmu: Some(mmu), ..PrinterStatus::default() }
    }

    fn verdicts(metadata: &FileMetadata, status: &PrinterStatus, spools: &SpoolmanState) -> Vec<(usize, Verdict)> {
        match_tools(metadata, status, &lanes(status, spools), &Setup::default())
            .into_iter()
            .map(|found| (found.request.tool, found.verdict))
            .collect()
    }

    fn file(weights: &[f64], types: &[&str], colors: &[&str]) -> FileMetadata {
        FileMetadata {
            filament_weights: weights.to_vec(),
            filament_weight_total: Some(weights.iter().sum()),
            filament_types: types.iter().map(|text| (*text).to_owned()).collect(),
            filament_colors: colors.iter().map(|color| (*color).to_owned()).collect(),
            ..FileMetadata::default()
        }
    }

    #[test]
    fn a_file_asks_each_used_tool_for_material_colour_grams_and_profile() {
        let requests = tool_requests(&fixture("PETG support"));

        assert_eq!(requests.iter().map(|request| request.tool).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(
            requests[3],
            ToolRequest {
                tool: 3,
                material: Some("PETG".into()),
                profile: Some("Elegoo PETG @ECC2 - Copy".into()),
                color: Some([0x2B, 0x06, 0xCB, 0xFF]),
                grams: Some(3.09),
                temperature: Some(250.0),
            }
        );
    }

    #[test]
    fn only_the_tools_the_print_calls_are_asked() {
        let only_t2 = fixture("only T2");
        assert_eq!(tool_requests(&only_t2).iter().map(|request| request.tool).collect::<Vec<_>>(), [2]);

        // Without Happy Hare's preprocessor there is no list of called tools: the planned weights tell.
        let without_preprocessor = FileMetadata { referenced_tools: Vec::new(), ..only_t2 };
        let requests = tool_requests(&without_preprocessor);
        assert_eq!((requests.len(), requests[0].tool, requests[0].grams), (1, 2, Some(17.76)));

        assert_eq!(tool_requests(&FileMetadata::default()).iter().map(|request| request.tool).collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn a_file_that_calls_two_of_its_four_tools_asks_only_those() {
        let requests = tool_requests(&fixture("two of four"));

        assert_eq!(requests.iter().map(|request| request.tool).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn swapped_black_and_white_gates_differ_in_colour() {
        let metadata = fixture("white to black");
        let gates = |colors: [[u8; 3]; 4]| {
            happy_hare(colors.iter().map(|color| gate("PLA", Some(*color))).collect(), &[0, 1, 2, 3])
        };
        let grey = [[0x63; 3], [0xB4; 3]];

        let fitting = gates([grey[0], grey[1], [0, 0, 0], [0xFF; 3]]);
        assert!(verdicts(&metadata, &fitting, &SpoolmanState::default())
            .iter()
            .all(|(_, verdict)| *verdict == Verdict::Fits));

        let swapped = gates([grey[0], grey[1], [0xFF; 3], [0, 0, 0]]);
        let found = verdicts(&metadata, &swapped, &SpoolmanState::default());
        assert_eq!(found[2], (2, Verdict::Differs(vec![Difference::Color])));
        assert_eq!(found[3], (3, Verdict::Differs(vec![Difference::Color])));
    }

    #[test]
    fn a_clear_filament_is_transparent_and_its_colour_not_compared() {
        let metadata = fixture("transparent");
        assert_eq!(tool_requests(&metadata)[3].color, Some([0xFF, 0xFF, 0xFF, 0x00]));

        let status = happy_hare(
            vec![gate("PETG", None), gate("PETG", None), gate("PETG", None), gate("PETG", Some([0, 0, 0]))],
            &[0, 1, 2, 3],
        );
        assert_eq!(verdicts(&metadata, &status, &SpoolmanState::default())[3], (3, Verdict::Fits));
    }

    #[test]
    fn a_printer_with_fewer_gates_than_the_file_has_tools_names_the_missing_ones() {
        // Franks Voron: one gate, the file is sliced for four tools.
        let status = happy_hare(vec![gate("PLA", Some([0xF7, 0x67, 0x07]))], &[0]);

        let found = verdicts(&fixture("4 tools"), &status, &SpoolmanState::default());

        assert_eq!(found, [(0, Verdict::Fits), (1, Verdict::NoGate), (2, Verdict::NoGate), (3, Verdict::NoGate)]);
    }

    #[test]
    fn an_empty_gate_is_named_instead_of_compared() {
        let mut empty = gate("PLA", None);
        empty.availability = GateAvailability::Empty;
        let status = happy_hare(vec![gate("PLA", None), empty], &[0, 1]);
        let metadata = FileMetadata { referenced_tools: vec![0, 1], ..file(&[5.0, 5.0], &["PLA", "PLA"], &[]) };

        let found = verdicts(&metadata, &status, &SpoolmanState::default());

        assert_eq!(found, [(0, Verdict::Fits), (1, Verdict::GateEmpty)]);
    }

    #[test]
    fn happy_hare_without_spoolman_compares_what_the_gate_knows() {
        let status = happy_hare(vec![gate("PLA", Some([0xF7, 0x67, 0x07])), gate("PLA", Some([0, 0, 0xFF]))], &[1, 0]);
        let metadata = FileMetadata {
            referenced_tools: vec![0, 1],
            ..file(&[10.0, 10.0], &["PETG", "PLA"], &["#F76707", "#0000FF"])
        };

        let found = verdicts(&metadata, &status, &SpoolmanState::default());

        // Tool 0 prints from gate 1 (blue PLA), tool 1 from gate 0 (orange PLA); no weights without Spoolman.
        assert_eq!(
            found,
            [
                (
                    0,
                    Verdict::Differs(vec![
                        Difference::Material { expected: "PETG".into(), loaded: "PLA".into() },
                        Difference::Color
                    ])
                ),
                (1, Verdict::Differs(vec![Difference::Color])),
            ]
        );
    }

    #[test]
    fn the_spool_of_a_gate_wins_over_what_happy_hare_knows_of_it() {
        let mut with_spool = gate("PLA", Some([0, 0, 0]));
        with_spool.spool_id = Some(7);
        let status = happy_hare(vec![with_spool], &[0]);
        let spools = spoolman(&[test_spool(7, "PETG", 5.0, [0xF7, 0x67, 0x07])], None);

        let found = verdicts(&file(&[20.0], &["PETG"], &["#F76707"]), &status, &spools);

        assert_eq!(found, [(0, Verdict::Differs(vec![Difference::Weight { needed: 20.0, left: 5.0, reserve: None }]))]);
    }

    #[test]
    fn spoolman_without_happy_hare_compares_tool_0_with_the_active_spool() {
        let spools = spoolman(&[test_spool(5, "PLA", 800.0, [0x26, 0xA6, 0x9A])], Some(5));
        let metadata = FileMetadata {
            referenced_tools: vec![0, 1],
            ..file(&[5.0, 5.0], &["PLA", "PLA"], &["#26A69A", "#000000"])
        };

        let status = PrinterStatus::default();
        let found = match_tools(&metadata, &status, &lanes(&status, &spools), &Setup::with_tool_commands(&[0, 1]));

        // A second tool without MMU is a toolchanger's macro nobody reports on.
        assert_eq!(found.iter().map(|found| &found.verdict).collect::<Vec<_>>(), [&Verdict::Fits, &Verdict::Unknown]);
    }

    #[test]
    fn with_a_tool_changer_each_tool_is_compared_with_the_spool_of_its_head() {
        use crate::sources::tool_macros::ToolSpool;
        use crate::sources::toolchanger::{ToolHead, ToolchangerStatus};
        let head = |number: usize, extruder: &str| ToolHead {
            name: format!("T{number}"),
            number: Some(number),
            extruder: Some(extruder.into()),
            active: number == 0,
        };
        let status = PrinterStatus {
            toolchanger: Some(ToolchangerStatus {
                status: "ready".into(),
                heads: vec![head(0, "extruder"), head(1, "extruder1")],
            }),
            tool_spools: vec![ToolSpool { tool: 1, macro_name: "T1".into(), spool_id: Some(6) }],
            ..PrinterStatus::default()
        };
        let spools = spoolman(
            &[test_spool(5, "PLA", 800.0, [0x26, 0xA6, 0x9A]), test_spool(6, "PETG", 800.0, [0, 0, 0])],
            Some(5),
        );
        let metadata = FileMetadata {
            referenced_tools: vec![0, 1],
            ..file(&[5.0, 5.0], &["PLA", "PLA"], &["#26A69A", "#000000"])
        };

        let found = match_tools(&metadata, &status, &lanes(&status, &spools), &Setup::with_tool_commands(&[0, 1]));

        assert_eq!(found[0].verdict, Verdict::Fits);
        assert!(matches!(&found[1].verdict, Verdict::Differs(differences) if differences.len() == 1));
    }

    #[test]
    fn without_happy_hare_and_spoolman_nothing_is_compared() {
        let toolchanger = Setup::with_tool_commands(&[0, 1, 2, 3]);
        let found = match_tools(&fixture("4 tools"), &PrinterStatus::default(), &[], &toolchanger);

        assert!(found.iter().all(|found| found.verdict == Verdict::Unknown));
    }

    #[test]
    fn without_happy_hare_a_tool_needs_its_command() {
        let verdicts_with = |klipper: &Setup| {
            match_tools(&fixture("4 tools"), &PrinterStatus::default(), &[], klipper)
                .into_iter()
                .map(|found| found.verdict)
                .collect::<Vec<_>>()
        };

        let missing = [Verdict::Unknown, Verdict::NoGate, Verdict::NoGate, Verdict::NoGate];
        assert_eq!(verdicts_with(&Setup::with_tool_commands(&[0])), missing);
        // Without Klipper's command list nothing is claimed.
        assert!(verdicts_with(&Setup::default()).iter().all(|verdict| *verdict == Verdict::Unknown));
    }

    #[test]
    fn tools_on_one_gate_add_up_their_weight() {
        let mut with_spool = gate("PLA", None);
        with_spool.spool_id = Some(7);
        let status = happy_hare(vec![with_spool, gate("PLA", None)], &[0, 0]);
        let spools = spoolman(&[test_spool(7, "PLA", 25.0, [0, 0, 0])], None);
        let metadata = FileMetadata { referenced_tools: vec![0, 1], ..file(&[15.0, 15.0], &["PLA", "PLA"], &[]) };

        let found = verdicts(&metadata, &status, &spools);

        // Each tool alone fits the 25 g left; both together from gate 0 need 30 g.
        let short = Verdict::Differs(vec![Difference::Weight { needed: 30.0, left: 25.0, reserve: None }]);
        assert_eq!(found, [(0, short.clone()), (1, short)]);
    }

    #[test]
    fn close_colours_count_as_equal_and_abs_plus_as_abs() {
        let spools = spoolman(&[test_spool(5, "ABS+", 500.0, [0xE3, 0x1F, 0x2B])], Some(5));

        let found = verdicts(&file(&[10.0], &["abs"], &["#FF2020"]), &PrinterStatus::default(), &spools);

        assert_eq!(found, [(0, Verdict::Fits)]);
    }

    #[test]
    fn a_multi_colour_spool_is_compared_by_its_first_colour() {
        let mut two_colours = test_spool(5, "PLA", 800.0, [0xFF, 0x00, 0x04]);
        two_colours.colors.push([0x26, 0xA6, 0x9A, 0xFF]);
        let spools = spoolman(&[two_colours], Some(5));
        let status = PrinterStatus::default();

        assert_eq!(verdicts(&file(&[10.0], &["PLA"], &["#FF1010"]), &status, &spools), [(0, Verdict::Fits)]);
        assert_eq!(
            verdicts(&file(&[10.0], &["PLA"], &["#26A69A"]), &status, &spools),
            [(0, Verdict::Differs(vec![Difference::Color]))]
        );
    }

    /// Happy Hare with EndlessSpool on: gate `n` holds spool `n + 1` and belongs to `groups[n]`.
    fn endless_spool(groups: &[u32], tool_to_gate: &[usize]) -> PrinterStatus {
        let gates = groups
            .iter()
            .enumerate()
            .map(|(index, group)| MmuGate {
                spool_id: Some(index as u32 + 1),
                endless_spool_group: Some(*group),
                ..gate("PLA", None)
            })
            .collect();
        let mut status = happy_hare(gates, tool_to_gate);
        status.mmu.as_mut().unwrap().endless_spool = true;
        status
    }

    #[test]
    fn endless_spool_adds_the_gates_of_the_group_that_no_tool_of_the_file_uses() {
        // T0 prints from gate 0 (14 g); gates 1 and 2 share its group, but T1 prints from gate 2.
        let status = endless_spool(&[0, 0, 0, 1], &[0, 2]);
        let spools = spoolman(
            &[
                test_spool(1, "PLA", 14.0, [0, 0, 0]),
                test_spool(2, "PLA", 230.0, [0, 0, 0]),
                test_spool(3, "PLA", 500.0, [0, 0, 0]),
                test_spool(4, "PLA", 900.0, [0, 0, 0]),
            ],
            None,
        );
        let metadata = FileMetadata { referenced_tools: vec![0, 1], ..file(&[18.0, 10.0], &["PLA", "PLA"], &[]) };

        assert_eq!(verdicts(&metadata, &status, &spools), [(0, Verdict::Fits), (1, Verdict::Fits)]);
        let lanes = lanes(&status, &spools);
        assert_eq!(endless_spool_reserve(0, &[0, 2], &status, &lanes), Some(230.0));
        assert_eq!(endless_spool_reserve(0, &[], &status, &lanes), Some(730.0));
        assert_eq!(endless_spool_reserve(3, &[], &status, &lanes), None);
    }

    #[test]
    fn without_endless_spool_only_the_own_spool_counts() {
        let mut status = endless_spool(&[0, 0], &[0]);
        status.mmu.as_mut().unwrap().endless_spool = false;
        let spools = spoolman(&[test_spool(1, "PLA", 14.0, [0, 0, 0]), test_spool(2, "PLA", 230.0, [0, 0, 0])], None);

        let found = verdicts(&file(&[18.0], &["PLA"], &[]), &status, &spools);

        assert_eq!(
            found,
            [(0, Verdict::Differs(vec![Difference::Weight { needed: 18.0, left: 14.0, reserve: None }]))]
        );
    }

    #[test]
    fn a_gate_knows_its_spool_first_then_what_happy_hare_has() {
        let mut named = gate("PETG", Some([1, 2, 3]));
        named.name = "Clear Blue".into();
        let status = happy_hare(vec![named.clone()], &[0]);

        let without_spool = gate_supply(&named, &lanes(&status, &spoolman(&[], None)));
        assert_eq!(without_spool.name.as_deref(), Some("Clear Blue"));
        assert_eq!((without_spool.colors.len(), without_spool.left), (1, None));

        named.spool_id = Some(7);
        let status = happy_hare(vec![named.clone()], &[0]);
        let spools = spoolman(&[test_spool(7, "PLA", 300.0, [9, 9, 9])], None);
        let with_spool = gate_supply(&named, &lanes(&status, &spools));
        assert_eq!((with_spool.material.as_deref(), with_spool.left), (Some("PLA"), Some(300.0)));
    }

    #[test]
    fn every_difference_empty_gate_and_missing_tool_is_a_verdict_with_its_supply() {
        let mut empty = gate("PLA", None);
        empty.availability = GateAvailability::Empty;
        let mut petg = gate("PETG", None);
        petg.spool_id = Some(6);
        let status = happy_hare(vec![petg, empty], &[0, 1]);
        let spools = spoolman(&[test_spool(6, "PETG", 20.0, [0; 3])], None);
        let metadata =
            FileMetadata { referenced_tools: vec![0, 1, 2], ..file(&[30.0, 5.0, 5.0], &["PLA", "PLA", "PLA"], &[]) };

        let lanes = lanes(&status, &spools);
        let found = match_tools(&metadata, &status, &lanes, &Setup::default());

        let verdicts: Vec<&Verdict> = found.iter().map(|found| &found.verdict).collect();
        let petg = Difference::Material { expected: "PLA".into(), loaded: "PETG".into() };
        let weight = Difference::Weight { needed: 30.0, left: 20.0, reserve: None };
        assert_eq!(verdicts, [&Verdict::Differs(vec![petg, weight]), &Verdict::GateEmpty, &Verdict::NoGate]);
        assert!(found.iter().all(|found| found.verdict.differs()));
        let supplies: Vec<Option<Supply>> = found.iter().map(|found| tool_supply(found, &status, &lanes)).collect();
        assert_eq!(
            supplies[0].as_ref().map(|supply| (supply.material.as_deref(), supply.empty)),
            Some((Some("PETG"), false))
        );
        assert!(supplies[1].as_ref().is_some_and(|supply| supply.empty));
        assert_eq!(supplies[2], None);
    }

    #[test]
    fn tools_that_share_a_gate_need_their_weights_together() {
        let mut pla = gate("PLA", None);
        pla.spool_id = Some(6);
        let status = happy_hare(vec![pla], &[0, 0]);
        let spools = spoolman(&[test_spool(6, "PLA", 500.0, [0; 3])], None);
        let metadata = FileMetadata { referenced_tools: vec![0, 1], ..file(&[30.0, 12.0], &["PLA", "PLA"], &[]) };

        let found = match_tools(&metadata, &status, &lanes(&status, &spools), &Setup::default());

        assert_eq!(found.iter().map(|found| found.needed).collect::<Vec<_>>(), [Some(42.0), Some(42.0)]);
        assert_eq!(tools_of_gate(&found, 0), [0, 1]);
    }

    #[test]
    fn eleven_tools_are_read_in_order() {
        let requests = tool_requests(&fixture("11 tools"));

        assert_eq!(requests.len(), 11);
        assert_eq!(requests[10].color, Some([0x80, 0x00, 0x80, 0xFF]));
        assert!(requests.iter().all(|request| request.material.as_deref() == Some("PLA")));
    }

    #[test]
    fn the_gates_that_fit_a_tool_are_marked_and_an_empty_one_never() {
        let mut empty_red = gate("PLA", Some([0xE0, 0x31, 0x31]));
        empty_red.availability = GateAvailability::Empty;
        let status = happy_hare(
            vec![
                gate("PLA", Some([0xE0, 0x31, 0x31])),
                gate("PETG", Some([0xE0, 0x31, 0x31])),
                gate("PLA", None),
                empty_red,
            ],
            &[0, 1, 2, 3],
        );
        let lanes = lanes(&status, &SpoolmanState::default());
        let matches = match_tools(&file(&[5.0], &["PLA"], &["#E8590C"]), &status, &lanes, &Setup::default());

        let fits = fitting_gates(&matches, 0, &status, &lanes);

        // Gate 1 has the wrong material, gate 2 no colour to compare, gate 3 is empty.
        assert_eq!(fits, [true, false, false, false]);
        assert!(fitting_gates(&matches, 0, &PrinterStatus::default(), &[]).is_empty());
        assert!(fitting_gates(&matches, 1, &status, &lanes).is_empty());
    }

    #[test]
    fn gates_of_one_group_share_what_endless_spool_can_continue_with() {
        // T0 prints from gate 0 (14 g), T1 from gate 2 (50 g); gate 1 (230 g) is free in their group.
        let status = endless_spool(&[0, 0, 0], &[0, 2]);
        let spools = spoolman(
            &[
                test_spool(1, "PLA", 14.0, [0, 0, 0]),
                test_spool(2, "PLA", 230.0, [0, 0, 0]),
                test_spool(3, "PLA", 50.0, [0, 0, 0]),
            ],
            None,
        );
        let both = |grams: &[f64]| FileMetadata { referenced_tools: vec![0, 1], ..file(grams, &["PLA", "PLA"], &[]) };

        // They lack 86 g and 50 g, together less than 230 g.
        assert_eq!(verdicts(&both(&[100.0, 100.0]), &status, &spools), [(0, Verdict::Fits), (1, Verdict::Fits)]);
        // They lack 86 g and 150 g, together more: each gets what the other does not lack, and both fall short.
        assert_eq!(
            verdicts(&both(&[100.0, 200.0]), &status, &spools),
            [
                (0, Verdict::Differs(vec![Difference::Weight { needed: 100.0, left: 14.0, reserve: Some(80.0) }])),
                (1, Verdict::Differs(vec![Difference::Weight { needed: 200.0, left: 50.0, reserve: Some(144.0) }])),
            ]
        );
    }

    #[test]
    fn a_gate_fits_a_tool_by_the_same_rule_as_its_verdict() {
        // T0 needs 30 g; gate 0 has 14 g and EndlessSpool 230 g in gate 1, gate 2 has 20 g in a group of its own.
        let status = endless_spool(&[0, 0, 1], &[0]);
        let spools = spoolman(
            &[
                test_spool(1, "PLA", 14.0, [0, 0, 0]),
                test_spool(2, "PLA", 230.0, [0, 0, 0]),
                test_spool(3, "PLA", 20.0, [0, 0, 0]),
            ],
            None,
        );
        let lanes = lanes(&status, &spools);
        let matches = match_tools(&file(&[30.0], &["PLA"], &[]), &status, &lanes, &Setup::default());

        assert_eq!((matches[0].verdict.clone(), matches[0].reserve), (Verdict::Fits, Some(230.0)));
        assert_eq!(fitting_gates(&matches, 0, &status, &lanes), [true, true, false]);
    }

    #[test]
    fn verdicts_that_do_not_fit_win_over_those_without_facts() {
        let differs = Verdict::Differs(vec![Difference::Color]);

        assert_eq!(Verdict::combine([&Verdict::Fits, &Verdict::Fits]), Verdict::Fits);
        assert_eq!(Verdict::combine([&Verdict::Fits, &Verdict::Unknown]), Verdict::Unknown);
        assert_eq!(Verdict::combine([&Verdict::Unknown, &Verdict::Fits]), Verdict::Unknown);
        assert_eq!(Verdict::combine([&Verdict::Unknown, &differs, &Verdict::NoGate]), differs);
        assert_eq!(Verdict::combine([&Verdict::Fits, &Verdict::GateEmpty]), Verdict::GateEmpty);
        assert_eq!(Verdict::combine([]), Verdict::Unknown);
    }
}
