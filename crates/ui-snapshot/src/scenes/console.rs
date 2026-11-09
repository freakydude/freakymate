//! The text entry of the console, physical and on-screen.

use super::STATUS_BAR_LIGHT;
use crate::session::Session;
use freakymate::{Navigation, Page, Printer};
use slint::{ComponentHandle, Model};
use std::time::Duration;

/// The command field of the console (800x480, no fitting).
const COMMAND_FIELD: (f32, f32) = (480.0, 444.0);

/// Keys of the on-screen keyboard, whose rows fill the dialog (800x480, no fitting).
const ON_SCREEN_Q: (f32, f32) = (128.0, 226.0);

const ON_SCREEN_S: (f32, f32) = (197.0, 293.0);

const ON_SCREEN_BACKSPACE: (f32, f32) = (628.0, 428.0);

const ON_SCREEN_ENTER: (f32, f32) = (731.0, 428.0);

/// The text entry edits like any text field: Backspace, Delete, Home, End, Shift and Ctrl are the keyboard's own
/// business. The result is read from the command that the entry sends. Checked in the reference layout only, where
/// the field and the keys lie at known places.
pub(super) fn typing(session: &mut Session) -> Result<(), String> {
    use slint::platform::Key;
    if !session.has_reference_layout() {
        return Ok(());
    }
    session.window.global::<Navigation>().set_page(Page::Console);
    session.pump(Duration::from_millis(300));
    let (x, y) = COMMAND_FIELD;
    session.press(x, y);
    session.release(x, y);
    session.pump(Duration::from_millis(300));
    session.capture("text-entry")?;

    // Physical keyboard: "XSTATUZ" -> Backspace -> "S" -> Home -> Delete (removes the X) -> End -> Enter.
    for character in "XSTATUZ".chars() {
        session.key(character.to_string());
    }
    session.key(Key::Backspace);
    session.key("S");
    session.key(Key::Home);
    session.key(Key::Delete);
    session.key(Key::End);
    session.capture("text-entry-typed")?;
    session.key(Key::Return);
    expect_console_command(session, "STATUS")?;

    // On-screen keys: the field must keep the focus while they are tapped. Q, backspace, S, enter.
    session.tap(x, y);
    session.pump(Duration::from_millis(300));
    for (key_x, key_y) in [ON_SCREEN_Q, ON_SCREEN_BACKSPACE, ON_SCREEN_S, ON_SCREEN_ENTER] {
        session.tap(key_x, key_y);
    }
    expect_console_command(session, "S")?;

    // Escape closes without sending, and a later Enter of the physical keyboard must not send the hidden text.
    session.tap(x, y);
    session.pump(Duration::from_millis(300));
    session.key("H");
    session.key(Key::Escape);
    session.pump(Duration::from_millis(300));
    session.key(Key::Return);
    expect_no_console_command(session, "H", "Escape, then Enter")?;

    // The same after a tap on the status bar, which cancels the entry.
    session.tap(x, y);
    session.pump(Duration::from_millis(300));
    session.key("J");
    let (light_x, light_y) = STATUS_BAR_LIGHT;
    session.tap(light_x, light_y);
    session.window.global::<Navigation>().set_light_open(false);
    session.pump(Duration::from_millis(300));
    session.key(Key::Return);
    expect_no_console_command(session, "J", "a tap on the status bar, then Enter")
}

/// A command that was typed into a closed entry must not reach the printer.
pub(super) fn expect_no_console_command(session: &mut Session, command: &str, after: &str) -> Result<(), String> {
    session.pump(Duration::from_millis(500));
    if session.window.global::<Printer>().get_console().iter().any(|line| line.is_command && line.text == command) {
        return Err(format!("{after} sent the command {command} of the closed entry"));
    }
    Ok(())
}

/// The command appears as the newest line of the console.
pub(super) fn expect_console_command(session: &mut Session, command: &str) -> Result<(), String> {
    session.pump(Duration::from_millis(500));
    let lines = session.window.global::<Printer>().get_console();
    let last = lines.iter().last();
    if last.as_ref().is_some_and(|line| line.is_command && line.text == command) {
        return Ok(());
    }
    Err(format!(
        "expected the console to end with the command {command}, it ends with {:?}",
        last.map(|line| line.text)
    ))
}
