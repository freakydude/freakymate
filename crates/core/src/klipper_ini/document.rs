//! One INI document kept line by line: what is not changed is written back exactly as it was read, comments
//! included. The rules follow the readers of Klipper (`klippy/configfile.py`) and Moonraker
//! (`moonraker/confighelper.py`): `[kind name]` headers, `key: value` or `key = value`, keys without case, full-line
//! comments with `#` or `;`, inline comments after whitespace, values continued on indented lines.

use super::ParseError;

/// Indent of a continuation line Sailhand writes; Moonraker reads a tab as four spaces, Klipper keeps it.
const CONTINUATION_INDENT: &str = "    ";

/// Between key and value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Separator {
    /// `key: value`, as the Klipper documentation writes options.
    Colon,
    /// `key = value`, as Klipper writes its SAVE_CONFIG block.
    Equals,
}

impl Separator {
    fn after_key(self) -> &'static str {
        match self {
            Separator::Colon => ":",
            Separator::Equals => " =",
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    key: String,
    value: String,
    /// Line in the file, 1-based; 0 for an entry Sailhand added.
    line: usize,
    /// The lines as read; `None` once the value changed, then the entry is written anew.
    text: Option<Vec<String>>,
    separator: Separator,
    /// An inline comment with the whitespace before it; it stays when a single-line value changes.
    comment: String,
}

#[derive(Debug, Clone)]
enum Item {
    /// A blank or comment line.
    Text(String),
    Entry(Entry),
}

#[derive(Debug, Clone)]
pub(crate) struct Section {
    kind: String,
    name: Option<String>,
    line: usize,
    /// Comment lines right above the header; they belong to this section and go with it.
    leading: Vec<String>,
    header: Option<String>,
    items: Vec<Item>,
    separator: Separator,
}

#[derive(Debug, Clone)]
pub(crate) struct Document {
    /// Blank and comment lines before the first section.
    preamble: Vec<String>,
    sections: Vec<Section>,
    /// For entries Sailhand adds.
    separator: Separator,
}

impl Document {
    pub(crate) fn new(preamble: &str, separator: Separator) -> Self {
        Self { preamble: preamble.lines().map(str::to_owned).collect(), sections: Vec::new(), separator }
    }

    /// `first_line` is the number of the line before `text` in the file, for messages.
    pub(crate) fn parse(text: &str, first_line: usize, separator: Separator) -> Result<Self, ParseError> {
        let mut parser = Parser { document: Self::new("", separator), pending: Vec::new(), open: false };
        for (index, line) in text.lines().enumerate() {
            parser.line(first_line + index + 1, line.trim_end_matches('\r'))?;
        }
        parser.flush_pending();
        Ok(parser.document)
    }

    pub(crate) fn render(&self) -> String {
        let mut lines: Vec<String> = self.preamble.clone();
        for section in &self.sections {
            section.render_into(&mut lines);
        }
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    pub(crate) fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub(crate) fn section(&self, kind: &str, name: Option<&str>) -> Option<&Section> {
        self.position(kind, name).map(|index| &self.sections[index])
    }

    /// The section, added after the last one of the same kind (else at the end) if there is none yet.
    pub(crate) fn ensure_section(&mut self, kind: &str, name: Option<&str>) -> &mut Section {
        if let Some(index) = self.position(kind, name) {
            return &mut self.sections[index];
        }
        let index =
            self.sections.iter().rposition(|section| section.kind == kind).map_or(self.sections.len(), |i| i + 1);
        let mut section = Section {
            kind: kind.to_owned(),
            name: name.map(str::to_owned),
            line: 0,
            leading: Vec::new(),
            header: None,
            items: Vec::new(),
            separator: self.separator,
        };
        if index < self.sections.len() {
            section.items.push(Item::Text(String::new()));
        }
        match index.checked_sub(1) {
            Some(previous) => self.sections[previous].end_with_blank_line(),
            None if self.preamble.last().is_some_and(|line| !line.trim().is_empty()) => {
                self.preamble.push(String::new())
            }
            None => {}
        }
        self.sections.insert(index, section);
        &mut self.sections[index]
    }

    /// Removes the header and the entries of the section; returns whether there was one. Its comment lines stay
    /// where they were: one may be a section someone commented out.
    pub(crate) fn remove_section(&mut self, kind: &str, name: Option<&str>) -> bool {
        let Some(index) = self.position(kind, name) else { return false };
        let section = self.sections.remove(index);
        let texts = section.items.into_iter().filter_map(|item| match item {
            Item::Text(text) => Some(text),
            Item::Entry(_) => None,
        });
        let kept: Vec<String> = section.leading.into_iter().chain(texts).collect();
        if !kept.iter().any(|line| is_comment(line.trim())) {
            return true;
        }
        match index.checked_sub(1) {
            Some(previous) => self.sections[previous].items.extend(kept.into_iter().map(Item::Text)),
            None => self.preamble.extend(kept),
        }
        true
    }

    fn position(&self, kind: &str, name: Option<&str>) -> Option<usize> {
        self.sections.iter().position(|section| section.kind == kind && section.name.as_deref() == name)
    }
}

impl Section {
    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }

    pub(crate) fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Line of the header; 0 for a section Sailhand added.
    pub(crate) fn line(&self) -> usize {
        self.line
    }

    /// `[kind name]`, for messages.
    pub(crate) fn title(&self) -> String {
        match &self.name {
            Some(name) => format!("[{} {name}]", self.kind),
            None => format!("[{}]", self.kind),
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.entry(key).map(|entry| entry.value.as_str())
    }

    /// Key, value and line of every entry, in the order of the file.
    pub(crate) fn entries(&self) -> impl Iterator<Item = (&str, &str, usize)> {
        self.items.iter().filter_map(|item| match item {
            Item::Entry(entry) => Some((entry.key.as_str(), entry.value.as_str(), entry.line)),
            Item::Text(_) => None,
        })
    }

    /// Line of the entry, else of the header; 0 for what Sailhand added.
    pub(crate) fn line_of(&self, key: &str) -> usize {
        self.entry(key).map_or(self.line, |entry| entry.line)
    }

    /// Sets the value; an entry with the same value keeps its text. A value with line breaks is written on indented
    /// lines below the key. New entries go after the last entry or comment of the section.
    pub(crate) fn set(&mut self, key: &str, value: &str) {
        let key = key.to_ascii_lowercase();
        if let Some(entry) = self.entry_mut(&key) {
            if entry.value != value {
                entry.value = value.to_owned();
                entry.text = None;
            }
            return;
        }
        let index = self.items.iter().rposition(|item| !is_blank(item)).map_or(0, |i| i + 1);
        // Like the entries someone already wrote here.
        let separator = self.items.iter().rev().find_map(|item| match item {
            Item::Entry(entry) => Some(entry.separator),
            Item::Text(_) => None,
        });
        let separator = separator.unwrap_or(self.separator);
        let entry = Entry { key, value: value.to_owned(), line: 0, text: None, separator, comment: String::new() };
        self.items.insert(index, Item::Entry(entry));
    }

    /// Returns whether there was such an entry.
    pub(crate) fn remove(&mut self, key: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| !matches!(item, Item::Entry(entry) if entry.key == key));
        self.items.len() != before
    }

    fn entry(&self, key: &str) -> Option<&Entry> {
        self.items.iter().find_map(|item| match item {
            Item::Entry(entry) if entry.key == key => Some(entry),
            _ => None,
        })
    }

    fn entry_mut(&mut self, key: &str) -> Option<&mut Entry> {
        self.items.iter_mut().find_map(|item| match item {
            Item::Entry(entry) if entry.key == key => Some(entry),
            _ => None,
        })
    }

    fn end_with_blank_line(&mut self) {
        if !self.items.last().is_some_and(is_blank) {
            self.items.push(Item::Text(String::new()));
        }
    }

    fn render_into(&self, lines: &mut Vec<String>) {
        lines.extend(self.leading.iter().cloned());
        lines.push(self.header.clone().unwrap_or_else(|| self.title()));
        for item in &self.items {
            match item {
                Item::Text(text) => lines.push(text.clone()),
                Item::Entry(entry) => entry.render_into(lines),
            }
        }
    }
}

impl Entry {
    fn render_into(&self, lines: &mut Vec<String>) {
        if let Some(text) = &self.text {
            lines.extend(text.iter().cloned());
            return;
        }
        let key = format!("{}{}", self.key, self.separator.after_key());
        if self.value.contains('\n') {
            lines.push(key);
            lines.extend(self.value.lines().map(|line| format!("{CONTINUATION_INDENT}{line}")));
        } else if self.value.is_empty() {
            lines.push(key);
        } else {
            lines.push(format!("{key} {}{}", self.value, self.comment));
        }
    }
}

fn is_blank(item: &Item) -> bool {
    matches!(item, Item::Text(text) if text.trim().is_empty())
}

fn is_comment(trimmed: &str) -> bool {
    trimmed.starts_with(['#', ';'])
}

/// Splits off an inline comment: `#` or `;` after whitespace, as Moonraker reads it (`re.search(r" +[#;]", line)`).
/// Returns the text before it and the comment with its leading whitespace.
fn split_inline_comment(text: &str) -> (&str, &str) {
    let bytes = text.as_bytes();
    let found = (1..bytes.len()).find(|&i| matches!(bytes[i], b'#' | b';') && bytes[i - 1].is_ascii_whitespace());
    match found {
        Some(index) => {
            let start = text[..index].trim_end().len();
            (&text[..start], &text[start..])
        }
        None => (text, ""),
    }
}

struct Parser {
    document: Document,
    /// Blank and comment lines after an entry: part of its value if an indented line follows, else lines of their own.
    pending: Vec<String>,
    /// Whether the last entry can still be continued.
    open: bool,
}

impl Parser {
    fn line(&mut self, number: usize, line: &str) -> Result<(), ParseError> {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_comment(trimmed) {
            if self.open {
                self.pending.push(line.to_owned());
            } else {
                self.push_text(line.to_owned());
            }
            return Ok(());
        }
        if self.open && line.starts_with([' ', '\t']) {
            self.continue_entry(line, trimmed);
            return Ok(());
        }
        self.flush_pending();
        if trimmed.starts_with('[') { self.header(number, line, trimmed) } else { self.entry(number, line, trimmed) }
    }

    fn continue_entry(&mut self, line: &str, trimmed: &str) {
        let pending = std::mem::take(&mut self.pending);
        let Some(entry) = self.last_entry() else { return };
        let text = entry.text.get_or_insert_with(Vec::new);
        text.extend(pending);
        text.push(line.to_owned());
        let (value, _) = split_inline_comment(trimmed);
        if entry.value.is_empty() {
            entry.value = value.to_owned();
        } else {
            entry.value = format!("{}\n{value}", entry.value);
        }
    }

    fn header(&mut self, number: usize, line: &str, trimmed: &str) -> Result<(), ParseError> {
        let (header, _) = split_inline_comment(trimmed);
        let inner = header
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .ok_or_else(|| ParseError::new(number, "a section header ends with `]`"))?
            .trim();
        let (kind, name) = match inner.split_once(char::is_whitespace) {
            Some((kind, name)) => (kind, Some(name.trim().to_owned())),
            None => (inner, None),
        };
        if kind.is_empty() {
            return Err(ParseError::new(number, "a section header needs a name, e.g. `[sailhand]`"));
        }
        if inner.contains(['[', ']']) {
            return Err(ParseError::new(number, "a section header has one pair of brackets, e.g. `[printer voron]`"));
        }
        let kind = kind.to_ascii_lowercase();
        if self.document.position(&kind, name.as_deref()).is_some() {
            return Err(ParseError::new(number, format!("section `{}` appears twice", line.trim())));
        }
        let leading = self.take_leading_comments();
        self.document.sections.push(Section {
            kind,
            name,
            line: number,
            leading,
            header: Some(line.to_owned()),
            items: Vec::new(),
            separator: self.document.separator,
        });
        Ok(())
    }

    fn entry(&mut self, number: usize, line: &str, trimmed: &str) -> Result<(), ParseError> {
        let Some(split) = trimmed.find([':', '=']) else {
            return Err(ParseError::new(number, "expected `key: value`"));
        };
        let key = trimmed[..split].trim().to_ascii_lowercase();
        if key.is_empty() {
            return Err(ParseError::new(number, "an option needs a name before `:`"));
        }
        let separator = if trimmed.as_bytes()[split] == b':' { Separator::Colon } else { Separator::Equals };
        let (value, comment) = split_inline_comment(&trimmed[split + 1..]);
        let Some(section) = self.document.sections.last_mut() else {
            return Err(ParseError::new(number, format!("option `{key}` stands before the first section")));
        };
        if section.entry(&key).is_some() {
            return Err(ParseError::new(number, format!("option `{key}` appears twice in {}", section.title())));
        }
        section.items.push(Item::Entry(Entry {
            key,
            value: value.trim().to_owned(),
            line: number,
            text: Some(vec![line.to_owned()]),
            separator,
            comment: comment.to_owned(),
        }));
        self.open = true;
        Ok(())
    }

    fn last_entry(&mut self) -> Option<&mut Entry> {
        match self.document.sections.last_mut()?.items.last_mut()? {
            Item::Entry(entry) => Some(entry),
            Item::Text(_) => None,
        }
    }

    fn push_text(&mut self, line: String) {
        match self.document.sections.last_mut() {
            Some(section) => section.items.push(Item::Text(line)),
            None => self.document.preamble.push(line),
        }
    }

    fn flush_pending(&mut self) {
        self.open = false;
        for line in std::mem::take(&mut self.pending) {
            self.push_text(line);
        }
    }

    /// The comment lines directly above a header, taken from where the parser put them.
    fn take_leading_comments(&mut self) -> Vec<String> {
        if let Some(section) = self.document.sections.last_mut() {
            return take_trailing_comments(&mut section.items);
        }
        let preamble = &mut self.document.preamble;
        let count = preamble.iter().rev().take_while(|line| is_comment(line.trim())).count();
        // Comments at the very top describe the file, not the first section.
        if count == preamble.len() {
            return Vec::new();
        }
        preamble.split_off(preamble.len() - count)
    }
}

fn take_trailing_comments(items: &mut Vec<Item>) -> Vec<String> {
    let count =
        items.iter().rev().take_while(|item| matches!(item, Item::Text(text) if is_comment(text.trim()))).count();
    let start = items.len() - count;
    items
        .drain(start..)
        .filter_map(|item| match item {
            Item::Text(text) => Some(text),
            Item::Entry(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Document {
        Document::parse(text, 0, Separator::Colon).unwrap()
    }

    fn error(text: &str) -> ParseError {
        Document::parse(text, 0, Separator::Colon).unwrap_err()
    }

    #[test]
    fn unchanged_text_is_written_back_as_it_was() {
        let text = "# top\n\n[printer voron]  # the big one\nmoonraker : http://voron:7125 ; lan\n; old\n\n\
                    [preset PLA]\nextruder=210\nbed = 60\n";

        assert_eq!(parse(text).render(), text);
    }

    #[test]
    fn headers_keys_and_values_are_read_like_klipper_and_moonraker() {
        let document = parse("[Printer My Voron]\nMoonraker: http://voron:7125  # comment\ncolor: a#b\n");
        let section = &document.sections()[0];

        assert_eq!((section.kind(), section.name()), ("printer", Some("My Voron")));
        assert_eq!(section.get("moonraker"), Some("http://voron:7125"));
        assert_eq!(section.get("color"), Some("a#b"));
        assert_eq!(section.line_of("color"), 3);
    }

    #[test]
    fn values_continue_on_indented_lines_without_blank_and_comment_lines() {
        let document = parse("[printer voron]\nhidden_macros:\n    PARK\n\n    # gone\n\tLOAD  ; inline\n\n[x]\n");

        assert_eq!(document.section("printer", Some("voron")).unwrap().get("hidden_macros"), Some("PARK\nLOAD"));
        assert!(document.section("x", None).is_some());
    }

    #[test]
    fn mistakes_name_their_line() {
        assert_eq!(error("[a]\nb: 1\n[a]\n").line, 3);
        assert_eq!(error("[a]\nb: 1\nB = 2\n").line, 3);
        assert_eq!(error("key = 1\n[a]\n").line, 1);
        assert_eq!(error("[a]\njust words\n").line, 2);
        assert_eq!(error("[a\n").line, 1);
        assert_eq!(error("[[printers]]\n").line, 1);
    }

    #[test]
    fn a_changed_value_keeps_its_inline_comment_and_the_rest_stays() {
        let mut document = parse("[backlight]\n# set on the screen\nbrightness: 80  # percent\nother: 1\n");
        let section = document.ensure_section("backlight", None);

        section.set("brightness", "40");
        section.set("other", "1");

        assert_eq!(document.render(), "[backlight]\n# set on the screen\nbrightness: 40  # percent\nother: 1\n");
    }

    #[test]
    fn new_entries_follow_the_last_entry_and_lists_go_on_indented_lines() {
        let mut document = parse("[printer voron]\nmoonraker: http://voron:7125\n\n[preset PLA]\n");

        document.ensure_section("printer", Some("voron")).set("hidden_macros", "PARK\nLOAD");

        assert_eq!(
            document.render(),
            "[printer voron]\nmoonraker: http://voron:7125\nhidden_macros:\n    PARK\n    LOAD\n\n[preset PLA]\n"
        );
    }

    #[test]
    fn a_new_section_goes_after_its_kind_with_blank_lines_around() {
        let mut document = parse("# settings\n[printer a]\nmoonraker: x\n[preset PLA]\nbed: 60\n");

        document.ensure_section("printer", Some("b")).set("moonraker", "y");
        document.ensure_section("backlight", None).set("brightness", "50");

        assert_eq!(
            document.render(),
            "# settings\n[printer a]\nmoonraker: x\n\n[printer b]\nmoonraker: y\n\n[preset PLA]\nbed: 60\n\n\
             [backlight]\nbrightness: 50\n"
        );
    }

    #[test]
    fn a_removed_section_leaves_its_comments_behind() {
        let text = "# file\n\n[printer a]\nmoonraker: x\n\n# the old one\n[printer b]\nmoonraker: y\n\n\
                    #[printer c]\n#moonraker: z\n\n[printer d]\nmoonraker: w\n";
        let mut document = parse(text);

        assert!(document.remove_section("printer", Some("b")));
        assert!(!document.remove_section("printer", Some("b")));
        assert!(document.remove_section("printer", Some("d")));

        assert_eq!(
            document.render(),
            "# file\n\n[printer a]\nmoonraker: x\n\n# the old one\n\n#[printer c]\n#moonraker: z\n\n"
        );
    }

    #[test]
    fn entries_written_by_sailhand_use_the_separator_of_the_document() {
        let mut document = Document::new("", Separator::Equals);
        let section = document.ensure_section("printer", Some("voron"));
        section.set("uuid", "29f7");
        section.set("known_macros", "");

        assert_eq!(document.render(), "[printer voron]\nuuid = 29f7\nknown_macros =\n");
    }

    #[test]
    fn new_entries_follow_the_separator_written_in_their_section() {
        let mut document = parse("[backlight]\ndevice = intel_backlight\n");
        let section = document.ensure_section("backlight", None);
        section.set("brightness", "50");

        assert!(section.remove("device"));
        assert!(!section.remove("device"));
        assert_eq!(document.render(), "[backlight]\nbrightness = 50\n");
    }
}
