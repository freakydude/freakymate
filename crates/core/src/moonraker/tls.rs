//! TLS for `https://` and `wss://` addresses.
//!
//! Moonraker serves TLS itself (`ssl_port`, `ssl_certificate_path`, `ssl_key_path` in `[server]`) or behind a reverse
//! proxy. Its certificate is mostly self-signed, so besides the roots of the system one certificate per address can be
//! trusted by its SHA-256 fingerprint, once the user has compared it ("trust on first use").

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use thiserror::Error;

const FINGERPRINT_BYTES: usize = 32;

/// SHA-256 over the DER encoding of a certificate, as `openssl x509 -fingerprint -sha256` and browsers show it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CertificateFingerprint([u8; FINGERPRINT_BYTES]);

impl CertificateFingerprint {
    pub fn of(certificate_der: &[u8]) -> Self {
        let digest = ring::digest::digest(&ring::digest::SHA256, certificate_der);
        let mut bytes = [0; FINGERPRINT_BYTES];
        bytes.copy_from_slice(digest.as_ref());
        Self(bytes)
    }
}

/// Upper-case hex pairs separated by colons.
impl fmt::Display for CertificateFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str(":")?;
            }
            write!(formatter, "{byte:02X}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for CertificateFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "CertificateFingerprint({self})")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid SHA-256 fingerprint '{0}', expected 32 hex pairs")]
pub struct InvalidFingerprint(String);

/// Accepts hex in either case, with or without colons or spaces between the pairs.
impl FromStr for CertificateFingerprint {
    type Err = InvalidFingerprint;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidFingerprint(text.to_owned());
        let digits: Vec<u8> = text.bytes().filter(|byte| !matches!(byte, b':' | b' ')).collect();
        if digits.len() != FINGERPRINT_BYTES * 2 {
            return Err(invalid());
        }
        let mut bytes = [0; FINGERPRINT_BYTES];
        let (pairs, _) = digits.as_chunks::<2>();
        for (byte, pair) in bytes.iter_mut().zip(pairs) {
            let pair = std::str::from_utf8(pair).map_err(|_| invalid())?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| invalid())?;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for CertificateFingerprint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CertificateFingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The server presented a certificate that neither the system roots nor the trusted fingerprint accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UntrustedCertificate {
    /// Of the certificate the server presented; what the user compares and may trust.
    pub fingerprint: CertificateFingerprint,
    /// The different certificate trusted for this address before: the server got a new one, or someone else answers.
    /// Trusting the new one overwrites it.
    pub trusted_before: Option<CertificateFingerprint>,
}

/// TLS settings of one connection attempt to one address; cheap to clone.
#[derive(Clone)]
pub struct Tls {
    config: Arc<ClientConfig>,
    verifier: Arc<TrustVerifier>,
}

impl Tls {
    /// Accepts certificates the system trusts and, in addition, the certificate with the `trusted` fingerprint.
    pub fn new(trusted: Option<CertificateFingerprint>) -> Self {
        let provider = crypto_provider();
        let verifier = Arc::new(TrustVerifier {
            system: system_verifier().clone(),
            trusted,
            rejected: Mutex::new(None),
            algorithms: provider.signature_verification_algorithms,
        });
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("ring supports the default TLS versions")
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();
        Self { config: Arc::new(config), verifier }
    }

    pub fn client_config(&self) -> Arc<ClientConfig> {
        Arc::clone(&self.config)
    }

    /// The HTTP client for file transfers and thumbnails, with the same trust as the WebSocket.
    pub fn http_client(&self) -> reqwest::Result<reqwest::Client> {
        reqwest::Client::builder().tls_backend_preconfigured(ClientConfig::clone(&self.config)).build()
    }

    /// The certificate the last handshake refused, if any; taking it clears it.
    pub fn take_rejected(&self) -> Option<UntrustedCertificate> {
        let fingerprint = lock(&self.verifier.rejected).take()?;
        let trusted_before = self.verifier.trusted.filter(|trusted| *trusted != fingerprint);
        Some(UntrustedCertificate { fingerprint, trusted_before })
    }

    /// The certificate the user trusted for this address, besides those the system trusts.
    pub fn trusted(&self) -> Option<CertificateFingerprint> {
        self.verifier.trusted
    }
}

/// `ring`, also as the process default so that any other rustls user in the process finds a provider.
fn crypto_provider() -> Arc<CryptoProvider> {
    static PROVIDER: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| {
            let provider = rustls::crypto::ring::default_provider();
            // Fails only if another provider was installed first; that one then serves the others.
            let _ = provider.clone().install_default();
            Arc::new(provider)
        })
        .clone()
}

/// Built once: loading the roots reads the system's certificate store. `None` where the system has none (Android).
fn system_verifier() -> &'static Option<Arc<WebPkiServerVerifier>> {
    static VERIFIER: OnceLock<Option<Arc<WebPkiServerVerifier>>> = OnceLock::new();
    VERIFIER.get_or_init(|| {
        let loaded = rustls_native_certs::load_native_certs();
        for error in &loaded.errors {
            log::debug!("system certificate store: {error}");
        }
        let mut roots = RootCertStore::empty();
        let (added, ignored) = roots.add_parsable_certificates(loaded.certs);
        log::debug!("system certificate store: {added} roots, {ignored} unreadable");
        if roots.is_empty() {
            return None;
        }
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), crypto_provider())
            .build()
            .inspect_err(|error| log::warn!("cannot use the system certificate store: {error}"))
            .ok()
    })
}

#[derive(Debug)]
struct TrustVerifier {
    system: Option<Arc<WebPkiServerVerifier>>,
    trusted: Option<CertificateFingerprint>,
    rejected: Mutex<Option<CertificateFingerprint>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for TrustVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let presented = CertificateFingerprint::of(end_entity);
        // The user compared exactly this certificate; name and validity period were part of that decision.
        if self.trusted == Some(presented) {
            return Ok(ServerCertVerified::assertion());
        }
        let refusal = match &self.system {
            Some(system) => match system.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            {
                Ok(verified) => return Ok(verified),
                Err(error) => error,
            },
            None => rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer),
        };
        log::debug!("certificate {presented} refused: {refusal}");
        *lock(&self.rejected) = Some(presented);
        Err(refusal)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, certificate, signature, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, certificate, signature, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str =
        "01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF";

    fn presented(verifier: &TrustVerifier, der: &[u8]) -> Result<ServerCertVerified, rustls::Error> {
        let name = ServerName::try_from("printer.local").unwrap();
        verifier.verify_server_cert(&CertificateDer::from(der.to_vec()), &[], &name, &[], UnixTime::now())
    }

    fn verifier(trusted: Option<CertificateFingerprint>) -> TrustVerifier {
        // No system roots, like Android: only the trusted fingerprint can let a certificate in.
        TrustVerifier {
            system: None,
            trusted,
            rejected: Mutex::new(None),
            algorithms: crypto_provider().signature_verification_algorithms,
        }
    }

    #[test]
    fn a_fingerprint_reads_back_what_it_writes() {
        let fingerprint: CertificateFingerprint = EXAMPLE.parse().unwrap();

        assert_eq!(fingerprint.to_string(), EXAMPLE);
        assert_eq!(EXAMPLE.to_lowercase().replace(':', " ").parse(), Ok(fingerprint));
    }

    #[test]
    fn a_wrong_fingerprint_is_refused() {
        assert!("01:23".parse::<CertificateFingerprint>().is_err());
        assert!(EXAMPLE.replace("EF", "EG").parse::<CertificateFingerprint>().is_err());
    }

    #[test]
    fn the_fingerprint_is_the_sha256_of_the_certificate() {
        // `printf abc | sha256sum`
        let expected =
            "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD";

        assert_eq!(CertificateFingerprint::of(b"abc").to_string(), expected);
    }

    #[test]
    fn the_trusted_certificate_is_accepted() {
        let verifier = verifier(Some(CertificateFingerprint::of(b"certificate")));

        assert!(presented(&verifier, b"certificate").is_ok());
        assert_eq!(*lock(&verifier.rejected), None);
    }

    #[test]
    fn an_unknown_certificate_is_refused_and_remembered() {
        let verifier = verifier(None);

        assert!(presented(&verifier, b"certificate").is_err());
        assert_eq!(lock(&verifier.rejected).take(), Some(CertificateFingerprint::of(b"certificate")));
    }

    #[test]
    fn a_changed_certificate_says_so() {
        let tls = Tls::new(Some(CertificateFingerprint::of(b"old")));

        assert!(presented(&tls.verifier, b"new").is_err());
        let rejected = tls.take_rejected().unwrap();
        assert_eq!(rejected.fingerprint, CertificateFingerprint::of(b"new"));
        assert_eq!(rejected.trusted_before, Some(CertificateFingerprint::of(b"old")));
        assert_eq!(tls.take_rejected(), None);
    }
}
