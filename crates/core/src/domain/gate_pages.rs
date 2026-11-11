//! The pages of the Gates tab: five tiles at a time, one unit per page group, the bypass as the last tile of the unit
//! that has it, else of the last unit (`docs/multi-tool.md`, D6). With swappable heads each head has the pages of its
//! own gates only. Only the split is decided here; the tiles stay in the order of the gates, so a page is a run of rows.

use super::machine_tree::Lane;

/// A page of tiles: a run of gate rows, optionally followed by the bypass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatePage {
    /// Row of the first gate of the page in the list of gates.
    pub first_row: usize,
    /// Gates on the page; none on a page that only holds the bypass.
    pub count: usize,
    /// The bypass is the last tile of the page.
    pub bypass: bool,
    /// The unit to name above the tiles; `None` with one unit or none (then no caption line).
    pub unit: Option<String>,
    /// The head the unit sits on, when it sits on one.
    pub head: Option<String>,
    /// Gate indices of the first and the last gate on the page (the span of the caption); `None` without gates and for
    /// the lanes of AFC, which have no numbers.
    pub gates: Option<(usize, usize)>,
}

/// The pages of one view (all gates, or the gates of one head) and the one it opens on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GatePages {
    pub pages: Vec<GatePage>,
    /// The first page of the unit that holds the loaded gate; 0 without one.
    pub start: usize,
}

/// Which unit ends with the bypass tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BypassAt {
    /// The view has no bypass tile.
    Nowhere,
    /// The last unit of the view: the units do not say which one has the bypass.
    LastUnit,
    /// The unit with this position in the list of units (`mmu_machine.unit_<n>.has_bypass`).
    Unit(usize),
}

/// What the split needs of one tile.
struct Slot<'a> {
    row: usize,
    lane: &'a Lane,
}

impl Slot<'_> {
    fn unit(&self) -> Option<usize> {
        self.lane.unit
    }
}

/// The pages of `tiles` (the gates of Happy Hare or the lanes of AFC, in the order of the list), `per_page` tiles each.
///
/// - `head`: only the lanes fed to this head; `None`: all of them.
/// - `bypass`: the unit whose last tile is the bypass (kept after the last gate even on a page of its own). A view
///   without lanes of that unit (another head's) has no bypass.
/// - `loaded`: the row of the lane that is loaded or selected; the view opens on its unit.
pub fn gate_pages(
    tiles: &[Lane],
    head: Option<&str>,
    bypass: BypassAt,
    loaded: Option<usize>,
    per_page: usize,
) -> GatePages {
    let per_page = per_page.max(1);
    let slots: Vec<Slot> = tiles
        .iter()
        .enumerate()
        .map(|(row, lane)| Slot { row, lane })
        .filter(|slot| head.is_none_or(|name| slot.lane.head.as_deref() == Some(name)))
        .collect();

    // One group per unit: gates of the same unit follow each other.
    let mut groups: Vec<&[Slot]> = Vec::new();
    let mut from = 0;
    for end in 1..=slots.len() {
        if end == slots.len() || slots[end].unit() != slots[from].unit() {
            groups.push(&slots[from..end]);
            from = end;
        }
    }
    let alone = match bypass {
        BypassAt::LastUnit => true,
        BypassAt::Unit(_) => head.is_none(),
        BypassAt::Nowhere => false,
    };
    if groups.is_empty() && alone {
        return GatePages {
            pages: vec![GatePage { first_row: 0, count: 0, bypass: true, unit: None, head: None, gates: None }],
            start: 0,
        };
    }

    let mut result = GatePages::default();
    let last_group = groups.len().saturating_sub(1);
    for (position, group) in groups.iter().enumerate() {
        let lane = group[0].lane;
        let bypass = match bypass {
            BypassAt::Nowhere => false,
            BypassAt::LastUnit => position == last_group,
            BypassAt::Unit(unit) => group[0].unit() == Some(unit),
        };
        let tiles = group.len() + usize::from(bypass);
        let unit = lane.unit_name.clone();
        let on_head = unit.as_ref().and(lane.head.clone());
        if loaded.is_some_and(|loaded| group.iter().any(|slot| slot.row == loaded)) {
            result.start = result.pages.len();
        }
        for page in 0..tiles.div_ceil(per_page).max(1) {
            let skipped = page * per_page;
            let shown = &group[skipped.min(group.len())..(skipped + per_page).min(group.len())];
            result.pages.push(GatePage {
                first_row: shown.first().map_or(group[0].row + group.len(), |slot| slot.row),
                count: shown.len(),
                bypass: bypass && skipped + per_page >= tiles,
                unit: unit.clone(),
                head: on_head.clone(),
                gates: shown.first().zip(shown.last()).and_then(|(first, last)| first.lane.gate.zip(last.lane.gate)),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(gate: usize, unit: usize, name: Option<&str>, head: Option<&str>) -> Lane {
        Lane {
            gate: Some(gate),
            name: None,
            unit: Some(unit),
            unit_name: name.map(str::to_owned),
            head: head.map(str::to_owned),
            extruder: "extruder".into(),
            tool: None,
            spool_id: None,
            spool_by_tool: false,
            spool: None,
            empty: false,
            active: false,
        }
    }

    /// `counts` gates per unit, units named when there are several.
    fn lanes(counts: &[usize], heads: &[Option<&str>]) -> Vec<Lane> {
        let names = ["ercf", "tradrack", "third"];
        let mut gate = 0;
        let mut lanes = Vec::new();
        for (unit, count) in counts.iter().enumerate() {
            for _ in 0..*count {
                lanes.push(lane(
                    gate,
                    unit,
                    (counts.len() > 1).then_some(names[unit]),
                    heads.get(unit).copied().flatten(),
                ));
                gate += 1;
            }
        }
        lanes
    }

    fn shape(pages: &GatePages) -> Vec<(usize, usize, bool)> {
        pages.pages.iter().map(|page| (page.first_row, page.count, page.bypass)).collect()
    }

    #[test]
    fn one_unit_pages_by_five_and_the_bypass_follows_the_last_gate() {
        let lanes = lanes(&[8], &[]);
        let pages = gate_pages(&lanes, None, BypassAt::LastUnit, None, 5);
        assert_eq!(shape(&pages), [(0, 5, false), (5, 3, true)]);
        assert!(pages.pages.iter().all(|page| page.unit.is_none() && page.head.is_none()));
        // Exactly five gates: the bypass has a page of its own.
        let lanes = self::lanes(&[5], &[]);
        assert_eq!(shape(&gate_pages(&lanes, None, BypassAt::LastUnit, None, 5)), [(0, 5, false), (5, 0, true)]);
    }

    #[test]
    fn without_gates_the_bypass_is_the_only_tile() {
        assert_eq!(shape(&gate_pages(&[], None, BypassAt::LastUnit, None, 5)), [(0, 0, true)]);
        assert!(gate_pages(&[], None, BypassAt::Nowhere, None, 5).pages.is_empty());
    }

    #[test]
    fn a_page_never_mixes_units_and_the_last_unit_holds_the_bypass() {
        let lanes = lanes(&[4, 4], &[]);
        let pages = gate_pages(&lanes, None, BypassAt::LastUnit, None, 5);
        assert_eq!(shape(&pages), [(0, 4, false), (4, 4, true)]);
        assert_eq!(pages.pages[0].unit.as_deref(), Some("ercf"));
        assert_eq!(pages.pages[1].unit.as_deref(), Some("tradrack"));
        assert_eq!(pages.pages[1].gates, Some((4, 7)));
    }

    #[test]
    fn the_bypass_follows_the_unit_that_has_it() {
        let lanes = lanes(&[4, 4, 4], &[]);
        let pages = gate_pages(&lanes, None, BypassAt::Unit(1), None, 5);
        assert_eq!(shape(&pages), [(0, 4, false), (4, 4, true), (8, 4, false)]);
    }

    #[test]
    fn a_head_without_the_unit_with_the_bypass_has_none() {
        let lanes = lanes(&[4, 4], &[Some("T0"), Some("T1")]);
        let at = BypassAt::Unit(0);
        assert_eq!(shape(&gate_pages(&lanes, Some("T0"), at, None, 5)), [(0, 4, true)]);
        assert_eq!(shape(&gate_pages(&lanes, Some("T1"), at, None, 5)), [(4, 4, false)]);
    }

    #[test]
    fn a_unit_with_nine_gates_takes_two_pages() {
        let lanes = lanes(&[9, 3], &[]);
        let pages = gate_pages(&lanes, None, BypassAt::Nowhere, None, 5);
        assert_eq!(shape(&pages), [(0, 5, false), (5, 4, false), (9, 3, false)]);
        assert_eq!(pages.pages[1].gates, Some((5, 8)));
        assert_eq!(pages.pages[1].unit.as_deref(), Some("ercf"));
    }

    #[test]
    fn the_view_opens_on_the_first_page_of_the_unit_of_the_loaded_gate() {
        let lanes = lanes(&[9, 3], &[]);
        let start = |loaded| gate_pages(&lanes, None, BypassAt::LastUnit, loaded, 5).start;
        assert_eq!(start(None), 0);
        assert_eq!(start(Some(7)), 0, "the second page of the first unit opens on its first page");
        assert_eq!(start(Some(10)), 2);
        assert_eq!(start(Some(99)), 0);
    }

    #[test]
    fn a_head_has_the_pages_of_its_own_gates_with_the_global_rows() {
        let lanes = lanes(&[4, 4], &[Some("T0"), Some("T1")]);
        let pages = gate_pages(&lanes, Some("T1"), BypassAt::LastUnit, Some(5), 5);
        assert_eq!(shape(&pages), [(4, 4, true)]);
        assert_eq!(pages.pages[0].head.as_deref(), Some("T1"));
        assert_eq!(pages.pages[0].unit.as_deref(), Some("tradrack"));
        assert_eq!(pages.start, 0);
        let first = gate_pages(&lanes, Some("T0"), BypassAt::Nowhere, None, 5);
        assert_eq!(shape(&first), [(0, 4, false)]);
    }

    #[test]
    fn a_unit_without_a_head_names_no_head() {
        let lanes = lanes(&[4, 4], &[None, Some("T1")]);
        let pages = gate_pages(&lanes, None, BypassAt::Nowhere, None, 5);
        assert_eq!(pages.pages[0].head, None);
        assert_eq!(pages.pages[1].head.as_deref(), Some("T1"));
    }

    #[test]
    fn a_head_with_a_single_unit_of_the_machine_has_no_caption() {
        let lanes = lanes(&[4], &[Some("T1")]);
        let pages = gate_pages(&lanes, Some("T1"), BypassAt::LastUnit, None, 5);
        assert_eq!((pages.pages[0].unit.clone(), pages.pages[0].head.clone()), (None, None));
    }
}
