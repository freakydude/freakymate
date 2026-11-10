//! The run, split into scenes that can be chosen on the command line.

use crate::session::Session;
use freakymate::Confirm;
use freakymate_core::sources::klipper::KlippyState;
use freakymate_core::{Command, ConnectionState, Query};
use slint::ComponentHandle;

mod calibrate;
mod console;
mod login;
mod mmu;
mod network;
mod pages;
mod printing;
mod spools;
mod system;
mod temperature;
mod toolchanger;

use calibrate::calibrate;
use console::typing;
use login::{log_in, logged_in};
use network::network;
use pages::pages;
use printing::printing;
use spools::spools;
use system::{idle, shutdown};
use temperature::taps;
use toolchanger::toolchanger;

/// Scenes run in this order; each one leaves the printer ready for the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// Every page in standby, with the file list.
    Pages,
    /// Spoolman: the spool in use, the temperature hint, the picker and the hints before a print.
    Spools,
    /// A tool changer (`-p toolchanger`): spool in use, hints, the picker of a tool macro, the prepared file.
    Toolchanger,
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
    pub const ALL: [Self; 11] = [
        Self::Pages,
        Self::Spools,
        Self::Toolchanger,
        Self::Network,
        Self::Taps,
        Self::Typing,
        Self::Calibrate,
        Self::Printing,
        Self::Shutdown,
        Self::Idle,
        Self::Login,
    ];
    pub const DEFAULT: [Self; 9] = [
        Self::Pages,
        Self::Spools,
        Self::Toolchanger,
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
                "unknown scene '{name}' (pages, spools, toolchanger, network, taps, typing, calibrate, printing, shutdown, idle, \
                 login, all)"
            )
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Pages => "pages",
            Self::Spools => "spools",
            Self::Toolchanger => "toolchanger",
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
            Self::Toolchanger => toolchanger(session),
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

/// The light symbol of the status bar (800x480, no fitting).
const STATUS_BAR_LIGHT: (f32, f32) = (700.0, 22.0);

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
