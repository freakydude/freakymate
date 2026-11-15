//! Moonraker's authorization, simplified: one user, the API key and opaque tokens instead of JWTs.
//!
//! Like Moonraker (`components/authorization.py`, `common.py`): a refusal over the WebSocket has the code -32602,
//! login and refresh work without authorization, a logout ends every login of the user, a one-time token for HTTP
//! is valid for five seconds. Not simulated: the expiry of access and refresh tokens, trusted clients,
//! `force_logins`, several users and LDAP.

use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The JSON-RPC code of Moonraker's 401.
pub const REFUSED: i64 = -32602;
/// Methods Moonraker answers without authorization.
pub const OPEN_METHODS: [&str; 4] = ["server.connection.identify", "access.login", "access.refresh_jwt", "access.info"];
const ONESHOT_TIMEOUT: Duration = Duration::from_secs(5);

/// The one user and the API key of a fake with authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessConfig {
    pub username: String,
    pub password: String,
    pub api_key: String,
    /// The client is in `trusted_clients`: it needs no login, but a token or key it shows must still be valid.
    pub trusted: bool,
}

impl AccessConfig {
    /// The API key of every fake with authorization; printed when the fake starts.
    pub const API_KEY: &'static str = "fake-api-key";

    pub fn new(username: &str, password: &str) -> Self {
        Self {
            username: username.to_owned(),
            password: password.to_owned(),
            api_key: Self::API_KEY.to_owned(),
            trusted: false,
        }
    }

    pub fn trusting(self) -> Self {
        Self { trusted: true, ..self }
    }
}

/// Without a configuration everything is open, as for a trusted client.
#[derive(Debug, Default)]
pub struct Access {
    config: Option<AccessConfig>,
    tokens: Mutex<Tokens>,
}

#[derive(Debug, Default)]
struct Tokens {
    issued: u64,
    /// Counts the logouts; a connection authorized by a login before the last one is no longer.
    logouts: u64,
    access: HashSet<String>,
    refresh: HashSet<String>,
    oneshot: HashMap<String, Instant>,
}

impl Tokens {
    fn issue(&mut self, kind: &str) -> String {
        self.issued += 1;
        format!("{kind}{}", self.issued)
    }
}

impl Access {
    pub fn new(config: Option<AccessConfig>) -> Self {
        Self { config, tokens: Mutex::default() }
    }

    /// Moonraker's `authorization` component is there: tokens and keys are checked.
    pub fn enabled(&self) -> bool {
        self.config.is_some()
    }

    /// Only an authorized connection gets answers.
    pub fn required(&self) -> bool {
        self.config.as_ref().is_some_and(|config| !config.trusted)
    }

    /// `access.info`: the fake trusts nobody once it wants a login.
    pub fn info(&self) -> Value {
        json!({
            "default_source": "moonraker",
            "available_sources": ["moonraker"],
            "login_required": self.required(),
            "trusted": !self.required(),
        })
    }

    pub fn log_in(&self, username: &str, password: &str) -> Result<Value, String> {
        let config = self.config.as_ref().ok_or("No user logged in")?;
        if username != config.username {
            return Err(format!("Unregistered User: {username}"));
        }
        if password != config.password {
            return Err("Invalid Password".to_owned());
        }
        let mut tokens = self.lock();
        let token = tokens.issue("access");
        let refresh_token = tokens.issue("refresh");
        tokens.access.insert(token.clone());
        tokens.refresh.insert(refresh_token.clone());
        Ok(json!({
            "username": username, "token": token, "refresh_token": refresh_token,
            "action": "user_logged_in", "source": "moonraker",
        }))
    }

    pub fn refresh(&self, refresh_token: &str) -> Result<Value, String> {
        let username = self.config.as_ref().map(|config| config.username.clone()).ok_or("Invalid Refresh Token")?;
        let mut tokens = self.lock();
        if !tokens.refresh.contains(refresh_token) {
            return Err("Invalid Refresh Token".to_owned());
        }
        let token = tokens.issue("access");
        tokens.access.insert(token.clone());
        Ok(json!({ "username": username, "token": token, "source": "moonraker", "action": "user_jwt_refresh" }))
    }

    /// Ends every login of the user, as Moonraker drops the user's signing key; returns the username.
    pub fn log_out(&self) -> Option<String> {
        let config = self.config.as_ref()?;
        let mut tokens = self.lock();
        tokens.logouts += 1;
        tokens.access.clear();
        tokens.refresh.clear();
        Some(config.username.clone())
    }

    /// The number of logouts so far; a connection logged in with it stays authorized until the next.
    pub fn logouts(&self) -> u64 {
        self.lock().logouts
    }

    pub fn accepts_token(&self, token: &str) -> bool {
        self.lock().access.contains(token)
    }

    pub fn accepts_api_key(&self, api_key: &str) -> bool {
        self.config.as_ref().is_some_and(|config| config.api_key == api_key)
    }

    pub fn oneshot_token(&self) -> String {
        let mut tokens = self.lock();
        let token = tokens.issue("ONESHOT");
        tokens.oneshot.insert(token.clone(), Instant::now());
        token
    }

    /// An HTTP request may pass with a fresh one-time token (used up by it) or the API key.
    pub fn allows_download(&self, oneshot_token: Option<&str>, api_key: Option<&str>) -> bool {
        if !self.required() || api_key.is_some_and(|key| self.accepts_api_key(key)) {
            return true;
        }
        let issued = oneshot_token.and_then(|token| self.lock().oneshot.remove(token));
        issued.is_some_and(|issued| issued.elapsed() < ONESHOT_TIMEOUT)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Tokens> {
        self.tokens.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access() -> Access {
        Access::new(Some(AccessConfig::new("test", "secret")))
    }

    fn field(answer: &Value, name: &str) -> String {
        answer[name].as_str().unwrap().to_owned()
    }

    #[test]
    fn a_login_gives_tokens_that_a_logout_ends() {
        let access = access();

        let login = access.log_in("test", "secret").unwrap();
        let refreshed = access.refresh(&field(&login, "refresh_token")).unwrap();

        assert!(access.accepts_token(&field(&login, "token")));
        assert!(access.accepts_token(&field(&refreshed, "token")));
        assert_eq!(access.log_out().as_deref(), Some("test"));
        assert!(!access.accepts_token(&field(&refreshed, "token")));
        assert!(access.refresh(&field(&login, "refresh_token")).is_err());
        assert_eq!(access.logouts(), 1);
    }

    #[test]
    fn wrong_credentials_are_refused() {
        let access = access();

        assert_eq!(access.log_in("test", "wrong").unwrap_err(), "Invalid Password");
        assert!(access.log_in("other", "secret").is_err());
        assert!(access.refresh("refresh1").is_err());
        assert!(!access.accepts_api_key("guess"));
        assert!(access.accepts_api_key(AccessConfig::API_KEY));
    }

    #[test]
    fn a_oneshot_token_opens_one_download() {
        let access = access();
        let token = access.oneshot_token();

        assert!(!access.allows_download(None, None));
        assert!(access.allows_download(Some(&token), None));
        assert!(!access.allows_download(Some(&token), None));
        assert!(access.allows_download(None, Some(AccessConfig::API_KEY)));
        assert!(Access::default().allows_download(None, None));
    }
}
