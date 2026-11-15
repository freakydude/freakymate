//! Moonraker's authorization over the WebSocket: logging in, tokens, and how a refusal looks.
//!
//! Moonraker offers every `/access/...` endpoint as a JSON-RPC method too (`components/authorization.py`); login and
//! refresh work without being authorized. A connection stays authorized once `server.connection.identify` took a
//! token or key, or once `access.login` succeeded on it, until it closes or the user logs out.

use super::{CallError, Connection, RpcError};
use serde_json::{Value, json};
use thiserror::Error;

/// Moonraker answers a refused request over the WebSocket with this code instead of HTTP's 401 (`common.py`,
/// `JsonRPC.execute_method`). It is also the code for invalid parameters, so it means a refusal only for the
/// requests here, whose parameters are always complete.
const REFUSED: i64 = -32602;

#[derive(Debug, Error)]
pub enum AccessError {
    /// Moonraker refused the credentials, or the connection is not authorized.
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Call(CallError),
    #[error("unreadable answer to {0}")]
    Unreadable(&'static str),
}

impl From<CallError> for AccessError {
    fn from(error: CallError) -> Self {
        match error {
            CallError::Rpc(RpcError { code: REFUSED, message }) => Self::Refused(message),
            error => Self::Call(error),
        }
    }
}

/// The answer to a login: an access token for this hour and a refresh token for the following ones.
#[derive(Clone, PartialEq, Eq)]
pub struct Login {
    pub username: String,
    pub token: String,
    pub refresh_token: String,
}

/// Identifies with Moonraker; `credential` is `("access_token" | "api_key", value)` when the display has one.
pub async fn identify(
    connection: &Connection,
    mut identity: Value,
    credential: Option<(&str, &str)>,
) -> Result<(), AccessError> {
    if let (Some((field, value)), Some(fields)) = (credential, identity.as_object_mut()) {
        fields.insert(field.to_owned(), json!(value));
    }
    connection.call("server.connection.identify", Some(identity)).await?;
    Ok(())
}

/// Logs the user in; the connection is authorized afterwards.
pub async fn log_in(connection: &Connection, username: &str, password: &str) -> Result<Login, AccessError> {
    let answer = connection.call("access.login", Some(json!({ "username": username, "password": password }))).await?;
    parse_login(&answer).ok_or(AccessError::Unreadable("access.login"))
}

/// A new access token from the refresh token of an earlier login; refused once the login has expired or the user
/// logged out (then everywhere, as the refresh token belongs to the user, not to this display).
pub async fn refresh(connection: &Connection, refresh_token: &str) -> Result<String, AccessError> {
    let answer = connection.call("access.refresh_jwt", Some(json!({ "refresh_token": refresh_token }))).await?;
    text_field(&answer, "token").ok_or(AccessError::Unreadable("access.refresh_jwt"))
}

pub async fn log_out(connection: &Connection) -> Result<(), AccessError> {
    connection.call("access.logout", None).await?;
    Ok(())
}

/// A token for one HTTP request (`?token=`), valid for five seconds and only from this machine's address.
pub async fn oneshot_token(connection: &Connection) -> Result<String, AccessError> {
    let answer = connection.call("access.oneshot_token", None).await?;
    answer.as_str().map(str::to_owned).ok_or(AccessError::Unreadable("access.oneshot_token"))
}

fn parse_login(answer: &Value) -> Option<Login> {
    Some(Login {
        username: text_field(answer, "username")?,
        token: text_field(answer, "token")?,
        refresh_token: text_field(answer, "refresh_token")?,
    })
}

fn text_field(answer: &Value, name: &str) -> Option<String> {
    answer.get(name).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_login_answer() {
        let answer = json!({
            "username": "frank", "token": "jwt", "refresh_token": "refresh",
            "action": "user_logged_in", "source": "moonraker",
        });

        let login = parse_login(&answer).unwrap();

        assert_eq!((login.username.as_str(), login.token.as_str()), ("frank", "jwt"));
        assert_eq!(login.refresh_token, "refresh");
        assert!(parse_login(&json!({ "username": "frank" })).is_none());
    }

    #[test]
    fn only_moonrakers_refusal_code_counts_as_refused() {
        let refused = AccessError::from(CallError::Rpc(RpcError { code: REFUSED, message: "Unauthorized".to_owned() }));
        let failed = AccessError::from(CallError::Rpc(RpcError { code: 400, message: "Invalid".to_owned() }));

        assert!(matches!(refused, AccessError::Refused(message) if message == "Unauthorized"));
        assert!(matches!(failed, AccessError::Call(_)));
        assert!(matches!(AccessError::from(CallError::Closed), AccessError::Call(CallError::Closed)));
    }
}
