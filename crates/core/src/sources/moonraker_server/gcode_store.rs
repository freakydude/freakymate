//! The console: Moonraker's `gcode_store` of earlier lines and the responses Klipper sends as they come.

use crate::moonraker::Call;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleKind {
    Command,
    Response,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleLine {
    pub kind: ConsoleKind,
    pub message: String,
    /// Unix time in seconds.
    pub time: f64,
}

impl ConsoleLine {
    /// Klipper marks error responses with `!!`.
    pub fn is_error(&self) -> bool {
        self.message.starts_with("!!")
    }

    /// Klipper marks information responses with `//`. A host action is not one: it is meant for the client, not
    /// for a person.
    pub fn is_information(&self) -> bool {
        self.message.starts_with("//") && !self.is_host_action()
    }

    /// An instruction from a macro to the client, such as `// action:prompt_begin Title` (the dialogs Mainsail and
    /// Fluidd show) or `// action:notification ...`; Klipper's "action:" prefix of `RESPOND TYPE=command`.
    pub fn is_host_action(&self) -> bool {
        self.message.strip_prefix("//").is_some_and(|rest| rest.trim_start().starts_with("action:"))
    }

    /// A temperature report, such as Klipper sends every second while `M109` or `M190` waits
    /// (`T:210.3 /210.0 B:60.1 /60.0`) or answers `M105` (`ok B:…`). Same rule as Mainsail's "hide temperatures"
    /// (`^(?:ok\s+)?(B|C|T\d*):`, `src/store/gui/console/getters.ts`).
    pub fn is_temperature_report(&self) -> bool {
        if self.kind != ConsoleKind::Response {
            return false;
        }
        let text = self.message.strip_prefix("ok").map_or(self.message.as_str(), str::trim_start);
        let mut characters = text.chars();
        match characters.next() {
            Some('B' | 'C') => characters.next() == Some(':'),
            Some('T') => characters.find(|character| !character.is_ascii_digit()) == Some(':'),
            _ => false,
        }
    }

    /// The text as shown. A response loses its marker (`!!` error, `//` information; the colour tells them apart)
    /// and the HTML of plugins; a command is shown as typed. Klipper puts the marker before every line of a message.
    pub fn text(&self) -> String {
        match self.kind {
            ConsoleKind::Command => self.message.clone(),
            ConsoleKind::Response => {
                let lines: Vec<&str> = self.message.split('\n').map(strip_response_marker).collect();
                strip_markup(&lines.join("\n"))
            }
        }
    }
}

fn strip_response_marker(line: &str) -> &str {
    line.strip_prefix("!!").or_else(|| line.strip_prefix("//")).map_or(line, str::trim_start)
}

/// Happy Hare and other plugins put HTML into their responses for Mainsail (`<span style=...>`, `&nbsp;`); the
/// touch UI shows the text only. A `<` that does not start a tag stays.
pub fn strip_markup(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let (before, tag_and_after) = rest.split_at(start);
        plain.push_str(before);
        let starts_tag =
            tag_and_after[1..].chars().next().is_some_and(|next| next.is_ascii_alphabetic() || next == '/');
        match tag_and_after.find('>').filter(|_| starts_tag) {
            Some(end) => rest = &tag_and_after[end + 1..],
            None => {
                plain.push('<');
                rest = &tag_and_after[1..];
            }
        }
    }
    plain.push_str(rest);
    plain.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
}

/// Lines of the console history the display asks for.
const CONSOLE_HISTORY: u32 = 100;

/// `server.gcode_store`.
pub fn history_call() -> Call {
    Call::with("server.gcode_store", json!({ "count": CONSOLE_HISTORY }))
}

pub fn parse_gcode_store(result: &Value) -> Vec<ConsoleLine> {
    result
        .get("gcode_store")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| ConsoleLine {
                    kind: if entry.get("type").and_then(Value::as_str) == Some("command") {
                        ConsoleKind::Command
                    } else {
                        ConsoleKind::Response
                    },
                    message: entry.get("message").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    time: entry.get("time").and_then(Value::as_f64).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_gcode_store() {
        let lines = parse_gcode_store(&json!({ "gcode_store": [
            { "message": "G28", "time": 1.0, "type": "command" },
            { "message": "ok", "time": 2.0, "type": "response" }
        ]}));

        assert_eq!(lines[0].kind, ConsoleKind::Command);
        assert_eq!(lines[1].message, "ok");
    }

    fn response(message: &str) -> ConsoleLine {
        ConsoleLine { kind: ConsoleKind::Response, message: message.to_owned(), time: 0.0 }
    }

    #[test]
    fn console_text_loses_the_klipper_marker_and_plugin_markup() {
        assert_eq!(response("!! Lost connection").text(), "Lost connection");
        assert_eq!(response("// Klipper state: Ready").text(), "Klipper state: Ready");
        assert_eq!(response(r#"// <b><span style="color:#87CEEB">EMPTY</span></b>&nbsp;gate"#).text(), "EMPTY gate");
    }

    #[test]
    fn every_line_of_a_message_loses_its_marker() {
        let shutdown = response(
            "// Shutdown due to webhooks request\n// Once the underlying issue is corrected, use the\n\
             // \"FIRMWARE_RESTART\" command to reset the firmware, reload the\n// config, and restart the host software.\n\
             // Printer is shutdown",
        );

        assert_eq!(
            shutdown.text(),
            "Shutdown due to webhooks request\nOnce the underlying issue is corrected, use the\n\
             \"FIRMWARE_RESTART\" command to reset the firmware, reload the\nconfig, and restart the host software.\n\
             Printer is shutdown"
        );
    }

    #[test]
    fn plugin_markup_over_several_lines_loses_marker_and_tags() {
        let art = response("// <span style=\"color:#C0C0C0\">(\\_/)</span>\n// <span style=\"color:#C0C0C0\">(\")_(\")</span> <b>Happy Hare</b>");

        assert_eq!(art.text(), "(\\_/)\n(\")_(\") Happy Hare");
    }

    #[test]
    fn lines_without_a_marker_stay_as_they_are() {
        assert_eq!(response("// first\nplain line\n!! last").text(), "first\nplain line\nlast");
        assert_eq!(response("!! Lost connection\n!! see log").text(), "Lost connection\nsee log");
    }

    #[test]
    fn a_command_is_shown_as_typed() {
        let command = ConsoleLine { kind: ConsoleKind::Command, message: "// G28".to_owned(), time: 0.0 };

        assert_eq!(command.text(), "// G28");
    }

    #[test]
    fn only_bang_bang_responses_are_errors() {
        assert!(response("!! Timer too close").is_error());
        assert!(!response("// ok").is_error());
    }

    #[test]
    fn host_actions_are_no_information_for_a_person() {
        assert!(response("// action:prompt_begin MMU Error").is_host_action());
        assert!(!response("// action:prompt_show").is_information());
        assert!(response("// Bypass enabled").is_information());
        assert!(!response("// Bypass enabled").is_host_action());
    }

    #[test]
    fn markup_of_plugin_responses_is_removed() {
        assert_eq!(
            strip_markup(r#"<span style="color:#C0C0C0">(\_/)</span> <b>Happy Hare</b> v4.1.0"#),
            r"(\_/) Happy Hare v4.1.0"
        );
        assert_eq!(strip_markup("Gate&nbsp;0 &amp; tool &lt;T0&gt;"), "Gate 0 & tool <T0>");
        assert_eq!(strip_markup("Move out of range: x < 0 and y <5"), "Move out of range: x < 0 and y <5");
        assert_eq!(strip_markup("half <span"), "half <span");
        assert_eq!(strip_markup(""), "");
    }

    #[test]
    fn temperature_reports_are_recognised_like_mainsail_does() {
        let response = |message: &str| ConsoleLine { kind: ConsoleKind::Response, message: message.into(), time: 0.0 };
        assert!(response("T:210.3 /210.0 B:60.1 /60.0").is_temperature_report());
        assert!(response("ok B:60.0 /60.0 T0:210.0 /210.0").is_temperature_report());
        assert!(response("T0:205.1 /210.0").is_temperature_report());
        assert!(response("B:59.8 /60.0").is_temperature_report());
        assert!(!response("// Toolhead at T0").is_temperature_report());
        assert!(!response("Tool T0 selected").is_temperature_report());
        assert!(!response("ok").is_temperature_report());
        let command = ConsoleLine { kind: ConsoleKind::Command, message: "T:1".into(), time: 0.0 };
        assert!(!command.is_temperature_report(), "only responses are filtered");
    }
}
