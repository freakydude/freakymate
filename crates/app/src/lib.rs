//! freakymate: touch UI for Klipper via Moonraker.
//!
//! Structure: `freakymate-ui` (`crates/ui/ui/*.slint`) describes the UI, [`presenter`] writes the state from
//! `freakymate-core` into it, [`actions`] forwards user input as commands.

pub use freakymate_ui::*;

pub mod actions;
pub mod app;
pub mod backlight_controller;
mod bed_map;
mod bed_mesh_view;
mod calibration_view;
mod chart;
mod command_name;
mod console_view;
pub mod display;
mod files_view;
mod format;
#[cfg(target_os = "linux")]
mod framebuffer;
mod history_view;
mod items;
pub mod kiosk_pointer;
pub mod logging;
mod macro_view;
mod mmu_view;
pub mod network_controller;
mod objects_view;
pub mod presenter;
pub mod settings_controller;
mod spool_view;
mod status_log;
mod status_view;
mod thumbnail;
mod view;
