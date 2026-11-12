//! The head level of the Filament page: the heads of a tool changer with what the page needs of each, how a head that
//! is not mounted is brought to the carriage, and the deviations of the changer that are worth a line
//! (`docs/multi-tool.md`, D1). Only a printer with swappable heads has rows.

use super::machine_tree::{Lane, MachineTree};
use crate::sources::toolchanger::ToolchangerStatus;
use crate::sources::PrinterStatus;

/// How a head that is not mounted gets to the carriage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickUp {
    /// The macro `T<n>` of the head, which has one lane.
    Macro(usize),
    /// `SELECT_TOOL` with the name of the head: the head has several `T<n>` (a unit of its own).
    Head,
}

/// What names a head on every screen: `T<n>`, which is unambiguous (a tool number belongs to one head: viesturz
/// `tool.py` reads it as a scalar, a duplicate is a config error); the section name for a head without a number.
pub fn head_label(name: &str, number: Option<usize>) -> String {
    number.map_or_else(|| name.to_owned(), |number| format!("T{number}"))
}

/// The section name as a second line under [`head_label`], only where it says something else.
pub fn head_subtitle(name: &str, number: Option<usize>) -> Option<String> {
    number.filter(|number| format!("T{number}") != name).map(|_| name.to_owned())
}

/// One head with what the page shows of it.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadRow {
    /// The name of the section (`T1`, `left`), which commands take.
    pub name: String,
    /// The label to show: [`head_label`].
    pub label: String,
    /// The section name when it differs from the label: [`head_subtitle`].
    pub subtitle: Option<String>,
    /// The `T<n>` the head answers to.
    pub number: Option<usize>,
    /// The Klipper extruder of its nozzle; `None` for a head without one.
    pub extruder: Option<String>,
    /// The head is the one on the carriage (as far as the changer selected it).
    pub mounted: bool,
    /// The detection pins find this head on the carriage.
    pub detected: bool,
    /// Lanes on the nozzle: more than one is a unit of its own (Gates tab).
    pub lane_count: usize,
    /// The lane whose spool the head's card shows: the one in use, else the first.
    pub lane: Option<Lane>,
    /// `None` where no bare pick-up exists (AFC selects through a lane) or the head has no number.
    pub pick_up: Option<PickUp>,
}

/// The heads of the tree in order; empty without swappable heads. `lanes` are the lanes of the tree with their spools.
pub fn head_rows(tree: &MachineTree, status: &PrinterStatus, lanes: &[Lane]) -> Vec<HeadRow> {
    if !tree.has_swappable_heads() {
        return Vec::new();
    }
    let changer = status.toolchanger.as_ref();
    tree.carriages
        .iter()
        .flat_map(|carriage| &carriage.heads)
        .filter_map(|head| {
            let name = head.name.as_ref()?;
            let nozzle = head.nozzles.first();
            let own: Vec<&Lane> = lanes.iter().filter(|lane| lane.head.as_ref() == Some(name)).collect();
            let lane = own.iter().find(|lane| lane.active).or(own.first()).map(|lane| (*lane).clone());
            let number = changer
                .and_then(|changer| changer.heads.iter().find(|tool| &tool.name == name))
                .and_then(|tool| tool.number)
                .or_else(|| lane.as_ref().and_then(|lane| lane.tool));
            let lane_count = nozzle.map_or(0, |nozzle| nozzle.lanes.len());
            let through_lane = own.iter().any(|lane| lane.name.is_some());
            let pick_up = match (lane_count, number) {
                _ if through_lane => None,
                (0 | 1, Some(number)) => Some(PickUp::Macro(number)),
                (2.., _) => Some(PickUp::Head),
                _ => None,
            };
            Some(HeadRow {
                name: name.clone(),
                label: head_label(name, number),
                subtitle: head_subtitle(name, number),
                number,
                extruder: nozzle.map(|nozzle| nozzle.extruder.clone()),
                mounted: head.active,
                detected: number.is_some() && changer.is_some_and(|changer| changer.detected == number),
                lane_count,
                lane,
                pick_up,
            })
        })
        .collect()
}

/// A deviation of the tool changer, one line under the tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangerNote {
    /// Not initialised yet; the button initialises.
    Uninitialized,
    /// In error (the text is only in the console); the button recovers.
    Error,
    /// The changer selected one head, the detection finds another (labels, `T<n>`).
    Mismatch { selected: String, detected: String },
}

/// The deviation of the changer, if it has one; nothing while it works as it should (also while it initialises or
/// changes, or while it is uninitialised but initialises itself at the next selection).
pub fn changer_note(changer: &ToolchangerStatus) -> Option<ChangerNote> {
    match changer.status.as_str() {
        // With `initialize_on: first-use` the next selection initialises by itself, so nothing would fail.
        "uninitialized" if !changer.initializes_on_first_use() => Some(ChangerNote::Uninitialized),
        "error" => Some(ChangerNote::Error),
        // While it changes heads the two differ for a moment, which is not a deviation.
        "ready" => match (changer.selected, changer.detected) {
            (Some(selected), Some(detected)) if selected != detected => {
                Some(ChangerNote::Mismatch { selected: format!("T{selected}"), detected: format!("T{detected}") })
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::toolchanger::ToolHead;

    fn head(name: &str, number: usize, active: bool) -> ToolHead {
        ToolHead {
            name: name.into(),
            number: Some(number),
            extruder: Some(format!("extruder{number}")),
            fan: None,
            active,
        }
    }

    fn changer(status: &str, selected: Option<usize>, detected: Option<usize>) -> ToolchangerStatus {
        ToolchangerStatus {
            status: status.into(),
            heads: vec![head("T0", 0, selected == Some(0)), head("T1", 1, selected == Some(1))],
            selected,
            detected,
            initialize_on: "home".into(),
        }
    }

    fn rows(changer: ToolchangerStatus) -> Vec<HeadRow> {
        let status = PrinterStatus { toolchanger: Some(changer), ..PrinterStatus::default() };
        let tree = MachineTree::build(&status);
        let lanes = tree.lanes(&Default::default());
        head_rows(&tree, &status, &lanes)
    }

    #[test]
    fn a_head_with_one_lane_is_picked_up_by_its_macro_and_the_mounted_one_by_nothing_special() {
        let rows = rows(changer("ready", Some(0), None));
        assert_eq!(rows.len(), 2);
        assert!(rows[0].mounted && !rows[1].mounted);
        assert_eq!(rows[1].pick_up, Some(PickUp::Macro(1)));
        assert_eq!(rows[1].extruder.as_deref(), Some("extruder1"));
        assert_eq!(rows[1].lane_count, 1);
    }

    #[test]
    fn a_head_is_labelled_by_its_number_and_named_below_only_where_that_differs() {
        assert_eq!((head_label("T1", Some(1)), head_subtitle("T1", Some(1))), ("T1".into(), None));
        assert_eq!((head_label("left", Some(1)), head_subtitle("left", Some(1))), ("T1".into(), Some("left".into())));
        assert_eq!((head_label("left", None), head_subtitle("left", None)), ("left".into(), None));
        // A name that only looks like another number is still shown.
        assert_eq!(head_subtitle("T2", Some(1)), Some("T2".into()));
    }

    #[test]
    fn a_head_is_detected_only_with_the_detection_pointing_at_it() {
        let rows = rows(changer("ready", Some(0), Some(1)));
        assert!(!rows[0].detected && rows[1].detected);
        let without = self::rows(changer("ready", Some(0), None));
        assert!(without.iter().all(|row| !row.detected));
    }

    #[test]
    fn a_printer_without_swappable_heads_has_no_rows() {
        let status = PrinterStatus::default();
        let tree = MachineTree::build(&status);
        assert!(head_rows(&tree, &status, &tree.lanes(&Default::default())).is_empty());
    }

    #[test]
    fn the_changer_names_its_deviations_but_not_its_work() {
        assert_eq!(changer_note(&changer("uninitialized", None, None)), Some(ChangerNote::Uninitialized));
        assert_eq!(changer_note(&changer("error", Some(0), None)), Some(ChangerNote::Error));
        assert_eq!(
            changer_note(&changer("ready", Some(0), Some(1))),
            Some(ChangerNote::Mismatch { selected: "T0".into(), detected: "T1".into() })
        );
        assert_eq!(changer_note(&changer("ready", Some(1), Some(1))), None);
        assert_eq!(changer_note(&changer("ready", Some(1), None)), None);
        assert_eq!(changer_note(&changer("changing", Some(0), Some(1))), None);
        assert_eq!(changer_note(&changer("initializing", None, None)), None);
    }

    #[test]
    fn an_uninitialized_changer_is_no_deviation_where_the_next_selection_initializes_it() {
        for (initialize_on, expected) in [
            ("first-use", None),
            ("", None),
            ("home", Some(ChangerNote::Uninitialized)),
            ("manual", Some(ChangerNote::Uninitialized)),
        ] {
            let changer =
                ToolchangerStatus { initialize_on: initialize_on.into(), ..changer("uninitialized", None, None) };
            assert_eq!(changer_note(&changer), expected, "initialize_on {initialize_on:?}");
        }
    }
}
