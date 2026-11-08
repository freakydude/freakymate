//! Carrying out commands: an intent becomes concrete calls that go to Moonraker in the order they were sent.

use super::session::Session;
use super::{Event, LONG_COMMAND, NEXT_COMMAND_ID};
use crate::commands::{Command, CommandError};
use crate::moonraker::{Call, CallError, PendingCall};
use crate::sources::moonraker_server::gcode_store::{ConsoleKind, ConsoleLine};
use crate::sources::moonraker_server::wled;
use crate::sources::PrinterStatus;
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

impl Session<'_> {
    /// Carries out a command; an intent first becomes the commands this printer needs.
    pub(super) fn execute(&mut self, command: Command) {
        let status = self.current_status();
        if command.moves_machine() && status.print_moving() {
            self.outputs.event(Event::CommandFailed { command, error: CommandError::PrintMovesMachine });
            return;
        }
        let commands = match command.clone().resolve(&self.sources, &status) {
            Ok(commands) => commands,
            Err(error) => {
                self.outputs.event(Event::CommandFailed { command, error });
                return;
            }
        };
        let mut targets_changed = false;
        for command in commands {
            targets_changed |= self.dispatch(command, &status);
        }
        if targets_changed {
            self.publish();
        }
    }

    /// Sends one concrete command; true if it set a heater target, which the snapshot shows at once.
    fn dispatch(&mut self, command: Command, status: &PrinterStatus) -> bool {
        if command.interrupts_print() && status.print_running() {
            self.outputs.event(Event::CommandFailed { command, error: CommandError::PrintRunning });
            return false;
        }
        let Some(call) = command.to_call(&status.motion) else { return false };
        // Every G-code script goes to the console, as Moonraker stores it in the console history: typed commands,
        // macros and the scripts behind the buttons alike.
        if let Some(script) = gcode_script(&call) {
            self.outputs.event(Event::ConsoleLine(console_line(ConsoleKind::Command, script)));
        }
        let target_set = match &command {
            Command::SetHeaterTarget { heater, target } => {
                self.pending_targets.request(heater, *target, Instant::now());
                true
            }
            _ => false,
        };
        let spools_change = command.changes_spools();
        // Busy from the start: Moonraker's own `busy` and notifications only follow once it has begun.
        let update_manager_call = matches!(command, Command::RefreshUpdates | Command::Upgrade { .. });
        if update_manager_call {
            self.outputs.event(Event::UpdateManagerBusy(true));
        }
        let wled_answers = matches!(command, Command::SetWledStrip { .. }).then(|| self.wled_answers.clone());
        let running = Running { command: command.clone(), script_line: first_script_line(&call) };
        let spool_events = self.outputs.events.clone();
        let running_events = self.outputs.events.clone();
        let events = self.outputs.events.clone();
        let failed = move |error: CallError| {
            let error = match error {
                CallError::Rpc(rpc) => CommandError::Rejected(rpc.message),
                CallError::Closed => CommandError::ConnectionClosed,
            };
            let _ = events.send(Event::CommandFailed { command, error });
        };
        // Sent here, in the order of the commands; a task per command could overtake an earlier one.
        match self.connection.start_call(call.method, call.params) {
            Ok(pending) => {
                tokio::spawn(async move {
                    let answer = answer_or_report_running(pending, running, &running_events).await;
                    if update_manager_call {
                        let _ = running_events.send(Event::UpdateManagerBusy(false));
                    }
                    match answer {
                        Ok(_) if spools_change => {
                            let _ = spool_events.send(Event::SpoolmanChanged);
                        }
                        Ok(answer) => {
                            if let (Some(sender), Some(strips)) = (wled_answers, answer.as_object()) {
                                let _ = sender.send(wled::parse_strips(strips));
                            }
                        }
                        Err(error) => failed(error),
                    }
                });
            }
            Err(error) => failed(error),
        }
        target_set
    }
}

/// Waits for the answer of a call; if it takes longer than `LONG_COMMAND`, reports the command as running until then.
async fn answer_or_report_running(
    pending: PendingCall,
    running: Running,
    events: &mpsc::UnboundedSender<Event>,
) -> Result<Value, CallError> {
    let response = pending.response();
    tokio::pin!(response);
    tokio::select! {
        answer = &mut response => answer,
        () = tokio::time::sleep(LONG_COMMAND) => {
            let id = NEXT_COMMAND_ID.fetch_add(1, Ordering::Relaxed);
            let _ = events.send(Event::CommandRunning { id, command: running.command, script_line: running.script_line });
            let answer = response.await;
            let _ = events.send(Event::CommandFinished { id });
            answer
        }
    }
}

/// A command waiting for its answer, as `Event::CommandRunning` reports it.
struct Running {
    command: Command,
    script_line: Option<String>,
}

/// The first line of the G-code script of a call, as the console shows it.
fn first_script_line(call: &Call) -> Option<String> {
    gcode_script(call).and_then(|script| script.lines().next()).map(str::to_owned)
}

/// The script of a call that runs G-code.
fn gcode_script(call: &Call) -> Option<&str> {
    if call.method != "printer.gcode.script" {
        return None;
    }
    call.params.as_ref().and_then(|params| params.get("script")).and_then(Value::as_str)
}

pub(super) fn console_line(kind: ConsoleKind, message: &str) -> ConsoleLine {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
    ConsoleLine { kind, message: message.to_owned(), time }
}
