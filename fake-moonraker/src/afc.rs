//! Simulated AFC (ArmoredTurtle/AFC-Klipper-Add-On): the objects `AFC`, `AFC_stepper <lane>` or `AFC_lane <name>`,
//! `AFC_extruder <name>`, `AFC_hub <name>` and one unit object, with the values the plugin reports.
//!
//! Source: AFC-Klipper-Add-On at commit 5ea84ea (`extras/…`). Operations run without elapsed time like the Happy Hare
//! simulation: a tool change ends in the state `Idle` at once. AFC reports most of its problems as a log line
//! (`AFC_error(msg, pause=False)`) and not as a G-code error; the fake answers them with a G-code error.

use crate::gcode::{AfcCommand, CommandError};
use crate::machine::AfcLayout;
use crate::spoolman;
use serde_json::{json, Map, Value};

/// `AFC.spoolman` is the server URL of Moonraker's `[spoolman]` section (`AFC.py` L409, `AFC_utils.py` L444-454). The
/// fake has no such option; this is Spoolman's default port on the machine itself.
const SPOOLMAN_SERVER: &str = "http://127.0.0.1:7912";

/// `AFCLaneState` (`AFC_lane.py` L65-76) as far as the fake reaches the values.
const LANE_NONE: &str = "None";
const LANE_LOADED: &str = "Loaded";
const LANE_TOOLED: &str = "Tooled";

/// Colours of `get_filament_status` (`AFC_functions.py` L564-571) with the defaults of `AFC.py` L186-191 run through
/// `HexConvert` (`AFC_functions.py` L812-826): `0,0.8,0,0` is `#00cc00`.
const STATUS_LED_IN_TOOL: &str = "#0000ff";
const STATUS_LED_READY: &str = "#00cc00";
const STATUS_LED_NOT_READY: &str = "#ff0000";

/// `AFC_hub` values of the Box Turtle template (`templates/AFC_Turtle_1.cfg`, section `[AFC_hub Turtle_1]`).
const HUB_BOWDEN_LENGTH: f64 = 1725.0;

/// `AFC_Toolchanger` has the type `Toolchanger`, the Box Turtle `Box_Turtle` (`AFC_Toolchanger.py` L42,
/// `AFC_BoxTurtle.py` L33); `AFC.get_status` lists a unit as `<type> <name>` (`AFC.py` L2551-2556).
const BOX_TURTLE_UNIT: &str = "Turtle_1";
const BOX_TURTLE_TYPE: &str = "Box_Turtle";
const TOOLCHANGER_UNIT: &str = "Tools";
const TOOLCHANGER_TYPE: &str = "Toolchanger";
/// A tool changer lane has no hub switch: `hub` is `direct` (`AFC_extruder.py` L327-331).
const DIRECT_HUB: &str = "direct";

/// The TurtleNeck buffer of the Box Turtle: `AFC_buffer TN` of type `switched` (the default, `AFC_buffer.py` L47) with an
/// advance and a trailing switch (L97-108). The clone has no template with these pins for a Box Turtle; the name `TN`
/// is that of the AFC tests (`tests/test_AFC_buffer.py` L204) and the pins (`^Turtle_1:ADV`, `^Turtle_1:TRL`) follow the
/// pattern of `templates/AFC_Claymore_1-AFC_Lite.cfg` L106-107: a profile choice.
const BUFFER_NAME: &str = "TN";
const BUFFER_ADVANCE_PIN: &str = "^Turtle_1:ADV";
const BUFFER_TRAILING_PIN: &str = "^Turtle_1:TRL";
/// `pin_tool_end` of the Box Turtle head: a sensor after the extruder gears (`AFC_extruder.py` L227, optional; the
/// templates only have it as `#pin_tool_end: None`, `AFC_Hardware-AFC.cfg` L6). The pin itself is a profile choice.
const TOOL_END_PIN: &str = "^PG11";
/// `AFC_buffer.last_state` before any switch moved (`AFC_buffer.py` L54) and while the buffer feeds (L365).
const BUFFER_STATE_IDLE: &str = "Unknown";
const BUFFER_STATE_ADVANCING: &str = "Advancing";
/// The Klipper object of the switches of AFC (`AFC_utils.py` L55): `enable_sensors_in_gui: True` as in
/// `config/AFC.cfg` L15 shows them; with `False` the object is called `_filament_switch_sensor <name>` (L82-84).
const SWITCH_SENSOR: &str = "filament_switch_sensor";

/// Spoolman spools on the lanes at the start: the red PLA, the black ABS (the spool Moonraker has active at its start)
/// and the blue PLA.
const SPOOL_ON_FIRST_LANE: u32 = 1;
const SPOOL_ON_SECOND_LANE: u32 = 4;
const SPOOL_ON_THIRD_LANE: u32 = 3;
/// `dist_hub` of the Box Turtle lanes in the template; a lane without the option has 60 (`AFC_lane.py` L201).
const BOX_TURTLE_DIST_HUB: [f64; 4] = [155.0, 80.0, 90.0, 145.0];
const DEFAULT_DIST_HUB: f64 = 60.0;

#[derive(Debug, Clone)]
struct Lane {
    name: String,
    /// The unit-local number (`unit: Turtle_1:2` is 2; a lane without it has 0, `AFC_lane.py` L150-154).
    index: u32,
    /// `T<n>` command of the lane (`map`).
    map: String,
    hub: &'static str,
    /// Position of the `AFC_extruder` in `Afc::heads`.
    head: usize,
    /// Prep and load sensors; one filament in the lane triggers both.
    loaded: bool,
    tool_loaded: bool,
    loaded_to_hub: bool,
    material: String,
    spool_id: Option<u32>,
    color: String,
    multi_color: Vec<String>,
    filament_name: String,
    weight: f64,
    initial_weight: f64,
    extruder_temp: Option<i64>,
    bed_temp: Option<i64>,
    status: &'static str,
    dist_hub: f64,
}

impl Lane {
    fn new(name: String, index: u32, map: String, hub: &'static str, head: usize, dist_hub: f64) -> Self {
        let mut lane = Self {
            name,
            index,
            map,
            hub,
            head,
            loaded: false,
            tool_loaded: false,
            loaded_to_hub: false,
            material: String::new(),
            spool_id: None,
            color: String::new(),
            multi_color: Vec::new(),
            filament_name: String::new(),
            weight: 0.0,
            initial_weight: 1000.0,
            extruder_temp: None,
            bed_temp: None,
            status: LANE_NONE,
            dist_hub,
        };
        lane.clear_spool();
        lane
    }

    /// `AFC_spool.py` L404-418 (`clear_values`).
    fn clear_spool(&mut self) {
        self.spool_id = None;
        self.material = String::new();
        self.color = String::new();
        self.multi_color = Vec::new();
        self.weight = 0.0;
        self.extruder_temp = None;
        self.bed_temp = None;
        self.filament_name = String::new();
    }

    /// `AFC_spool.py` L420-470 (`set_spoolID`): what Spoolman knows of the spool.
    fn take_spool(&mut self, id: u32, spool: &Value) {
        let filament = &spool["filament"];
        self.spool_id = Some(id);
        self.material = filament["material"].as_str().unwrap_or_default().to_owned();
        self.extruder_temp = filament["settings_extruder_temp"].as_i64();
        self.bed_temp = filament["settings_bed_temp"].as_i64();
        self.weight = spool["remaining_weight"].as_f64().unwrap_or_default();
        self.initial_weight = spool["initial_weight"].as_f64().unwrap_or(1000.0);
        self.filament_name = filament["name"].as_str().unwrap_or_default().to_owned();
        // The first colour of a multi-colour filament is the lane's colour.
        match filament["multi_color_hexes"].as_str() {
            Some(hexes) => {
                self.multi_color = hexes.split(',').map(str::to_owned).collect();
                self.color = format!("#{}", self.multi_color[0]);
            }
            None => {
                self.multi_color = Vec::new();
                self.color = format!("#{}", filament["color_hex"].as_str().unwrap_or_default());
            }
        }
    }

    /// Filament with a Spoolman spool in it, as after PREP and a Spoolman lookup.
    fn with_spool(mut self, id: u32) -> Self {
        self.loaded = true;
        self.status = LANE_LOADED;
        if let Some(spool) = spoolman::initial_spool(id) {
            self.take_spool(id, &spool);
        }
        self
    }
}

/// An `AFC_extruder` section: the toolhead sensors and the lane in it.
#[derive(Debug, Clone)]
struct Head {
    name: String,
    /// `extruder_name` of the section; the Klipper extruder of the head (defaults to the section's own name).
    extruder_name: String,
    tool_stn: f64,
    tool_stn_unload: f64,
    /// `pin_tool_start`; `buffer` is a valid value that needs no switch (`AFC_extruder.py` L288-298).
    tool_start: Option<&'static str>,
    /// `pin_tool_end` (`AFC_extruder.py` L227).
    tool_end: Option<&'static str>,
    /// `buffer` (`AFC_extruder.py` L233): needed when `pin_tool_start` is `buffer` (`AFC_lane.py` L636-646).
    buffer: Option<&'static str>,
    lane_loaded: Option<String>,
}

impl Head {
    /// A switch reports the filament in the toolhead; the `buffer` start needs none and stays false.
    fn tool_start_status(&self) -> bool {
        self.lane_loaded.is_some() && self.tool_start.is_some_and(|pin| pin != "buffer")
    }

    /// The sensor after the extruder gears sees the filament of the lane in the toolhead.
    fn tool_end_status(&self) -> bool {
        self.lane_loaded.is_some() && self.tool_end.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct Afc {
    layout: AfcLayout,
    lanes: Vec<Lane>,
    heads: Vec<Head>,
    /// The `T<n>` commands in the order AFC registered them (`AFC.tool_cmds`); `SET_MAP` swaps the lanes behind them.
    maps: Vec<String>,
    /// Index of the head on the shuttle of a tool changer; the single head of a Box Turtle is always there.
    shuttle: Option<usize>,
    /// The `AFC_buffer` of the unit (`buffer` of `AFC_BoxTurtle`, `AFC_unit.py` L78); the lanes use it.
    buffer: Option<&'static str>,
}

impl Afc {
    pub fn new(layout: AfcLayout) -> Self {
        match layout {
            AfcLayout::BoxTurtle => Self::box_turtle(),
            AfcLayout::Toolchanger => Self::toolchanger(),
        }
    }

    /// `AFC_BoxTurtle Turtle_1` with `AFC_stepper lane1` to `lane4`, one `AFC_hub Turtle_1` and `AFC_extruder
    /// extruder`; lane 2 is loaded in the toolhead.
    fn box_turtle() -> Self {
        let lane = |number: usize| {
            Lane::new(
                format!("lane{number}"),
                number as u32,
                format!("T{}", number - 1),
                BOX_TURTLE_UNIT,
                0,
                BOX_TURTLE_DIST_HUB[number - 1],
            )
        };
        let mut lanes = vec![
            lane(1).with_spool(SPOOL_ON_FIRST_LANE),
            lane(2).with_spool(SPOOL_ON_SECOND_LANE),
            lane(3).with_spool(SPOOL_ON_THIRD_LANE),
            lane(4),
        ];
        lanes[1].tool_loaded = true;
        lanes[1].loaded_to_hub = true;
        lanes[1].status = LANE_TOOLED;
        let head = Head {
            name: "extruder".to_owned(),
            extruder_name: "extruder".to_owned(),
            // The defaults of `AFC_extruder.py` L236-241, which the template leaves alone.
            tool_stn: 72.0,
            tool_stn_unload: 100.0,
            tool_start: Some("buffer"),
            tool_end: Some(TOOL_END_PIN),
            buffer: Some(BUFFER_NAME),
            lane_loaded: Some("lane2".to_owned()),
        };
        let mut afc = Self::assemble(AfcLayout::BoxTurtle, lanes, vec![head], Some(0));
        afc.buffer = Some(BUFFER_NAME);
        afc
    }

    /// `AFC_Toolchanger Tools` with `AFC_extruder e0` to `e3`, each with its own `AFC_lane` (the template
    /// `AFC_Hardware_U1.cfg`); the head `e1` is on the shuttle with its lane in the toolhead.
    fn toolchanger() -> Self {
        let head = |number: usize| Head {
            name: format!("e{number}"),
            extruder_name: if number == 0 { "extruder".to_owned() } else { format!("extruder{number}") },
            tool_stn: 50.0,
            tool_stn_unload: 60.0,
            tool_start: None,
            tool_end: None,
            buffer: None,
            lane_loaded: None,
        };
        let lane = |number: usize| {
            Lane::new(format!("e{number}"), 0, format!("T{number}"), DIRECT_HUB, number, DEFAULT_DIST_HUB)
        };
        let lanes = vec![
            lane(0).with_spool(SPOOL_ON_FIRST_LANE),
            lane(1).with_spool(SPOOL_ON_SECOND_LANE),
            lane(2).with_spool(SPOOL_ON_THIRD_LANE),
            lane(3),
        ];
        let mut afc = Self::assemble(AfcLayout::Toolchanger, lanes, (0..4).map(head).collect(), Some(1));
        afc.load_into_tool(1);
        afc
    }

    fn assemble(layout: AfcLayout, lanes: Vec<Lane>, heads: Vec<Head>, shuttle: Option<usize>) -> Self {
        let maps = lanes.iter().map(|lane| lane.map.clone()).collect();
        Self { layout, lanes, heads, maps, shuttle, buffer: None }
    }

    /// The Klipper extruder of the head on the shuttle of a tool changer; `None` for the Box Turtle.
    pub fn shuttle_extruder(&self) -> Option<&str> {
        match self.layout {
            AfcLayout::BoxTurtle => None,
            AfcLayout::Toolchanger => self.shuttle.map(|head| self.heads[head].extruder_name.as_str()),
        }
    }

    /// The `AFC_extruder` sections (heads of a tool changer).
    pub fn head_count(&self) -> usize {
        self.heads.len()
    }

    pub fn is_toolchanger(&self) -> bool {
        self.layout == AfcLayout::Toolchanger
    }

    /// The `T<n>` commands of the lanes.
    pub fn maps(&self) -> &[String] {
        &self.maps
    }

    /// `AFC.current`: the lane in the toolhead that is on the shuttle (`AFC_functions.py` L437-447, L472-483).
    fn current(&self) -> Option<usize> {
        let head = self.shuttle?;
        let name = self.heads[head].lane_loaded.as_deref()?;
        self.lane_index(name)
    }

    /// The spool of the lane in the toolhead, which AFC makes the active one in Moonraker (`AFC_spool.py` L310-322,
    /// `AFC_lane.py` L1527, L1546).
    pub fn active_spool(&self) -> Option<u32> {
        self.current().and_then(|lane| self.lanes[lane].spool_id)
    }

    fn lane_index(&self, name: &str) -> Option<usize> {
        self.lanes.iter().position(|lane| lane.name == name)
    }

    /// A lane as `register_mux_command(..., "LANE", name, ...)` finds it: only the exact name.
    fn mux_lane(&self, lane: Option<&str>) -> Result<usize, CommandError> {
        let name = lane.ok_or_else(|| CommandError::new("Missing parameter LANE"))?;
        self.lane_index(name).ok_or_else(|| {
            let names: Vec<String> = self.lanes.iter().map(|lane| lane.name.clone()).collect();
            crate::printer::mux_error(name, "LANE", &names)
        })
    }

    pub fn apply(&mut self, command: AfcCommand, homed: bool, printing: bool) -> Result<(), CommandError> {
        match command {
            AfcCommand::ChangeTool { lane } => self.change_tool_command(lane.as_deref(), homed),
            AfcCommand::ToolLoad { lane } => self.tool_load_command(lane.as_deref(), homed),
            AfcCommand::ToolUnload { lane } => {
                self.tool_unload_command(lane.as_deref());
                Ok(())
            }
            AfcCommand::LaneUnload { lane } => self.lane_unload_command(lane.as_deref(), printing),
            AfcCommand::SetSpoolId { lane, spool_id } => self.set_spool_id(lane.as_deref(), spool_id.as_deref()),
            AfcCommand::SetColor { lane, color } => {
                let lane = self.mux_lane(lane.as_deref())?;
                // `AFC_spool.py` L197-212: a missing COLOR is black; the colour keeps one `#`.
                let color = color.unwrap_or_else(|| "#000000".to_owned());
                self.lanes[lane].color = format!("#{}", color.replace('#', ""));
                self.lanes[lane].multi_color = Vec::new();
                Ok(())
            }
            AfcCommand::SetMaterial { lane, material } => {
                let lane = self.mux_lane(lane.as_deref())?;
                let material = material.ok_or_else(|| CommandError::new("Missing parameter MATERIAL"))?;
                self.lanes[lane].material = material;
                Ok(())
            }
            AfcCommand::SetMap { lane, map } => self.set_map(lane.as_deref(), map.as_deref()),
        }
    }

    /// `T<n>`: the lane behind the command (`AFC_functions.py` L270-302, `AFC.cmd_CHANGE_TOOL` L2293-2380).
    pub fn change_tool_by_map(&mut self, tool: usize, homed: bool) -> Result<(), CommandError> {
        let map = format!("T{tool}");
        match self.lanes.iter().position(|lane| lane.map == map) {
            Some(lane) => self.change_tool(lane, homed),
            None => Err(CommandError::new(format!("Unknown command:\"{map}\""))),
        }
    }

    fn change_tool_command(&mut self, lane: Option<&str>, homed: bool) -> Result<(), CommandError> {
        // `CHANGE_TOOL LANE=` looks the lane up case-insensitively among the lanes behind a `T<n>` (L2354-2360).
        let found = lane.and_then(|name| self.lanes.iter().position(|lane| lane.name.eq_ignore_ascii_case(name)));
        match found {
            Some(lane) => self.change_tool(lane, homed),
            None => Err(CommandError::new("I did not understand the change -- CHANGE_TOOL")),
        }
    }

    /// `AFC.CHANGE_TOOL` (L2381-2480): unloads the lane in the toolhead, then loads the new one. A lane that is not
    /// loaded fails before anything moves here; AFC unloads first.
    fn change_tool(&mut self, lane: usize, homed: bool) -> Result<(), CommandError> {
        require_homed(homed)?;
        if self.current() == Some(lane) {
            return Ok(());
        }
        self.require_filament(lane)?;
        if let Some(current) = self.current() {
            self.unload_from_tool(current);
        }
        self.load_into_tool(lane);
        Ok(())
    }

    fn tool_load_command(&mut self, lane: Option<&str>, homed: bool) -> Result<(), CommandError> {
        let lane = self.mux_lane(lane)?;
        if self.lanes[lane].tool_loaded {
            return Err(CommandError::new(format!("Not loading {}, already loaded", self.lanes[lane].name)));
        }
        require_homed(homed)?;
        self.require_filament(lane)?;
        // `TOOL_LOAD` unloads what the head of the lane holds first (L1490-1505).
        let head = self.lanes[lane].head;
        if let Some(loaded) = self.heads[head].lane_loaded.clone().and_then(|name| self.lane_index(&name)) {
            self.unload_from_tool(loaded);
        }
        self.load_into_tool(lane);
        Ok(())
    }

    /// `TOOL_UNLOAD [LANE=]` (L1828-1860): the lane in the toolhead without a name; an unknown lane is only logged.
    fn tool_unload_command(&mut self, lane: Option<&str>) {
        let lane = match lane {
            Some(name) => self.lane_index(name),
            None => self.current(),
        };
        if let Some(lane) = lane.filter(|lane| self.lanes[*lane].tool_loaded) {
            self.unload_from_tool(lane);
        }
    }

    /// `LANE_UNLOAD LANE=` (L1321-1380): ejects a lane that is not in the toolhead; the filament is taken out by hand.
    fn lane_unload_command(&mut self, lane: Option<&str>, printing: bool) -> Result<(), CommandError> {
        if printing {
            return Err(CommandError::new("Cannot eject lane while printer is printing"));
        }
        let index = self.mux_lane(lane)?;
        let lane = &mut self.lanes[index];
        if lane.tool_loaded {
            return Ok(());
        }
        lane.loaded = false;
        lane.loaded_to_hub = false;
        lane.status = LANE_NONE;
        lane.clear_spool();
        Ok(())
    }

    fn require_filament(&self, lane: usize) -> Result<(), CommandError> {
        if self.lanes[lane].loaded {
            Ok(())
        } else {
            Err(CommandError::new(
                "Current lane not loaded, LOAD TRIGGER NOT TRIGGERED\n||==>--||----||-----||\nTRG   LOAD   HUB   TOOL\nPlease load lane before continuing.",
            ))
        }
    }

    /// `AFC_lane.set_tool_unloaded` (L1536-1547) after `TOOL_UNLOAD`: the lane is not in the toolhead and has no
    /// status; the filament sits at the hub.
    fn unload_from_tool(&mut self, lane: usize) {
        let head = self.lanes[lane].head;
        self.lanes[lane].tool_loaded = false;
        self.lanes[lane].status = LANE_NONE;
        self.heads[head].lane_loaded = None;
    }

    /// `AFC_lane.set_tool_loaded` (L1510-1530): the lane is in the toolhead of its head, which a tool changer puts on
    /// the shuttle first (`TOOL_LOAD`, `cur_lane.tool_swap()`).
    fn load_into_tool(&mut self, lane: usize) {
        let head = self.lanes[lane].head;
        if self.is_toolchanger() {
            self.shuttle = Some(head);
        }
        self.lanes[lane].tool_loaded = true;
        self.lanes[lane].loaded_to_hub = true;
        self.lanes[lane].status = LANE_TOOLED;
        self.heads[head].lane_loaded = Some(self.lanes[lane].name.clone());
    }

    /// `AFC_spool.cmd_SET_SPOOL_ID` (L325-370) with Spoolman: only the plain refusals are simulated; AFC logs them.
    fn set_spool_id(&mut self, lane: Option<&str>, spool_id: Option<&str>) -> Result<(), CommandError> {
        let lane = self.mux_lane(lane)?;
        let text = spool_id.unwrap_or_default();
        if text.is_empty() {
            // An empty id takes the spool out of the lane (`set_spoolID`: `clear_values`).
            self.lanes[lane].clear_spool();
            return Ok(());
        }
        let Ok(id) = text.parse::<u32>() else { return Ok(()) };
        let taken = self.lanes.iter().any(|other| other.spool_id == Some(id));
        if taken && self.lanes[lane].spool_id != Some(id) {
            return Ok(());
        }
        let Some(spool) = spoolman::initial_spool(id) else { return Ok(()) };
        self.lanes[lane].take_spool(id, &spool);
        Ok(())
    }

    /// `AFC_spool.cmd_SET_MAP` (L148-195): the lane takes the command and the lane that had it takes the old one.
    fn set_map(&mut self, lane: Option<&str>, map: Option<&str>) -> Result<(), CommandError> {
        let lane = self.mux_lane(lane)?;
        let Some(map) = map.map(str::to_ascii_uppercase).filter(|map| self.maps.contains(map)) else {
            return Ok(());
        };
        let Some(other) = self.lanes.iter().position(|other| other.map == map) else { return Ok(()) };
        let own = self.lanes[lane].map.clone();
        self.lanes[other].map = own;
        self.lanes[lane].map = map;
        Ok(())
    }

    /// All objects of AFC.
    pub fn status(&self) -> Map<String, Value> {
        let mut objects = Map::new();
        objects.insert("AFC".to_owned(), self.afc_json());
        for (index, lane) in self.lanes.iter().enumerate() {
            let object = match self.layout {
                AfcLayout::BoxTurtle => format!("AFC_stepper {}", lane.name),
                AfcLayout::Toolchanger => format!("AFC_lane {}", lane.name),
            };
            objects.insert(object, self.lane_json(index));
        }
        for head in 0..self.heads.len() {
            objects.insert(format!("AFC_extruder {}", self.heads[head].name), self.head_json(head));
        }
        if let Some(buffer) = self.buffer {
            objects.insert(format!("AFC_buffer {buffer}"), self.buffer_json());
        }
        for (name, detected) in self.switch_sensors() {
            objects
                .insert(format!("{SWITCH_SENSOR} {name}"), json!({ "enabled": true, "filament_detected": detected }));
        }
        match self.layout {
            AfcLayout::BoxTurtle => {
                objects.insert(format!("AFC_hub {BOX_TURTLE_UNIT}"), self.hub_json());
                objects.insert(format!("AFC_BoxTurtle {BOX_TURTLE_UNIT}"), self.unit_json());
            }
            AfcLayout::Toolchanger => {
                objects.insert(format!("AFC_Toolchanger {TOOLCHANGER_UNIT}"), self.unit_json());
            }
        }
        objects
    }

    fn unit(&self) -> (&'static str, &'static str) {
        match self.layout {
            AfcLayout::BoxTurtle => (BOX_TURTLE_TYPE, BOX_TURTLE_UNIT),
            AfcLayout::Toolchanger => (TOOLCHANGER_TYPE, TOOLCHANGER_UNIT),
        }
    }

    fn buffer_names(&self) -> Vec<&'static str> {
        self.buffer.into_iter().collect()
    }

    /// A lane of the buffer is in the toolhead: the buffer is enabled then (`AFC_buffer.enable_buffer`, L310-317).
    fn buffer_active_lane(&self) -> Option<&str> {
        self.lanes.iter().find(|lane| lane.tool_loaded).map(|lane| lane.name.as_str())
    }

    /// `AFC_buffer.last_state`. The fake does not keep the last state: `Advancing` while a lane feeds from the buffer,
    /// else the state before any switch moved. A buffer that was used keeps its last state in AFC.
    fn buffer_state(&self) -> &'static str {
        if self.buffer_active_lane().is_some() {
            BUFFER_STATE_ADVANCING
        } else {
            BUFFER_STATE_IDLE
        }
    }

    /// The switch sensors AFC registers (`AFC_utils.add_filament_switch`) with the AFC state the fake keeps, named
    /// without the object prefix. Only what the config has pins for: the Box Turtle template has `prep` and `load` for
    /// every lane (`templates/AFC_Turtle_1.cfg` L35-36, ...; `AFC_lane.py` L257-282), a hub with a `switch_pin` (L123;
    /// `AFC_hub.py` L69-70), a TurtleNeck buffer (`AFC_buffer.py` L97-108) and the head's tool sensors
    /// (`AFC_extruder.py` L295-322; none for `pin_tool_start: buffer`). The U1 template has none of these pins
    /// (`templates/AFC_Hardware_U1.cfg`: no `prep`, `load`, `switch_pin`, only `u1_filament_sensor_name`).
    fn switch_sensors(&self) -> Vec<(String, bool)> {
        let mut sensors = Vec::new();
        if self.layout == AfcLayout::BoxTurtle {
            for lane in &self.lanes {
                sensors.push((format!("{}_prep", lane.name), lane.loaded));
                sensors.push((format!("{}_load", lane.name), lane.loaded));
            }
            sensors.push((format!("{BOX_TURTLE_UNIT}_Hub"), self.hub_state()));
        }
        if let Some(buffer) = self.buffer {
            // Neither switch is pressed while the buffer sits between them; the fake does not move it.
            sensors.push((format!("{buffer}_expanded"), false));
            sensors.push((format!("{buffer}_compressed"), false));
        }
        for head in &self.heads {
            if head.tool_start.is_some_and(|pin| pin != "buffer") {
                sensors.push((format!("{}_tool_start", head.name), head.tool_start_status()));
            }
            if head.tool_end.is_some() {
                sensors.push((format!("{}_tool_end", head.name), head.tool_end_status()));
            }
        }
        sensors
    }

    /// The switch of the hub sees the filament of a lane in the toolhead.
    fn hub_state(&self) -> bool {
        self.lanes.iter().any(|lane| lane.tool_loaded)
    }

    /// `AFC_buffer.get_status` (`AFC_buffer.py` L664-702) of a buffer of type `switched` with the defaults of the
    /// options (L68-73, L90-92): no fault detection, no fault tracking. The rotation distance of the lane's stepper
    /// is not simulated.
    fn buffer_json(&self) -> Value {
        let active = self.buffer_active_lane();
        json!({
            "state": self.buffer_state(),
            "lanes": self.lane_names(),
            "enabled": active.is_some(),
            "rotation_distance": null,
            "active_lane": active,
            "multiplier_high": 1.1,
            "multiplier_low": 0.9,
            "multiplier": 1,
            "fault_detection_enabled": false,
            "error_sensitivity": 0,
            "fault_timer": null,
            "distance_to_fault": null,
        })
    }

    fn lane_names(&self) -> Vec<&str> {
        self.lanes.iter().map(|lane| lane.name.as_str()).collect()
    }

    /// The hubs that are real `AFC_hub` objects; the lanes of a tool changer have the `direct` hub (`afcUnit.get_status`,
    /// `AFC_unit.py` L264-278: `is_direct_hub`).
    fn hub_names(&self) -> Vec<&'static str> {
        match self.layout {
            AfcLayout::BoxTurtle => vec![BOX_TURTLE_UNIT],
            AfcLayout::Toolchanger => Vec::new(),
        }
    }

    /// `AFC.get_status` (`AFC.py` L2537-2570).
    fn afc_json(&self) -> Value {
        let (unit_type, unit_name) = self.unit();
        json!({
            "current_load": self.current().map(|lane| self.lanes[lane].name.as_str()),
            "current_lane": null,
            "next_lane": null,
            "current_state": "Idle",
            "current_toolchange": 0,
            "number_of_toolchanges": 0,
            "spoolman": SPOOLMAN_SERVER,
            "td1_present": false,
            "lane_data_enabled": false,
            "error_state": false,
            "bypass_state": false,
            "quiet_mode": false,
            "position_saved": false,
            "units": [format!("{unit_type} {unit_name}")],
            "lanes": self.lane_names(),
            "maps": self.maps,
            "extruders": self.heads.iter().map(|head| head.name.as_str()).collect::<Vec<_>>(),
            "hubs": self.hub_names(),
            "buffers": self.buffer_names(),
            "message": { "message": "", "type": "" },
            "led_state": true,
        })
    }

    /// `AFCLane.get_status` (`AFC_lane.py` L2183-2234).
    fn lane_json(&self, index: usize) -> Value {
        let lane = &self.lanes[index];
        let in_tool = self.heads[lane.head].lane_loaded.as_deref() == Some(lane.name.as_str());
        let (filament_status, filament_status_led) = match (lane.loaded, in_tool) {
            (true, true) => ("In Tool", STATUS_LED_IN_TOOL),
            (true, false) => ("Ready", STATUS_LED_READY),
            (false, _) => ("Not Ready", STATUS_LED_NOT_READY),
        };
        let mut status = json!({
            "name": lane.name,
            "unit": self.unit().1,
            "hub": lane.hub,
            "extruder": self.heads[lane.head].name,
            "buffer": self.buffer,
            "buffer_status": self.buffer.map(|_| self.buffer_state()),
            "lane": lane.index,
            "map": lane.map,
            "load": lane.loaded,
            "prep": lane.loaded,
            "tool_loaded": lane.tool_loaded,
            "loaded_to_hub": lane.loaded_to_hub,
            "material": lane.material,
            "remember_spool": false,
            "spool_id": lane.spool_id,
            "color": lane.color,
            "filament_name": lane.filament_name,
            "multi_color_hexes": lane.multi_color,
            "initial_weight": lane.initial_weight,
            "weight": lane.weight,
            "extruder_temp": lane.extruder_temp,
            "bed_temp": lane.bed_temp,
            "runout_lane": null,
            "filament_status": filament_status,
            "filament_status_led": filament_status_led,
            "status": lane.status,
            "dist_hub": lane.dist_hub,
            "td1_td": "",
            "td1_color": "",
            "td1_scan_time": "",
        });
        // Only the stepper lanes of a Box Turtle have `_endstops` (`AFC_stepper.py` L113, `AFC_lane.py` L739): the
        // load switch of the stepper itself, then the lane's own prep, load and hub endstops.
        if self.layout == AfcLayout::BoxTurtle {
            let name = &lane.name;
            status["endstops"] = json!(format!("load,{name}_prep,{name}_load,{name}_hub"));
        }
        status
    }

    /// `AFCExtruder.get_status` (`AFC_extruder.py` L1065-1084).
    fn head_json(&self, index: usize) -> Value {
        let head = &self.heads[index];
        let lanes: Vec<&str> =
            self.lanes.iter().filter(|lane| lane.head == index).map(|lane| lane.name.as_str()).collect();
        let on_shuttle = match self.layout {
            AfcLayout::BoxTurtle => true,
            AfcLayout::Toolchanger => self.shuttle == Some(index),
        };
        json!({
            "tool_stn": head.tool_stn,
            "tool_stn_unload": head.tool_stn_unload,
            "tool_sensor_after_extruder": 0.0,
            "tool_unload_speed": 25.0,
            "tool_load_speed": 25.0,
            "buffer": head.buffer,
            "lane_loaded": head.lane_loaded,
            "tool_start": head.tool_start,
            "tool_start_status": head.tool_start_status(),
            "tool_end": head.tool_end,
            "tool_end_status": head.tool_end_status(),
            "lanes": lanes,
            "on_shuttle": on_shuttle,
            "is_standalone": false,
            "next_pickup": false,
            "status": "Idle",
        })
    }

    /// `afc_hub.get_status` (`AFC_hub.py` L193-209); the switch sees the filament of a lane in the toolhead.
    fn hub_json(&self) -> Value {
        json!({
            "state": self.hub_state(),
            "cut": false,
            "cut_cmd": "AFC",
            "cut_dist": 50.0,
            "cut_clear": 120.0,
            "cut_min_length": 300.0,
            "cut_servo_pass_angle": 10.0,
            "cut_servo_clip_angle": 180.0,
            "cut_servo_prep_angle": 80.0,
            "lanes": self.lane_names(),
            "afc_bowden_length": HUB_BOWDEN_LENGTH,
        })
    }

    /// `afcUnit.get_status` (`AFC_unit.py` L264-278).
    fn unit_json(&self) -> Value {
        json!({
            "lanes": self.lane_names(),
            "extruders": self.heads.iter().map(|head| head.name.as_str()).collect::<Vec<_>>(),
            "hubs": self.hub_names(),
            "buffers": self.buffer_names(),
        })
    }

    /// The settings of the AFC sections. Klipper lowercases section names; `AFC_extruder` records its `extruder_name`
    /// even when the option is missing (`AFC_extruder.py` L222-223: default is the section's own name), and that is
    /// the only place that names the head's Klipper extruder.
    pub fn config_sections(&self) -> Vec<(String, Value)> {
        let mut sections = vec![
            // `AFC.py` L228: without `auto_home` a tool load on an unhomed printer is refused.
            ("afc".to_owned(), json!({ "auto_home": false, "enable_sensors_in_gui": true })),
        ];
        match self.layout {
            AfcLayout::BoxTurtle => self.box_turtle_sections(&mut sections),
            AfcLayout::Toolchanger => self.toolchanger_sections(&mut sections),
        }
        sections
    }

    fn box_turtle_sections(&self, sections: &mut Vec<(String, Value)>) {
        sections.push((
            format!("afc_boxturtle {}", BOX_TURTLE_UNIT.to_lowercase()),
            json!({ "hub": BOX_TURTLE_UNIT, "extruder": "extruder", "buffer": BUFFER_NAME }),
        ));
        for lane in &self.lanes {
            sections.push((
                format!("afc_stepper {}", lane.name),
                json!({
                    "unit": format!("{BOX_TURTLE_UNIT}:{}", lane.index),
                    "dist_hub": lane.dist_hub,
                    "prep": format!("^!{BOX_TURTLE_UNIT}:TRG{}", lane.index),
                    "load": format!("^{BOX_TURTLE_UNIT}:EXT{}", lane.index),
                }),
            ));
        }
        sections.push((
            format!("afc_hub {}", BOX_TURTLE_UNIT.to_lowercase()),
            // `templates/AFC_Turtle_1.cfg` L123.
            json!({
                "switch_pin": format!("^{BOX_TURTLE_UNIT}:HUB"),
                "afc_bowden_length": HUB_BOWDEN_LENGTH,
                "move_dis": 75.0,
                "cut": false,
            }),
        ));
        if let Some(buffer) = self.buffer {
            sections.push((
                format!("afc_buffer {}", buffer.to_lowercase()),
                json!({
                    "type": "switched",
                    "advance_pin": BUFFER_ADVANCE_PIN,
                    "trailing_pin": BUFFER_TRAILING_PIN,
                }),
            ));
        }
        for head in &self.heads {
            sections.push((format!("afc_extruder {}", head.name), head_settings(head)));
        }
    }

    fn toolchanger_sections(&self, sections: &mut Vec<(String, Value)>) {
        sections.push((format!("afc_toolchanger {}", TOOLCHANGER_UNIT.to_lowercase()), json!({})));
        for head in &self.heads {
            let mut settings = head_settings(head);
            // The head injects its lane and fills these options in (`AFC_extruder.py` L327-331).
            settings["toolchanger_unit"] = json!(TOOLCHANGER_UNIT);
            settings["unit"] = json!(TOOLCHANGER_UNIT);
            settings["extruder"] = json!(head.name);
            settings["hub"] = json!(DIRECT_HUB);
            settings["standalone"] = json!(true);
            sections.push((format!("afc_extruder {}", head.name), settings));
        }
    }
}

fn head_settings(head: &Head) -> Value {
    let mut settings = json!({
        "extruder_name": head.extruder_name,
        "tool_stn": head.tool_stn,
        "tool_stn_unload": head.tool_stn_unload,
        "tool_sensor_after_extruder": 0.0,
        "tool_unload_speed": 25.0,
        "tool_load_speed": 25.0,
    });
    if let Some(pin) = head.tool_start {
        settings["pin_tool_start"] = json!(pin);
    }
    if let Some(pin) = head.tool_end {
        settings["pin_tool_end"] = json!(pin);
    }
    if let Some(buffer) = head.buffer {
        settings["buffer"] = json!(buffer);
    }
    settings
}

/// `AFC.py` L1471-1475: `check_homed` refuses without `auto_home`.
fn require_homed(homed: bool) -> Result<(), CommandError> {
    if homed {
        Ok(())
    } else {
        Err(CommandError::new("Please home printer before doing a tool load"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_turtle() -> Afc {
        Afc::new(AfcLayout::BoxTurtle)
    }

    fn lane(afc: &Afc, name: &str) -> Value {
        afc.status()[&format!("AFC_stepper {name}")].clone()
    }

    #[test]
    fn the_box_turtle_starts_with_lane_two_in_the_toolhead() {
        let afc = box_turtle();
        let status = afc.status();

        assert_eq!(status["AFC"]["current_load"], "lane2");
        assert_eq!(status["AFC"]["units"], json!(["Box_Turtle Turtle_1"]));
        assert_eq!(lane(&afc, "lane2")["status"], "Tooled");
        assert_eq!(lane(&afc, "lane2")["spool_id"], 4);
        assert_eq!(lane(&afc, "lane4")["prep"], false);
        assert_eq!(lane(&afc, "lane4")["spool_id"], Value::Null);
        assert_eq!(status["AFC_extruder extruder"]["lane_loaded"], "lane2");
    }

    #[test]
    fn a_tool_change_moves_the_lane_in_the_toolhead() {
        let mut afc = box_turtle();

        afc.change_tool_by_map(2, true).unwrap();

        let status = afc.status();
        assert_eq!(status["AFC"]["current_load"], "lane3");
        assert_eq!(lane(&afc, "lane3")["tool_loaded"], true);
        assert_eq!(lane(&afc, "lane3")["status"], "Tooled");
        assert_eq!(lane(&afc, "lane2")["tool_loaded"], false);
        assert_eq!(lane(&afc, "lane2")["status"], "None");
        assert_eq!(afc.active_spool(), Some(3));
    }

    #[test]
    fn a_tool_change_needs_a_homed_printer_and_a_loaded_lane() {
        let mut afc = box_turtle();

        assert_eq!(
            afc.change_tool_by_map(0, false).unwrap_err().message(),
            "Please home printer before doing a tool load"
        );
        assert!(afc.change_tool_by_map(3, true).unwrap_err().message().starts_with("Current lane not loaded"));
        assert_eq!(afc.status()["AFC"]["current_load"], "lane2");
        assert_eq!(afc.change_tool_by_map(9, true).unwrap_err().message(), "Unknown command:\"T9\"");
    }

    #[test]
    fn set_spool_id_takes_the_filament_of_the_spool_and_refuses_a_spool_of_another_lane() {
        let mut afc = box_turtle();

        afc.apply(AfcCommand::SetSpoolId { lane: Some("lane4".into()), spool_id: Some("6".into()) }, true, false)
            .unwrap();
        afc.apply(AfcCommand::SetSpoolId { lane: Some("lane1".into()), spool_id: Some("6".into()) }, true, false)
            .unwrap();

        assert_eq!(lane(&afc, "lane4")["spool_id"], 6);
        assert_eq!(lane(&afc, "lane4")["material"], "PETG");
        assert_eq!(lane(&afc, "lane4")["color"], "#00CC00");
        assert_eq!(lane(&afc, "lane1")["spool_id"], 1);
    }

    #[test]
    fn a_colour_gets_one_hash_and_a_unknown_lane_is_a_mux_error() {
        let mut afc = box_turtle();

        afc.apply(AfcCommand::SetColor { lane: Some("lane1".into()), color: Some("FF0000".into()) }, true, false)
            .unwrap();

        assert_eq!(lane(&afc, "lane1")["color"], "#FF0000");
        let error =
            afc.apply(AfcCommand::SetColor { lane: Some("lane9".into()), color: None }, true, false).unwrap_err();
        assert!(error.message().starts_with("The value 'lane9' is not valid for LANE"));
    }

    #[test]
    fn set_map_swaps_the_commands_of_two_lanes() {
        let mut afc = box_turtle();

        afc.apply(AfcCommand::SetMap { lane: Some("lane1".into()), map: Some("t1".into()) }, true, false).unwrap();

        assert_eq!(lane(&afc, "lane1")["map"], "T1");
        assert_eq!(lane(&afc, "lane2")["map"], "T0");
        assert_eq!(afc.status()["AFC"]["maps"], json!(["T0", "T1", "T2", "T3"]));
    }

    #[test]
    fn unloading_the_toolhead_and_ejecting_a_lane() {
        let mut afc = box_turtle();

        afc.apply(AfcCommand::ToolUnload { lane: None }, true, false).unwrap();
        afc.apply(AfcCommand::LaneUnload { lane: Some("lane1".into()) }, true, false).unwrap();

        assert_eq!(afc.status()["AFC"]["current_load"], Value::Null);
        assert_eq!(afc.active_spool(), None);
        assert_eq!(lane(&afc, "lane1")["load"], false);
        assert_eq!(lane(&afc, "lane1")["spool_id"], Value::Null);
        assert_eq!(lane(&afc, "lane2")["load"], true);
        assert!(afc
            .apply(AfcCommand::LaneUnload { lane: Some("lane3".into()) }, true, true)
            .unwrap_err()
            .message()
            .contains("printing"));
    }

    #[test]
    fn the_tool_changer_moves_the_shuttle_with_the_tool_change() {
        let mut afc = Afc::new(AfcLayout::Toolchanger);
        assert_eq!(afc.shuttle_extruder(), Some("extruder1"));

        afc.change_tool_by_map(0, true).unwrap();

        let status = afc.status();
        assert_eq!(afc.shuttle_extruder(), Some("extruder"));
        assert_eq!(status["AFC"]["current_load"], "e0");
        assert_eq!(status["AFC_extruder e0"]["on_shuttle"], true);
        assert_eq!(status["AFC_extruder e1"]["on_shuttle"], false);
        assert_eq!(status["AFC_extruder e1"]["lane_loaded"], Value::Null);
        assert_eq!(status["AFC_lane e0"]["status"], "Tooled");
    }
}
