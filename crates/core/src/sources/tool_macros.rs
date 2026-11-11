//! The tool macros `T0` … `Tn` and the variables they may declare by the convention of Fluidd and Mainsail. Like every
//! macro they are black boxes; only a variable declared under exactly this name is read (see `docs/printer-setup.md`,
//! "Spools of tool heads").
//!
//! `spool_id` holds the Spoolman spool of a tool: both clients assign it with `SET_GCODE_VARIABLE` and, if the printer
//! has `[save_variables]`, keep it with `SAVE_VARIABLE` under `t<n>__spool_id`.

use super::klipper::Setup;
use super::subscription::Subscription;
use super::ObjectStore;

const SPOOL_VARIABLE: &str = "spool_id";
const MACRO_PREFIX: &str = "gcode_macro ";

/// A tool macro that declares `spool_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpool {
    pub tool: usize,
    /// The macro name as written in the configuration (`T0` or `t0`): `SET_GCODE_VARIABLE` matches it exactly.
    pub macro_name: String,
    /// `None` when the variable is `None` (no spool assigned).
    pub spool_id: Option<u32>,
}

/// The number `n` of a tool macro `T<n>`, in any case.
fn tool_number(macro_name: &str) -> Option<usize> {
    let number = macro_name.strip_prefix(['T', 't'])?;
    (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())).then(|| number.parse().ok())?
}

pub(super) fn subscribe(klipper: &Setup, subscription: &mut Subscription) {
    for name in klipper.macros.iter().filter(|name| tool_number(name).is_some()) {
        subscription.add_any_case(&format!("{MACRO_PREFIX}{name}"), Some(&[SPOOL_VARIABLE]));
    }
}

/// The tool macros that declare `spool_id`, by tool number.
pub(super) fn read_spools(store: &ObjectStore) -> Vec<ToolSpool> {
    let mut spools: Vec<ToolSpool> = store
        .names()
        .filter_map(|object| {
            let (kind, name) = object.split_at_checked(MACRO_PREFIX.len())?;
            let tool = tool_number(name).filter(|_| kind.eq_ignore_ascii_case(MACRO_PREFIX))?;
            let spool_id = store.object(object)?.get(SPOOL_VARIABLE)?;
            Some(ToolSpool {
                tool,
                macro_name: name.to_owned(),
                spool_id: spool_id.as_u64().and_then(|id| u32::try_from(id).ok()),
            })
        })
        .collect();
    spools.sort_by_key(|spool| spool.tool);
    spools
}

/// The G-code that assigns `spool_id` (or none) to the tool macro `macro_name`, as Fluidd and Mainsail send it; with
/// `[save_variables]` also kept across restarts (Klipper wants that name in lower case).
pub fn assign_spool(macro_name: &str, spool_id: Option<u32>, save_variables: bool) -> String {
    let value = spool_id.map_or_else(|| "None".to_owned(), |id| id.to_string());
    let assign = format!("SET_GCODE_VARIABLE MACRO={macro_name} VARIABLE={SPOOL_VARIABLE} VALUE={value}");
    if save_variables {
        let saved = macro_name.to_ascii_lowercase();
        format!("{assign}\nSAVE_VARIABLE VARIABLE={saved}__{SPOOL_VARIABLE} VALUE={value}")
    } else {
        assign
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn spools(status: Value) -> Vec<ToolSpool> {
        let mut store = ObjectStore::default();
        store.replace(&status);
        read_spools(&store)
    }

    #[test]
    fn only_tool_macros_that_declare_the_variable_hold_a_spool() {
        let found = spools(json!({
            "gcode_macro T1": { "spool_id": 7 },
            "gcode_macro t0": { "spool_id": null },
            "gcode_macro T2": {},
            "gcode_macro TURN_OFF": { "spool_id": 3 },
            "gcode_macro PARK": { "spool_id": 4 },
        }));

        assert_eq!(
            found,
            [
                ToolSpool { tool: 0, macro_name: "t0".into(), spool_id: None },
                ToolSpool { tool: 1, macro_name: "T1".into(), spool_id: Some(7) },
            ]
        );
    }

    #[test]
    fn assigning_sends_what_fluidd_and_mainsail_send() {
        assert_eq!(assign_spool("T2", Some(12), false), "SET_GCODE_VARIABLE MACRO=T2 VARIABLE=spool_id VALUE=12");
        assert_eq!(
            assign_spool("t0", None, true),
            "SET_GCODE_VARIABLE MACRO=t0 VARIABLE=spool_id VALUE=None\nSAVE_VARIABLE VARIABLE=t0__spool_id VALUE=None"
        );
    }
}
