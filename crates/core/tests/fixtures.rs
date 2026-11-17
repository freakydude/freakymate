//! Recorded Happy Hare objects of real printers (`scripts/capture-printer-dump.sh`) against the rules of the core:
//! when a Happy Hare version changes a field, the matching dump shows it here.

use sailhand_core::Provenance;
use sailhand_core::sources::{ObjectStore, PrinterStatus};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Rules that hold for every Happy Hare 4.1 machine, whatever its hardware.
fn check_machine(name: &str, status: &Value) {
    let mut store = ObjectStore::default();
    store.replace(status);
    let mmu = PrinterStatus::from_store(&store).mmu.unwrap_or_else(|| panic!("{name}: no mmu object"));

    assert!(mmu.is_supported(), "{name}: unsupported version {}", mmu.version);
    assert!(mmu.num_gates > 0, "{name}: no gates");
    let has_encoder = status["mmu"]["encoder"].is_object();
    let has_proportional_sensor =
        mmu.sensors.iter().any(|sensor| sensor.name == "filament_proportional" && sensor.triggered.is_some());
    match mmu.flow_rate {
        Some(rate) if has_encoder => {
            assert_eq!(rate.provenance, Provenance::Measured, "{name}: encoder rate is measured")
        }
        Some(rate) => {
            assert!(has_proportional_sensor, "{name}: a flow rate without encoder needs the proportional sensor");
            assert_eq!(rate.provenance, Provenance::Estimated, "{name}: sensor rate is an estimate");
        }
        None => assert!(!has_encoder && !has_proportional_sensor, "{name}: flow rate expected but missing"),
    }
}

#[test]
fn recorded_machines_follow_the_rules() {
    let Ok(entries) = fs::read_dir(fixture_dir()) else { return };
    for path in entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
    {
        let name = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("fixture");
        let status: Value = serde_json::from_str(&fs::read_to_string(&path).expect("fixture is readable"))
            .unwrap_or_else(|error| panic!("{name}: invalid JSON: {error}"));
        check_machine(name, &status);
    }
}

#[test]
fn the_checks_accept_a_machine_with_a_proportional_buffer_and_no_encoder() {
    check_machine(
        "inline",
        &json!({
            "mmu": {
                "num_gates": 1, "encoder": null,
                "sensors": { "filament_proportional": false },
                "sync_feedback_state": "neutral", "sync_feedback_flow_rate": 100.0
            },
            "mmu_machine": { "happy_hare_version": "4.1.0", "num_gates": 1, "num_units": 1 }
        }),
    );
}
