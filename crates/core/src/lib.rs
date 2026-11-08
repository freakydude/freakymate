//! UI-independent core of freakymate: Moonraker connection, printer state, commands and settings.
//!
//! Layers, each using only the ones before it: [`moonraker`] is the transport and knows no printer objects;
//! [`sources`] has one module per origin of data (Klipper, Moonraker's components, Happy Hare, Spoolman, ...);
//! [`domain`] answers questions that need several sources; [`service`] only collects. The entry point is
//! [`PrinterService`]; the UI reads [`Snapshot`]s and [`Event`]s and sends [`Command`]s.

pub mod backlight;
pub mod color;
pub mod commands;
pub mod credentials;
pub mod domain;
pub mod moonraker;
pub mod network;
pub mod service;
pub mod settings;
pub mod setup;
pub mod sourced;
pub mod sources;
mod toml_merge;

pub use commands::Command;
pub use credentials::CredentialStore;
pub use moonraker::Endpoint;
pub use service::{Access, ConnectionState, Event, PrinterService, Query, ServiceChannels, Snapshot};
pub use settings::{Settings, SettingsStore};
pub use sourced::{Provenance, Sourced};

/// Version of this build: set by CI from git tags, otherwise the crate version marked as a local build.
pub const VERSION: &str = match option_env!("FREAKYMATE_VERSION") {
    Some(version) => version,
    None => concat!(env!("CARGO_PKG_VERSION"), "-dev"),
};

/// Log target for facts about the machine and the printer (`name: value`); the log file repeats the latest of each
/// in its header.
pub const HEADER_TARGET: &str = "freakymate::header";
