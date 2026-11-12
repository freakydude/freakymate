//! Why a command cannot be carried out.

use thiserror::Error;

/// A command that is refused before it reaches the printer; the text is shown to people.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error("this printer has neither quad gantry level nor Z tilt")]
    NoLeveling,
    #[error("no way to load filament: no Happy Hare and no LOAD_FILAMENT macro")]
    NoLoadWay,
    #[error("no way to unload filament: no Happy Hare and no UNLOAD_FILAMENT macro")]
    NoUnloadWay,
    #[error("this printer has no LED")]
    NoLight,
    #[error("this printer has neither a probe nor a Z endstop with position_endstop")]
    NoZCalibration,
    #[error("this printer has no macro to pause at a layer")]
    NoLayerPause,
    #[error("the print file reports no layers")]
    NoLayerReport,
    #[error("this printer has no MMU")]
    NoMmu,
    #[error("Happy Hare takes material and colour of the gates from Spoolman (spoolman_support: pull)")]
    GateMapFromSpoolman,
    #[error("Happy Hare reports no EndlessSpool groups")]
    NoEndlessSpoolGroups,
    #[error("this printer has no {0}")]
    NoTuningValue(String),
    #[error("{0} is not in the configuration")]
    TuningNotConfigured(String),
    #[error("unknown heater {0}")]
    UnknownHeater(String),
    #[error("gate {0} does not exist")]
    UnknownGate(usize),
    #[error("this printer has no AFC")]
    NoAfc,
    #[error("AFC has no lane {0}")]
    UnknownLane(String),
    #[error("the bypass has no spool, material or ejection of its own")]
    NotForBypass,
    #[error("the tool changer of this printer has no command to drop the head")]
    NoDropCommand,
    #[error("the macro T{0} declares no spool_id")]
    NoToolSpoolVariable(usize),
    /// A source or Klipper did not accept the command or a value in it; their reason.
    #[error("{0}")]
    Rejected(String),
    #[error("not possible while a print is moving the machine")]
    PrintMovesMachine,
    #[error("not possible while a print is running")]
    PrintRunning,
    #[error("not possible during a print, also when paused: the print would resume from a wrong position")]
    PrintNeedsPosition,
    #[error("connection to Moonraker closed")]
    ConnectionClosed,
    #[error("not connected to Moonraker")]
    NotConnected,
    #[error("Moonraker wants a login")]
    LoginRequired,
}

impl From<String> for CommandError {
    fn from(reason: String) -> Self {
        Self::Rejected(reason)
    }
}
