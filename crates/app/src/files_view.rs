//! The file list: the files of the printer with their facts and thumbnails, and the files marked in the list.

use crate::items::replace_rows;
use crate::{format, view, FileItem, Printer};
use freakymate_core::domain::file_catalog::FileCatalog;
use freakymate_core::sources::moonraker_server::files::FileEntry;
use slint::{Image, Model, ModelRc, SharedString, VecModel};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

/// What is known about the files of the printer.
#[derive(Default)]
pub struct FileState {
    pub catalog: FileCatalog,
    pub thumbnails: HashMap<String, Image>,
    /// Paths of the files marked in the list; they outlive a rebuild of the rows.
    marked: BTreeSet<String>,
}

impl FileState {
    /// Marks or unmarks a file; marked files can be deleted, a single one can be prepared.
    pub fn toggle_mark(&mut self, path: &str) {
        if !self.marked.remove(path) {
            self.marked.insert(path.to_owned());
        }
    }

    pub fn clear_marks(&mut self) {
        self.marked.clear();
    }

    /// The marked files, which are no longer marked afterwards.
    pub fn take_marked(&mut self) -> Vec<String> {
        std::mem::take(&mut self.marked).into_iter().collect()
    }

    /// Files that vanished from the list are no longer marked.
    pub fn forget_vanished_marks(&mut self, files: &[FileEntry]) {
        let listed: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
        self.marked.retain(|path| listed.contains(path.as_str()));
    }

    /// The row of a file.
    pub fn row(&self, file: &FileEntry) -> FileItem {
        let info = self.catalog.info(file);
        let facts = view::file_facts(&info);
        let thumbnail = self.thumbnails.get(&file.path);
        FileItem {
            path: file.path.as_str().into(),
            name: format::job_name(&file.path).into(),
            modified: facts.modified.into(),
            material: facts.material.into(),
            weight: facts.weight.into(),
            nozzle: facts.nozzle.into(),
            time: facts.time.into(),
            extruder_temperature: facts.extruder_temperature.into(),
            bed_temperature: facts.bed_temperature.into(),
            chamber_temperature: facts.chamber_temperature.into(),
            thumbnail: thumbnail.cloned().unwrap_or_default(),
            has_thumbnail: thumbnail.is_some(),
            show_thumbnail: thumbnail.is_some() || info.thumbnail_expected(),
            marked: self.marked.contains(&file.path),
        }
    }
}

#[derive(Default)]
pub struct FilesView {
    rows: Rc<VecModel<FileItem>>,
    marked_names: Rc<VecModel<SharedString>>,
}

impl FilesView {
    pub fn bind(&self, printer: &Printer) {
        printer.set_marked_file_names(ModelRc::from(self.marked_names.clone()));
        printer.set_files(ModelRc::from(self.rows.clone()));
    }

    pub fn show_rows(&self, state: &FileState) {
        replace_rows(&self.rows, state.catalog.entries().iter().map(|entry| state.row(entry)).collect());
    }

    /// Brings one row up to date, e.g. after its thumbnail arrived.
    pub fn update_row(&self, state: &FileState, filename: &str) {
        let Some(index) = state.catalog.position(filename) else { return };
        if index < self.rows.row_count() {
            self.rows.set_row_data(index, state.row(&state.catalog.entries()[index]));
        }
    }

    /// How many files are marked with their names, and which one if it is a single one (the one to prepare).
    pub fn show_marks(&self, printer: &Printer, state: &FileState) {
        printer.set_marked_file_count(i32::try_from(state.marked.len()).unwrap_or(i32::MAX));
        let single = match state.marked.iter().collect::<Vec<_>>().as_slice() {
            [path] => Some((*path).clone()),
            _ => None,
        };
        printer.set_marked_file_path(single.as_deref().unwrap_or_default().into());
        printer.set_marked_file_name(single.as_deref().map(format::job_name).unwrap_or_default().into());
        replace_rows(&self.marked_names, state.marked.iter().map(|path| format::job_name(path).into()).collect());
    }
}
