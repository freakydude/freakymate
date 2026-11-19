//! Connects the `Network` global with the network service of this host.

use crate::items::replace_rows;
use crate::presenter::Presenter;
use crate::{AdapterItem, AdapterStatus, AppWindow, Network, WifiItem};
use sailhand_core::network::{
    Adapter, AdapterKind, AdapterState, Availability, NetworkChannels, NetworkCommand, NetworkNotice, NetworkService,
    NetworkState, Security, signal_bars,
};
use sailhand_core::{Endpoint, PrinterService, Query};
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
    adapters: Rc<VecModel<AdapterItem>>,
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
            adapters: Rc::default(),
        });
        let network = window.global::<Network>();
        network.set_networks(ModelRc::from(controller.networks.clone()));
        network.set_adapters(ModelRc::from(controller.adapters.clone()));
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
        network.set_wifi_enabled(state.wifi_enabled);
        let selected = state.adapters.iter().find(|adapter| Some(&adapter.interface) == state.selected.as_ref());
        network.set_selected(selected.map(adapter_item).unwrap_or_default());
        network.set_busy(state.busy.as_deref().unwrap_or_default().into());
        replace_rows(&self.networks, state.networks.iter().map(wifi_item).collect());
        replace_rows(&self.adapters, state.adapters.iter().map(adapter_item).collect());
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
        let controller = Rc::downgrade(self);
        network.on_select(move |interface| {
            if let Some(controller) = controller.upgrade() {
                controller.select(interface.to_string());
            }
        });
        let controller = Rc::downgrade(self);
        network.on_scan(move || {
            if let Some((controller, interface)) = with_selected(&controller) {
                controller.service.send(NetworkCommand::Scan { interface });
            }
        });
        let controller = Rc::downgrade(self);
        network.on_connect(move |ssid, password, hidden| {
            if let Some((controller, interface)) = with_selected(&controller) {
                let password = (!password.is_empty()).then(|| password.to_string());
                controller.service.send(NetworkCommand::Connect {
                    interface,
                    ssid: ssid.to_string(),
                    password,
                    hidden,
                });
            }
        });
        let controller = Rc::downgrade(self);
        network.on_disconnect(move || {
            if let Some((controller, interface)) = with_selected(&controller) {
                controller.service.send(NetworkCommand::Disconnect { interface });
            }
        });
        let service = self.service.clone();
        network.on_forget(move |ssid| service.send(NetworkCommand::Forget { ssid: ssid.to_string() }));
        let service = self.service.clone();
        network.on_set_wifi_enabled(move |enabled| service.send(NetworkCommand::SetWifiEnabled(enabled)));
        let service = self.service.clone();
        network.on_set_wired(move |interface, connected| {
            service.send(NetworkCommand::SetWired { interface: interface.to_string(), connected });
        });
    }

    /// Shows the chosen adapter at once; the service then reads its networks.
    fn select(&self, interface: String) {
        self.state.borrow_mut().selected = Some(interface.clone());
        self.show();
        self.service.send(NetworkCommand::Select { interface });
    }

    fn refresh(&self) {
        if self.managed() {
            self.service.send(NetworkCommand::Refresh);
        } else {
            self.printer.query(Query::HostNetwork);
        }
    }
}

/// The controller and the interface of the adapter shown on the right, if both still exist.
fn with_selected(controller: &std::rc::Weak<NetworkController>) -> Option<(Rc<NetworkController>, String)> {
    let controller = controller.upgrade()?;
    let interface = controller.state.borrow().selected.clone()?;
    Some((controller, interface))
}

fn wifi_item(network: &sailhand_core::network::WifiNetwork) -> WifiItem {
    WifiItem {
        ssid: network.ssid.as_str().into(),
        bars: i32::from(signal_bars(network.signal)),
        secured: network.security != Security::Open,
        supported: network.security != Security::Unsupported,
        known: network.known,
        active: network.active,
        in_range: network.in_range,
        active_on_other: network.active_on_other.as_deref().unwrap_or_default().into(),
    }
}

fn adapter_item(adapter: &Adapter) -> AdapterItem {
    let text = |value: &Option<String>| SharedString::from(value.as_deref().unwrap_or_default());
    AdapterItem {
        interface: adapter.interface.as_str().into(),
        wireless: adapter.kind == AdapterKind::Wifi,
        status: match adapter.state {
            AdapterState::Unavailable => AdapterStatus::Unavailable,
            AdapterState::Disconnected => AdapterStatus::Disconnected,
            AdapterState::Connecting => AdapterStatus::Connecting,
            AdapterState::Connected => AdapterStatus::Connected,
        },
        ssid: text(&adapter.ssid),
        address: text(&adapter.address),
        prefix: adapter.prefix.map(i32::from).unwrap_or_default(),
        gateway: text(&adapter.gateway),
        speed: adapter.speed_mbit.and_then(|speed| i32::try_from(speed).ok()).unwrap_or_default(),
        mac: text(&adapter.mac),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sailhand_core::network::WifiNetwork;

    #[test]
    fn maps_networks_for_the_ui() {
        let item = wifi_item(&WifiNetwork {
            ssid: "Office".into(),
            signal: 90,
            security: Security::Unsupported,
            known: false,
            active: false,
            in_range: true,
            active_on_other: None,
        });
        assert_eq!(item.bars, 3);
        assert!(item.secured);
        assert!(!item.supported);
    }

    #[test]
    fn maps_adapters_for_the_ui() {
        let item = adapter_item(&Adapter {
            interface: "end0".into(),
            kind: AdapterKind::Ethernet,
            state: AdapterState::Connected,
            address: Some("192.168.1.42".into()),
            prefix: Some(24),
            speed_mbit: Some(1000),
            ..Adapter::default()
        });
        assert!(!item.wireless);
        assert_eq!(item.status, AdapterStatus::Connected);
        assert_eq!((item.address.as_str(), item.prefix, item.speed), ("192.168.1.42", 24, 1000));
        assert_eq!(item.gateway, "");
    }
}
