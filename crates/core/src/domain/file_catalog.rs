//! The file list with what is known about each file from three sources: its metadata (Moonraker's file manager), its
//! last completed print (Moonraker's job history) and the spool that print used (Spoolman).

use crate::sources::moonraker_server::files::{FileEntry, FileMetadata};
use crate::sources::moonraker_server::history::LastPrint;
use crate::sources::spoolman::Spool;
use std::collections::HashMap;

/// This many files of the list get their metadata and thumbnail right away; the rest stays plain.
const DETAILS_LIMIT: usize = 40;

/// Everything known about one file of the list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileInfo<'a> {
    pub entry: &'a FileEntry,
    pub metadata: Option<&'a FileMetadata>,
    pub last_print: Option<&'a LastPrint>,
    /// The one spool the last print used, if Spoolman is known to have it.
    pub spool: Option<&'a Spool>,
}

/// The file list together with what arrives later for its files: metadata, the last completed prints and their
/// spools. The data comes from separate queries; this joins them and decides which files still need details.
#[derive(Debug, Clone, Default)]
pub struct FileCatalog {
    entries: Vec<FileEntry>,
    metadata: HashMap<String, FileMetadata>,
    last_prints: HashMap<String, LastPrint>,
    spools: HashMap<u32, Spool>,
}

impl FileCatalog {
    /// Takes a new file list. A file that was changed since the last list loses its metadata, so it is loaded again;
    /// the changed files are returned, because whatever else was kept for them (thumbnails) is outdated as well.
    pub fn set_entries(&mut self, entries: Vec<FileEntry>) -> Vec<String> {
        let before: HashMap<&str, f64> =
            self.entries.iter().map(|entry| (entry.path.as_str(), entry.modified)).collect();
        let changed: Vec<String> = entries
            .iter()
            .filter(|entry| before.get(entry.path.as_str()).is_some_and(|modified| *modified != entry.modified))
            .map(|entry| entry.path.clone())
            .collect();
        for path in &changed {
            self.metadata.remove(path);
        }
        self.entries = entries;
        changed
    }

    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    /// Files at the top of the list whose metadata has not been loaded yet.
    pub fn needing_details(&self) -> Vec<String> {
        self.entries
            .iter()
            .take(DETAILS_LIMIT)
            .filter(|entry| !self.metadata.contains_key(&entry.path))
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Metadata also arrives for the file of the running job, which need not be in the list.
    pub fn store_metadata(&mut self, metadata: FileMetadata) {
        self.metadata.insert(metadata.filename.clone(), metadata);
    }

    pub fn store_last_prints(&mut self, last_prints: HashMap<String, LastPrint>) {
        self.last_prints = last_prints;
    }

    /// Spools of Spoolman that last prints may have used; a spool that is known stays until it is replaced.
    pub fn store_spools(&mut self, spools: impl IntoIterator<Item = Spool>) {
        self.spools.extend(spools.into_iter().map(|spool| (spool.id, spool)));
    }

    pub fn metadata(&self, path: &str) -> Option<&FileMetadata> {
        self.metadata.get(path)
    }

    pub fn last_print(&self, path: &str) -> Option<&LastPrint> {
        self.last_prints.get(path)
    }

    pub fn spool(&self, id: u32) -> Option<&Spool> {
        self.spools.get(&id)
    }

    pub fn position(&self, path: &str) -> Option<usize> {
        self.entries.iter().position(|entry| entry.path == path)
    }

    pub fn info<'a>(&'a self, entry: &'a FileEntry) -> FileInfo<'a> {
        let last_print = self.last_print(&entry.path);
        let spool = last_print
            .filter(|last_print| last_print.spool_ids.len() == 1)
            .and_then(|last_print| self.spools.get(&last_print.spool_ids[0]));
        FileInfo { entry, metadata: self.metadata(&entry.path), last_print, spool }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> FileEntry {
        FileEntry { path: path.to_owned(), modified: 1.0, size: 10 }
    }

    fn metadata(path: &str) -> FileMetadata {
        FileMetadata { filename: path.to_owned(), estimated_time: Some(60.0), ..FileMetadata::default() }
    }

    #[test]
    fn the_catalog_joins_entry_metadata_and_last_print_of_a_file() {
        let mut catalog = FileCatalog::default();
        catalog.set_entries(vec![entry("a.gcode"), entry("b.gcode")]);
        catalog.store_metadata(metadata("a.gcode"));
        catalog.store_last_prints(HashMap::from([(
            "a.gcode".to_owned(),
            LastPrint { print_duration: 5.0, filament_used: 1.0, ..LastPrint::default() },
        )]));

        let a = catalog.info(&catalog.entries()[0]);
        let b = catalog.info(&catalog.entries()[1]);

        assert_eq!((a.metadata.is_some(), a.last_print.is_some()), (true, true));
        assert_eq!((b.metadata.is_some(), b.last_print.is_some()), (false, false));
        assert_eq!(catalog.position("b.gcode"), Some(1));
        assert_eq!(catalog.position("c.gcode"), None);
    }

    #[test]
    fn only_files_without_metadata_at_the_top_of_the_list_need_details() {
        let mut catalog = FileCatalog::default();
        let names: Vec<String> = (0..DETAILS_LIMIT + 5).map(|index| format!("f{index}.gcode")).collect();
        catalog.set_entries(names.iter().map(|name| entry(name)).collect());
        catalog.store_metadata(metadata("f0.gcode"));

        let needing = catalog.needing_details();

        assert_eq!(needing.len(), DETAILS_LIMIT - 1);
        assert!(!needing.contains(&"f0.gcode".to_owned()));
        assert!(!needing.contains(&format!("f{DETAILS_LIMIT}.gcode")));
    }

    #[test]
    fn the_spool_of_the_last_print_is_known_only_for_a_print_with_one_spool() {
        let mut catalog = FileCatalog::default();
        catalog.set_entries(vec![entry("a.gcode"), entry("b.gcode")]);
        catalog.store_last_prints(HashMap::from([
            ("a.gcode".to_owned(), LastPrint { print_duration: 5.0, filament_used: 1.0, spool_ids: vec![18] }),
            ("b.gcode".to_owned(), LastPrint { print_duration: 5.0, filament_used: 1.0, spool_ids: vec![18, 21] }),
        ]));
        catalog.store_spools([Spool { id: 18, ..Spool::default() }]);

        assert_eq!(catalog.info(&catalog.entries()[0]).spool.map(|spool| spool.id), Some(18));
        assert_eq!(catalog.info(&catalog.entries()[1]).spool, None);
    }

    #[test]
    fn a_changed_file_needs_its_details_again() {
        let mut catalog = FileCatalog::default();
        catalog.set_entries(vec![entry("a.gcode"), entry("b.gcode")]);
        catalog.store_metadata(metadata("a.gcode"));
        catalog.store_metadata(metadata("b.gcode"));

        let changed = catalog.set_entries(vec![FileEntry { modified: 2.0, ..entry("a.gcode") }, entry("b.gcode")]);

        assert_eq!(changed, ["a.gcode"]);
        assert_eq!(catalog.needing_details(), ["a.gcode"]);
        assert!(catalog.metadata("b.gcode").is_some());
    }

    #[test]
    fn metadata_of_a_file_outside_the_list_is_kept() {
        let mut catalog = FileCatalog::default();
        catalog.store_metadata(metadata("running.gcode"));

        assert!(catalog.metadata("running.gcode").is_some());
        assert!(catalog.needing_details().is_empty());
    }
}
