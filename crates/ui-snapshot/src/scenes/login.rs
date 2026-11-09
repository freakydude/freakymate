//! The login of Moonraker: refused, accepted, logged in, logged out.

use super::ask;
use crate::session::Session;
use freakymate::{Actions, Confirm, Page, Printer};
use freakymate_core::ConnectionState;
use slint::ComponentHandle;
use std::time::Duration;

/// "Log in" on the connection page (800x480 layout).
const LOG_IN_BUTTON: (f32, f32) = (656.0, 230.0);

/// Before the printer is ready: the connection page asks for a login, a wrong password is refused, the right one
/// opens the connection.
pub(super) fn log_in(session: &mut Session) -> Result<(), String> {
    session.wait_until("login required", |snapshot| snapshot.connection == ConnectionState::LoginRequired)?;
    session.capture("login-required")?;
    session.window.global::<Actions>().invoke_log_in("test".into(), "wrong".into());
    session.pump(Duration::from_secs(1));
    if session.window.global::<Printer>().get_login_error().is_empty() {
        return Err("a wrong password was not reported".into());
    }
    session.capture("login-refused")?;
    if !session.has_reference_layout() {
        session.window.global::<Actions>().invoke_log_in("test".into(), "secret".into());
        return Ok(());
    }
    // As a user would: "Log in", the user, Enter, the password, Enter.
    let (x, y) = LOG_IN_BUTTON;
    session.tap(x, y);
    session.pump(Duration::from_millis(300));
    type_and_enter(session, "test");
    session.capture("login-password")?;
    type_and_enter(session, "secret");
    Ok(())
}

pub(super) fn type_and_enter(session: &mut Session, text: &str) {
    for character in text.chars() {
        session.key(character.to_string());
    }
    session.key(slint::platform::Key::Return);
    session.pump(Duration::from_millis(300));
}

/// The settings show the user and offer to log out; the logout leads back to the login.
pub(super) fn logged_in(session: &mut Session) -> Result<(), String> {
    session.show_page(Page::Settings, "settings-logged-in")?;
    if session.window.global::<Printer>().get_login_user() != "test" {
        return Err("the settings do not show the logged-in user".into());
    }
    ask(session, "Log out", "The next connection needs the password again.", "log-out", "");
    session.capture("confirm-log-out")?;
    session.window.global::<Confirm>().invoke_execute();
    session.wait_until("login required again", |snapshot| snapshot.connection == ConnectionState::LoginRequired)?;
    session.capture("logged-out")
}
