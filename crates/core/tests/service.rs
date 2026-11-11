//! End-to-end test of the service against the fake Moonraker in the same process.

use fake_moonraker::access::AccessConfig;
use fake_moonraker::machine::MachineProfile;
use fake_moonraker::server::{serve, ServerConfig};
use freakymate_core::credentials::Credential;
use freakymate_core::domain::filament::{self, FilamentHandling};
use freakymate_core::domain::{job_end, light, pause, spools};
use freakymate_core::sources::happy_hare::commands::{Gates, ServoCommand};
use freakymate_core::sources::happy_hare::{GateAvailability, LoadedFilament, ServoPosition};
use freakymate_core::sources::klipper::tuning::TuningKey;
use freakymate_core::sources::klipper::{Axis, FanKind, KlippyState, Leveling, PrintState, WhiteChannel};
use freakymate_core::sources::moonraker_server::files::ThumbnailImage;
use freakymate_core::sources::moonraker_server::gcode_store::ConsoleKind;
use freakymate_core::sources::PrinterStatus;
use freakymate_core::{
    Access, Command, ConnectionState, CredentialStore, Endpoint, Event, PrinterService, Query, ServiceChannels,
    Snapshot,
};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::timeout;

const WAIT_LIMIT: Duration = Duration::from_secs(10);

async fn start_fake() -> Endpoint {
    start_fake_with(MachineProfile::voron()).await
}

async fn start_fake_with(profile: MachineProfile) -> Endpoint {
    start_fake_configured(profile, None).await
}

/// A fake that trusts nobody: user `test`, password `secret`, and its API key.
async fn start_guarded_fake() -> Endpoint {
    start_fake_configured(MachineProfile::voron(), Some(AccessConfig::new("test", "secret"))).await
}

async fn start_fake_configured(profile: MachineProfile, access: Option<AccessConfig>) -> Endpoint {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = ServerConfig { time_scale: 1000.0, status_interval: Duration::from_millis(20), profile, access };
    tokio::spawn(serve(listener, config));
    Endpoint::parse(&address.to_string()).unwrap()
}

fn start_service(endpoint: Endpoint, credentials: CredentialStore) -> (PrinterService, ServiceChannels) {
    PrinterService::start(&tokio::runtime::Handle::current(), endpoint, credentials)
}

/// A credential file in a fresh folder, removed when the returned guard is dropped.
fn temporary_credentials() -> (tempfile::TempDir, CredentialStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = CredentialStore::new(dir.path().join("credentials.json"));
    (dir, store)
}

fn login_required(snapshot: &Snapshot) -> bool {
    snapshot.connection == ConnectionState::LoginRequired
}

async fn wait_for(channels: &mut ServiceChannels, condition: impl Fn(&Snapshot) -> bool) -> Snapshot {
    timeout(WAIT_LIMIT, async {
        loop {
            if condition(&channels.snapshots.borrow_and_update()) {
                return channels.snapshots.borrow().clone();
            }
            channels.snapshots.changed().await.expect("service stopped");
        }
    })
    .await
    .expect("condition not reached in time")
}

async fn next_event(channels: &mut ServiceChannels, matches: impl Fn(&Event) -> bool) -> Event {
    timeout(WAIT_LIMIT, async {
        loop {
            let event = channels.events.recv().await.expect("service stopped");
            if matches(&event) {
                return event;
            }
        }
    })
    .await
    .expect("event not received in time")
}

fn ready(snapshot: &Snapshot) -> bool {
    snapshot.connection == ConnectionState::Connected && snapshot.status.klippy_state == KlippyState::Ready
}

#[tokio::test(flavor = "multi_thread")]
async fn connects_detects_capabilities_and_follows_state() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());

    let snapshot = wait_for(&mut channels, ready).await;
    let klipper = &snapshot.sources.klipper;
    assert_eq!(klipper.leveling, Some(Leveling::QuadGantry));
    assert!(klipper.has_macro("LOAD_FILAMENT"));
    assert!(!klipper.has_macro("_PURGE_HELPER"));
    let park = klipper.macro_details.get("PARK").expect("macro details are read when subscribing");
    assert!(park.params.iter().any(|param| param.name == "Z"));
    assert!(snapshot.sources.moonraker.power);
    assert!(snapshot.status.heater("heater_generic chamber").is_some());

    service.send(Command::SetHeaterTarget { heater: "extruder".into(), target: 200.0 });
    service.send(Command::Home { x: true, y: true, z: true });
    let snapshot = wait_for(&mut channels, |snapshot| {
        snapshot.status.heater("extruder").is_some_and(|heater| heater.target == 200.0)
            && snapshot.status.motion.homed.all()
    })
    .await;
    assert_eq!(snapshot.status.motion.position[0], 0.0);

    service.send(Command::Jog { axis: Axis::X, distance: 25.0 });
    wait_for(&mut channels, |snapshot| snapshot.status.motion.position[0] == 25.0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn follows_happy_hare_gates_sensors_and_loading() {
    let endpoint = start_fake_with(MachineProfile::happy_hare(4)).await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());

    let snapshot = wait_for(&mut channels, |snapshot| ready(snapshot) && snapshot.status.mmu.is_some()).await;
    assert_eq!(filament::handling(&snapshot.sources, &snapshot.status), FilamentHandling::Mmu);
    let mmu = snapshot.status.mmu.unwrap();
    assert_eq!((mmu.num_gates, mmu.filament), (4, LoadedFilament::Unloaded));
    assert!(mmu.is_supported());
    assert_eq!(mmu.gates[3].availability, GateAvailability::Empty);
    assert!(mmu.sensors.iter().all(|sensor| sensor.name != "mmu_pre_gate"));

    service.send(Command::MmuHome);
    wait_for(&mut channels, |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.homed)).await;
    // Commands reach the printer in the order they were sent, so no waiting between the two.
    service.send(Command::MmuSelectGate { gate: 1 });
    service.send(Command::MmuLoad);
    let snapshot = wait_for(&mut channels, |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.filament == LoadedFilament::Loaded)
    })
    .await;

    let mmu = snapshot.status.mmu.unwrap();
    assert_eq!(mmu.gate, 1);
    let triggered =
        |name: &str| mmu.sensors.iter().find(|sensor| sensor.name == name).and_then(|sensor| sensor.triggered);
    assert_eq!((triggered("mmu_exit"), triggered("toolhead")), (Some(true), Some(true)));

    // While loaded, eject only unloads; a second eject empties the gate.
    service.send(Command::MmuEject { gate: None });
    wait_for(&mut channels, |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.filament == LoadedFilament::Unloaded)
    })
    .await;
    service.send(Command::MmuEject { gate: None });
    let snapshot = wait_for(&mut channels, |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.gates[1].availability == GateAvailability::Empty)
    })
    .await;
    assert_eq!(snapshot.status.mmu.unwrap().gate, 1);
}

/// The service commands: a preload fills the empty gate, a check of all gates keeps it, the servo goes down and the
/// motors off take the home.
#[tokio::test(flavor = "multi_thread")]
async fn runs_the_service_commands_of_happy_hare() {
    let endpoint = start_fake_with(MachineProfile::happy_hare(4)).await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    let snapshot = wait_for(&mut channels, |snapshot| ready(snapshot) && snapshot.status.mmu.is_some()).await;
    assert_eq!(snapshot.status.mmu.unwrap().servo, Some(ServoPosition::Up));

    service.send(Command::MmuHome);
    service.send(Command::MmuPreload { gate: 3 });
    service.send(Command::MmuCheckGates { gates: Gates::All });
    service.send(Command::MmuServo { position: ServoCommand::Down });
    service.send(Command::MmuMotorsOff);
    let snapshot = wait_for(&mut channels, |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.servo == Some(ServoPosition::Down) && !mmu.homed)
    })
    .await;
    assert!(snapshot.status.mmu.unwrap().gates.iter().all(|gate| gate.availability == GateAvailability::Available));
}

/// The commands of the gate dialog without Spoolman: material and colour go into Happy Hare's gate map and keep the
/// gate's temperature, and a gate is selected and loaded in one go.
#[tokio::test(flavor = "multi_thread")]
async fn sets_the_filament_of_a_gate_and_loads_it() {
    let endpoint = start_fake_with(MachineProfile::ercf(4)).await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    let snapshot = wait_for(&mut channels, |snapshot| ready(snapshot) && snapshot.status.mmu.is_some()).await;
    let temperature = snapshot.status.mmu.unwrap().gates[1].temperature;

    service.send(Command::MmuHome);
    service.send(Command::SetGateFilament { gate: 1, material: Some("PETG".into()), color: None });
    service.send(Command::SetGateFilament { gate: 1, material: None, color: Some([0xff, 0x80, 0x00]) });
    service.send(Command::MmuLoadGate { gate: 1 });
    let snapshot = wait_for(&mut channels, |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.gate == 1 && mmu.filament == LoadedFilament::Loaded)
    })
    .await;
    let gate = &snapshot.status.mmu.unwrap().gates[1];
    assert_eq!((gate.material.as_str(), gate.color), ("PETG", Some([0xff, 0x80, 0x00, 0xff])));
    assert_eq!(gate.temperature, temperature);
}

/// Commands sent one after another must reach the printer in that order; a heater target is absolute, so a
/// swapped pair would leave the printer on the older value.
#[tokio::test(flavor = "multi_thread")]
async fn commands_reach_the_printer_in_the_order_they_were_sent() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    for target in 100..200 {
        service.send(Command::SetHeaterTarget { heater: "extruder".into(), target: f64::from(target) });
    }
    wait_for(&mut channels, |snapshot| snapshot.status.heater("extruder").is_some_and(|heater| heater.target == 199.0))
        .await;
    // A swapped command would arrive later and move the target away from the last one.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let snapshot = channels.snapshots.borrow().clone();
    assert_eq!(snapshot.status.heater("extruder").map(|heater| heater.target), Some(199.0));
}

#[tokio::test(flavor = "multi_thread")]
async fn quick_heater_steps_build_on_each_other() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    // Each step starts from the target the previous one set, even before Klipper reported it back.
    for _ in 0..4 {
        service.send(Command::StepHeater { heater: "extruder".into(), step: 10.0 });
    }

    wait_for(&mut channels, |snapshot| snapshot.status.heater("extruder").is_some_and(|heater| heater.target == 40.0))
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn presets_and_cooling_down_act_on_every_heater() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.send(Command::ApplyPreset { extruder: 210.0, bed: 60.0 });
    let targets = |snapshot: &Snapshot| {
        ["extruder", "heater_bed"].map(|name| snapshot.status.heater(name).map(|heater| heater.target))
    };
    wait_for(&mut channels, |snapshot| targets(snapshot) == [Some(210.0), Some(60.0)]).await;

    service.send(Command::CoolDown);
    wait_for(&mut channels, |snapshot| targets(snapshot) == [Some(0.0), Some(0.0)]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn intents_the_printer_cannot_carry_out_are_reported() {
    let endpoint = start_fake_with(MachineProfile::basic()).await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.send(Command::Level);
    let refused = next_event(&mut channels, |event| matches!(event, Event::CommandFailed { .. })).await;

    assert!(matches!(refused, Event::CommandFailed { command: Command::Level, error }
        if error.to_string().contains("neither quad gantry level nor Z tilt")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_that_waits_long_for_its_answer_is_reported_as_running_until_it_has_one() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    // Quick commands never show up as running.
    service.send(Command::SetHeaterTarget { heater: "extruder".into(), target: 100.0 });
    service.send(Command::RunGcode { script: "G4 P1500\nM104 S0".into() });
    let running = next_event(&mut channels, |event| {
        matches!(event, Event::CommandRunning { .. } | Event::CommandFinished { .. })
    })
    .await;
    let Event::CommandRunning { id, script_line, .. } = running else {
        panic!("expected a running command, got {running:?}")
    };
    assert_eq!(script_line.as_deref(), Some("G4 P1500"));

    let finished = next_event(&mut channels, |event| matches!(event, Event::CommandFinished { .. })).await;
    assert_eq!(finished, Event::CommandFinished { id });
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_the_spools_of_the_gates_from_spoolman() {
    let endpoint = start_fake_with(MachineProfile::happy_hare(4)).await;
    let (_service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    let snapshot = wait_for(&mut channels, ready).await;
    assert!(snapshot.sources.moonraker.spoolman);

    let Event::Spoolman(state) = next_event(&mut channels, |event| matches!(event, Event::Spoolman(_))).await else {
        unreachable!()
    };

    assert!(state.connected);
    // Gate 3 stays empty; the gates 0 to 2 have the spools 1, 7 (two colours) and 8 (three colours), and Moonraker
    // starts with spool 4 active.
    let mut ids: Vec<u32> = state.spools.keys().copied().collect();
    ids.sort_unstable();
    assert_eq!(ids, [1, 4, 7, 8]);
    assert_eq!(state.active_spool_id, Some(4));
    let spool = &state.spools[&7];
    assert_eq!(
        (spool.material.as_str(), spool.remaining_weight, spool.extruder_temp),
        ("PLA", Some(38.0), Some(215.0))
    );
    assert!(spool.is_low());
    assert_eq!((state.spools[&1].colors.len(), spool.colors.len(), state.spools[&8].colors.len()), (1, 2, 3));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_printer_without_spoolman_has_no_spoolman_component() {
    let endpoint = start_fake_with(MachineProfile::basic()).await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    let snapshot = wait_for(&mut channels, ready).await;

    service.query(Query::Spoolman);
    service.query(Query::PowerDevices);

    assert!(!snapshot.sources.moonraker.spoolman);
    // The first event is that of the power devices: nothing was asked of a Spoolman that is not there.
    let first = next_event(&mut channels, |event| matches!(event, Event::Spoolman(_) | Event::PowerDevices(_))).await;
    assert!(matches!(first, Event::PowerDevices(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_active_spool_is_set_and_reported_back() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.send(Command::SetActiveSpool { spool_id: Some(6) });

    let state = loop {
        let Event::Spoolman(state) = next_event(&mut channels, |event| matches!(event, Event::Spoolman(_))).await
        else {
            unreachable!()
        };
        if state.active_spool_id == Some(6) {
            break state;
        }
    };
    assert_eq!(state.spools[&6].material, "PETG");
    let lanes = spools::lanes(&PrinterStatus::default(), &state);
    assert_eq!(spools::active_spool(&lanes).map(|spool| spool.id), Some(6));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spool_is_assigned_to_a_gate_and_its_weight_is_corrected() {
    let endpoint = start_fake_with(MachineProfile::happy_hare(4)).await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.send(Command::AssignGateSpool { gate: 1, spool_id: Some(8) });
    let snapshot = wait_for(&mut channels, |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.gates.get(1).is_some_and(|gate| gate.spool_id == Some(8)))
    })
    .await;
    assert_eq!(snapshot.status.mmu.unwrap().gates[1].spool_id, Some(8));

    service.send(Command::MeasureSpool { spool_id: 8, weight: 480.0 });
    next_event(&mut channels, |event| matches!(event, Event::SpoolmanChanged)).await;
    service.query(Query::Spoolman);
    loop {
        let Event::Spoolman(state) = next_event(&mut channels, |event| matches!(event, Event::Spoolman(_))).await
        else {
            unreachable!()
        };
        if state.spools.get(&8).is_some_and(|spool| spool.remaining_weight == Some(300.0)) {
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_picker_list_is_most_recently_used_first() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.query(Query::SpoolList);
    let Event::SpoolList(spools) = next_event(&mut channels, |event| matches!(event, Event::SpoolList(_))).await else {
        unreachable!()
    };

    assert_eq!(spools.len(), 8);
    assert_eq!(spools[0].id, 8);
    assert_eq!(spools.last().map(|spool| spool.id), Some(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_file_comes_back_as_a_new_file_list_without_asking() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.send(Command::DeleteFile { path: "calibration_cube.gcode".into() });
    let Event::Files(files) = next_event(&mut channels, |event| matches!(event, Event::Files(_))).await else {
        unreachable!()
    };

    assert!(files.iter().all(|file| file.path != "calibration_cube.gcode"));
    assert!(files.iter().any(|file| file.path == "benchy.gcode"));
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_print_job_and_loads_file_details() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.query(Query::Files);
    let Event::Files(files) = next_event(&mut channels, |event| matches!(event, Event::Files(_))).await else {
        unreachable!()
    };
    assert!(files.iter().any(|file| file.path == "benchy.gcode"));

    service.send(Command::StartPrint { filename: "benchy.gcode".into() });
    wait_for(&mut channels, |snapshot| snapshot.status.job.state == PrintState::Printing).await;
    let metadata = next_event(
        &mut channels,
        |event| matches!(event, Event::FileMetadata(meta) if meta.filename == "benchy.gcode"),
    )
    .await;
    let thumbnail = next_event(&mut channels, |event| matches!(event, Event::Thumbnail { .. })).await;
    assert!(matches!(metadata, Event::FileMetadata(meta) if meta.estimated_time == Some(7200.0)));
    // Of 32x32, 300x300 and 400x300 the square one fills the slot best.
    assert!(matches!(thumbnail, Event::Thumbnail { image: ThumbnailImage::Rgba { width: 300, height: 300, .. }, .. }));

    service.send(Command::RestartService { service: "klipper".into() });
    let refused = next_event(&mut channels, |event| matches!(event, Event::CommandFailed { .. })).await;
    assert!(matches!(refused, Event::CommandFailed { error, .. } if error.to_string().contains("print is running")));
    service.send(Command::Jog { axis: Axis::X, distance: 10.0 });
    let refused = next_event(&mut channels, |event| matches!(event, Event::CommandFailed { .. })).await;
    assert!(matches!(refused, Event::CommandFailed { error, .. } if error.to_string().contains("moving the machine")));

    service.send(Command::CancelPrint);
    wait_for(&mut channels, |snapshot| snapshot.status.job.state == PrintState::Cancelled).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_layer_pause_pauses_and_the_history_tells_when_the_job_ended() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.send(Command::StartPrint { filename: "calibration_cube.gcode".into() });
    let printing = wait_for(&mut channels, |snapshot| snapshot.status.job.current_layer.is_some()).await;
    let offer = pause::layer_pause_offer(&printing.sources, &printing.status).expect("layer pause offered");
    service.send(Command::PauseAtLayer { layer: offer.current_layer + 1 });
    let paused = wait_for(&mut channels, |snapshot| snapshot.status.job.state == PrintState::Paused).await;
    assert!(pause::interruption(&paused.status).is_some_and(|interruption| !interruption.failed));

    service.send(Command::CancelPrint);
    let cancelled = wait_for(&mut channels, |snapshot| snapshot.status.job.state == PrintState::Cancelled).await;
    let ended = timeout(WAIT_LIMIT, async {
        loop {
            if let Event::PrintHistory(history) = channels.events.recv().await.expect("service stopped") {
                if let Some(end) = history.newest.and_then(|newest| job_end::ended_at(&cancelled.status.job, &newest)) {
                    return end;
                }
            }
        }
    })
    .await
    .expect("history without the cancelled job");
    assert!(ended > 0.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn loads_the_last_completed_print_of_each_file() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.query(Query::PrintHistory);
    let Event::PrintHistory(history) = next_event(&mut channels, |event| matches!(event, Event::PrintHistory(_))).await
    else {
        unreachable!()
    };

    // The newer cancelled job of the benchy does not count.
    assert_eq!(history.last_prints["benchy.gcode"].print_duration, 7_650.0);
    assert!(!history.last_prints.contains_key("long_job.gcode"));
    assert_eq!(history.newest.map(|job| job.status).as_deref(), Some("cancelled"));
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_errors_and_recovers_from_emergency_stop() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.send(Command::RunGcode { script: "G1 X10".into() });
    let failure = next_event(&mut channels, |event| matches!(event, Event::CommandFailed { .. })).await;
    assert!(matches!(failure, Event::CommandFailed { error, .. } if error.to_string() == "Must home axis first"));

    service.send(Command::EmergencyStop);
    let snapshot = wait_for(&mut channels, |snapshot| snapshot.status.klippy_state == KlippyState::Shutdown).await;
    assert!(snapshot.status.klippy_message.contains("M112"));

    service.send(Command::FirmwareRestart);
    wait_for(&mut channels, ready).await;
}

/// After a shutdown mid-print `print_stats` still reports the print, but it is lost; the restart must not be refused.
#[tokio::test(flavor = "multi_thread")]
async fn firmware_restart_after_shutdown_during_a_print() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;
    service.send(Command::StartPrint { filename: "benchy.gcode".into() });
    wait_for(&mut channels, |snapshot| snapshot.status.job.state == PrintState::Printing).await;

    service.send(Command::EmergencyStop);
    let snapshot = wait_for(&mut channels, |snapshot| snapshot.status.klippy_state == KlippyState::Shutdown).await;
    assert!(snapshot.status.job.state.is_active());

    service.send(Command::FirmwareRestart);
    wait_for(&mut channels, ready).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queries_host_functions() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.query(Query::PowerDevices);
    let Event::PowerDevices(devices) = next_event(&mut channels, |event| matches!(event, Event::PowerDevices(_))).await
    else {
        unreachable!()
    };
    assert_eq!(devices.len(), 2);

    service.query(Query::Updates);
    let Event::Updates(updates) = next_event(&mut channels, |event| matches!(event, Event::Updates(_))).await else {
        unreachable!()
    };
    assert!(updates.iter().any(|update| update.name == "klipper" && update.has_update()));

    service.send(Command::RunGcode { script: "M104 S10".into() });
    wait_for(&mut channels, |snapshot| snapshot.status.heater("extruder").is_some_and(|heater| heater.target == 10.0))
        .await;
    service.query(Query::ConsoleHistory);
    let Event::ConsoleHistory(lines) =
        next_event(&mut channels, |event| matches!(event, Event::ConsoleHistory(_))).await
    else {
        unreachable!()
    };
    assert!(lines.iter().any(|line| line.message == "M104 S10"));
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_line_responses_arrive_as_one_message_with_line_breaks() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.send(Command::RunGcode { script: "HELP".into() });
    let Event::ConsoleLine(line) =
        next_event(&mut channels, |event| matches!(event, Event::ConsoleLine(line) if line.message.contains('\n')))
            .await
    else {
        unreachable!()
    };
    assert_eq!(line.kind, ConsoleKind::Response);
    assert!(line.message.lines().count() > 1);

    service.query(Query::ConsoleHistory);
    let Event::ConsoleHistory(lines) =
        next_event(&mut channels, |event| matches!(event, Event::ConsoleHistory(_))).await
    else {
        unreachable!()
    };
    assert!(lines.iter().any(|stored| stored.message == line.message));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_macro_appears_in_the_console_like_a_typed_command() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    wait_for(&mut channels, ready).await;

    service.send(Command::RunMacro { name: "G28".into() });
    let Event::ConsoleLine(line) =
        next_event(&mut channels, |event| matches!(event, Event::ConsoleLine(line) if line.message == "G28")).await
    else {
        unreachable!()
    };
    assert_eq!(line.kind, ConsoleKind::Command);
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_disconnected_when_server_is_missing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::parse(&listener.local_addr().unwrap().to_string()).unwrap();
    drop(listener);
    let (_service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());

    wait_for(&mut channels, |snapshot| snapshot.connection == ConnectionState::Disconnected).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn fans_lights_sensors_and_tuning_values_follow_the_printer() {
    let endpoint = start_fake().await;
    let (service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    let snapshot = wait_for(&mut channels, ready).await;

    let klipper = &snapshot.sources.klipper;
    assert!(klipper.fans.iter().any(|fan| fan == "fan_generic nevermore"));
    assert_eq!(light::main_light(&snapshot.sources), Some("neopixel chamber"));
    assert_eq!(snapshot.sources.wled_strips, ["wled enclosure"]);
    assert_eq!(klipper.white_channel("neopixel chamber"), WhiteChannel::All);
    assert_eq!(klipper.white_channel("neopixel status"), WhiteChannel::None);
    assert!(snapshot.sources.led_effects.iter().any(|effect| effect == "led_effect rainbow"));
    assert_eq!(klipper.tuning.get(TuningKey::MaxAccel), Some(3000.0));
    assert!(snapshot
        .status
        .sensors
        .iter()
        .any(|sensor| sensor.object == "tmc2240 stepper_x" && sensor.temperature.is_none()));
    assert_eq!(snapshot.status.fans[0].kind, FanKind::Part);

    service.send(Command::ToggleLight);
    service.send(Command::SetFanSpeed { fan: "fan_generic exhaust".into(), speed: 0.5 });
    service.send(Command::SetLedEffect { effect: "rainbow".into(), on: true });
    service.send(Command::StepTuning { key: TuningKey::MaxAccel, direction: 1.0 });
    service.send(Command::SetLight { led: "wled enclosure".into(), level: 0.5 });
    let changed = wait_for(&mut channels, |snapshot| {
        let light_off = snapshot.status.leds.iter().any(|led| led.object == "neopixel chamber" && !led.on);
        let strip_dimmed = snapshot
            .status
            .leds
            .iter()
            .any(|led| led.object == "wled enclosure" && led.on && led.level == Some(128.0 / 255.0));
        let exhaust =
            snapshot.status.fans.iter().any(|fan| fan.object == "fan_generic exhaust" && fan.rpm == Some(1400.0));
        let effect = snapshot.status.led_effects.iter().any(|effect| effect.name == "rainbow" && effect.enabled);
        light_off && strip_dimmed && exhaust && effect && snapshot.status.motion.max_accel == 3500.0
    })
    .await;
    assert_eq!(changed.status.motion.max_accel, 3500.0);

    // The fake samples once per simulated second; ask until the first samples are there.
    timeout(WAIT_LIMIT, async {
        loop {
            service.query(Query::TemperatureHistory);
            let event = next_event(&mut channels, |event| matches!(event, Event::TemperatureHistory(_))).await;
            let Event::TemperatureHistory(history) = event else { unreachable!() };
            if history
                .get("extruder")
                .is_some_and(|series| !series.temperatures.is_empty() && !series.targets.is_empty())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("temperature history not filled in time");
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_in_keeps_the_login_and_logs_out() {
    let endpoint = start_guarded_fake().await;
    let (_dir, credentials) = temporary_credentials();
    let (service, mut channels) = start_service(endpoint.clone(), credentials.clone());

    wait_for(&mut channels, login_required).await;
    service.log_in("test", "wrong");
    let failed = next_event(&mut channels, |event| matches!(event, Event::LoginFailed(_))).await;
    assert_eq!(failed, Event::LoginFailed("Invalid Password".to_owned()));
    service.log_in("test", "secret");
    let snapshot = wait_for(&mut channels, ready).await;
    assert_eq!(snapshot.access, Access::User("test".to_owned()));
    assert!(
        matches!(credentials.credential(&endpoint), Some(Credential::Login { username, .. }) if username == "test")
    );

    // A new start gets in with the stored refresh token, without the password.
    drop(service);
    let (service, mut channels) = start_service(endpoint.clone(), credentials.clone());
    let snapshot = wait_for(&mut channels, ready).await;
    assert_eq!(snapshot.access, Access::User("test".to_owned()));

    service.log_out();
    wait_for(&mut channels, login_required).await;
    assert_eq!(credentials.credential(&endpoint), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_api_key_opens_the_connection_and_the_thumbnails() {
    let endpoint = start_guarded_fake().await;
    let (_dir, credentials) = temporary_credentials();
    credentials.store(&endpoint, Some(Credential::ApiKey { api_key: AccessConfig::API_KEY.to_owned() })).unwrap();
    let (service, mut channels) = start_service(endpoint, credentials);

    let snapshot = wait_for(&mut channels, ready).await;
    assert_eq!(snapshot.access, Access::ApiKey);
    service.query(Query::FileDetails { filename: "benchy.gcode".into() });
    let thumbnail = next_event(&mut channels, |event| matches!(event, Event::Thumbnail { .. })).await;
    assert!(matches!(thumbnail, Event::Thumbnail { image: ThumbnailImage::Rgba { .. }, .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_logged_in_display_fetches_thumbnails_with_a_one_time_token() {
    let endpoint = start_guarded_fake().await;
    let (service, mut channels) = start_service(endpoint, CredentialStore::in_memory());

    wait_for(&mut channels, login_required).await;
    service.log_in("test", "secret");
    wait_for(&mut channels, ready).await;
    service.query(Query::FileDetails { filename: "benchy.gcode".into() });

    next_event(&mut channels, |event| matches!(event, Event::Thumbnail { .. })).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_key_asks_for_a_login_but_does_not_lock_out_a_trusted_display() {
    let wrong = Some(Credential::ApiKey { api_key: "guess".to_owned() });
    let guarded = start_guarded_fake().await;
    let (_guarded_dir, credentials) = temporary_credentials();
    credentials.store(&guarded, wrong.clone()).unwrap();
    let (_service, mut channels) = start_service(guarded, credentials);
    wait_for(&mut channels, login_required).await;

    // Moonraker refuses a wrong key even from a trusted client; the display then goes on without it.
    let trusting =
        start_fake_configured(MachineProfile::voron(), Some(AccessConfig::new("test", "secret").trusting())).await;
    let (_trusting_dir, credentials) = temporary_credentials();
    credentials.store(&trusting, wrong).unwrap();
    let (_service, mut channels) = start_service(trusting, credentials);
    let snapshot = wait_for(&mut channels, ready).await;
    assert_eq!(snapshot.access, Access::Open);
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_fail_while_moonraker_wants_a_login() {
    let endpoint = start_guarded_fake().await;
    let (service, mut channels) = start_service(endpoint, CredentialStore::in_memory());

    wait_for(&mut channels, login_required).await;
    service.send(Command::Home { x: true, y: true, z: true });

    next_event(&mut channels, |event| matches!(event, Event::CommandFailed { .. })).await;
}

/// Head, gate or AFC lane, unit, extruder and whether it is in use, for each lane of the tree.
type LaneSummary = (Option<String>, Option<usize>, Option<String>, Option<String>, String, bool);

async fn tree_of(profile: MachineProfile, lanes: usize) -> (Vec<LaneSummary>, Snapshot) {
    let endpoint = start_fake_with(profile).await;
    let (_service, mut channels) =
        PrinterService::start(&tokio::runtime::Handle::current(), endpoint, CredentialStore::in_memory());
    let has_lanes = |snapshot: &Snapshot| {
        ready(snapshot)
            && snapshot
                .tree
                .lanes(&Default::default())
                .iter()
                .filter(|lane| lane.gate.is_some() || lane.name.is_some())
                .count()
                == lanes
    };
    let snapshot = wait_for(&mut channels, has_lanes).await;
    let summary = snapshot
        .tree
        .lanes(&Default::default())
        .into_iter()
        .map(|lane| (lane.head, lane.gate, lane.name, lane.unit_name, lane.extruder, lane.active))
        .collect();
    (summary, snapshot)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gates_of_two_happy_hare_units_are_named_with_their_unit() {
    let (lanes, _) = tree_of(MachineProfile::happy_hare_units(), 8).await;

    assert!(lanes.iter().all(|lane| lane.0.is_none() && lane.4 == "extruder"));
    let units: Vec<_> = lanes.iter().map(|lane| lane.3.as_deref()).collect();
    assert_eq!(units, [[Some("ERCF"); 4], [Some("Tradrack"); 4]].concat());
}

#[tokio::test(flavor = "multi_thread")]
async fn each_happy_hare_unit_feeds_the_head_of_its_extruder() {
    let (lanes, snapshot) = tree_of(MachineProfile::toolchanger_mmu(), 8).await;

    assert_eq!(snapshot.tree.carriages[0].heads.len(), 2);
    let heads: Vec<_> = lanes.iter().map(|lane| (lane.0.as_deref(), lane.4.as_str())).collect();
    assert_eq!(heads, [[(Some("T0"), "extruder"); 4], [(Some("T1"), "extruder1"); 4]].concat());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_lanes_of_a_box_turtle_feed_the_one_nozzle() {
    let (lanes, snapshot) = tree_of(MachineProfile::afc(), 4).await;

    let names: Vec<_> = lanes.iter().map(|lane| (lane.2.as_deref(), lane.5)).collect();
    assert_eq!(names, [(Some("lane1"), false), (Some("lane2"), true), (Some("lane3"), false), (Some("lane4"), false)]);
    let tree_lanes = snapshot.tree.lanes(&Default::default());
    assert_eq!(tree_lanes.iter().map(|lane| lane.empty).collect::<Vec<_>>(), [false, false, false, true]);
    assert_eq!(tree_lanes[2].spool_holder(), Some(spools::SpoolHolder::Tool(2)));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_heads_of_afcs_tool_changer_each_carry_their_lane() {
    let (lanes, snapshot) = tree_of(MachineProfile::afc_toolchanger(), 4).await;

    let heads = &snapshot.tree.carriages[0].heads;
    assert_eq!(heads.iter().map(|head| head.active).collect::<Vec<_>>(), [false, true, false, false]);
    let placed: Vec<_> = lanes.iter().map(|lane| (lane.0.as_deref(), lane.2.as_deref(), lane.4.as_str())).collect();
    assert_eq!(
        placed,
        [
            (Some("e0"), Some("e0"), "extruder"),
            (Some("e1"), Some("e1"), "extruder1"),
            (Some("e2"), Some("e2"), "extruder2"),
            (Some("e3"), Some("e3"), "extruder3"),
        ]
    );
}
