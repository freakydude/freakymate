//! Pausing at a layer with Mainsail's macros, see [`crate::sources::mainsail_config`].

use crate::commands::{Command, CommandError};
use crate::sources::{PrinterStatus, Sources};

pub(super) fn pause_at(sources: &Sources, status: &PrinterStatus, layer: u32) -> Result<Vec<Command>, CommandError> {
    let current = status.job.current_layer.ok_or(CommandError::NoLayerReport)?;
    sources
        .layer_pause
        .and_then(|macros| macros.pause_at(layer, current))
        .map(|script| vec![Command::RunGcode { script }])
        .ok_or(CommandError::NoLayerPause)
}

/// Takes back a pause, whichever macro planned it.
pub(super) fn clear(sources: &Sources) -> Result<Vec<Command>, CommandError> {
    let scripts = sources.layer_pause.map(|macros| macros.clear()).unwrap_or_default();
    if scripts.is_empty() {
        return Err(CommandError::NoLayerPause);
    }
    Ok(scripts.into_iter().map(|script| Command::RunGcode { script }).collect())
}
