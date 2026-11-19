//! TLS for the fake: a self-signed certificate and a listener that wraps accepted connections.
//!
//! Real Moonraker serves TLS itself on `ssl_port` (default 7130) with `ssl_certificate_path` and
//! `ssl_key_path` in `[server]` (https://moonraker.readthedocs.io/en/latest/configuration/#server).
//! The fake keeps the port given by `--port` and generates its certificate at start.

use axum::serve::Listener;
use rcgen::{CertificateParams, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::fmt::Write as _;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

/// Names the generated certificate is valid for (an IP SAN is parsed from the second entry).
const CERTIFICATE_NAMES: [&str; 2] = ["localhost", "127.0.0.1"];

/// Certificate and key the fake presents.
#[derive(Debug, Clone)]
pub struct TlsIdentity {
    certificate: CertificateDer<'static>,
    key: Vec<u8>,
}

impl TlsIdentity {
    /// A new self-signed certificate for `localhost` and `127.0.0.1`.
    pub fn generate() -> Result<Self, rcgen::Error> {
        let key_pair = KeyPair::generate()?;
        let params = CertificateParams::new(CERTIFICATE_NAMES.map(String::from))?;
        let certificate = params.self_signed(&key_pair)?;
        Ok(Self { certificate: certificate.der().clone(), key: key_pair.serialize_der() })
    }

    /// SHA-256 over the DER certificate as upper-case hex pairs with colons, like `openssl x509 -fingerprint -sha256`.
    pub fn fingerprint(&self) -> String {
        let digest = ring::digest::digest(&ring::digest::SHA256, self.certificate.as_ref());
        let mut text = String::new();
        for (index, byte) in digest.as_ref().iter().enumerate() {
            if index > 0 {
                text.push(':');
            }
            // Writing to a String cannot fail.
            let _ = write!(text, "{byte:02X}");
        }
        text
    }

    fn acceptor(&self) -> Result<TlsAcceptor, rustls::Error> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                vec![self.certificate.clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone())),
            )?;
        Ok(TlsAcceptor::from(Arc::new(config)))
    }
}

/// Accepts TCP connections and runs the TLS handshake; a failed handshake is logged and skipped.
pub struct TlsListener {
    tcp: TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsListener {
    pub fn new(tcp: TcpListener, identity: &TlsIdentity) -> io::Result<Self> {
        let acceptor = identity.acceptor().map_err(io::Error::other)?;
        Ok(Self { tcp, acceptor })
    }
}

impl Listener for TlsListener {
    type Io = TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, address) = match self.tcp.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    eprintln!("accept failed: {error}");
                    continue;
                }
            };
            match self.acceptor.accept(stream).await {
                Ok(stream) => return (stream, address),
                Err(error) => eprintln!("TLS handshake with {address} failed: {error}"),
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_is_32_upper_case_hex_pairs_that_differ_per_certificate() {
        let first = TlsIdentity::generate().unwrap().fingerprint();
        assert_eq!(first.split(':').count(), 32);
        assert!(
            first.split(':').all(|pair| pair.len() == 2 && pair.chars().all(|c| matches!(c, '0'..='9' | 'A'..='F')))
        );
        assert_ne!(first, TlsIdentity::generate().unwrap().fingerprint());
    }
}
