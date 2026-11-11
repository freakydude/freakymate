//! A tool changer as `klipper-toolchanger` (viesturz) reports it: `toolchanger`, one `tool T<n>` and one extruder per
//! tool, the user macros `T<n>` with their `spool_id` variable, and Klipper's `save_variables`.
//!
//! Behaviour follows viesturz/klipper-toolchanger `klipper/extras/toolchanger.py` at commit 94756df (read 2026-11-11);
//! line numbers below refer to that file.

use super::*;
use crate::gcode::{ToolSelector, ToolchangerCommand, ToolchangerState};
use crate::machine::InitializeOn;
use std::collections::BTreeMap;

/// Tool offsets of the first tools (`gcode_x_offset`, `gcode_y_offset`, `gcode_z_offset`); further tools have none.
const TOOL_OFFSETS: [[f64; 3]; 3] = [[0.0, 0.0, 0.0], [0.12, -0.05, 0.3], [-0.08, 0.1, -0.2]];
/// `spool_id` of the macros at the start (Fluidd and Mainsail keep the Spoolman spool of a tool there).
const START_SPOOL_IDS: [Option<u32>; 3] = [Some(1), Some(3), None];
const SPOOL_ID_VARIABLE: &str = "spool_id";

/// Which plugin runs the tool changer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// viesturz/klipper-toolchanger: the objects `toolchanger` and `tool T<n>` and the macros `T<n>`.
    Viesturz,
    /// AFC's `AFC_Toolchanger`: Klipper only sees the extruders; the heads and lanes are AFC's objects, which the
    /// printer keeps in step (`toolhead.extruder` follows the head on the shuttle).
    Afc,
}

/// The state of the tool changer; the heater of `extruder` (the first tool) stays in the printer.
#[derive(Debug, Clone)]
pub(super) struct Toolchanger {
    kind: Kind,
    tool_count: usize,
    /// The tool that is physically on the carriage (`detected_tool`; the fake has detection pins).
    mounted: Option<usize>,
    /// The tool the plugin has selected (`active_tool`): none until it initialised, and again after any rejected
    /// command (`_handle_command_error`, Z.219-223).
    active: Option<usize>,
    /// `toolchanger.status`; anything but `ready` makes the plugin refuse tool changes.
    state: ToolchangerState,
    /// `error_message` (Z.149, 477): empty until `process_error` ran, and never cleared afterwards.
    error_message: String,
    /// `require_tool_present` of the config: with it the plugin registers no `UNSELECT_TOOL`.
    require_tool_present: bool,
    initialize_on: InitializeOn,
    /// `error_gcode: PAUSE` is configured.
    error_gcode_pause: bool,
    /// Index of the extruder `toolhead.extruder` names; `UNSELECT_TOOL` leaves it as it was.
    active_extruder: usize,
    /// The heaters of `extruder1`, `extruder2`, ...
    extra_extruders: Vec<Heater>,
    /// Variables of the user macros by upper-case macro name.
    macro_variables: BTreeMap<String, Map<String, Value>>,
    saved_variables: Map<String, Value>,
}

impl Toolchanger {
    /// The plugin after the connect: `uninitialized` with no active tool (`_handle_connect`, Z.214-217).
    pub(super) fn new(profile: &MachineProfile) -> Self {
        let tool_count = profile.tools;
        let macro_variables = (0..tool_count)
            .map(|tool| {
                let spool_id = START_SPOOL_IDS.get(tool).copied().flatten();
                let variables = Map::from_iter([(SPOOL_ID_VARIABLE.to_owned(), json!(spool_id))]);
                (tool_macro_name(tool), variables)
            })
            .collect();
        Self {
            kind: Kind::Viesturz,
            tool_count,
            mounted: Some(0),
            active: None,
            state: ToolchangerState::Uninitialized,
            error_message: String::new(),
            require_tool_present: profile.require_tool_present,
            initialize_on: profile.initialize_on,
            error_gcode_pause: profile.error_gcode_pause,
            active_extruder: 0,
            extra_extruders: vec![Heater::cold(); tool_count.saturating_sub(1)],
            macro_variables,
            saved_variables: Map::new(),
        }
    }

    /// The extruders of the heads of an AFC tool changer; `active_extruder` is the one on the shuttle. None of the
    /// plugin's objects exist, so `status` and the macros stay empty.
    pub(super) fn afc_heads(head_count: usize, active_extruder: usize) -> Self {
        Self {
            kind: Kind::Afc,
            tool_count: head_count,
            mounted: None,
            active: None,
            state: ToolchangerState::Ready,
            error_message: String::new(),
            require_tool_present: false,
            initialize_on: InitializeOn::FirstUse,
            error_gcode_pause: false,
            active_extruder,
            extra_extruders: vec![Heater::cold(); head_count.saturating_sub(1)],
            macro_variables: BTreeMap::new(),
            saved_variables: Map::new(),
        }
    }

    /// The commands the plugin registers beside the tool macros; `UNSELECT_TOOL` only without `require_tool_present`
    /// (viesturz `toolchanger.py`). Empty for AFC, which has none of them.
    pub(super) fn plugin_commands(&self) -> Vec<&'static str> {
        if self.kind == Kind::Afc {
            return Vec::new();
        }
        let mut commands = vec!["SELECT_TOOL", "INITIALIZE_TOOLCHANGER", "SET_GCODE_VARIABLE", "SAVE_VARIABLE"];
        if !self.require_tool_present {
            commands.push("UNSELECT_TOOL");
        }
        commands
    }

    /// `initialize` (Z.368-404): the status becomes `ready` and the detected tool the active one. With
    /// `require_tool_present` and no tool on the carriage it fails.
    fn initialize(&mut self) -> Result<(), CommandError> {
        self.active = self.mounted;
        if let Some(tool) = self.mounted {
            self.active_extruder = tool;
        }
        if self.require_tool_present && self.active.is_none() {
            return Err(CommandError::new(
                "toolchanger failed to initialize, require_tool_present set and no tool present after initialization",
            ));
        }
        self.state = ToolchangerState::Ready;
        Ok(())
    }

    /// `select_tool` (Z.406-427): `first-use` initialises first; the plugin changes tools only in the status `ready`.
    /// `None` unselects.
    fn select_tool(&mut self, tool: Option<usize>) -> Result<(), CommandError> {
        if self.state == ToolchangerState::Uninitialized && self.initialize_on == InitializeOn::FirstUse {
            self.initialize()?;
        }
        if self.state != ToolchangerState::Ready {
            return Err(CommandError::new(format!(
                "Cannot select tool, toolchanger status is {}, reason: {}",
                self.state.as_str(),
                self.error_message
            )));
        }
        if self.active != tool {
            self.mounted = tool;
            self.active = tool;
            if let Some(tool) = tool {
                self.active_extruder = tool;
            }
        }
        Ok(())
    }

    /// `_handle_home_rails_begin` (Z.210-212): with `initialize_on: home` the first homing initialises.
    pub(super) fn initialize_on_home(&mut self) -> Result<(), CommandError> {
        if self.kind == Kind::Viesturz
            && self.initialize_on == InitializeOn::Home
            && self.state == ToolchangerState::Uninitialized
        {
            self.initialize()?;
        }
        Ok(())
    }

    /// `_handle_command_error` (Z.219-223): any rejected G-code command resets the plugin.
    pub(crate) fn note_command_error(&mut self) {
        if self.kind == Kind::Viesturz {
            self.state = ToolchangerState::Uninitialized;
            self.active = None;
        }
    }

    /// The state of the test hook `FAKE_TOOLCHANGER_STATE`. `error` is what `process_error` leaves (Z.475-503, message
    /// of `validate_detected_tool`, Z.572-578: the active tool is expected, none detected).
    fn force_state(&mut self, state: ToolchangerState) {
        if state == ToolchangerState::Error {
            let expected = self.active.or(self.mounted).map_or("None".to_owned(), tool_object_name);
            self.error_message = format!("Expected tool {expected} but active is None");
        }
        if state == ToolchangerState::Uninitialized {
            // The plugin is only ever `uninitialized` without an active tool (Z.214-217, 219-223).
            self.active = None;
        }
        self.state = state;
    }

    /// Whether `process_error` runs `error_gcode: PAUSE` for an error that was just forced.
    pub(super) fn pauses_on_error(&self) -> bool {
        self.error_gcode_pause
    }

    pub(super) fn has_tool(&self, tool: usize) -> bool {
        tool < self.tool_count
    }

    /// Makes the extruder with this Klipper name the active one; an unknown name changes nothing.
    pub(super) fn activate_extruder(&mut self, name: &str) {
        if let Some(index) = self.extruder_index(name) {
            self.active_extruder = index;
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
        // Without the plugin `SELECT_TOOL` and the macro commands do not exist; only Klipper's own `ACTIVATE_EXTRUDER`
        // does.
        if self.kind == Kind::Afc && !matches!(command, ToolchangerCommand::ActivateExtruder { .. }) {
            return Ok(());
        }
        match command {
            ToolchangerCommand::Initialize { recover } => {
                // Z.266-273; `initialize` raises while changing, which the fake never rests in.
                let was_error = self.state == ToolchangerState::Error;
                self.initialize()?;
                if was_error && recover && self.active.is_none() {
                    return Err(CommandError::new("Cannot recover, no tool"));
                }
            }
            ToolchangerCommand::FakeState(state) => self.force_state(state),
            ToolchangerCommand::SelectTool(selector) => {
                let tool = self.resolve(&selector)?;
                self.select_tool(Some(tool))?;
            }
            ToolchangerCommand::UnselectTool => {
                if self.require_tool_present {
                    return Err(CommandError::new("Unknown command:\"UNSELECT_TOOL\""));
                }
                // Without an active tool the command does nothing (Z.335-340); otherwise it is `select_tool(None)`.
                if self.active.is_some() {
                    self.select_tool(None)?;
                }
            }
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
        if self.kind == Kind::Afc {
            return objects;
        }
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
        let number_of = |tool: Option<usize>| tool.map_or(-1, |tool| tool as i64);
        json!({
            "name": "toolchanger",
            "status": self.state.as_str(),
            "tool": self.active.map(tool_object_name),
            "tool_number": number_of(self.active),
            "detected_tool": self.mounted.map(tool_object_name),
            "detected_tool_number": number_of(self.mounted),
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
            "active": self.active == Some(tool),
            "gcode_x_offset": x,
            "gcode_y_offset": y,
            "gcode_z_offset": z,
        })
    }

    /// The configfile sections of the extra extruders and of the tool macros.
    pub(super) fn config_sections(&self) -> Map<String, Value> {
        let mut sections = Map::new();
        if self.kind == Kind::Viesturz {
            // `settings` lists the defaults too (`initialize_on` Z.109-110, `require_tool_present` Z.112).
            let mut section = json!({
                "initialize_on": self.initialize_on.as_str(),
                "require_tool_present": self.require_tool_present,
            });
            if self.error_gcode_pause {
                section["error_gcode"] = json!("PAUSE");
            }
            sections.insert("toolchanger".to_owned(), section);
        }
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
pub(crate) fn mux_error(value: &str, key: &str, values: &[String]) -> CommandError {
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
        let forces_error = matches!(command, ToolchangerCommand::FakeState(ToolchangerState::Error));
        // Without a tool changer these commands are unknown to Klipper and the fake ignores them.
        let Some(toolchanger) = self.toolchanger.as_mut() else { return Ok(()) };
        toolchanger.apply(command)?;
        // `process_error` runs `error_gcode`, which is `PAUSE` in the example config (Z.481-493).
        if forces_error && toolchanger.pauses_on_error() && self.print_state == PrintState::Printing {
            self.pause_print()?;
        }
        Ok(())
    }

    /// Klipper's `gcode:command_error` event, which resets the tool changer plugin.
    pub(crate) fn note_command_error(&mut self) {
        if let Some(toolchanger) = self.toolchanger.as_mut() {
            toolchanger.note_command_error();
        }
    }

    /// `initialize_on: home`: the homing initialises the plugin first.
    pub(super) fn initialize_toolchanger_on_home(&mut self) -> Result<(), CommandError> {
        match self.toolchanger.as_mut() {
            Some(toolchanger) => toolchanger.initialize_on_home(),
            None => Ok(()),
        }
    }

    /// The commands of the tool changer plugin beside the tool macros; empty without one.
    pub(crate) fn toolchanger_commands(&self) -> Vec<&'static str> {
        self.toolchanger.as_ref().map(Toolchanger::plugin_commands).unwrap_or_default()
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
