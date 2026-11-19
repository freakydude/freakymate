//! Trust on first use of a self-signed certificate, end to end against the fake Moonraker served over TLS.

use fake_moonraker::server::{ServerConfig, serve};
use fake_moonraker::tls::TlsIdentity;
use sailhand_core::moonraker::CertificateFingerprint;
use sailhand_core::{
    ConnectionState, CredentialStore, Endpoint, Event, PrinterService, Query, ServiceChannels, Snapshot,
};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::timeout;

const WAIT_LIMIT: Duration = Duration::from_secs(10);

/// A TLS fake on a free port; returns its address as `https://` and the certificate fingerprint.
async fn start_tls_fake() -> (Endpoint, String) {
    let identity = TlsIdentity::generate().unwrap();
    let fingerprint = identity.fingerprint();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = ServerConfig {
        time_scale: 1000.0,
        status_interval: Duration::from_millis(20),
        tls: Some(identity),
        ..ServerConfig::default()
    };
    tokio::spawn(serve(listener, config));
    (Endpoint::parse(&format!("https://{address}")).unwrap(), fingerprint)
}

fn temporary_credentials() -> (tempfile::TempDir, std::path::PathBuf, CredentialStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credentials.json");
    let store = CredentialStore::new(path.clone());
    (dir, path, store)
}

fn start_service(endpoint: &Endpoint, credentials: CredentialStore) -> (PrinterService, ServiceChannels) {
    PrinterService::start(&tokio::runtime::Handle::current(), endpoint.clone(), credentials)
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

fn untrusted(snapshot: &Snapshot) -> bool {
    snapshot.connection == ConnectionState::UntrustedCertificate
}

fn connected(snapshot: &Snapshot) -> bool {
    snapshot.connection == ConnectionState::Connected
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_certificate_waits_for_the_user_to_trust_it() {
    let (endpoint, fingerprint) = start_tls_fake().await;
    let (_dir, _path, credentials) = temporary_credentials();
    let (_service, mut channels) = start_service(&endpoint, credentials);

    let snapshot = wait_for(&mut channels, untrusted).await;
    let offered = snapshot.untrusted_certificate.expect("the certificate is offered for comparison");
    assert_eq!(offered.fingerprint.to_string(), fingerprint);
    assert_eq!(offered.trusted_before, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trusted_certificate_connects_and_is_remembered() {
    let (endpoint, fingerprint) = start_tls_fake().await;
    let (_dir, path, credentials) = temporary_credentials();
    let (service, mut channels) = start_service(&endpoint, credentials.clone());

    let offered = wait_for(&mut channels, untrusted).await.untrusted_certificate.unwrap();
    service.trust_certificate(offered.fingerprint);
    let snapshot = wait_for(&mut channels, connected).await;
    assert_eq!(snapshot.trusted_certificate.map(|pinned| pinned.to_string()), Some(fingerprint.clone()));

    assert!(std::fs::read_to_string(&path).unwrap().contains(&fingerprint));
    let (_second, mut second_channels) = start_service(&endpoint, credentials);
    wait_for(&mut second_channels, connected).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_certificate_is_reported_as_replacing_the_trusted_one() {
    let (endpoint, fingerprint) = start_tls_fake().await;
    let (_dir, _path, credentials) = temporary_credentials();
    credentials.trust_certificate(&endpoint, CertificateFingerprint::of(b"other")).unwrap();
    let (_service, mut channels) = start_service(&endpoint, credentials);

    let snapshot = wait_for(&mut channels, untrusted).await;
    let offered = snapshot.untrusted_certificate.unwrap();
    assert_eq!(offered.fingerprint.to_string(), fingerprint);
    assert_eq!(offered.trusted_before, Some(CertificateFingerprint::of(b"other")));
}

#[tokio::test(flavor = "multi_thread")]
async fn files_over_https_use_the_same_trust() {
    let (endpoint, _fingerprint) = start_tls_fake().await;
    let (_dir, _path, credentials) = temporary_credentials();
    let (service, mut channels) = start_service(&endpoint, credentials);

    let offered = wait_for(&mut channels, untrusted).await.untrusted_certificate.unwrap();
    service.trust_certificate(offered.fingerprint);
    wait_for(&mut channels, connected).await;
    service.query(Query::FileDetails { filename: "benchy.gcode".into() });

    let thumbnail = timeout(WAIT_LIMIT, async {
        loop {
            let event = channels.events.recv().await.expect("service stopped");
            if matches!(event, Event::Thumbnail { .. }) {
                return event;
            }
        }
    })
    .await
    .expect("thumbnail not received in time");
    assert!(matches!(thumbnail, Event::Thumbnail { .. }));
}
