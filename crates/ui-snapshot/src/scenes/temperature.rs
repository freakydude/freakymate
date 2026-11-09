//! Heat tabs, nozzle history, stalled fan and taps while temperatures change.

use crate::session::Session;
use freakymate::{Backlight, HeatTab, Navigation, Page};
use freakymate_core::sources::moonraker_server::wled::{self, StripAction};
use freakymate_core::{Command, Snapshot};
use slint::ComponentHandle;
use std::time::Duration;

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
    Ok(())
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
