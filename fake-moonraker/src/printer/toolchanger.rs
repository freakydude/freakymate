//! A tool changer as `klipper-toolchanger` (viesturz) reports it: `toolchanger`, one `tool T<n>` and one extruder per
//! tool, the user macros `T<n>` with their `spool_id` variable, and Klipper's `save_variables`.

use super::*;
use crate::gcode::{ToolSelector, ToolchangerCommand};
use std::collections::BTreeMap;

/// Tool offsets of the first tools (`gcode_x_offset`, `gcode_y_offset`, `gcode_z_offset`); further tools have none.
const TOOL_OFFSETS: [[f64; 3]; 3] = [[0.0, 0.0, 0.0], [0.12, -0.05, 0.3], [-0.08, 0.1, -0.2]];
/// `spool_id` of the macros at the start (Fluidd and Mainsail keep the Spoolman spool of a tool there).
const START_SPOOL_IDS: [Option<u32>; 3] = [Some(1), Some(3), None];
const SPOOL_ID_VARIABLE: &str = "spool_id";

/// The state of the tool changer; the heater of `extruder` (the first tool) stays in the printer.
#[derive(Debug, Clone)]
pub(super) struct Toolchanger {
    tool_count: usize,
    mounted: Option<usize>,
    /// Index of the extruder `toolhead.extruder` names; `UNSELECT_TOOL` leaves it as it was.
    active_extruder: usize,
    /// The heaters of `extruder1`, `extruder2`, ...
    extra_extruders: Vec<Heater>,
    /// Variables of the user macros by upper-case macro name.
    macro_variables: BTreeMap<String, Map<String, Value>>,
    saved_variables: Map<String, Value>,
}

impl Toolchanger {
    pub(super) fn new(tool_count: usize) -> Self {
        let macro_variables = (0..tool_count)
            .map(|tool| {
                let spool_id = START_SPOOL_IDS.get(tool).copied().flatten();
                let variables = Map::from_iter([(SPOOL_ID_VARIABLE.to_owned(), json!(spool_id))]);
                (tool_macro_name(tool), variables)
            })
            .collect();
        Self {
            tool_count,
            mounted: Some(0),
            active_extruder: 0,
            extra_extruders: vec![Heater::cold(); tool_count.saturating_sub(1)],
            macro_variables,
            saved_variables: Map::new(),
        }
    }

    pub(super) fn advance(&mut self, seconds: f64) {
        self.extra_extruders.iter_mut().for_each(|heater| heater.advance(seconds));
    }

    pub(super) fn switch_heaters_off(&mut self) {
        self.extra_extruders.iter_mut().for_each(|heater| heater.target = 0.0);
    }

    /// The heaters of `extruder1`, `extruder2`, ... with their Klipper names.
    pub(super) fn extra_extruders(&self) -> impl Iterator<Item = (String, &Heater)> {
        self.extra_extruders.iter().enumerate().map(|(index, heater)| (extruder_name(index + 1), heater))
    }

    pub(super) fn extra_extruder_mut(&mut self, name: &str) -> Option<&mut Heater> {
        let index = self.extruder_index(name)?.checked_sub(1)?;
        self.extra_extruders.get_mut(index)
    }

    /// The heater of the active extruder if it is not the printer's own `extruder`.
    pub(super) fn active_extra_extruder_mut(&mut self) -> Option<&mut Heater> {
        self.extra_extruders.get_mut(self.active_extruder.checked_sub(1)?)
    }

    pub(super) fn active_extruder_name(&self) -> String {
        extruder_name(self.active_extruder)
    }

    pub(super) fn apply(&mut self, command: ToolchangerCommand) -> Result<(), CommandError> {
        match command {
            ToolchangerCommand::SelectTool(selector) => {
                let tool = self.resolve(&selector)?;
                self.mounted = Some(tool);
                self.active_extruder = tool;
            }
            ToolchangerCommand::UnselectTool => self.mounted = None,
            ToolchangerCommand::ActivateExtruder { extruder } => {
                let names: Vec<String> = (0..self.tool_count).map(extruder_name).collect();
                self.active_extruder =
                    self.extruder_index(&extruder).ok_or_else(|| mux_error(&extruder, "EXTRUDER", &names))?;
            }
            ToolchangerCommand::SetGcodeVariable { macro_name, variable, value } => {
                self.set_macro_variable(&macro_name, &variable, value)?;
            }
            ToolchangerCommand::SaveVariable { variable, value } => {
                if variable.to_ascii_lowercase() != variable {
                    return Err(CommandError::new("VARIABLE must not contain upper case"));
                }
                self.saved_variables.insert(variable, value);
            }
        }
        Ok(())
    }

    pub(super) fn select(&mut self, tool: usize) -> Result<(), CommandError> {
        self.apply(ToolchangerCommand::SelectTool(ToolSelector::Number(tool)))
    }

    fn resolve(&self, selector: &ToolSelector) -> Result<usize, CommandError> {
        let found = match selector {
            ToolSelector::Number(number) => Some(*number).filter(|number| *number < self.tool_count),
            // The plugin looks the name up as a Klipper object, so only the full name (`tool T1`) is found.
            ToolSelector::Name(name) => (0..self.tool_count).find(|tool| tool_object_name(*tool) == *name),
        };
        found.ok_or_else(|| match selector {
            ToolSelector::Number(number) => CommandError::new(format!("Select tool: T{number} not found")),
            ToolSelector::Name(name) => CommandError::new(format!("Unknown config object '{name}'")),
        })
    }

    fn extruder_index(&self, name: &str) -> Option<usize> {
        let digits = name.strip_prefix("extruder")?;
        let index = if digits.is_empty() { 0 } else { digits.parse().ok().filter(|index| *index > 0)? };
        (index < self.tool_count).then_some(index)
    }

    /// Like Klipper's `SET_GCODE_VARIABLE`: a mux command on the macro name, and the variable must exist as written.
    fn set_macro_variable(&mut self, macro_name: &str, variable: &str, value: Value) -> Result<(), CommandError> {
        let names: Vec<String> = self.macro_variables.keys().cloned().collect();
        let variables =
            self.macro_variables.get_mut(macro_name).ok_or_else(|| mux_error(macro_name, "MACRO", &names))?;
        let slot = variables
            .get_mut(variable)
            .ok_or_else(|| CommandError::new(format!("Unknown gcode_macro variable '{variable}'")))?;
        *slot = value;
        Ok(())
    }

    /// The objects of the tool changer: `toolchanger`, `tool T<n>`, `gcode_macro T<n>` and `save_variables`.
    pub(super) fn status(&self) -> Map<String, Value> {
        let mut objects = Map::new();
        objects.insert("toolchanger".to_owned(), self.toolchanger_json());
        for tool in 0..self.tool_count {
            objects.insert(tool_object_name(tool), self.tool_json(tool));
        }
        for (name, variables) in &self.macro_variables {
            objects.insert(format!("gcode_macro {name}"), Value::Object(variables.clone()));
        }
        objects.insert("save_variables".to_owned(), json!({ "variables": self.saved_variables }));
        objects
    }

    fn toolchanger_json(&self) -> Value {
        let tool = self.mounted.map(tool_object_name);
        let number = self.mounted.map_or(-1, |tool| tool as i64);
        json!({
            "name": "toolchanger",
            "status": "ready",
            "tool": tool,
            "tool_number": number,
            "detected_tool": tool,
            "detected_tool_number": number,
            "tool_numbers": (0..self.tool_count).collect::<Vec<_>>(),
            "tool_names": (0..self.tool_count).map(tool_object_name).collect::<Vec<_>>(),
            "has_detection": true,
        })
    }

    fn tool_json(&self, tool: usize) -> Value {
        let mounted = self.mounted == Some(tool);
        let [x, y, z] = TOOL_OFFSETS.get(tool).copied().unwrap_or_default();
        json!({
            "name": tool_object_name(tool),
            "toolchanger": "toolchanger",
            "detect_state": if mounted { "mounted" } else { "absent" },
            "tool_number": tool,
            "extruder": extruder_name(tool),
            "heater": extruder_name(tool),
            "extruder_stepper": null,
            "fan": null,
            "active": mounted,
            "gcode_x_offset": x,
            "gcode_y_offset": y,
            "gcode_z_offset": z,
        })
    }

    /// The configfile sections of the extra extruders and of the tool macros.
    pub(super) fn config_sections(&self) -> Map<String, Value> {
        let mut sections = Map::new();
        for (name, _) in self.extra_extruders() {
            sections.insert(name, extruder_config());
        }
        for (tool, (name, variables)) in self.macro_variables.iter().enumerate() {
            sections.insert(
                format!("gcode_macro {name}"),
                json!({
                    "variable_spool_id": variables[SPOOL_ID_VARIABLE].as_u64().map_or("None".to_owned(), |id| id.to_string()),
                    "gcode": format!("SELECT_TOOL T={tool}"),
                }),
            );
        }
        sections
    }
}

/// Klipper's error for a mux command whose key matches no registered value (`gcode.py`, `_cmd_mux`). The fake knows
/// only its own values, so the options list just those.
fn mux_error(value: &str, key: &str, values: &[String]) -> CommandError {
    let guess = match values.iter().find(|candidate| !value.is_empty() && candidate.contains(value)) {
        Some(candidate) => format!(". Did you mean '{candidate}'?"),
        None => {
            let options: Vec<String> = values.iter().map(|candidate| format!("'{candidate}'")).collect();
            format!(". Options: {}", options.join(", "))
        }
    };
    CommandError::new(format!("The value '{value}' is not valid for {key}{guess}"))
}

fn tool_macro_name(tool: usize) -> String {
    format!("T{tool}")
}

fn tool_object_name(tool: usize) -> String {
    format!("tool T{tool}")
}

pub(super) fn extruder_name(index: usize) -> String {
    if index == 0 {
        "extruder".to_owned()
    } else {
        format!("extruder{index}")
    }
}

/// The settings of an extruder; `extruder` itself lists a few more (pressure advance, ...).
pub(super) fn extruder_config() -> Value {
    json!({
        "nozzle_diameter": 0.4,
        "filament_diameter": 1.75,
        "min_extrude_temp": MIN_EXTRUDE_TEMPERATURE,
        "min_temp": MIN_HEATER_TEMPERATURE,
        "max_temp": MAX_EXTRUDER_TARGET,
    })
}

impl Printer {
    /// The heater of the active extruder: `extruder` unless a tool of the tool changer made another one active.
    pub(super) fn active_extruder_mut(&mut self) -> &mut Heater {
        match self.toolchanger.as_mut().and_then(Toolchanger::active_extra_extruder_mut) {
            Some(heater) => heater,
            None => &mut self.extruder,
        }
    }

    /// The heater `M104 T<n>` sets, as Klipper picks it: `extruder` for `T0`, `extruder<n>` otherwise, the active one
    /// without `T`.
    pub(super) fn extruder_of_tool_mut(&mut self, tool: Option<usize>) -> Result<&mut Heater, CommandError> {
        match tool {
            None => Ok(self.active_extruder_mut()),
            Some(0) => Ok(&mut self.extruder),
            Some(index) => self
                .toolchanger
                .as_mut()
                .and_then(|toolchanger| toolchanger.extra_extruder_mut(&extruder_name(index)))
                .ok_or_else(|| CommandError::new("Extruder not configured")),
        }
    }

    pub(super) fn apply_toolchanger(&mut self, command: ToolchangerCommand) -> Result<(), CommandError> {
        // Without a tool changer these commands are unknown to Klipper and the fake ignores them.
        match self.toolchanger.as_mut() {
            Some(toolchanger) => toolchanger.apply(command),
            None => Ok(()),
        }
    }

    /// `T<n>` of the tool changer; `None` if the printer has none.
    pub(super) fn select_tool(&mut self, tool: usize) -> Option<Result<(), CommandError>> {
        self.toolchanger.as_mut().map(|toolchanger| toolchanger.select(tool))
    }

    /// The extra extruders and the tool changer's own objects.
    pub(super) fn add_toolchanger_status(&self, objects: &mut Map<String, Value>) {
        let Some(toolchanger) = &self.toolchanger else { return };
        for (name, heater) in toolchanger.extra_extruders() {
            objects.insert(name, self.heater_json(heater));
        }
        objects.extend(toolchanger.status());
    }
}
