//! Heat tabs, nozzle history, PID tuning, stalled fan and taps while temperatures change.

use crate::session::Session;
use freakymate::{Backlight, HeatTab, Navigation, Page, Printer};
use freakymate_core::sources::moonraker_server::wled::{self, StripAction};
use freakymate_core::{Command, Snapshot};
use slint::ComponentHandle;
use std::time::{Duration, Instant};

/// The exhaust fan measures at 50 %, then its tachometer goes silent: red after a while; afterwards off again. The
/// core only learns a reading some seconds after a change of speed (kick start), so this scene waits that long.
pub(super) fn stalled_fan(session: &mut Session) -> Result<(), String> {
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
pub(super) fn heat_tabs(session: &mut Session) -> Result<(), String> {
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
        // Standby: only the black cover.
        session.window.global::<Backlight>().set_off(true);
        session.capture("screen-off")?;
        session.window.global::<Backlight>().set_off(false);
    }
    if status.heater("extruder").is_some() {
        heater_deviation(session)?;
        pid_tuning(session)?;
    }
    Ok(())
}

/// The nozzle's dialog with the PID footer: the start, `PID_CALIBRATE` running (the status bar names it, the dialog
/// shows nothing of its own), and the result Klipper keeps for `SAVE_CONFIG`. The tuning is short at 60 °C in the fake.
fn pid_tuning(session: &mut Session) -> Result<(), String> {
    const TARGET: f64 = 60.0;
    const RUNNING_LABEL: &str = "PID tune";
    // A command shows as running after one second (`LONG_COMMAND`); the tuning in the fake lasts a few seconds.
    const NAMED_WITHIN: Duration = Duration::from_secs(5);
    session.presenter.open_history("extruder");
    session.pump(Duration::from_secs(1));
    session.capture("temperature-pid-start")?;
    session.service.send(Command::PidCalibrate { heater: "extruder".into(), target: TARGET });
    let deadline = Instant::now() + NAMED_WITHIN;
    while session.window.global::<Printer>().get_running_command() != RUNNING_LABEL {
        if Instant::now() > deadline {
            return Err("timed out waiting for the status bar to name the tuning".into());
        }
        session.pump(Duration::from_millis(50));
    }
    session.capture("temperature-pid-running")?;
    session.wait_until("Klipper to keep the found values", |snapshot| {
        snapshot.status.calibration.pending.iter().any(|section| section.section == "extruder")
    })?;
    session.pump(Duration::from_secs(1));
    session.capture("temperature-pid-done")?;
    session.presenter.close_history();
    session.service.send(Command::SaveConfig);
    session.wait_until("the saved values", |snapshot| snapshot.status.calibration.pending.is_empty())
}

/// The nozzle holds its target, then reads 20 °C more (`FAKE_OVERSHOOT`): the number turns yellow, the stroke leaves
/// the tolerance window on the scale. Afterwards the nozzle is off again.
fn heater_deviation(session: &mut Session) -> Result<(), String> {
    const TARGET: f64 = 50.0;
    // Settled: within `SETTLED_TOLERANCE` of the target. Deviating: more than `OVERSHOOT_SEEN` above it, half of the
    // 20 °C that `FAKE_OVERSHOOT` adds, so the reading has certainly left the target.
    const SETTLED_TOLERANCE: f64 = 2.0;
    const OVERSHOOT_SEEN: f64 = 10.0;
    let nozzle = |snapshot: &Snapshot| snapshot.status.heater("extruder").map(|heater| heater.temperature);
    session.service.send(Command::RunGcode { script: format!("M104 S{TARGET}") });
    session.wait_until("the nozzle to reach its target", |snapshot| {
        nozzle(snapshot).is_some_and(|temperature| (temperature - TARGET).abs() < SETTLED_TOLERANCE)
    })?;
    // The target has been held for a status update, so the deviation counts as one.
    session.pump(Duration::from_millis(500));
    session.service.send(Command::RunGcode { script: "FAKE_OVERSHOOT HEATER=extruder DELTA=20".into() });
    session.wait_until("the nozzle to read above its window", |snapshot| {
        nozzle(snapshot).is_some_and(|temperature| temperature > TARGET + OVERSHOOT_SEEN)
    })?;
    session.show_page(Page::Temperature, "temperature-deviation")?;
    session.service.send(Command::RunGcode { script: "FAKE_OVERSHOOT HEATER=extruder DELTA=0\nM104 S0".into() });
    session.wait_until("the nozzle target to be off", |snapshot| {
        snapshot.status.heater("extruder").is_some_and(|heater| heater.target == 0.0)
    })
}

/// The temperature history of the nozzle, once it has heated: Moonraker's stored minutes plus the live value.
pub(super) fn nozzle_history(session: &mut Session) -> Result<(), String> {
    session.presenter.open_history("extruder");
    session.pump(Duration::from_secs(1));
    session.capture("history-nozzle")?;
    session.presenter.close_history();
    Ok(())
}

/// The "Cool down" button of the nozzle row on the Heat page, below the tabs (800x480, no fitting).
const COOL_DOWN_BUTTON: (f32, f32) = (628.0, 148.0);

const TAP_ROUNDS: usize = 10;

/// A finger rests this long: several status updates arrive meanwhile, as the temperatures change all the time.
const TAP_DURATION: Duration = Duration::from_millis(500);

/// A tap that spans status updates must still count: the heater rows may not be rebuilt under the finger. Checked
/// in the reference layout only, where the button lies at a known place.
pub(super) fn taps(session: &mut Session) -> Result<(), String> {
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

/// A tap on the nozzle temperature of the status bar opens the heaters; checked in the reference layout only.
/// The nozzle temperature of the status bar while a print runs (800x480, no fitting).
const STATUS_BAR_NOZZLE: (f32, f32) = (510.0, 22.0);

pub(super) fn temperature_leads_to_heaters(session: &mut Session) -> Result<(), String> {
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
