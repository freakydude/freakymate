//! Loading and unloading with what the printer has: Happy Hare, else the macros.

use crate::commands::{Command, CommandError};
use crate::domain::filament::{self, FilamentHandling};
use crate::sources::macro_conventions::{LOAD_FILAMENT_MACRO, UNLOAD_FILAMENT_MACRO};
use crate::sources::{PrinterStatus, Sources};

pub(super) fn load(sources: &Sources, status: &PrinterStatus) -> Result<Vec<Command>, CommandError> {
    match filament::handling(sources, status) {
        FilamentHandling::Mmu => Ok(vec![Command::MmuLoad]),
        FilamentHandling::Macros(macros) if macros.load => {
            Ok(vec![Command::RunMacro { name: LOAD_FILAMENT_MACRO.into() }])
        }
        _ => Err(CommandError::NoLoadWay),
    }
}

pub(super) fn unload(sources: &Sources, status: &PrinterStatus) -> Result<Vec<Command>, CommandError> {
    match filament::handling(sources, status) {
        FilamentHandling::Mmu => Ok(vec![Command::MmuUnload]),
        FilamentHandling::Macros(macros) if macros.unload => {
            Ok(vec![Command::RunMacro { name: UNLOAD_FILAMENT_MACRO.into() }])
        }
        _ => Err(CommandError::NoUnloadWay),
    }
}
