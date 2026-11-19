//! Connects the `PrinterChoice` global with the saved printers and the search for printers on the network.

use crate::items::replace_rows;
use crate::presenter::Presenter;
use crate::settings_controller::SettingsController;
use crate::{AppWindow, PrinterChoice, PrinterItem};
use sailhand_core::printers::{DiscoveryService, DiscoveryState, PrinterEntry, Reachability, name_of, printer_list};
use sailhand_core::{CredentialStore, Endpoint};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::watch;

pub struct PrintersController {
    window: slint::Weak<AppWindow>,
    settings: Rc<SettingsController>,
    discovery: DiscoveryService,
    credentials: CredentialStore,
    presenter: Rc<Presenter>,
    state: RefCell<DiscoveryState>,
    entries: RefCell<Vec<PrinterEntry>>,
    rows: Rc<VecModel<PrinterItem>>,
}

/// What the printers act on besides the window.
pub struct PrinterBridges {
    pub settings: Rc<SettingsController>,
    pub discovery: DiscoveryService,
    pub credentials: CredentialStore,
    pub presenter: Rc<Presenter>,
}

impl PrintersController {
    pub fn bind(window: &AppWindow, bridges: PrinterBridges) -> Rc<Self> {
        let PrinterBridges { settings, discovery, credentials, presenter } = bridges;
        let controller = Rc::new(Self {
            window: window.as_weak(),
            settings,
            discovery,
            credentials,
            presenter,
            state: RefCell::default(),
            entries: RefCell::default(),
            rows: Rc::default(),
        });
        let choice = window.global::<PrinterChoice>();
        choice.set_printers(ModelRc::from(controller.rows.clone()));
        controller.connect_callbacks(&choice);
        controller.saved_changed();
        controller
    }

    /// Follows the search in the Slint event loop.
    pub fn follow(self: &Rc<Self>, mut states: watch::Receiver<DiscoveryState>) -> Result<(), slint::EventLoopError> {
        let controller = Rc::clone(self);
        slint::spawn_local(async move {
            loop {
                let state = states.borrow_and_update().clone();
                controller.apply_discovery(state);
                if states.changed().await.is_err() {
                    break;
                }
            }
        })?;
        Ok(())
    }

    pub fn apply_discovery(&self, state: DiscoveryState) {
        self.state.replace(state);
        self.show();
    }

    fn connect_callbacks(self: &Rc<Self>, choice: &PrinterChoice<'_>) {
        let discovery = self.discovery.clone();
        choice.on_watch(move |visible| discovery.watch(visible));
        let controller = Rc::downgrade(self);
        choice.on_choose(move |address| {
            if let Some(controller) = controller.upgrade() {
                controller.choose(&address);
            }
        });
        let controller = Rc::downgrade(self);
        choice.on_add(move |address| {
            if let Some(controller) = controller.upgrade() {
                controller.add(&address);
            }
        });
        let controller = Rc::downgrade(self);
        choice.on_forget(move |address| {
            if let Some(controller) = controller.upgrade() {
                controller.forget(&address);
            }
        });
    }

    fn choose(&self, address: &str) {
        let entry = self.entries.borrow().iter().find(|entry| entry.address == address).cloned();
        let Some(entry) = entry else { return };
        match Endpoint::parse(&entry.address) {
            Ok(endpoint) => self.switch(endpoint, &entry.name, entry.uuid.as_deref()),
            Err(error) => self.presenter.show_notice(&error.to_string(), true),
        }
    }

    /// An address typed by hand; a saved printer at that address keeps its name.
    fn add(&self, address: &str) {
        match Endpoint::parse(address) {
            Ok(endpoint) => {
                let saved = self.settings.saved_printers();
                let known = saved.iter().find(|printer| printer.address == endpoint.display());
                let name = known.map_or_else(|| name_of(&endpoint), |printer| printer.name.clone());
                let uuid = known.and_then(|printer| printer.uuid.clone());
                self.switch(endpoint, &name, uuid.as_deref());
            }
            Err(error) => self.presenter.show_notice(&error.to_string(), true),
        }
    }

    fn switch(&self, endpoint: Endpoint, name: &str, uuid: Option<&str>) {
        self.settings.switch_printer(endpoint, name, uuid);
        self.saved_changed();
    }

    /// Removes the saved printer together with its login and trusted certificate.
    fn forget(&self, address: &str) {
        if !self.settings.forget_printer(address) {
            return;
        }
        if let Ok(endpoint) = Endpoint::parse(address)
            && let Err(error) = self.credentials.forget(&endpoint)
        {
            log::warn!("cannot remove the credentials of {address}: {error}");
        }
        self.saved_changed();
    }

    /// Checks the saved printers anew and shows the list.
    fn saved_changed(&self) {
        let addresses = self.settings.saved_printers().into_iter().map(|printer| printer.address).collect();
        self.discovery.check(addresses);
        self.show();
    }

    fn show(&self) {
        let entries =
            printer_list(&self.settings.saved_printers(), &self.settings.active_address(), &self.state.borrow());
        replace_rows(&self.rows, entries.iter().map(printer_item).collect());
        if let Some(window) = self.window.upgrade() {
            let active = entries.iter().find(|entry| entry.active).map(|entry| entry.name.as_str()).unwrap_or_default();
            window.global::<PrinterChoice>().set_active_name(active.into());
        }
        self.entries.replace(entries);
    }
}

fn printer_item(entry: &PrinterEntry) -> PrinterItem {
    let reachability = match entry.reachability {
        Reachability::Checking => "checking",
        Reachability::Reachable => "reachable",
        Reachability::Unreachable => "unreachable",
    };
    PrinterItem {
        name: entry.name.as_str().into(),
        address: entry.address.as_str().into(),
        saved: entry.saved,
        active: entry.active,
        reachability: reachability.into(),
    }
}
