//! Reads and writes files in the INI dialect of Klipper's `printer.cfg` and Moonraker's `moonraker.conf`, so they
//! read the same and Mainsail highlights them the same. Text a person wrote stays as it was; only changed values are
//! rewritten. Like Klipper, a file can end in a block below the SAVE_CONFIG line that the program writes in full and
//! nobody edits by hand.

mod document;
pub(crate) mod value;

pub(crate) use document::{Document, Section, Separator};

/// Klipper's `AUTOSAVE_HEADER` (`klippy/configfile.py`), word for word, so the block looks like the one people know.
const SAVE_CONFIG_HEADER: &str = "\n#*# <---------------------- SAVE_CONFIG ---------------------->\n\
#*# DO NOT EDIT THIS BLOCK OR BELOW. The contents are auto-generated.\n#*#\n";
/// The line that starts the block.
const SAVE_CONFIG_MARKER: &str = "#*# <---------------------- SAVE_CONFIG ---------------------->";
/// Every line of the block starts with it; Klipper strips `#*# `.
const SAVE_CONFIG_PREFIX: &str = "#*#";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {message}")]
pub(crate) struct ParseError {
    pub(crate) line: usize,
    pub(crate) message: String,
}

impl ParseError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self { line, message: message.into() }
    }
}

/// A file: the part people edit and the block below the SAVE_CONFIG line.
#[derive(Debug, Clone)]
pub(crate) struct ConfigFile {
    pub(crate) body: Document,
    pub(crate) saved: Document,
}

impl ConfigFile {
    /// A new file starting with these comment lines.
    pub(crate) fn new(preamble: &str) -> Self {
        Self { body: Document::new(preamble, Separator::Colon), saved: Self::empty_block() }
    }

    /// A mistake above the SAVE_CONFIG line is an error. A block below it that cannot be read is only logged and
    /// left out, since it is written anew from what the program knows; a line there without `#*#` is an error,
    /// so nothing someone wrote gets lost.
    pub(crate) fn parse(text: &str) -> Result<Self, ParseError> {
        let lines: Vec<&str> = text.lines().map(|line| line.trim_end_matches('\r')).collect();
        let marker = lines.iter().position(|line| line.trim_end() == SAVE_CONFIG_MARKER);
        let body_lines = &lines[..marker.unwrap_or(lines.len())];
        let body = Document::parse(&body_lines.join("\n"), 0, Separator::Colon)?;
        let saved = match marker {
            Some(index) => Self::parse_block(&lines[index + 1..], index + 1)?,
            None => Self::empty_block(),
        };
        Ok(Self { body, saved })
    }

    pub(crate) fn render(&self) -> String {
        let mut text = self.body.render().trim_end().to_owned();
        text.push('\n');
        if !self.saved.is_empty() {
            text.push_str(SAVE_CONFIG_HEADER);
            for line in self.saved.render().lines() {
                text.push_str(format!("{SAVE_CONFIG_PREFIX} {line}").trim_end());
                text.push('\n');
            }
        }
        text
    }

    fn empty_block() -> Document {
        Document::new("", Separator::Equals)
    }

    /// `before` is the number of lines above the block.
    fn parse_block(lines: &[&str], before: usize) -> Result<Document, ParseError> {
        let mut stripped = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter().enumerate() {
            match line.strip_prefix(SAVE_CONFIG_PREFIX) {
                Some(rest) => stripped.push(rest.strip_prefix(' ').unwrap_or(rest)),
                None if line.trim().is_empty() => stripped.push(""),
                None => {
                    return Err(ParseError::new(
                        before + index + 1,
                        "below the SAVE_CONFIG line every line starts with `#*#`; move this one above it",
                    ));
                }
            }
        }
        // The notice lines of the header come before the first section.
        let first = stripped.iter().position(|line| line.starts_with('[')).unwrap_or(stripped.len());
        match Document::parse(&stripped[first..].join("\n"), before + first, Separator::Equals) {
            Ok(document) => Ok(document),
            Err(error) => {
                log::warn!("SAVE_CONFIG block, {error}; it is written anew");
                Ok(Self::empty_block())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# settings\n\n[printer voron]\nmoonraker: http://voron:7125\n\n\
#*# <---------------------- SAVE_CONFIG ---------------------->\n\
#*# DO NOT EDIT THIS BLOCK OR BELOW. The contents are auto-generated.\n#*#\n\
#*# [sailhand]\n#*# printer = voron\n#*#\n#*# [printer voron]\n#*# uuid = 29f7\n";

    #[test]
    fn the_block_below_save_config_is_read_apart_and_written_like_klipper() {
        let file = ConfigFile::parse(FILE).unwrap();

        assert_eq!(file.body.section("printer", Some("voron")).unwrap().get("moonraker"), Some("http://voron:7125"));
        assert!(file.body.section("sailhand", None).is_none());
        assert_eq!(file.saved.section("sailhand", None).unwrap().get("printer"), Some("voron"));
        assert_eq!(file.saved.section("printer", Some("voron")).unwrap().line_of("uuid"), 13);
        assert_eq!(file.render(), FILE);
    }

    #[test]
    fn a_file_without_a_block_gets_one_when_there_is_something_to_save() {
        let mut file = ConfigFile::parse("[printer voron]\nmoonraker: x\n\n\n").unwrap();
        assert_eq!(file.render(), "[printer voron]\nmoonraker: x\n");

        file.saved.ensure_section("sailhand", None).set("printer", "voron");

        assert!(file.render().ends_with(
            "moonraker: x\n\n#*# <---------------------- SAVE_CONFIG ---------------------->\n\
#*# DO NOT EDIT THIS BLOCK OR BELOW. The contents are auto-generated.\n#*#\n#*# [sailhand]\n#*# printer = voron\n"
        ));
    }

    #[test]
    fn a_line_without_the_prefix_below_the_block_is_an_error() {
        let text = format!("{FILE}fullscreen: True\n");

        assert_eq!(ConfigFile::parse(&text).unwrap_err().line, 14);
    }

    #[test]
    fn an_unreadable_block_is_left_out() {
        let text = FILE.replace("#*# uuid = 29f7", "#*# just words");

        let file = ConfigFile::parse(&text).unwrap();

        assert!(file.saved.is_empty());
        assert!(file.body.section("printer", Some("voron")).is_some());
    }

    #[test]
    fn windows_line_endings_are_read() {
        let file = ConfigFile::parse(&FILE.replace('\n', "\r\n")).unwrap();

        assert_eq!(file.render(), FILE);
    }
}
