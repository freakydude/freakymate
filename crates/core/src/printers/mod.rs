//! The printers this display can use, one Moonraker each: the saved ones, those announced on the network, and which
//! of them answer. [`printer_list`] puts them in the order the Printers page shows.

mod discovery;

pub use discovery::{DiscoveryBackend, DiscoveryService, DiscoveryState};

use crate::moonraker::Endpoint;
use crate::settings::SavedPrinter;
use std::collections::BTreeMap;

/// A Moonraker announced over zeroconf (`[zeroconf]` in `moonraker.conf`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundPrinter {
    pub name: String,
    /// As [`Endpoint::display`] writes it.
    pub address: String,
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reachability {
    /// Not checked yet.
    Checking,
    Reachable,
    Unreachable,
}

/// One row of the Printers page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterEntry {
    pub name: String,
    /// The address to connect to; for a saved printer found at a new address (same `uuid`), the new one.
    pub address: String,
    pub uuid: Option<String>,
    pub saved: bool,
    /// The printer the display uses now; its state comes from the connection, not from [`Self::reachability`].
    pub active: bool,
    pub reachability: Reachability,
}

/// The name for a printer known only by its address: the host.
pub fn name_of(endpoint: &Endpoint) -> String {
    endpoint.host().to_owned()
}

/// Order of the page: the active printer, saved and reachable ones, found ones not saved, then saved ones that do not
/// answer (or are not checked yet); by name within each group.
pub fn printer_list(saved: &[SavedPrinter], active: &str, state: &DiscoveryState) -> Vec<PrinterEntry> {
    let mut entries: Vec<PrinterEntry> = saved.iter().map(|printer| saved_entry(printer, state)).collect();
    for found in &state.found {
        if !entries.iter().any(|entry| matches(entry, found)) {
            entries.push(found_entry(found));
        }
    }
    match entries.iter_mut().find(|entry| entry.address == active) {
        Some(entry) => entry.active = true,
        None => entries.push(unsaved_active_entry(active)),
    }
    entries.sort_by_cached_key(|entry| (group(entry), entry.name.to_lowercase()));
    entries
}

fn saved_entry(printer: &SavedPrinter, state: &DiscoveryState) -> PrinterEntry {
    let found = state.found.iter().find(|found| same_printer(printer, found));
    let reachability = match (found, state.reachable.get(&printer.address)) {
        (Some(_), _) | (None, Some(true)) => Reachability::Reachable,
        (None, Some(false)) => Reachability::Unreachable,
        (None, None) => Reachability::Checking,
    };
    PrinterEntry {
        name: printer.name.clone(),
        address: found.map_or_else(|| printer.address.clone(), |found| found.address.clone()),
        uuid: printer.uuid.clone().or_else(|| found.and_then(|found| found.uuid.clone())),
        saved: true,
        active: false,
        reachability,
    }
}

fn found_entry(found: &FoundPrinter) -> PrinterEntry {
    PrinterEntry {
        name: found.name.clone(),
        address: found.address.clone(),
        uuid: found.uuid.clone(),
        saved: false,
        active: false,
        reachability: Reachability::Reachable,
    }
}

/// An address from the command line, or a printer forgotten while in use.
fn unsaved_active_entry(active: &str) -> PrinterEntry {
    let name = Endpoint::parse(active).map_or_else(|_| active.to_owned(), |endpoint| name_of(&endpoint));
    PrinterEntry {
        name,
        address: active.to_owned(),
        uuid: None,
        saved: false,
        active: true,
        reachability: Reachability::Checking,
    }
}

fn same_printer(printer: &SavedPrinter, found: &FoundPrinter) -> bool {
    match (&printer.uuid, &found.uuid) {
        (Some(saved), Some(announced)) => saved == announced,
        _ => printer.address == found.address,
    }
}

fn matches(entry: &PrinterEntry, found: &FoundPrinter) -> bool {
    entry.address == found.address || (entry.uuid.is_some() && entry.uuid == found.uuid)
}

fn group(entry: &PrinterEntry) -> u8 {
    match entry {
        PrinterEntry { active: true, .. } => 0,
        PrinterEntry { saved: true, reachability: Reachability::Reachable, .. } => 1,
        PrinterEntry { saved: false, .. } => 2,
        PrinterEntry { .. } => 3,
    }
}

/// Reachability per saved address, as the discovery checks it.
pub type ReachableAddresses = BTreeMap<String, bool>;

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(name: &str, address: &str, uuid: Option<&str>) -> SavedPrinter {
        SavedPrinter {
            name: name.to_owned(),
            address: address.to_owned(),
            uuid: uuid.map(str::to_owned),
            ..SavedPrinter::default()
        }
    }

    fn found(name: &str, address: &str, uuid: &str) -> FoundPrinter {
        FoundPrinter { name: name.to_owned(), address: address.to_owned(), uuid: Some(uuid.to_owned()) }
    }

    fn names(entries: &[PrinterEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn the_list_runs_active_saved_reachable_found_then_unreachable() {
        let saved = [
            saved("ender", "http://10.0.0.5:7125", None),
            saved("zeta", "http://10.0.0.9:7125", None),
            saved("voron", "http://10.0.0.6:7125", Some("v")),
            saved("anet", "http://10.0.0.8:7125", None),
        ];
        let state = DiscoveryState {
            found: vec![found("mainsailos", "http://10.0.0.23:7125", "m"), found("voron", "http://10.0.0.6:7125", "v")],
            reachable: [("http://10.0.0.5:7125".to_owned(), false), ("http://10.0.0.9:7125".to_owned(), true)].into(),
        };

        let list = printer_list(&saved, "http://10.0.0.6:7125", &state);

        assert_eq!(names(&list), ["voron", "zeta", "mainsailos", "anet", "ender"]);
        assert!(list[0].active && list[0].saved);
        assert!(!list[2].saved);
        assert_eq!(list[3].reachability, Reachability::Checking);
        assert_eq!(list[4].reachability, Reachability::Unreachable);
    }

    #[test]
    fn a_saved_printer_found_at_a_new_address_offers_the_new_one() {
        let saved = [saved("voron", "http://10.0.0.6:7125", Some("v"))];
        let state = DiscoveryState { found: vec![found("voron", "http://10.0.0.7:7125", "v")], ..Default::default() };

        let list = printer_list(&saved, "http://127.0.0.1:7125", &state);

        let voron = list.iter().find(|entry| entry.name == "voron").unwrap();
        assert_eq!((voron.address.as_str(), voron.saved), ("http://10.0.0.7:7125", true));
        assert_eq!(voron.reachability, Reachability::Reachable);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn an_unsaved_active_printer_is_listed_first_with_its_host_as_name() {
        let list = printer_list(&[], "http://voron.local:7125", &DiscoveryState::default());

        assert_eq!(names(&list), ["voron.local"]);
        assert!(list[0].active && !list[0].saved);
    }

    #[test]
    fn a_found_printer_that_is_in_use_is_marked_active_not_listed_twice() {
        let state = DiscoveryState { found: vec![found("voron", "http://10.0.0.6:7125", "v")], ..Default::default() };

        let list = printer_list(&[], "http://10.0.0.6:7125", &state);

        assert_eq!(list.len(), 1);
        assert!(list[0].active);
    }
}
