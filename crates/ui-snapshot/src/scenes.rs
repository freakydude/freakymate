//! The run, split into scenes that can be chosen on the command line.

use crate::session::Session;
use freakymate::{
    Actions, Confirm, FilamentTab, HeatTab, MeshDialogs, MmuDialogs, MoveTab, Navigation, Page, Printer, SpoolPicker,
    TuneTab,
};
use freakymate_core::network::{Availability, NetworkCommand, NetworkState};
use freakymate_core::sources::happy_hare::ServoPosition;
use freakymate_core::sources::klipper::tuning::TuningKey;
use freakymate_core::sources::klipper::{Activity, CalibrationMode, KlippyState, PrintState, TestZ};
use freakymate_core::sources::moonraker_server::wled::{self, StripAction};
use freakymate_core::{Command, ConnectionState, Query, Snapshot};
use slint::{ComponentHandle, Model};
use std::time::Duration;

/// The fake's idle timeout passes after about 30 s at time scale 20.
const IDLE_WAIT_LIMIT: Duration = Duration::from_secs(60);
/// "Log in" on the connection page (800x480 layout).
const LOG_IN_BUTTON: (f32, f32) = (656.0, 230.0);

/// Scenes run in this order; each one leaves the printer ready for the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// Every page in standby, with the file list.
    Pages,
    /// Spoolman: the spool in use, the temperature hint, the picker and the hints before a print.
    Spools,
    /// Settings > Network: list, connecting, confirmation, read-only fallback.
    Network,
    /// Taps that last longer than a status update still count while the temperatures change.
    Taps,
    /// Editing keys in the text entry of the console, physical and on-screen.
    Typing,
    /// The Calibrate tab of the Move page: results of the bed mesh and the screws, the dialogs of the waiting modes.
    Calibrate,
    /// Print running, system page locked, cancel.
    Printing,
    /// Emergency stop and firmware restart.
    Shutdown,
    /// Idle timeout; slow, therefore not part of the default selection.
    Idle,
    /// Moonraker wants a login: refused, accepted, logged in on the settings, logged out. Needs a fake started with
    /// `--auth test:secret`, so it is only run when chosen by name.
    Login,
}

impl Scene {
    pub const ALL: [Self; 10] = [
        Self::Pages,
        Self::Spools,
        Self::Network,
        Self::Taps,
        Self::Typing,
        Self::Calibrate,
        Self::Printing,
        Self::Shutdown,
        Self::Idle,
        Self::Login,
    ];
    pub const DEFAULT: [Self; 8] = [
        Self::Pages,
        Self::Spools,
        Self::Network,
        Self::Taps,
        Self::Typing,
        Self::Calibrate,
        Self::Printing,
        Self::Shutdown,
    ];

    /// Parses `pages,network` or `all` (every scene that runs against a fake without login).
    pub fn parse_list(text: &str) -> Result<Vec<Self>, String> {
        if text == "all" {
            return Ok(Self::ALL.into_iter().filter(|scene| *scene != Self::Login).collect());
        }
        text.split(',').map(|name| Self::parse(name.trim())).collect()
    }

    fn parse(name: &str) -> Result<Self, String> {
        Self::ALL.into_iter().find(|scene| scene.name() == name).ok_or_else(|| {
            format!(
                "unknown scene '{name}' (pages, spools, network, taps, typing, calibrate, printing, shutdown, idle, \
                 login, all)"
            )
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Pages => "pages",
            Self::Spools => "spools",
            Self::Network => "network",
            Self::Taps => "taps",
            Self::Typing => "typing",
            Self::Calibrate => "calibrate",
            Self::Printing => "printing",
            Self::Shutdown => "shutdown",
            Self::Idle => "idle",
            Self::Login => "login",
        }
    }

    fn run(self, session: &mut Session) -> Result<(), String> {
        match self {
            Self::Pages => pages(session),
            Self::Spools => spools(session),
            Self::Network => network(session),
            Self::Taps => taps(session),
            Self::Typing => typing(session),
            Self::Calibrate => calibrate(session),
            Self::Printing => printing(session),
            Self::Shutdown => shutdown(session),
            Self::Idle => idle(session),
            Self::Login => logged_in(session),
        }
    }
}

/// Connects (logging in first for the scene `login`), homes and loads host data, then runs the chosen scenes in their
/// fixed order.
pub fn run(session: &mut Session, chosen: &[Scene]) -> Result<(), String> {
    session.capture("connecting")?;
    if chosen.contains(&Scene::Login) {
        log_in(session)?;
    }
    session.wait_until("printer ready", |snapshot| {
        snapshot.connection == ConnectionState::Connected && snapshot.status.klippy_state == KlippyState::Ready
    })?;
    session.service.send(Command::Home { x: true, y: true, z: true });
    session.service.query(Query::PowerDevices);
    session.service.query(Query::Updates);
    session.service.query(Query::ConsoleHistory);
    session.service.send(Command::RunGcode { script: "STATUS".into() });
    session.service.send(Command::RunGcode { script: "HELP".into() });
    session.wait_until("homing", |snapshot| snapshot.status.motion.homed.all())?;

    for scene in Scene::ALL.into_iter().filter(|scene| chosen.contains(scene)) {
        scene.run(session)?;
    }
    Ok(())
}

fn pages(session: &mut Session) -> Result<(), String> {
    session.show_page(Page::Print, "print-standby")?;
    session.presenter.request_files();
    session.window.global::<Navigation>().set_files_open(true);
    session.settle_replies(Duration::from_secs(2));
    session.capture("print-files")?;
    session.window.global::<Navigation>().set_files_open(false);
    // The console comes before the Thermal page, whose fan test would fill it with the fake's commands.
    for (page, name) in [
        (Page::Move, "move"),
        (Page::Console, "console"),
        (Page::Temperature, "temperature"),
        (Page::Tune, "tune"),
        (Page::Filament, "filament"),
        (Page::System, "system"),
        (Page::Updates, "updates"),
        (Page::Settings, "settings"),
    ] {
        session.show_page(page, name)?;
        if page == Page::Temperature {
            heat_tabs(session)?;
        }
        let has_tuning = session.presenter.snapshot().sources.klipper.tuning.get(TuningKey::MaxAccel).is_some();
        if page == Page::Tune && has_tuning {
            session.window.global::<Navigation>().set_tune_tab(TuneTab::Expert);
            session.show_page(Page::Tune, "tune-expert")?;
            session.window.global::<Navigation>().set_tune_tab(TuneTab::Basics);
        }
        if page == Page::Filament && session.presenter.snapshot().status.mmu.is_some() {
            session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
            session.show_page(Page::Filament, "filament-mmu")?;
            mmu_service(session)?;
            mmu_gate(session)?;
            mmu_tools_and_groups(session)?;
            mmu_bypass(session)?;
            session.window.global::<Navigation>().set_filament_tab(FilamentTab::Extruder);
            session.show_page(Page::Filament, "filament-extruder")?;
            session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
        }
    }
    messages(session)?;
    command_running(session)
}

/// Where taps land in the 800x480 layout: the tile of gate 2 on the Gates tab, and the button "Select" of the gate
/// dialog.
const GATE_2_TILE: (f32, f32) = (450.0, 190.0);
const GATE_DIALOG_SELECT: (f32, f32) = (172.0, 230.0);

/// Service tab of the MMU, first as it opens, then after the buttons for the servo and the motors went through the app
/// to the fake: the servo is down and Home stands out again. Homes the MMU, lifts the servo and returns to the Gates tab
/// afterwards.
fn mmu_service(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Service);
    session.show_page(Page::Filament, "filament-mmu-service")?;
    {
        let actions = session.window.global::<Actions>();
        actions.invoke_mmu_servo_down();
        actions.invoke_mmu_motors_off();
    }
    session.wait_until("servo down and MMU not homed", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| !mmu.homed && mmu.servo == Some(ServoPosition::Down))
    })?;
    session.show_page(Page::Filament, "filament-mmu-service-motors-off")?;
    if session.window.global::<Printer>().get_mmu_servo() != "down" {
        return Err("the service tab does not show the servo down".into());
    }
    {
        let actions = session.window.global::<Actions>();
        actions.invoke_mmu_home();
        actions.invoke_mmu_servo_up();
    }
    session.wait_until("MMU homed with the servo up", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.homed && mmu.servo == Some(ServoPosition::Up))
    })?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    Ok(())
}

/// Where taps land in the 800x480 layout: the tile of T3 on the Tools tab, and gate 1 in the choice of gate.
const TOOL_3_TILE: (f32, f32) = (380.0, 240.0);
const TOOL_DIALOG_GATE_1: (f32, f32) = (300.0, 166.0);

/// Tools and EndlessSpool groups through the app to the fake, both set on the Tools tab only: T3 moves to gate 1 and
/// gate 1 into the group of gate 0, with EndlessSpool on. In the 800x480 layout the move of T3 goes through taps:
/// T3 opens the choice of gate, gate 1 maps it and closes the dialog. Everything goes back afterwards.
fn mmu_tools_and_groups(session: &mut Session) -> Result<(), String> {
    let tool_3_gate =
        |session: &Session| session.presenter.snapshot().status.mmu.and_then(|mmu| mmu.tool_to_gate.get(3).copied());
    session.window.global::<Actions>().invoke_mmu_set_endless_spool(true);
    session
        .wait_until("EndlessSpool on", |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.endless_spool))?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Tools);
    if session.has_reference_layout() {
        session.window.global::<Navigation>().set_page(Page::Filament);
        session.pump(Duration::from_millis(300));
        let before = tool_3_gate(session);
        let (x, y) = TOOL_3_TILE;
        session.tap(x, y);
        session.pump(Duration::from_millis(500));
        if session.window.global::<MmuDialogs>().get_tool() != 3 || tool_3_gate(session) != before {
            return Err("a tap on the tile of T3 does not open the choice of gate, or it maps the tool".into());
        }
        session.show_page(Page::Filament, "filament-mmu-tool-dialog")?;
        let (x, y) = TOOL_DIALOG_GATE_1;
        session.tap(x, y);
    } else {
        session.window.global::<Actions>().invoke_mmu_map_tool(3, 1);
    }
    session.wait_until("T3 on gate 1", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.tool_to_gate.get(3) == Some(&1))
    })?;
    if session.window.global::<MmuDialogs>().get_tool() != -1 {
        return Err("the choice of gate stays open after a gate was tapped".into());
    }
    session.window.global::<Actions>().invoke_mmu_set_endless_spool_group(1, 0);
    session.wait_until("gate 1 in group 0", |snapshot| {
        snapshot
            .status
            .mmu
            .as_ref()
            .is_some_and(|mmu| mmu.gates.get(1).and_then(|gate| gate.endless_spool_group) == Some(0))
    })?;
    session.show_page(Page::Filament, "filament-mmu-tools")?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    {
        let actions = session.window.global::<Actions>();
        actions.invoke_mmu_reset_tool_map();
        actions.invoke_mmu_set_endless_spool(false);
    }
    session.wait_until("tools back on their gates, EndlessSpool off", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| {
            !mmu.endless_spool && mmu.tool_to_gate.iter().enumerate().all(|(tool, gate)| tool == *gate)
        })
    })
}

/// The bypass: its dialog, then selected through the app; the fake refuses it while filament is loaded, so the scene
/// runs with the filament unloaded. Gate 2 is selected again afterwards.
fn mmu_bypass(session: &mut Session) -> Result<(), String> {
    session.window.global::<MmuDialogs>().set_bypass_open(true);
    session.show_page(Page::Filament, "filament-mmu-bypass")?;
    session.window.global::<MmuDialogs>().set_bypass_open(false);
    session.window.global::<Actions>().invoke_mmu_select_bypass();
    session.wait_until("bypass selected", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.bypass_selected())
    })?;
    if !session.window.global::<Printer>().get_mmu_bypass() {
        return Err("the Gates tab does not show the bypass as selected".into());
    }
    session.window.global::<Navigation>().set_mmu_page(1);
    session.show_page(Page::Filament, "filament-mmu-bypass-selected")?;
    session.window.global::<Navigation>().set_mmu_page(0);
    session.service.send(Command::MmuSelectGate { gate: 2 });
    session
        .wait_until("gate 2 selected again", |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.gate == 2))
}

/// Dialog of a gate. In the 800x480 layout a tap on the tile of gate 2 opens it without moving the selector, and its
/// button "Select" selects the gate and closes it. Without Spoolman (profile `ercf`) material and colour go to
/// Happy Hare's gate map and the dialog marks what Happy Hare reports back.
fn mmu_gate(session: &mut Session) -> Result<(), String> {
    let selected_gate = |session: &Session| session.presenter.snapshot().status.mmu.map_or(-1, |mmu| mmu.gate);
    if session.has_reference_layout() {
        let before = selected_gate(session);
        let (x, y) = GATE_2_TILE;
        session.tap(x, y);
        session.pump(Duration::from_millis(500));
        if session.window.global::<MmuDialogs>().get_gate() != 2 || selected_gate(session) != before {
            return Err("a tap on the tile of gate 2 does not open its dialog, or it moves the selector".into());
        }
        session.show_page(Page::Filament, "filament-mmu-gate")?;
        let (x, y) = GATE_DIALOG_SELECT;
        session.tap(x, y);
        session.wait_until("gate 2 selected in its dialog", |snapshot| {
            snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.gate == 2)
        })?;
        if session.window.global::<MmuDialogs>().get_gate() != -1 {
            return Err("the gate dialog stays open after Select".into());
        }
    }
    if session.window.global::<Printer>().get_has_spoolman() {
        return Ok(());
    }
    session.window.global::<MmuDialogs>().set_gate(1);
    {
        let actions = session.window.global::<Actions>();
        actions.invoke_set_gate_material(1, "PETG".into());
        actions.invoke_set_gate_color(1, slint::Color::from_rgb_u8(0xff, 0x80, 0x00));
    }
    session.wait_until("PETG in orange in gate 1", |snapshot| {
        let gate = snapshot.status.mmu.as_ref().and_then(|mmu| mmu.gates.get(1).cloned()).unwrap_or_default();
        gate.material == "PETG" && gate.color == Some([0xff, 0x80, 0x00])
    })?;
    session.show_page(Page::Filament, "filament-mmu-gate-local")?;
    session.window.global::<MmuDialogs>().set_gate(-1);
    Ok(())
}

/// A command that waits long for its answer: the status bar shows the turning wheel and the command until the answer
/// is there. The fake answers `G4` only after the dwell.
fn command_running(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::RunGcode { script: "G4 P2500".into() });
    session.pump(Duration::from_millis(1300));
    if session.window.global::<Printer>().get_running_command() != "G4 P2500" {
        return Err("the status bar does not show the running command".into());
    }
    session.show_page(Page::Print, "command-running")?;
    session.pump(Duration::from_millis(1500));
    if !session.window.global::<Printer>().get_running_command().is_empty() {
        return Err("the status bar still shows the command after its answer".into());
    }
    Ok(())
}

/// The filament path during a load: the fake holds the filament in the bowden, then lets it go back to the gate.
fn filament_path_midway(session: &mut Session) -> Result<(), String> {
    let at = |position: i32| {
        move |snapshot: &freakymate_core::Snapshot| {
            snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.filament_pos == position)
        }
    };
    session.service.send(Command::RunGcode { script: "FAKE_MMU_FILAMENT POS=3 DIRECTION=1".into() });
    session.wait_until("filament in the bowden", at(3))?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    session.show_page(Page::Filament, "filament-mmu-loading")?;
    session.service.send(Command::RunGcode { script: "FAKE_MMU_FILAMENT POS=0 DIRECTION=0".into() });
    session.wait_until("filament back in the gate", at(0))
}

/// The message flyout with an error from a command the printer refuses.
fn messages(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::RunGcode { script: "SET_FAN_SPEED FAN=missing SPEED=1".into() });
    session.pump(Duration::from_secs(1));
    session.window.global::<Navigation>().set_messages_open(true);
    session.capture("messages")?;
    session.window.global::<Navigation>().set_messages_open(false);
    // The scenes after it should not show this made-up error in the status bar.
    session.presenter.clear_messages();
    session.pump(Duration::from_millis(200));
    Ok(())
}

/// The exhaust fan measures at 50 %, then its tachometer goes silent: red after a while; afterwards off again. The
/// core only learns a reading some seconds after a change of speed (kick start), so this scene waits that long.
fn stalled_fan(session: &mut Session) -> Result<(), String> {
    let exhaust_stalled =
        |snapshot: &Snapshot| snapshot.status.fans.iter().any(|fan| fan.object == "fan_generic exhaust" && fan.stalled);
    session.service.send(Command::RunGcode { script: "SET_FAN_SPEED FAN=exhaust SPEED=0.5".into() });
    session.wait_until("the exhaust fan to measure", |snapshot| {
        snapshot
            .status
            .fans
            .iter()
            .any(|fan| fan.object == "fan_generic exhaust" && fan.rpm.is_some_and(|rpm| rpm > 0.0))
    })?;
    // Settling time of the core (3 s plus Klipper's default kick start of 0.1 s) and a little more.
    session.pump(Duration::from_millis(4500));
    session.service.send(Command::RunGcode { script: "FAKE_FAN_STALL FAN=exhaust".into() });
    session.wait_until("the exhaust fan to count as stalled", exhaust_stalled)?;
    session.show_page(Page::Temperature, "temperature-fan-stalled")?;
    session.service.send(Command::RunGcode {
        script: "FAKE_FAN_STALL FAN=exhaust STALL=0\nSET_FAN_SPEED FAN=exhaust SPEED=0".into(),
    });
    session.wait_until("the exhaust fan to be off", |snapshot| {
        !exhaust_stalled(snapshot)
            && snapshot.status.fans.iter().any(|fan| fan.object == "fan_generic exhaust" && fan.speed == 0.0)
    })
}

/// The Fans and Sensors tabs of the Thermal page and the light panel, where the printer has them.
fn heat_tabs(session: &mut Session) -> Result<(), String> {
    let status = session.presenter.snapshot().status;
    let navigation = session.window.global::<Navigation>();
    if !status.fans.is_empty() || !status.leds.is_empty() {
        navigation.set_heat_tab(HeatTab::Fans);
        session.show_page(Page::Temperature, "temperature-fans")?;
        if status.fans.iter().any(|fan| fan.object == "fan_generic exhaust") {
            stalled_fan(session)?;
        }
    }
    if !status.sensors.is_empty() {
        session.window.global::<Navigation>().set_heat_tab(HeatTab::Sensors);
        session.show_page(Page::Temperature, "temperature-sensors")?;
    }
    session.window.global::<Navigation>().set_heat_tab(HeatTab::Heaters);
    if !status.leds.is_empty() {
        // A WLED strip switched on with its preset: on, with the brightness unknown.
        for strip in session.presenter.snapshot().sources.wled_strips.iter().filter_map(|led| wled::strip_name(led)) {
            session.service.send(Command::SetWledStrip { strip: strip.to_owned(), action: StripAction::On });
        }
        session.pump(Duration::from_millis(500));
        session.window.global::<Navigation>().set_light_open(true);
        session.capture("light-panel")?;
        session.window.global::<Navigation>().set_light_open(false);
    }
    Ok(())
}

/// The temperature history of the nozzle, once it has heated: Moonraker's stored minutes plus the live value.
fn nozzle_history(session: &mut Session) -> Result<(), String> {
    session.presenter.open_history("extruder");
    session.pump(Duration::from_secs(1));
    session.capture("history-nozzle")?;
    session.presenter.close_history();
    Ok(())
}

/// Without Spoolman in Moonraker there is nothing to show and the scene is skipped.
fn spools(session: &mut Session) -> Result<(), String> {
    session.settle_replies(Duration::from_secs(2));
    let snapshot = session.presenter.snapshot();
    if !snapshot.sources.moonraker.spoolman {
        return Ok(());
    }
    let has_mmu = snapshot.status.mmu.is_some();
    if has_mmu {
        session.service.send(Command::MmuHome);
        session.wait_until("MMU homed", |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.homed))?;
        session.service.send(Command::MmuSelectGate { gate: 1 });
        session
            .wait_until("gate 1 selected", |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.gate == 1))?;
        filament_path_midway(session)?;
    } else {
        session.service.send(Command::SetActiveSpool { spool_id: Some(6) });
    }
    session.settle_replies(Duration::from_secs(2));
    session.service.send(Command::SetHeaterTarget { heater: "extruder".into(), target: 210.0 });
    session.pump(Duration::from_secs(1));
    session.show_page(Page::Print, "print-spool")?;
    session.show_page(Page::Temperature, "temperature-spool")?;

    session.window.global::<Navigation>().set_page(Page::Print);
    // Opens like a tap on the strip or gate 1: the spool in use is selected.
    let current = session.window.global::<Printer>().get_spool_id();
    session.window.global::<SpoolPicker>().invoke_show(if has_mmu { 1 } else { -1 }, current);
    session.settle_replies(Duration::from_secs(2));
    session.capture("spool-picker")?;
    session.window.global::<SpoolPicker>().set_open(false);

    session.presenter.request_files();
    session.settle_replies(Duration::from_secs(2));
    let hints = session.presenter.print_hints_text("long_job.gcode");
    session.window.global::<Confirm>().invoke_ask(
        "Start print".into(),
        format!("long_job{hints}").into(),
        "start-print".into(),
        "long_job.gcode".into(),
        false,
    );
    session.pump(Duration::from_millis(300));
    session.capture("confirm-start-hints")?;
    session.window.global::<Confirm>().set_open(false);
    session.service.send(Command::SetHeaterTarget { heater: "extruder".into(), target: 0.0 });
    session.pump(Duration::from_millis(500));
    Ok(())
}

fn network(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_page(Page::Network);
    session.network_service.send(NetworkCommand::Refresh);
    session.pump(Duration::from_millis(500));
    session.capture("network")?;

    session.network_service.send(NetworkCommand::Connect { ssid: "Cafe Guest".into(), password: None, hidden: false });
    session.pump(Duration::from_millis(200));
    session.capture("network-connecting")?;
    session.settle_replies(Duration::from_secs(2));

    ask(session, "Remove network", "The saved password of \"Garage\" is deleted.", "forget-network", "Garage");
    session.capture("network-forget")?;
    session.window.global::<Confirm>().set_open(false);

    session.network.apply_state(NetworkState {
        availability: Availability::Unavailable("NetworkManager not reachable".into()),
        ..NetworkState::default()
    });
    session.service.query(Query::HostNetwork);
    session.pump(Duration::from_secs(1));
    session.capture("network-unavailable")?;
    session.network_service.send(NetworkCommand::Refresh);
    Ok(())
}

/// The "Cool down" button of the nozzle row on the Heat page, below the tabs (800x480, no fitting).
const COOL_DOWN_BUTTON: (f32, f32) = (628.0, 148.0);
const TAP_ROUNDS: usize = 10;
/// A finger rests this long: several status updates arrive meanwhile, as the temperatures change all the time.
const TAP_DURATION: Duration = Duration::from_millis(500);

/// A tap that spans status updates must still count: the heater rows may not be rebuilt under the finger. Checked
/// in the reference layout only, where the button lies at a known place.
fn taps(session: &mut Session) -> Result<(), String> {
    if !session.has_reference_layout() {
        return Ok(());
    }
    session.window.global::<Navigation>().set_page(Page::Temperature);
    let heating = |target| Command::SetHeaterTarget { heater: "extruder".into(), target };
    let target = |snapshot: &freakymate_core::Snapshot| snapshot.status.heater("extruder").map(|heater| heater.target);
    let mut missed = 0;
    for _ in 0..TAP_ROUNDS {
        session.service.send(heating(200.0));
        session.wait_until("nozzle target", |snapshot| target(snapshot) == Some(200.0))?;
        session.pump(Duration::from_millis(300));
        let (x, y) = COOL_DOWN_BUTTON;
        session.press(x, y);
        session.pump(TAP_DURATION);
        session.release(x, y);
        session.pump(Duration::from_millis(100));
        if target(&session.presenter.snapshot()) != Some(0.0) {
            missed += 1;
        }
    }
    if missed > 0 {
        return Err(format!("{missed} of {TAP_ROUNDS} taps on Cool down were lost while the temperatures changed"));
    }
    Ok(())
}

/// The Layer tile of the print page and the main button of the layer pause dialog (800x480, no fitting).
const LAYER_TILE: (f32, f32) = (580.0, 133.0);
const LAYER_PAUSE_BUTTON: (f32, f32) = (440.0, 320.0);
/// How far ahead the scene plans a pause first, so that it shows without pausing.
const PLANNED_LAYERS_AHEAD: i32 = 40;
/// The nozzle temperature of the status bar while a print runs and no message is shown (800x480, no fitting).
const STATUS_BAR_NOZZLE: (f32, f32) = (440.0, 22.0);
/// The Objects button of the paused print page, then in its dialog: the fake's BENCHY_1 on the bed map (centre
/// 200/160 mm on a 300 mm bed, map at 96/128 with 268 px), the Exclude button and the close cross (800x480).
const OBJECTS_BUTTON: (f32, f32) = (740.0, 204.0);
const MAP_BENCHY_1: (f32, f32) = (275.0, 253.0);
const EXCLUDE_BUTTON: (f32, f32) = (560.0, 432.0);
const CLOSE_OBJECTS_BUTTON: (f32, f32) = (752.0, 92.0);
/// The light symbol of the status bar while a print runs and a message is shown (800x480, no fitting).
const STATUS_BAR_LIGHT: (f32, f32) = (641.0, 22.0);

/// The command field of the console (800x480, no fitting).
const COMMAND_FIELD: (f32, f32) = (480.0, 444.0);

/// Keys of the on-screen keyboard, whose rows fill the dialog (800x480, no fitting).
const ON_SCREEN_Q: (f32, f32) = (128.0, 226.0);
const ON_SCREEN_S: (f32, f32) = (197.0, 293.0);
const ON_SCREEN_BACKSPACE: (f32, f32) = (628.0, 428.0);
const ON_SCREEN_ENTER: (f32, f32) = (731.0, 428.0);

/// The text entry edits like any text field: Backspace, Delete, Home, End, Shift and Ctrl are the keyboard's own
/// business. The result is read from the command that the entry sends. Checked in the reference layout only, where
/// the field and the keys lie at known places.
fn typing(session: &mut Session) -> Result<(), String> {
    use slint::platform::Key;
    if !session.has_reference_layout() {
        return Ok(());
    }
    session.window.global::<Navigation>().set_page(Page::Console);
    session.pump(Duration::from_millis(300));
    let (x, y) = COMMAND_FIELD;
    session.press(x, y);
    session.release(x, y);
    session.pump(Duration::from_millis(300));
    session.capture("text-entry")?;

    // Physical keyboard: "XSTATUZ" -> Backspace -> "S" -> Home -> Delete (removes the X) -> End -> Enter.
    for character in "XSTATUZ".chars() {
        session.key(character.to_string());
    }
    session.key(Key::Backspace);
    session.key("S");
    session.key(Key::Home);
    session.key(Key::Delete);
    session.key(Key::End);
    session.capture("text-entry-typed")?;
    session.key(Key::Return);
    expect_console_command(session, "STATUS")?;

    // On-screen keys: the field must keep the focus while they are tapped. Q, backspace, S, enter.
    session.tap(x, y);
    session.pump(Duration::from_millis(300));
    for (key_x, key_y) in [ON_SCREEN_Q, ON_SCREEN_BACKSPACE, ON_SCREEN_S, ON_SCREEN_ENTER] {
        session.tap(key_x, key_y);
    }
    expect_console_command(session, "S")?;

    // Escape closes without sending, and a later Enter of the physical keyboard must not send the hidden text.
    session.tap(x, y);
    session.pump(Duration::from_millis(300));
    session.key("H");
    session.key(Key::Escape);
    session.pump(Duration::from_millis(300));
    session.key(Key::Return);
    expect_no_console_command(session, "H", "Escape, then Enter")?;

    // The same after a tap on the status bar, which cancels the entry.
    session.tap(x, y);
    session.pump(Duration::from_millis(300));
    session.key("J");
    let (light_x, light_y) = STATUS_BAR_LIGHT;
    session.tap(light_x, light_y);
    session.window.global::<Navigation>().set_light_open(false);
    session.pump(Duration::from_millis(300));
    session.key(Key::Return);
    expect_no_console_command(session, "J", "a tap on the status bar, then Enter")
}

/// A command that was typed into a closed entry must not reach the printer.
fn expect_no_console_command(session: &mut Session, command: &str, after: &str) -> Result<(), String> {
    session.pump(Duration::from_millis(500));
    if session.window.global::<Printer>().get_console().iter().any(|line| line.is_command && line.text == command) {
        return Err(format!("{after} sent the command {command} of the closed entry"));
    }
    Ok(())
}

/// The command appears as the newest line of the console.
fn expect_console_command(session: &mut Session, command: &str) -> Result<(), String> {
    session.pump(Duration::from_millis(500));
    let lines = session.window.global::<Printer>().get_console();
    let last = lines.iter().last();
    if last.as_ref().is_some_and(|line| line.is_command && line.text == command) {
        return Ok(());
    }
    Err(format!(
        "expected the console to end with the command {command}, it ends with {:?}",
        last.map(|line| line.text)
    ))
}

/// The Move entry of the navigation (800x480, no fitting).
const NAV_MOVE: (f32, f32) = (38.0, 155.0);

/// Measures what the printer can measure, then shows each waiting mode in its dialog and leaves it; at the end the
/// changes are saved, so the scenes after it start without pending ones.
fn calibrate(session: &mut Session) -> Result<(), String> {
    let offered = session.presenter.snapshot().sources.klipper.calibration;
    session.window.global::<Navigation>().set_move_tab(MoveTab::Calibrate);
    session.show_page(Page::Move, "calibrate")?;
    if offered.bed_mesh {
        session.service.send(Command::CalibrateBedMesh);
        session.wait_until("bed mesh", |snapshot| {
            snapshot.status.calibration.bed_mesh.as_ref().is_some_and(|mesh| !mesh.active.is_empty())
        })?;
    }
    if offered.screws_tilt {
        session.service.send(Command::CalculateScrewsTilt);
        session.wait_until("screw results", |snapshot| !snapshot.status.calibration.screws.is_empty())?;
    }
    session.pump(Duration::from_millis(300));
    session.capture("calibrate-results")?;
    if offered.bed_mesh {
        bed_mesh_dialogs(session)?;
    }

    session.service.send(Command::CalibrateZ);
    session.wait_until("manual probe", |snapshot| snapshot.status.calibration.mode.is_some())?;
    session.service.send(Command::TestZ { step: TestZ::By(-1.0) });
    session.service.send(Command::TestZ { step: TestZ::BisectUp });
    session.wait_until("heights tried", |snapshot| {
        matches!(snapshot.status.calibration.mode, Some(CalibrationMode::ManualProbe { lower: Some(_), .. }))
    })?;
    session.pump(Duration::from_millis(300));
    session.capture("calibrate-probe")?;
    session.service.send(Command::AcceptCalibration);
    session.wait_until("probe accepted", |snapshot| snapshot.status.calibration.mode.is_none())?;
    session.pump(Duration::from_millis(300));
    session.capture("calibrate-pending")?;

    if offered.bed_screws {
        session.service.send(Command::AdjustBedScrews);
        session.wait_until("bed screws", |snapshot| snapshot.status.calibration.mode.is_some())?;
        session.service.send(Command::AcceptCalibration);
        session.wait_until("first screw accepted", |snapshot| {
            matches!(snapshot.status.calibration.mode, Some(CalibrationMode::BedScrews { accepted: 1, .. }))
        })?;
        session.pump(Duration::from_millis(300));
        session.capture("calibrate-screws")?;
        // Closing the dialog, here by a tap on the navigation, leaves the mode.
        if session.has_reference_layout() {
            let (x, y) = NAV_MOVE;
            session.tap(x, y);
        } else {
            session.service.send(Command::AbortCalibration);
        }
        session.wait_until("bed screws left", |snapshot| snapshot.status.calibration.mode.is_none())?;
    }
    session.service.send(Command::SaveConfig);
    session.wait_until("saved", |snapshot| !snapshot.status.save_config_pending)?;
    session.window.global::<Navigation>().set_move_tab(MoveTab::Axes);
    Ok(())
}

/// The tiles of the bed mesh card on the Calibrate tab (800x480, no fitting): the map and the profile selector.
const MESH_MAP_TILE: (f32, f32) = (660.0, 178.0);
const MESH_PROFILE_TILE: (f32, f32) = (660.0, 240.0);

/// The map of the measured mesh with its faulty region, then the profiles: one saved under a new name and removed
/// again, so the scenes after it see the profiles they started with.
fn bed_mesh_dialogs(session: &mut Session) -> Result<(), String> {
    if session.has_reference_layout() {
        let (x, y) = MESH_MAP_TILE;
        session.tap(x, y);
    } else {
        session.window.global::<MeshDialogs>().set_map_open(true);
    }
    session.pump(Duration::from_millis(300));
    if !session.window.global::<MeshDialogs>().get_map_open() {
        return Err("a tap on the mesh thumbnail did not open the map".into());
    }
    session.capture("calibrate-mesh")?;
    session.window.global::<MeshDialogs>().set_map_open(false);

    if session.has_reference_layout() {
        let (x, y) = MESH_PROFILE_TILE;
        session.tap(x, y);
    } else {
        session.window.global::<MeshDialogs>().set_profiles_open(true);
    }
    session.window.global::<Actions>().invoke_save_bed_mesh("smooth-pei".into());
    session.wait_until("profile saved", |snapshot| has_mesh_profile(snapshot, "smooth-pei"))?;
    session.pump(Duration::from_millis(300));
    if !session.window.global::<MeshDialogs>().get_profiles_open() {
        return Err("a tap on the profile selector did not open the profiles".into());
    }
    session.capture("calibrate-mesh-profiles")?;
    session.window.global::<Actions>().invoke_remove_bed_mesh("smooth-pei".into());
    session.wait_until("profile removed", |snapshot| !has_mesh_profile(snapshot, "smooth-pei"))?;
    session.window.global::<MeshDialogs>().set_profiles_open(false);
    Ok(())
}

fn has_mesh_profile(snapshot: &Snapshot, name: &str) -> bool {
    snapshot
        .status
        .calibration
        .bed_mesh
        .as_ref()
        .is_some_and(|mesh| mesh.profiles.iter().any(|profile| profile == name))
}

fn printing(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::StartPrint { filename: "benchy.gcode".into() });
    session.wait_until("print running", |snapshot| {
        snapshot.status.job.state == PrintState::Printing && snapshot.status.job.progress > 0.005
    })?;
    session.pump(Duration::from_secs(1));
    session.show_page(Page::Print, "print-printing")?;
    temperature_leads_to_heaters(session)?;
    nozzle_history(session)?;
    tangle_boost(session)?;
    session.show_page(Page::Temperature, "temperature-printing")?;
    session.show_page(Page::System, "system-printing")?;

    session.window.global::<Navigation>().set_page(Page::Print);
    pause_at_next_layer(session)?;
    session.capture("print-paused")?;
    exclude_object(session)?;
    session.service.send(Command::ResumePrint);
    session.wait_until("print resumed", |snapshot| snapshot.status.job.state == PrintState::Printing)?;
    ask(session, "Cancel print", "The current print will be stopped and cannot be resumed.", "cancel-print", "");
    session.capture("confirm-cancel")?;
    session.window.global::<Confirm>().invoke_execute();
    session.wait_until("print cancelled", |snapshot| snapshot.status.job.state == PrintState::Cancelled)?;
    wait_for_end_time(session)?;
    session.capture("print-cancelled")?;
    print_error(session)
}

/// A pause planned layers ahead shows at the Layer tile and in its dialog (the fake has Mainsail's macros and their
/// variables). The dialog's main button then plans a pause at the next layer, and the fake pauses there. Another
/// layout sends the command instead of tapping.
fn pause_at_next_layer(session: &mut Session) -> Result<(), String> {
    let current = session.window.global::<Printer>().get_current_layer();
    let ahead = u32::try_from(current + PLANNED_LAYERS_AHEAD).unwrap_or_default();
    session.service.send(Command::PauseAtLayer { layer: ahead });
    session.wait_until("pause planned ahead", |snapshot| {
        snapshot.status.layer_pause_plan.is_some_and(|plan| plan.at_layer == Some(ahead))
    })?;
    session.pump(Duration::from_secs(1));
    session.capture("print-pause-planned")?;
    if session.has_reference_layout() {
        let (x, y) = LAYER_TILE;
        session.tap(x, y);
        session.capture("layer-pause")?;
        let (x, y) = LAYER_PAUSE_BUTTON;
        session.tap(x, y);
    } else {
        let next = u32::try_from(session.window.global::<Printer>().get_current_layer() + 1).unwrap_or_default();
        session.service.send(Command::PauseAtLayer { layer: next });
    }
    session.wait_until("paused at the next layer", |snapshot| snapshot.status.job.state == PrintState::Paused)?;
    session.pump(Duration::from_secs(1));
    Ok(())
}

/// A tap on the nozzle temperature of the status bar opens the heaters; checked in the reference layout only.
fn temperature_leads_to_heaters(session: &mut Session) -> Result<(), String> {
    if !session.has_reference_layout() {
        return Ok(());
    }
    let (x, y) = STATUS_BAR_NOZZLE;
    session.tap(x, y);
    let navigation = session.window.global::<Navigation>();
    if navigation.get_page() != Page::Temperature || navigation.get_heat_tab() != HeatTab::Heaters {
        return Err("a tap on the nozzle temperature did not open the heaters".into());
    }
    navigation.set_page(Page::Print);
    Ok(())
}

/// The end of a job comes from the history, which the service reads once the job has ended.
fn wait_for_end_time(session: &mut Session) -> Result<(), String> {
    for _ in 0..40 {
        if session.window.global::<Printer>().get_ended() != "-" {
            return Ok(());
        }
        session.pump(Duration::from_millis(100));
    }
    Err("the end of the job did not arrive from the history".into())
}

/// A print that Klipper stops with an error (the fake's `FAKE_PRINT_ERROR`).
fn print_error(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::StartPrint { filename: "benchy.gcode".into() });
    session.wait_until("print running again", |snapshot| snapshot.status.job.state == PrintState::Printing)?;
    session.service.send(Command::RunGcode { script: "FAKE_PRINT_ERROR".into() });
    session.wait_until("print failed", |snapshot| snapshot.status.job.state == PrintState::Error)?;
    wait_for_end_time(session)?;
    session.capture("print-error")
}

/// Raises the tension in the fake's buffer until tangle prevention boosts the gear current, and takes it back after the
/// picture. Skipped without a buffer that tangle prevention watches (Happy Hare with a proportional sensor).
fn tangle_boost(session: &mut Session) -> Result<(), String> {
    let has_buffer = session.presenter.snapshot().status.mmu.as_ref().is_some_and(|mmu| mmu.flowguard.is_some());
    if !has_buffer {
        return Ok(());
    }
    let boosting = |snapshot: &freakymate_core::Snapshot| {
        snapshot.status.mmu.as_ref().and_then(|mmu| mmu.flowguard.as_ref()).is_some_and(|flowguard| flowguard.boosting)
    };
    session.service.send(Command::RunGcode { script: "FAKE_MMU_TANGLE BOOST=1".into() });
    session.wait_until("tangle prevention boosting", boosting)?;
    session.show_page(Page::Print, "print-tangle-boost")?;
    session.service.send(Command::RunGcode { script: "FAKE_MMU_TANGLE BOOST=0".into() });
    session.wait_until("boost released", |snapshot| !boosting(snapshot))
}

/// Excludes one object of the print and checks that the Objects dialog data follows. In the reference layout it opens
/// the dialog, selects the object on the bed map and taps Exclude; another layout sends the command. Skipped without
/// the module.
fn exclude_object(session: &mut Session) -> Result<(), String> {
    if session.presenter.snapshot().status.print_objects.is_empty() {
        return Ok(());
    }
    if session.has_reference_layout() {
        for (x, y) in [OBJECTS_BUTTON, MAP_BENCHY_1] {
            session.tap(x, y);
        }
        session.capture("print-objects-map")?;
        status_bar_cancels_the_dialog(session)?;
        for (x, y) in [OBJECTS_BUTTON, MAP_BENCHY_1, EXCLUDE_BUTTON] {
            session.tap(x, y);
        }
    } else {
        session.service.send(Command::ExcludeObject { name: "BENCHY_1".into() });
    }
    session.wait_until("object excluded", |snapshot| {
        snapshot.status.print_objects.iter().any(|object| object.name == "BENCHY_1" && object.excluded)
    })?;
    session.pump(Duration::from_secs(1));
    let items = session.window.global::<Printer>().get_print_objects();
    let excluded: Vec<String> = (0..items.row_count())
        .filter_map(|row| items.row_data(row))
        .filter(|item| item.excluded)
        .map(|item| item.name.to_string())
        .collect();
    if excluded != ["BENCHY_1"] {
        return Err(format!("Printer.print-objects shows excluded {excluded:?}, expected [\"BENCHY_1\"]"));
    }
    session.capture("print-objects-excluded")?;
    if session.has_reference_layout() {
        let (x, y) = CLOSE_OBJECTS_BUTTON;
        session.tap(x, y);
    }
    Ok(())
}

/// A tap on the status bar cancels the open Objects dialog with BENCHY_1 selected: the light panel opens, and a tap
/// where Exclude was reaches the print page instead.
fn status_bar_cancels_the_dialog(session: &mut Session) -> Result<(), String> {
    let (x, y) = STATUS_BAR_LIGHT;
    session.tap(x, y);
    let navigation = session.window.global::<Navigation>();
    if !navigation.get_light_open() {
        return Err("a tap on the light symbol did not open the light panel".into());
    }
    navigation.set_light_open(false);
    let (x, y) = EXCLUDE_BUTTON;
    session.tap(x, y);
    session.pump(Duration::from_secs(1));
    if session.presenter.snapshot().status.print_objects.iter().any(|object| object.excluded) {
        return Err("the Objects dialog stayed open after a tap on the status bar".into());
    }
    Ok(())
}

/// A confirmation that is open when Klipper shuts down closes like a cancel.
fn shutdown(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_page(Page::Print);
    ask(session, "Cancel print", "The current print will be stopped and cannot be resumed.", "cancel-print", "");
    session.service.send(Command::EmergencyStop);
    session.wait_until("shutdown", |snapshot| snapshot.status.klippy_state == KlippyState::Shutdown)?;
    session.pump(Duration::from_secs(1));
    if session.window.global::<Confirm>().get_open() {
        return Err("the confirmation stayed open after Klipper shut down".into());
    }
    session.capture("klippy-shutdown")?;
    session.service.send(Command::FirmwareRestart);
    session.wait_until("ready again", |snapshot| snapshot.status.klippy_state == KlippyState::Ready)
}

/// Before the printer is ready: the connection page asks for a login, a wrong password is refused, the right one
/// opens the connection.
fn log_in(session: &mut Session) -> Result<(), String> {
    session.wait_until("login required", |snapshot| snapshot.connection == ConnectionState::LoginRequired)?;
    session.capture("login-required")?;
    session.window.global::<Actions>().invoke_log_in("test".into(), "wrong".into());
    session.pump(Duration::from_secs(1));
    if session.window.global::<Printer>().get_login_error().is_empty() {
        return Err("a wrong password was not reported".into());
    }
    session.capture("login-refused")?;
    if !session.has_reference_layout() {
        session.window.global::<Actions>().invoke_log_in("test".into(), "secret".into());
        return Ok(());
    }
    // As a user would: "Log in", the user, Enter, the password, Enter.
    let (x, y) = LOG_IN_BUTTON;
    session.tap(x, y);
    session.pump(Duration::from_millis(300));
    type_and_enter(session, "test");
    session.capture("login-password")?;
    type_and_enter(session, "secret");
    Ok(())
}

fn type_and_enter(session: &mut Session, text: &str) {
    for character in text.chars() {
        session.key(character.to_string());
    }
    session.key(slint::platform::Key::Return);
    session.pump(Duration::from_millis(300));
}

/// The settings show the user and offer to log out; the logout leads back to the login.
fn logged_in(session: &mut Session) -> Result<(), String> {
    session.show_page(Page::Settings, "settings-logged-in")?;
    if session.window.global::<Printer>().get_login_user() != "test" {
        return Err("the settings do not show the logged-in user".into());
    }
    ask(session, "Log out", "The next connection needs the password again.", "log-out", "");
    session.capture("confirm-log-out")?;
    session.window.global::<Confirm>().invoke_execute();
    session.wait_until("login required again", |snapshot| snapshot.connection == ConnectionState::LoginRequired)?;
    session.capture("logged-out")
}

fn idle(session: &mut Session) -> Result<(), String> {
    session.wait_until_for("idle timeout", IDLE_WAIT_LIMIT, |snapshot| snapshot.status.activity == Activity::Idle)?;
    session.show_page(Page::Print, "print-idle")
}

/// Opens the confirmation dialog as a page would.
fn ask(session: &Session, title: &str, message: &str, action: &str, argument: &str) {
    session.window.global::<Confirm>().invoke_ask(title.into(), message.into(), action.into(), argument.into(), true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scene_lists() {
        assert_eq!(Scene::parse_list("network, pages"), Ok(vec![Scene::Network, Scene::Pages]));
        assert_eq!(Scene::parse_list("all").map(|scenes| scenes.len()), Ok(Scene::ALL.len() - 1));
        assert_eq!(Scene::parse_list("login"), Ok(vec![Scene::Login]));
        assert!(Scene::parse_list("pages,unknown").is_err());
    }
}
