//! Messages of the status bar and its flyout: errors and informations from Klipper and from the app.
//!
//! Every new message is an event the status bar shows for a while; unread errors are only counted until the flyout
//! is opened. Pure logic without Slint; the presenter puts the result on screen.

use std::collections::VecDeque;
use std::time::Duration;

const LIMIT: usize = 50;
/// Klipper reports a rejected command twice: as RPC error and as `!!` response. A response this close to the
/// failure and repeating its reason is the same event.
const DUPLICATE_WINDOW_SECONDS: f64 = 2.0;
/// An error stays in the status bar for this long; the counter keeps it unread after that.
const ERROR_DISPLAY_TIME: Duration = Duration::from_secs(15);
/// An information stays in the status bar for a base time plus the time to read it.
const INFO_BASE: Duration = Duration::from_secs(5);
/// Relaxed reading is about 13 to 15 characters per second.
const INFO_PER_CHARACTER: Duration = Duration::from_millis(70);
const MAX_INFO_DISPLAY: Duration = Duration::from_secs(15);

/// Layout of the flyout list, in logical pixels: a message row has the symbol before and the time after its text.
const ROW_GAP: f32 = 6.0;
/// Height of a section heading ("New", "Earlier") including the gap below it.
const SECTION_HEIGHT: f32 = 24.0;
const LINE_HEIGHT: f32 = 22.0;
const ROW_PADDING: f32 = 20.0;
/// Roughly what fits into the width of the flyout at the normal font size, beside the symbol and the time.
const CHARACTERS_PER_LINE: usize = 40;
const MAX_LINES: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: u64,
    /// The whole text; a multi-line message stays one entry.
    pub text: String,
    pub is_error: bool,
    /// Unix time in seconds.
    pub time: f64,
}

impl Message {
    /// The first line, which the status bar shows.
    pub fn headline(&self) -> &str {
        self.text.lines().next().unwrap_or_default()
    }
}

#[derive(Debug, Default)]
pub struct StatusLog {
    messages: VecDeque<Message>,
    next_id: u64,
    /// The event the status bar shows; it runs out by itself.
    event: Option<u64>,
    /// Messages with a smaller id were there when the flyout was closed last: they are "Earlier", the rest is "New".
    boundary: u64,
    list_open: bool,
    /// Reason of the last rejected command, to recognise its duplicate response.
    last_failure: Option<(String, f64)>,
    /// The last `!!` response (id, text, time): it may arrive before the rejection it repeats.
    last_error_response: Option<(u64, String, f64)>,
    /// What the status bar's summary showed last (connection, Klipper and print state), to notice a change.
    state: Option<String>,
}

impl StatusLog {
    /// Adds a message and makes it the event the status bar shows. Returns its id.
    pub fn add(&mut self, text: &str, is_error: bool, now: f64) -> u64 {
        let id = self.add_quietly(text, is_error, now);
        self.event = Some(id);
        id
    }

    /// Adds a message to the list only: the status bar keeps showing what it shows. Returns its id.
    pub fn add_quietly(&mut self, text: &str, is_error: bool, now: f64) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.messages.push_back(Message { id, text: text.to_owned(), is_error, time: now });
        if self.messages.len() > LIMIT {
            self.messages.pop_front();
        }
        id
    }

    /// A command of the app was rejected. `shown` goes to the list, `reason` is what Klipper repeats in its response.
    /// Klipper's response may come first: then the rejection, which also names the command, replaces it.
    pub fn add_command_failure(&mut self, shown: &str, reason: &str, now: f64) -> u64 {
        if let Some((id, _, _)) = self
            .last_error_response
            .take()
            .filter(|(_, text, time)| now - time <= DUPLICATE_WINDOW_SECONDS && text.contains(reason))
        {
            self.messages.retain(|message| message.id != id);
        }
        self.last_failure = Some((reason.to_owned(), now));
        self.add(shown, true, now)
    }

    /// A `!!` or `//` response of Klipper. Returns `None` for an error that repeats the failure just reported.
    pub fn add_response(&mut self, text: &str, is_error: bool, now: f64) -> Option<u64> {
        if is_error && self.repeats_last_failure(text, now) {
            return None;
        }
        let id = self.add(text, is_error, now);
        if is_error {
            self.last_error_response = Some((id, text.to_owned(), now));
        }
        Some(id)
    }

    fn repeats_last_failure(&self, text: &str, now: f64) -> bool {
        self.last_failure
            .as_ref()
            .is_some_and(|(reason, time)| now - time <= DUPLICATE_WINDOW_SECONDS && text.contains(reason.as_str()))
    }

    /// The event the status bar shows, if any.
    pub fn event(&self) -> Option<&Message> {
        let id = self.event?;
        self.messages.iter().find(|message| message.id == id)
    }

    /// All messages, newest first.
    pub fn messages(&self) -> impl Iterator<Item = &Message> {
        self.messages.iter().rev()
    }

    /// Whether the flyout lists the message under "New": it came after the flyout was closed last.
    pub fn is_new(&self, message: &Message) -> bool {
        message.id >= self.boundary
    }

    /// Errors that came since the flyout was closed last. Zero while it is open, as everything is seen then.
    pub fn unread_errors(&self) -> usize {
        if self.list_open {
            return 0;
        }
        self.messages.iter().filter(|message| message.is_error && self.is_new(message)).count()
    }

    /// The flyout was opened: what came since it was closed last is "New" until it closes again.
    pub fn open_list(&mut self) {
        self.list_open = true;
    }

    /// The flyout was closed: everything in it is "Earlier" from now on.
    pub fn close_list(&mut self) {
        self.list_open = false;
        self.boundary = self.next_id;
    }

    /// The event ran out of display time. An older error does not take its place: the counter keeps it.
    pub fn hide_event(&mut self, id: u64) {
        if self.event == Some(id) {
            self.event = None;
        }
    }

    /// Tells the log the state the summary shows. When it changes, an information in the status bar ends at once,
    /// as the state speaks for itself; it stays in the list. An error keeps its display time. Returns whether the
    /// event was ended.
    pub fn note_state(&mut self, state: &str) -> bool {
        let changed = self.state.as_deref().is_some_and(|last| last != state);
        if self.state.as_deref() != Some(state) {
            self.state = Some(state.to_owned());
        }
        let information_shown = self.event().is_some_and(|message| !message.is_error);
        if changed && information_shown {
            self.event = None;
        }
        changed && information_shown
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.event = None;
        self.boundary = self.next_id;
    }
}

/// Height of a message row: one line per wrapped line of the text, but never below the minimum.
pub fn row_height(text: &str) -> f32 {
    let lines: usize = text.lines().map(|line| line.chars().count().div_ceil(CHARACTERS_PER_LINE).max(1)).sum();
    lines.clamp(1, MAX_LINES) as f32 * LINE_HEIGHT + ROW_PADDING
}

/// A message of the flyout list with its place: rows are laid out by hand, as the list scrolls and sections vary.
#[derive(Debug, PartialEq)]
pub struct Placed<'a> {
    pub message: &'a Message,
    pub new: bool,
    /// Distance from the top of the list, in logical pixels.
    pub top: f32,
    pub height: f32,
}

/// The flyout list: newest first, a heading for "New" and one for "Earlier" when there are such messages.
#[derive(Debug, PartialEq)]
pub struct Layout<'a> {
    pub rows: Vec<Placed<'a>>,
    /// Height of the whole list.
    pub height: f32,
    pub has_new: bool,
    pub has_earlier: bool,
    /// Where the "Earlier" heading sits; zero without earlier messages.
    pub earlier_top: f32,
}

impl StatusLog {
    pub fn layout(&self) -> Layout<'_> {
        let has_new = self.messages().any(|message| self.is_new(message));
        let has_earlier = self.messages().any(|message| !self.is_new(message));
        let mut top = if has_new { SECTION_HEIGHT } else { 0.0 };
        let mut earlier_top = -1.0;
        let rows = self
            .messages()
            .map(|message| {
                let new = self.is_new(message);
                if !new && earlier_top < 0.0 {
                    earlier_top = top;
                    top += SECTION_HEIGHT;
                }
                let height = row_height(&message.text);
                let placed = Placed { message, new, top, height };
                top += height + ROW_GAP;
                placed
            })
            .collect();
        Layout { rows, height: top, has_new, has_earlier, earlier_top: earlier_top.max(0.0) }
    }
}

/// How long a message stays in the status bar as an event.
pub fn display_time(text: &str, is_error: bool) -> Duration {
    if is_error {
        ERROR_DISPLAY_TIME
    } else {
        info_display_time(text)
    }
}

/// How long an information stays in the status bar.
fn info_display_time(text: &str) -> Duration {
    let characters = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    (INFO_BASE + INFO_PER_CHARACTER * characters).min(MAX_INFO_DISPLAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_message_is_shown_and_listed_first() {
        let mut log = StatusLog::default();
        log.add("first", true, 1.0);
        log.add("second", false, 2.0);

        assert_eq!(log.event().map(Message::headline), Some("second"));
        let listed: Vec<&str> = log.messages().map(|message| message.text.as_str()).collect();
        assert_eq!(listed, ["second", "first"]);
    }

    #[test]
    fn closing_the_list_keeps_the_event_and_the_list() {
        let mut log = StatusLog::default();
        log.add("error", true, 1.0);

        log.open_list();
        log.close_list();

        assert!(log.event().is_some());
        assert_eq!(log.messages().count(), 1);
    }

    #[test]
    fn hiding_an_old_event_keeps_a_newer_one() {
        let mut log = StatusLog::default();
        let old = log.add("old info", false, 1.0);
        log.add("error", true, 2.0);

        log.hide_event(old);

        assert_eq!(log.event().map(Message::headline), Some("error"));
    }

    #[test]
    fn response_repeating_a_failure_is_skipped() {
        let mut log = StatusLog::default();
        log.add_command_failure("G1 X10: Must home axis first", "Must home axis first", 10.0);

        assert!(log.add_response("Must home axis first: 0 0", true, 10.5).is_none());
        assert_eq!(log.messages().count(), 1);
    }

    #[test]
    fn a_rejection_after_its_response_replaces_it() {
        let mut log = StatusLog::default();
        log.add_response("Unknown fan 'missing'", true, 10.0);

        log.add_command_failure("SET_FAN_SPEED FAN=missing: Unknown fan 'missing'", "Unknown fan 'missing'", 10.2);

        let listed: Vec<&str> = log.messages().map(|message| message.text.as_str()).collect();
        assert_eq!(listed, ["SET_FAN_SPEED FAN=missing: Unknown fan 'missing'"]);
        assert_eq!(log.event().map(Message::headline), Some(listed[0]));
    }

    #[test]
    fn repeat_after_the_window_is_a_new_message() {
        let mut log = StatusLog::default();
        log.add_command_failure("G1 X10: Must home axis first", "Must home axis first", 10.0);

        assert!(log.add_response("Must home axis first", true, 20.0).is_some());
    }

    #[test]
    fn informations_are_never_skipped_as_duplicates() {
        let mut log = StatusLog::default();
        log.add_command_failure("x", "Must home", 10.0);

        assert!(log.add_response("Must home", false, 10.1).is_some());
    }

    #[test]
    fn list_is_limited_to_the_newest_messages() {
        let mut log = StatusLog::default();
        for index in 0..60 {
            log.add(&format!("message {index}"), true, index as f64);
        }

        assert_eq!(log.messages().count(), LIMIT);
        assert_eq!(log.messages().last().map(|message| message.text.as_str()), Some("message 10"));
    }

    #[test]
    fn multi_line_message_is_one_entry_with_its_first_line_in_the_status() {
        let mut log = StatusLog::default();
        log.add("Available commands:\nG0, G1\nG28", false, 1.0);

        assert_eq!(log.messages().count(), 1);
        assert_eq!(log.event().map(Message::headline), Some("Available commands:"));
    }

    #[test]
    fn information_time_grows_with_length_up_to_a_limit() {
        assert_eq!(info_display_time(""), Duration::from_secs(5));
        assert_eq!(info_display_time(&"x".repeat(25)), Duration::from_millis(6750));
        assert_eq!(info_display_time(&"x".repeat(1000)), Duration::from_secs(15));
    }

    #[test]
    fn the_layout_puts_new_messages_first_and_a_heading_before_the_earlier_ones() {
        let mut log = StatusLog::default();
        log.add("old", false, 1.0);
        log.open_list();
        log.close_list();
        log.add("fresh", false, 2.0);

        let layout = log.layout();

        let row = row_height("fresh");
        assert!(layout.has_new && layout.has_earlier);
        let tops: Vec<f32> = layout.rows.iter().map(|placed| placed.top).collect();
        assert_eq!(tops, [SECTION_HEIGHT, SECTION_HEIGHT + row + ROW_GAP + SECTION_HEIGHT]);
        assert_eq!(layout.earlier_top, SECTION_HEIGHT + row + ROW_GAP);
        assert_eq!(layout.height, 2.0 * SECTION_HEIGHT + 2.0 * (row + ROW_GAP));
        assert_eq!(layout.rows.iter().map(|placed| placed.new).collect::<Vec<_>>(), [true, false]);
    }

    #[test]
    fn the_layout_of_an_empty_log_is_empty() {
        let log = StatusLog::default();

        let layout = log.layout();

        assert!(layout.rows.is_empty() && !layout.has_new && !layout.has_earlier);
        assert_eq!((layout.height, layout.earlier_top), (0.0, 0.0));
    }

    #[test]
    fn row_height_grows_with_wrapped_lines_up_to_a_limit() {
        let frame = ROW_PADDING;
        assert_eq!(row_height("short"), 22.0 + frame);
        assert_eq!(row_height("line one\nline two\nline three"), 3.0 * 22.0 + frame);
        assert_eq!(row_height(&"x".repeat(100)), 3.0 * 22.0 + frame);
        assert_eq!(row_height(&"x\n".repeat(30)), 10.0 * 22.0 + frame);
    }

    #[test]
    fn clear_empties_list_event_and_counter() {
        let mut log = StatusLog::default();
        log.add("error", true, 1.0);

        log.clear();

        assert!(log.event().is_none());
        assert_eq!(log.messages().count(), 0);
        assert_eq!(log.unread_errors(), 0);
    }

    #[test]
    fn an_expired_event_does_not_bring_back_an_older_error() {
        let mut log = StatusLog::default();
        log.add("fault", true, 1.0);
        let info = log.add("note", false, 2.0);
        assert_eq!(log.event().map(Message::headline), Some("note"));

        log.hide_event(info);

        assert!(log.event().is_none());
        assert_eq!(log.unread_errors(), 1);
    }

    #[test]
    fn a_quiet_message_is_listed_without_replacing_the_event() {
        let mut log = StatusLog::default();
        log.add("fault", true, 1.0);

        log.add_quietly("note", false, 2.0);

        assert_eq!(log.event().map(Message::headline), Some("fault"));
        assert_eq!(log.messages().count(), 2);
    }

    #[test]
    fn an_error_is_an_event_like_an_information() {
        let mut log = StatusLog::default();
        let error = log.add("fault", true, 1.0);

        assert_eq!(log.event().map(Message::headline), Some("fault"));
        log.hide_event(error);
        assert!(log.event().is_none());
    }

    #[test]
    fn a_state_change_ends_an_information_but_not_an_error() {
        let mut log = StatusLog::default();
        assert!(!log.note_state("ready printing"), "the first state is only remembered");
        log.add("Pause at the next layer sent", false, 1.0);
        assert!(!log.note_state("ready printing"), "no change, the information stays");
        assert!(log.event().is_some());
        assert!(log.note_state("ready cancelled"));
        assert!(log.event().is_none());
        assert_eq!(log.messages().count(), 1, "the message stays in the list");
        log.add("Move out of range", true, 2.0);
        assert!(!log.note_state("ready standby"));
        assert!(log.event().is_some_and(|message| message.is_error));
    }

    #[test]
    fn errors_display_for_fifteen_seconds_informations_by_their_length() {
        assert_eq!(display_time("x", true), Duration::from_secs(15));
        assert_eq!(display_time("", false), Duration::from_secs(5));
    }

    #[test]
    fn only_errors_count_as_unread() {
        let mut log = StatusLog::default();
        log.add("fault", true, 1.0);
        log.add("note", false, 2.0);
        log.add("another fault", true, 3.0);

        assert_eq!(log.unread_errors(), 2);
    }

    #[test]
    fn opening_the_list_resets_the_counter_and_keeps_new_messages_new() {
        let mut log = StatusLog::default();
        log.add("fault", true, 1.0);
        log.add("note", false, 2.0);

        log.open_list();

        assert_eq!(log.unread_errors(), 0);
        assert!(log.messages().all(|message| log.is_new(message)));
    }

    #[test]
    fn everything_is_new_at_the_first_opening() {
        let mut log = StatusLog::default();
        log.add("note", false, 1.0);
        log.open_list();

        assert!(log.messages().all(|message| log.is_new(message)));
    }

    #[test]
    fn a_message_during_the_open_list_is_new_and_not_unread() {
        let mut log = StatusLog::default();
        log.add("old", true, 1.0);
        log.open_list();
        log.close_list();
        log.open_list();

        log.add("fault", true, 2.0);

        assert_eq!(log.unread_errors(), 0);
        let new: Vec<&str> =
            log.messages().filter(|message| log.is_new(message)).map(|message| message.text.as_str()).collect();
        assert_eq!(new, ["fault"]);
    }

    #[test]
    fn closing_the_list_makes_everything_earlier() {
        let mut log = StatusLog::default();
        log.add("fault", true, 1.0);
        log.open_list();
        log.add("while open", true, 2.0);

        log.close_list();

        assert!(log.messages().all(|message| !log.is_new(message)));
        assert_eq!(log.unread_errors(), 0);
    }

    #[test]
    fn the_next_opening_shows_only_what_came_after_the_closing_as_new() {
        let mut log = StatusLog::default();
        log.add("before", true, 1.0);
        log.open_list();
        log.close_list();
        log.add("after", true, 2.0);
        assert_eq!(log.unread_errors(), 1);

        log.open_list();

        let new: Vec<&str> =
            log.messages().filter(|message| log.is_new(message)).map(|message| message.text.as_str()).collect();
        assert_eq!(new, ["after"]);
        assert_eq!(log.unread_errors(), 0);
    }
}
