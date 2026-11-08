//! Access to Moonraker servers that do not trust this machine, kept apart from the settings.
//!
//! The settings may lie in `printer_data/config`, which Moonraker serves to Mainsail and Fluidd and which is often
//! backed up to a git repository. The credentials therefore always stay in the user's configuration directory, in a
//! file only the owner can read.

use crate::atomic_file::write_atomically;
use crate::moonraker::Endpoint;
use crate::settings::APP_FOLDER;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "credentials.json";

/// How this display gets into one Moonraker server.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Credential {
    /// Moonraker's API key, entered in the file by hand.
    ApiKey { api_key: String },
    /// A user logged in on the touch screen; the refresh token stands for the password, which is not kept.
    Login { username: String, refresh_token: String },
}

/// Never shows the secret, so it cannot end up in a log.
impl std::fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey { .. } => formatter.write_str("ApiKey"),
            Self::Login { username, .. } => write!(formatter, "Login({username})"),
        }
    }
}

/// The file: one credential per Moonraker address (as [`Endpoint::display`] writes it).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CredentialFile {
    moonraker: BTreeMap<String, Credential>,
}

/// Reads and writes the credentials; without a path (tests, snapshots) nothing is kept.
#[derive(Debug, Clone, Default)]
pub struct CredentialStore {
    path: Option<PathBuf>,
}

impl CredentialStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: Some(path.into()) }
    }

    /// Default location: `<config dir>/freakymate/credentials.json`; none if the system has no such directory.
    pub fn default_location() -> Self {
        Self { path: dirs::config_dir().map(|directory| directory.join(APP_FOLDER).join(FILE_NAME)) }
    }

    /// A store that keeps nothing.
    pub fn in_memory() -> Self {
        Self::default()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// An unreadable file counts as empty: the display then connects like a trusted client.
    pub fn credential(&self, endpoint: &Endpoint) -> Option<Credential> {
        self.read().moonraker.remove(&endpoint.display())
    }

    /// Stores the credential for this address, or removes it with `None`.
    pub fn store(&self, endpoint: &Endpoint, credential: Option<Credential>) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let mut file = self.read();
        match credential {
            Some(credential) => file.moonraker.insert(endpoint.display(), credential),
            None => file.moonraker.remove(&endpoint.display()),
        };
        let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        write_atomically(path, &text, true)
    }

    fn read(&self) -> CredentialFile {
        let Some(path) = &self.path else { return CredentialFile::default() };
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                log::warn!("ignoring unreadable {}: {error}", path.display());
                CredentialFile::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => CredentialFile::default(),
            Err(error) => {
                log::warn!("cannot read {}: {error}", path.display());
                CredentialFile::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store in a folder that does not exist yet; the folder is removed when the returned guard is dropped.
    fn temporary_store() -> (tempfile::TempDir, CredentialStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(dir.path().join("freakymate").join(FILE_NAME));
        (dir, store)
    }

    fn login() -> Credential {
        Credential::Login { username: "frank".to_owned(), refresh_token: "secret".to_owned() }
    }

    #[test]
    fn credentials_are_kept_per_address() {
        let (_dir, store) = temporary_store();
        let voron = Endpoint::parse("voron.local:7125").unwrap();
        let ender = Endpoint::parse("http://ender.local").unwrap();

        store.store(&voron, Some(login())).unwrap();
        store.store(&ender, Some(Credential::ApiKey { api_key: "key".to_owned() })).unwrap();

        assert_eq!(store.credential(&voron), Some(login()));
        assert_eq!(store.credential(&ender), Some(Credential::ApiKey { api_key: "key".to_owned() }));
        store.store(&voron, None).unwrap();
        assert_eq!(store.credential(&voron), None);
        assert!(store.credential(&ender).is_some());
    }

    #[test]
    fn a_hand_written_api_key_is_read() {
        let (_dir, store) = temporary_store();
        let path = store.path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, r#"{ "moonraker": { "http://pi:7125": { "api_key": "abc" } } }"#).unwrap();

        let credential = store.credential(&Endpoint::parse("pi:7125").unwrap());

        assert_eq!(credential, Some(Credential::ApiKey { api_key: "abc".to_owned() }));
    }

    #[test]
    fn a_broken_file_counts_as_empty() {
        let (_dir, store) = temporary_store();
        let path = store.path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{ nope").unwrap();

        assert_eq!(store.credential(&Endpoint::parse("pi").unwrap()), None);
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_can_read_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, store) = temporary_store();

        store.store(&Endpoint::parse("pi").unwrap(), Some(login())).unwrap();

        let mode = std::fs::metadata(store.path().unwrap()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn debug_output_hides_the_secrets() {
        let shown = format!("{:?} {:?}", login(), Credential::ApiKey { api_key: "key".to_owned() });

        assert_eq!(shown, "Login(frank) ApiKey");
    }

    #[test]
    fn a_store_without_path_keeps_nothing() {
        let store = CredentialStore::in_memory();
        let endpoint = Endpoint::parse("pi").unwrap();

        store.store(&endpoint, Some(login())).unwrap();

        assert_eq!(store.credential(&endpoint), None);
    }
}
