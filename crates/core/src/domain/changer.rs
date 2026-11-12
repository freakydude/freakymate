//! What the Changer page shows of AFC (`docs/multi-tool.md`, D7): its tool map on Tools and its service on Service.
//! Happy Hare keeps its own tabs; the tool changer's state card comes from `toolchanger` (see [`super::heads`]).

use crate::sources::afc::{HubReturn, ToolLane};
use crate::sources::PrinterStatus;

/// What the Service tab offers for AFC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AfcService {
    /// `current_state` of AFC (`Idle`, `Error`, `ToolSwap`, …; `AFC.py` L57-68).
    pub state: String,
    /// AFC is in its error state (`error_state`); "Clear error" sends `RESET_FAILURE`, which clears it and sets AFC
    /// idle without resuming a print (`AFC_error.py` L172-195).
    pub error: bool,
    /// The lanes "Back to hub" (`AFC_LANE_RESET`) can pull back, each with why not right now.
    pub hub_returns: Vec<HubReturn>,
}

/// The tools of AFC with their lanes for the Tools tab (`SET_MAP`, `RESET_AFC_MAPPING`); empty without AFC or while
/// Happy Hare has the say, whose own tool map the tab shows then (the same rule as [`Command::ResetToolMap`]).
///
/// [`Command::ResetToolMap`]: crate::Command::ResetToolMap
pub fn afc_tool_map(status: &PrinterStatus) -> Vec<ToolLane> {
    if status.happy_hare().is_some() {
        return Vec::new();
    }
    status.afc.as_ref().map(|afc| afc.tool_lanes()).unwrap_or_default()
}

/// The service of AFC; `None` without AFC.
pub fn afc_service(status: &PrinterStatus) -> Option<AfcService> {
    let afc = status.afc.as_ref()?;
    Some(AfcService { state: afc.state.clone(), error: afc.error, hub_returns: afc.hub_returns() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::afc::{AfcHub, AfcLane, AfcStatus, HubReturnBlock};
    use crate::sources::happy_hare::MmuStatus;

    fn lane(name: &str, tool: Option<usize>, hub: Option<&str>, load: Option<bool>) -> AfcLane {
        AfcLane {
            name: name.into(),
            unit: "Turtle_1".into(),
            tool,
            hub: hub.map(str::to_owned),
            load,
            ..AfcLane::default()
        }
    }

    /// A Box Turtle with a hub switch: lane1 and lane2 hold filament, lane3 is empty, lane4 has no load pin; the
    /// head of AFC's tool changer `e0` loads straight (`hub: direct`).
    fn afc(current_load: Option<&str>, hub_switch: Option<bool>) -> PrinterStatus {
        PrinterStatus {
            afc: Some(AfcStatus {
                state: "Error".into(),
                error: true,
                current_load: current_load.map(str::to_owned),
                lanes: vec![
                    lane("lane2", Some(1), Some("Turtle_1"), Some(true)),
                    lane("lane1", Some(0), Some("Turtle_1"), Some(true)),
                    lane("lane3", Some(2), Some("Turtle_1"), Some(false)),
                    lane("lane4", None, Some("Turtle_1"), None),
                    lane("e0", Some(4), None, None),
                ],
                hubs: vec![AfcHub { name: "Turtle_1".into(), switch: hub_switch }],
                ..AfcStatus::default()
            }),
            ..PrinterStatus::default()
        }
    }

    fn returns(status: &PrinterStatus) -> Vec<(String, Option<HubReturnBlock>)> {
        let service = afc_service(status).unwrap();
        service.hub_returns.into_iter().map(|entry| (entry.lane, entry.blocked)).collect()
    }

    #[test]
    fn the_tool_map_lists_each_mapped_lane_by_tool() {
        let map = afc_tool_map(&afc(None, Some(true)));

        let pairs: Vec<(usize, &str)> = map.iter().map(|entry| (entry.tool, entry.lane.as_str())).collect();
        assert_eq!(pairs, [(0, "lane1"), (1, "lane2"), (2, "lane3"), (4, "e0")]);
    }

    #[test]
    fn happy_hare_in_charge_or_no_afc_has_no_afc_tool_map() {
        let mmu = MmuStatus { version: "4.1.0".into(), enabled: true, ..MmuStatus::default() };
        let both = PrinterStatus { mmu: Some(mmu), ..afc(None, Some(true)) };
        assert!(afc_tool_map(&both).is_empty());
        assert!(afc_tool_map(&PrinterStatus::default()).is_empty());
        assert_eq!(afc_service(&PrinterStatus::default()), None);
    }

    #[test]
    fn back_to_hub_offers_lanes_with_filament_at_their_hub_and_says_why_not_now() {
        // The hub sees filament and the toolhead is empty: every lane with filament can go back.
        let free = returns(&afc(None, Some(true)));
        assert_eq!(free, [("lane2".into(), None), ("lane1".into(), None), ("lane4".into(), None)]);

        let tool_loaded = returns(&afc(Some("lane2"), Some(true)));
        assert!(tool_loaded.iter().all(|(_, blocked)| *blocked == Some(HubReturnBlock::ToolLoaded("lane2".into()))));

        let hub_clear = returns(&afc(None, Some(false)));
        assert!(hub_clear.iter().all(|(_, blocked)| *blocked == Some(HubReturnBlock::HubClear)));

        // A virtual hub has no switch of its own; AFC decides from the lanes' load switches.
        assert!(returns(&afc(None, None)).iter().all(|(_, blocked)| blocked.is_none()));
    }

    #[test]
    fn the_service_carries_the_error_state() {
        let service = afc_service(&afc(None, Some(true))).unwrap();
        assert!(service.error);
        assert_eq!(service.state, "Error");
    }
}
