//! The status bar and its flyout: the event the bar shows, the counter of unread errors and the list of all messages.

use crate::items::replace_rows;
use crate::status_log::{display_time, StatusLog};
use crate::{format, AppWindow, MessageItem, Printer};
use freakymate_core::sources::moonraker_server::gcode_store::ConsoleLine;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

/// Shared with the timer that hides the event again, hence the `Rc`.
pub struct StatusView {
    window: slint::Weak<AppWindow>,
    log: RefCell<StatusLog>,
    messages: Rc<VecModel<MessageItem>>,
    /// Only the newest event is shown, hence one timer.
    timer: Timer,
}

impl StatusView {
    pub fn new(window: &AppWindow) -> Rc<Self> {
        let view = Rc::new(Self {
            window: window.as_weak(),
            log: RefCell::default(),
            messages: Rc::default(),
            timer: Timer::default(),
        });
        window.global::<Printer>().set_messages(ModelRc::from(view.messages.clone()));
        view
    }

    /// Puts the log on screen: the flyout list with its sections, the event the status bar shows and the counter.
    pub fn refresh(&self) {
        let log = self.log.borrow();
        let layout = log.layout();
        let items: Vec<MessageItem> = layout
            .rows
            .iter()
            .map(|placed| MessageItem {
                text: SharedString::from(placed.message.text.as_str()),
                time: format::unix_clock(placed.message.time).into(),
                is_error: placed.message.is_error,
                new: placed.new,
                top: placed.top,
                height: placed.height,
            })
            .collect();
        replace_rows(&self.messages, items);
        let Some(window) = self.window.upgrade() else { return };
        let printer = window.global::<Printer>();
        printer.set_messages_height(layout.height);
        printer.set_messages_has_new(layout.has_new);
        printer.set_messages_has_earlier(layout.has_earlier);
        printer.set_messages_earlier_top(layout.earlier_top);
        printer.set_unread_errors(i32::try_from(log.unread_errors()).unwrap_or(i32::MAX));
        let event = log.event();
        printer.set_status_message(event.map(|message| SharedString::from(message.headline())).unwrap_or_default());
        printer.set_status_message_error(event.is_some_and(|message| message.is_error));
        printer.set_status_message_visible(event.is_some());
    }

    /// Notes the summary of connection, Klipper and print state, and logs a change of it.
    pub fn note_state(&self, state: &str) {
        if self.log.borrow_mut().note_state(state) {
            self.refresh();
        }
    }

    pub fn notice(self: &Rc<Self>, text: &str, is_error: bool) {
        let id = self.log.borrow_mut().add(text, is_error, now_seconds());
        self.show_event(id, text, is_error);
    }

    /// A command Klipper or Moonraker rejected; its duplicate response is recognised by the reason.
    pub fn command_failed(self: &Rc<Self>, shown: &str, reason: &str) {
        let id = self.log.borrow_mut().add_command_failure(shown, reason, now_seconds());
        self.show_event(id, shown, true);
    }

    /// A `!!` (error) or `//` (information) response of Klipper. Other responses stay in the console.
    /// While a print runs, informations only go to the list: Klipper sends many of them and the status bar would flicker.
    pub fn console_response(self: &Rc<Self>, line: &ConsoleLine, printing: bool) {
        let is_error = line.is_error();
        if !is_error && !line.is_information() {
            return;
        }
        let text = line.text();
        if is_error || !printing {
            let added = self.log.borrow_mut().add_response(&text, is_error, now_seconds());
            if let Some(id) = added {
                self.show_event(id, &text, is_error);
            }
        } else {
            self.log.borrow_mut().add_quietly(&text, false, now_seconds());
            self.refresh();
        }
    }

    pub fn open_list(&self) {
        self.log.borrow_mut().open_list();
        self.refresh();
    }

    pub fn close_list(&self) {
        self.log.borrow_mut().close_list();
        self.refresh();
    }

    pub fn clear(&self) {
        self.log.borrow_mut().clear();
        self.refresh();
    }

    /// Shows a message in the status bar as an event; it leaves again after a while. Errors stay in the counter until
    /// the flyout is opened.
    fn show_event(self: &Rc<Self>, id: u64, text: &str, is_error: bool) {
        let view = Rc::downgrade(self);
        self.timer.start(TimerMode::SingleShot, display_time(text, is_error), move || {
            let Some(view) = view.upgrade() else { return };
            view.log.borrow_mut().hide_event(id);
            view.refresh();
        });
        self.refresh();
    }
}

fn now_seconds() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64())
}
