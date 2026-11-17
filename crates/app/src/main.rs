// Do not open a console window in Windows release builds.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use clap::Parser;
use freakymate::startup::{self, Cli};
use std::process::ExitCode;

fn main() -> ExitCode {
    startup::run(Cli::parse())
}
