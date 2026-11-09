//! What a print asks of each tool and how that compares with the filament where the tool prints from. Only facts that
//! both sides have are compared, and everything here is a hint: nothing forbids anything.
//!
//! Where the filament of a tool comes from depends on what the printer has: with Happy Hare the gate of the tool
//! (`ttg_map`), its spool from Spoolman first, otherwise what Happy Hare knows of the gate; without Happy Hare only
//! tool 0 and the active spool of Spoolman. Without either there is nothing to compare. Tools that share a gate share
//! its spool, so their planned weights add up. Without Happy Hare a tool beyond 0 needs a command `T<n>` in Klipper's
//! command list (a toolchanger, a blackbox here); a file that calls a missing one stops at an unknown command.

use super::spools::Lane;
use crate::color::{is_clear, parse_hex, rgb, Rgba};
use crate::sources::happy_hare::{GateAvailability, MmuGate};
use crate::sources::klipper::Setup;
use crate::sources::moonraker_server::files::FileMetadata;
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
    /// Planned grams against what the spool has left; tools that share the spool add up.
    Weight {
        needed: f64,
        left: f64,
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
}

/// A tool of the file, the gate it prints from (with Happy Hare) and how it compares.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolMatch {
    pub request: ToolRequest,
    pub gate: Option<usize>,
    pub verdict: Verdict,
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
    let requests = tool_requests(metadata);
    let needs = needs_per_gate(&requests, status);
    requests
        .into_iter()
        .map(|request| {
            let shared = gate_of(request.tool, status).and_then(|gate| needs.get(&gate).copied());
            let needed = shared.or(request.grams);
            match_tool(request, needed, status, lanes, klipper)
        })
        .collect()
}

/// The planned weight of all tools that print from the same gate, per gate: they all take from its spool.
fn needs_per_gate(requests: &[ToolRequest], status: &PrinterStatus) -> HashMap<usize, f64> {
    let mut needs = HashMap::new();
    for request in requests {
        if let (Some(gate), Some(grams)) = (gate_of(request.tool, status), request.grams) {
            *needs.entry(gate).or_insert(0.0) += grams;
        }
    }
    needs
}

/// The gate Happy Hare maps the tool to; `None` without Happy Hare or for a tool it does not have.
fn gate_of(tool: usize, status: &PrinterStatus) -> Option<usize> {
    status.happy_hare().filter(|mmu| !mmu.gates.is_empty()).and_then(|mmu| mmu.tool_to_gate.get(tool).copied())
}

/// Per gate index of Happy Hare: whether the gate is a good choice for the tool. Stricter than the verdict of a tool:
/// the gate must know the material and colour the file asks for, and they must fit. An empty gate never fits. Empty
/// without Happy Hare.
pub fn fitting_gates(request: &ToolRequest, status: &PrinterStatus, lanes: &[Lane]) -> Vec<bool> {
    let Some(mmu) = status.happy_hare() else { return Vec::new() };
    let mut fits = vec![false; mmu.gates.iter().map(|gate| gate.index + 1).max().unwrap_or(0)];
    for gate in mmu.gates.iter().filter(|gate| gate.availability != GateAvailability::Empty) {
        let lane = lanes.iter().find(|lane| lane.gate == Some(gate.index));
        let supply = supply(lane, Some(gate));
        let knows_material = request.material.is_none() || supply.material.is_some();
        let knows_color = !request.color.is_some_and(|color| !is_clear(color)) || !supply.colors.is_empty();
        fits[gate.index] = knows_material && knows_color && compare(request, request.grams, &supply) == Verdict::Fits;
    }
    fits
}

/// One tool against where it prints from; `needed` is the planned weight that comes from that spool.
fn match_tool(
    request: ToolRequest,
    needed: Option<f64>,
    status: &PrinterStatus,
    lanes: &[Lane],
    klipper: &Setup,
) -> ToolMatch {
    let Some(mmu) = status.happy_hare().filter(|mmu| !mmu.gates.is_empty()) else {
        if request.tool > 0 && klipper.has_tool_command(request.tool) == Some(false) {
            return ToolMatch { request, gate: None, verdict: Verdict::NoGate };
        }
        let lane = lanes.first().filter(|_| request.tool == 0);
        let verdict = compare(&request, needed, &supply(lane, None));
        return ToolMatch { request, gate: None, verdict };
    };
    let Some(gate) = mmu.tool_to_gate.get(request.tool).copied() else {
        return ToolMatch { request, gate: None, verdict: Verdict::NoGate };
    };
    let gate_state = mmu.gates.iter().find(|candidate| candidate.index == gate);
    let verdict = if gate_state.is_some_and(|state| state.availability == GateAvailability::Empty) {
        Verdict::GateEmpty
    } else {
        let lane = lanes.iter().find(|lane| lane.gate == Some(gate));
        compare(&request, needed, &supply(lane, gate_state))
    };
    ToolMatch { request, gate: Some(gate), verdict }
}

/// What sits where a tool prints from.
#[derive(Debug, Default)]
struct Supply {
    material: Option<String>,
    colors: Vec<Rgba>,
    left: Option<f64>,
}

/// The spool of the lane (Spoolman) first, otherwise what Happy Hare knows of the gate (no weight there).
fn supply(lane: Option<&Lane>, gate: Option<&MmuGate>) -> Supply {
    if let Some(spool) = lane.and_then(|lane| lane.spool.as_ref()) {
        return Supply {
            material: Some(spool.material.clone()).filter(|material| !material.is_empty()),
            colors: spool.colors.clone(),
            left: spool.remaining_weight,
        };
    }
    gate.map_or_else(Supply::default, |gate| Supply {
        material: Some(gate.material.clone()).filter(|material| !material.is_empty()),
        colors: gate.color.into_iter().collect(),
        left: None,
    })
}

fn compare(request: &ToolRequest, needed: Option<f64>, supply: &Supply) -> Verdict {
    let mut compared = false;
    let mut differences = Vec::new();
    if let (Some(expected), Some(loaded)) = (&request.material, &supply.material) {
        compared = true;
        if !same_material(expected, loaded) {
            differences.push(Difference::Material { expected: expected.clone(), loaded: loaded.clone() });
        }
    }
    if let Some(wanted) = request.color.filter(|color| !is_clear(*color)) {
        // A multi-colour spool fits when one of its colours is close; a clear one is nothing to compare with.
        let nearest = supply
            .colors
            .iter()
            .filter(|loaded| !is_clear(**loaded))
            .map(|loaded| color_distance(rgb(wanted), rgb(*loaded)))
            .min_by(f64::total_cmp);
        if let Some(distance) = nearest {
            compared = true;
            if distance > COLOR_DISTANCE {
                differences.push(Difference::Color);
            }
        }
    }
    if let (Some(needed), Some(left)) = (needed, supply.left) {
        compared = true;
        if needed > left {
            differences.push(Difference::Weight { needed, left });
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

/// A hint before the print: what does not fit for a tool. Only hints, nothing here forbids anything.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintHint {
    pub tool: usize,
    pub kind: PrintHintKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PrintHintKind {
    /// The filament differs from what the file expects.
    Differs(Difference),
    GateEmpty,
    NoGate,
}

/// The hints for the start dialog: every difference, empty gate and missing tool, in tool order.
pub fn print_hints(metadata: &FileMetadata, status: &PrinterStatus, lanes: &[Lane], klipper: &Setup) -> Vec<PrintHint> {
    match_tools(metadata, status, lanes, klipper)
        .into_iter()
        .flat_map(|found| hints_of(found.request.tool, found.verdict))
        .collect()
}

fn hints_of(tool: usize, verdict: Verdict) -> Vec<PrintHint> {
    let hint = |kind| PrintHint { tool, kind };
    match verdict {
        Verdict::Fits | Verdict::Unknown => Vec::new(),
        Verdict::GateEmpty => vec![hint(PrintHintKind::GateEmpty)],
        Verdict::NoGate => vec![hint(PrintHintKind::NoGate)],
        Verdict::Differs(differences) => {
            differences.into_iter().map(|difference| hint(PrintHintKind::Differs(difference))).collect()
        }
    }
}

/// What can make a tool fit what the file asks for. Only suggestions, in the order to try.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remedy {
    /// Other gates hold what the file asks for: map the tool to one of them.
    UseGate(Vec<usize>),
    /// Load fitting filament into the tool's gate, or correct what the gate records (spool, material, colour).
    LoadGate(usize),
    /// Without MMU: load fitting filament, or choose the spool in use that holds it.
    LoadSpool,
    /// The printer has no such tool: slice the file for the printer's tools.
    Reslice,
}

/// The remedies for a tool that does not fit; `fitting` is [`fitting_gates`] of its request. Empty when it fits or
/// nothing can be compared.
pub fn remedies(found: &ToolMatch, fitting: &[bool]) -> Vec<Remedy> {
    match found.verdict {
        Verdict::Fits | Verdict::Unknown => Vec::new(),
        Verdict::NoGate => vec![Remedy::Reslice],
        Verdict::GateEmpty | Verdict::Differs(_) => {
            let others: Vec<usize> =
                (0..fitting.len()).filter(|gate| fitting[*gate] && found.gate != Some(*gate)).collect();
            let load = found.gate.map_or(Remedy::LoadSpool, Remedy::LoadGate);
            if others.is_empty() {
                vec![load]
            } else {
                vec![Remedy::UseGate(others), load]
            }
        }
    }
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

        assert_eq!(found, [(0, Verdict::Differs(vec![Difference::Weight { needed: 20.0, left: 5.0 }]))]);
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
    fn without_happy_hare_and_spoolman_nothing_is_compared() {
        let toolchanger = Setup::with_tool_commands(&[0, 1, 2, 3]);
        let found = match_tools(&fixture("4 tools"), &PrinterStatus::default(), &[], &toolchanger);

        assert!(found.iter().all(|found| found.verdict == Verdict::Unknown));
        assert!(print_hints(&fixture("4 tools"), &PrinterStatus::default(), &[], &toolchanger).is_empty());
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
        let short = Verdict::Differs(vec![Difference::Weight { needed: 30.0, left: 25.0 }]);
        assert_eq!(found, [(0, short.clone()), (1, short)]);
    }

    #[test]
    fn close_colours_count_as_equal_and_abs_plus_as_abs() {
        let spools = spoolman(&[test_spool(5, "ABS+", 500.0, [0xE3, 0x1F, 0x2B])], Some(5));

        let found = verdicts(&file(&[10.0], &["abs"], &["#FF2020"]), &PrinterStatus::default(), &spools);

        assert_eq!(found, [(0, Verdict::Fits)]);
    }

    #[test]
    fn a_multi_colour_spool_fits_when_one_of_its_colours_is_close() {
        let mut two_colours = test_spool(5, "PLA", 800.0, [0xFF, 0x00, 0x04]);
        two_colours.colors.push([0x26, 0xA6, 0x9A, 0xFF]);
        let spools = spoolman(&[two_colours], Some(5));
        let status = PrinterStatus::default();

        assert_eq!(verdicts(&file(&[10.0], &["PLA"], &["#26A69A"]), &status, &spools), [(0, Verdict::Fits)]);
        assert_eq!(
            verdicts(&file(&[10.0], &["PLA"], &["#0000FF"]), &status, &spools),
            [(0, Verdict::Differs(vec![Difference::Color]))]
        );
    }

    #[test]
    fn the_start_hints_name_every_difference_empty_gate_and_missing_tool() {
        let mut empty = gate("PLA", None);
        empty.availability = GateAvailability::Empty;
        let mut petg = gate("PETG", None);
        petg.spool_id = Some(6);
        let status = happy_hare(vec![petg, empty], &[0, 1]);
        let spools = spoolman(&[test_spool(6, "PETG", 20.0, [0; 3])], None);
        let metadata =
            FileMetadata { referenced_tools: vec![0, 1, 2], ..file(&[30.0, 5.0, 5.0], &["PLA", "PLA", "PLA"], &[]) };

        let hints = print_hints(&metadata, &status, &lanes(&status, &spools), &Setup::default());

        assert_eq!(
            hints,
            [
                PrintHint {
                    tool: 0,
                    kind: PrintHintKind::Differs(Difference::Material {
                        expected: "PLA".into(),
                        loaded: "PETG".into()
                    })
                },
                PrintHint { tool: 0, kind: PrintHintKind::Differs(Difference::Weight { needed: 30.0, left: 20.0 }) },
                PrintHint { tool: 1, kind: PrintHintKind::GateEmpty },
                PrintHint { tool: 2, kind: PrintHintKind::NoGate },
            ]
        );
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
        let request = tool_requests(&file(&[5.0], &["PLA"], &["#E8590C"])).remove(0);

        let fits = fitting_gates(&request, &status, &lanes(&status, &SpoolmanState::default()));

        // Gate 1 has the wrong material, gate 2 no colour to compare, gate 3 is empty.
        assert_eq!(fits, [true, false, false, false]);
        assert!(fitting_gates(&request, &PrinterStatus::default(), &[]).is_empty());
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

    #[test]
    fn remedies_name_other_fitting_gates_before_loading_the_own() {
        let found = |gate, verdict| ToolMatch { request: ToolRequest::default(), gate, verdict };
        let differs = Verdict::Differs(vec![Difference::Material { expected: "PLA".into(), loaded: "ASA".into() }]);

        assert!(remedies(&found(Some(0), Verdict::Fits), &[true]).is_empty());
        assert!(remedies(&found(Some(0), Verdict::Unknown), &[false]).is_empty());
        assert_eq!(remedies(&found(Some(0), differs.clone()), &[false]), [Remedy::LoadGate(0)]);
        assert_eq!(
            remedies(&found(Some(1), Verdict::GateEmpty), &[true, false, true]),
            [Remedy::UseGate(vec![0, 2]), Remedy::LoadGate(1)]
        );
        assert_eq!(remedies(&found(None, differs), &[]), [Remedy::LoadSpool]);
        assert_eq!(remedies(&found(None, Verdict::NoGate), &[true]), [Remedy::Reslice]);
    }
}
