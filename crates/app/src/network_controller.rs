//! Connects the `Network` global with the network service of this host.

use crate::items::replace_rows;
use crate::presenter::Presenter;
use crate::{AppWindow, Network, WifiItem, WiredItem};
use freakymate_core::network::{
    Availability, NetworkChannels, NetworkCommand, NetworkNotice, NetworkService, NetworkState, Security, signal_bars,
};
use freakymate_core::{Endpoint, PrinterService, Query};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct NetworkController {
    window: slint::Weak<AppWindow>,
    service: NetworkService,
    printer: PrinterService,
    presenter: Rc<Presenter>,
    /// Simulated networks count as local so they can be tried on any computer.
    simulated: bool,
    local: Cell<bool>,
    state: RefCell<NetworkState>,
    networks: Rc<VecModel<WifiItem>>,
    wired: Rc<VecModel<WiredItem>>,
}

impl NetworkController {
    pub fn bind(
        window: &AppWindow,
        service: NetworkService,
        printer: PrinterService,
        presenter: Rc<Presenter>,
        endpoint: &Endpoint,
        simulated: bool,
    ) -> Rc<Self> {
        let controller = Rc::new(Self {
            window: window.as_weak(),
            service,
            printer,
            presenter,
            simulated,
            local: Cell::new(simulated || endpoint.is_local()),
            state: RefCell::default(),
            networks: Rc::default(),
            wired: Rc::default(),
        });
        let network = window.global::<Network>();
        network.set_networks(ModelRc::from(controller.networks.clone()));
        network.set_wired(ModelRc::from(controller.wired.clone()));
        controller.connect_callbacks(&network);
        controller.show();
        controller
    }

    /// Follows state and notices of the service in the Slint event loop.
    pub fn follow(self: &Rc<Self>, channels: NetworkChannels) -> Result<(), slint::EventLoopError> {
        let NetworkChannels { mut states, mut notices } = channels;
        let controller = Rc::clone(self);
        slint::spawn_local(async move {
            loop {
                let state = states.borrow_and_update().clone();
                controller.apply_state(state);
                if states.changed().await.is_err() {
                    break;
                }
            }
        })?;
        let controller = Rc::clone(self);
        slint::spawn_local(async move {
            while let Some(notice) = notices.recv().await {
                controller.show_notice(&notice);
            }
        })?;
        Ok(())
    }

    /// The Moonraker address changed; management is only offered for a local Moonraker.
    pub fn set_endpoint(&self, endpoint: &Endpoint) {
        self.local.set(self.simulated || endpoint.is_local());
        self.show();
    }

    pub fn apply_state(&self, state: NetworkState) {
        *self.state.borrow_mut() = state;
        self.show();
    }

    pub fn show_notice(&self, notice: &NetworkNotice) {
        self.presenter.show_notice(&notice.message, notice.error);
    }

    fn managed(&self) -> bool {
        self.local.get() && self.state.borrow().availability == Availability::Managed
    }

    fn show(&self) {
        let Some(window) = self.window.upgrade() else { return };
        let network = window.global::<Network>();
        let state = self.state.borrow();
        network.set_managed(self.managed());
        network.set_local(self.local.get());
        network.set_unavailable_reason(match &state.availability {
            Availability::Unavailable(reason) => reason.as_str().into(),
            Availability::Checking => "checking".into(),
            Availability::Managed => SharedString::new(),
        });
        network.set_wifi_present(state.wifi_present);
        network.set_wifi_enabled(state.wifi_enabled);
        network.set_active_ssid(state.active_ssid.as_deref().unwrap_or_default().into());
        network.set_wifi_address(state.wifi_address.as_deref().unwrap_or_default().into());
        network.set_busy(state.busy.as_deref().unwrap_or_default().into());
        replace_rows(&self.networks, state.networks.iter().map(wifi_item).collect());
        replace_rows(&self.wired, state.wired.iter().map(wired_item).collect());
    }

    fn connect_callbacks(self: &Rc<Self>, network: &Network) {
        let controller = Rc::downgrade(self);
        network.on_refresh(move || {
            if let Some(controller) = controller.upgrade() {
                controller.refresh();
            }
        });
        let service = self.service.clone();
        network.on_watch(move |visible| service.watch(visible));
        let service = self.service.clone();
        network.on_scan(move || service.send(NetworkCommand::Scan));
        let service = self.service.clone();
        network.on_connect(move |ssid, password, hidden| {
            let password = (!password.is_empty()).then(|| password.to_string());
            service.send(NetworkCommand::Connect { ssid: ssid.to_string(), password, hidden });
        });
        let service = self.service.clone();
        network.on_disconnect(move || service.send(NetworkCommand::Disconnect));
        let service = self.service.clone();
        network.on_forget(move |ssid| service.send(NetworkCommand::Forget { ssid: ssid.to_string() }));
        let service = self.service.clone();
        network.on_set_wifi_enabled(move |enabled| service.send(NetworkCommand::SetWifiEnabled(enabled)));
        let service = self.service.clone();
        network.on_set_wired(move |interface, connected| {
            service.send(NetworkCommand::SetWired { interface: interface.to_string(), connected });
        });
    }

    fn refresh(&self) {
        if self.managed() {
            self.service.send(NetworkCommand::Refresh);
        } else {
            self.printer.query(Query::HostNetwork);
        }
    }
}

fn wifi_item(network: &freakymate_core::network::WifiNetwork) -> WifiItem {
    WifiItem {
        ssid: network.ssid.as_str().into(),
        bars: i32::from(signal_bars(network.signal)),
        secured: network.security != Security::Open,
        supported: network.security != Security::Unsupported,
        known: network.known,
        active: network.active,
        in_range: network.in_range,
    }
}

fn wired_item(link: &freakymate_core::network::WiredLink) -> WiredItem {
    WiredItem {
        interface: link.interface.as_str().into(),
        cable: link.cable,
        connected: link.connected,
        address: link.address.as_deref().unwrap_or_default().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freakymate_core::network::WifiNetwork;

    #[test]
    fn maps_networks_for_the_ui() {
        let item = wifi_item(&WifiNetwork {
            ssid: "Office".into(),
            signal: 90,
            security: Security::Unsupported,
            known: false,
            active: false,
            in_range: true,
        });
        assert_eq!(item.bars, 3);
        assert!(item.secured);
        assert!(!item.supported);
    }
}
