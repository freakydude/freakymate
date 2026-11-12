//! The diagnostic report against the fake Moonraker in the same process.

use fake_moonraker::machine::MachineProfile;
use fake_moonraker::server::{serve, ServerConfig};
use freakymate_core::report::collect;
use freakymate_core::{CredentialStore, Endpoint};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpListener;

async fn start_fake(profile: MachineProfile) -> Endpoint {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = ServerConfig { time_scale: 1000.0, status_interval: Duration::from_millis(20), profile, access: None };
    tokio::spawn(serve(listener, config));
    Endpoint::parse(&address.to_string()).unwrap()
}

async fn report_of(profile: MachineProfile) -> Value {
    let endpoint = start_fake(profile).await;
    collect(&endpoint, &CredentialStore::in_memory(), json!({ "Version": "test" })).await.unwrap()
}

#[tokio::test]
async fn reports_versions_objects_and_what_was_detected() {
    let report = report_of(MachineProfile::happy_hare(4)).await;
    assert_eq!(report["freakymate"]["Version"], "test");
    assert_eq!(report["moonraker"]["access"], "trusted or open");
    assert!(report["moonraker"]["server.info"]["moonraker_version"].is_string());
    let objects = &report["klipper"]["printer.objects.query"];
    assert!(objects["mmu"]["gate_status"].is_array(), "full status of the MMU");
    assert!(objects["print_stats"]["state"].is_string());
    assert!(report["klipper"]["printer.objects.list"]["objects"].is_array());
    let summary = report["detected"]["summary"].as_str().unwrap();
    assert!(!summary.is_empty());
    assert!(report["detected"]["machine_tree"].as_array().is_some_and(|lines| !lines.is_empty()));
}

#[tokio::test]
async fn leaves_out_the_host_name_and_the_raw_configuration() {
    let report = report_of(MachineProfile::afc()).await;
    assert!(report["moonraker"]["printer.info"].get("hostname").is_none());
    let configfile = &report["klipper"]["printer.objects.query"]["configfile"];
    assert!(configfile.get("config").is_none());
    assert!(report["moonraker"]["machine.system_info"].get("network").is_none());
}
