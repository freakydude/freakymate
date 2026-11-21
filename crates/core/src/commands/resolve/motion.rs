//! The moves of the Axes tab: steps, typed positions and the live pad.

use super::Command;
use crate::sources::PrinterStatus;
use crate::sources::klipper::moves;
use crate::sources::klipper::{Axis, Setup};

pub(super) fn jog(status: &PrinterStatus, axis: Axis, distance: f64) -> Vec<Command> {
    moves::jog(&status.motion, axis, distance).map(|planned| Command::Move { planned }).into_iter().collect()
}

pub(super) fn move_to(status: &PrinterStatus, axis: Axis, position: f64) -> Vec<Command> {
    vec![Command::Move { planned: moves::move_to(&status.motion, axis, position) }]
}

pub(super) fn drive(setup: &Setup, status: &PrinterStatus, deflection: [f64; 3], max_speed: f64) -> Vec<Command> {
    let limits = moves::drive_limits(setup, &status.motion, &status.job);
    moves::drive(&status.motion, deflection, max_speed, &limits)
        .map(|planned| Command::Move { planned })
        .into_iter()
        .collect()
}
