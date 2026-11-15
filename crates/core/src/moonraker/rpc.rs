//! JSON-RPC 2.0 messages as Moonraker exchanges them over the WebSocket.

use serde_json::{Map, Value, json};
use thiserror::Error;

/// Error returned by Moonraker or Klipper for a request.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message} ({code})")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// Incoming message from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Response { id: u64, result: Result<Value, RpcError> },
    Notification { method: String, params: Value },
}

pub fn encode_request(id: u64, method: &str, params: Option<Value>) -> String {
    let mut request = Map::new();
    request.insert("jsonrpc".to_owned(), json!("2.0"));
    request.insert("method".to_owned(), json!(method));
    if let Some(params) = params {
        request.insert("params".to_owned(), params);
    }
    request.insert("id".to_owned(), json!(id));
    Value::Object(request).to_string()
}

/// Reads a server message; unknown or broken messages yield `None`.
pub fn decode(text: &str) -> Option<Incoming> {
    let message: Value = serde_json::from_str(text).ok()?;
    if let Some(method) = message.get("method").and_then(Value::as_str) {
        return Some(Incoming::Notification {
            method: method.to_owned(),
            params: message.get("params").cloned().unwrap_or(Value::Null),
        });
    }
    let id = message.get("id").and_then(Value::as_u64)?;
    let result = match message.get("error") {
        Some(error) => Err(RpcError {
            code: error.get("code").and_then(Value::as_i64).unwrap_or_default(),
            message: error.get("message").and_then(Value::as_str).unwrap_or("unknown error").to_owned(),
        }),
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    };
    Some(Incoming::Response { id, result })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_request_with_and_without_params() {
        let with: Value =
            serde_json::from_str(&encode_request(3, "printer.gcode.script", Some(json!({"script": "G28"})))).unwrap();
        let without: Value = serde_json::from_str(&encode_request(4, "server.info", None)).unwrap();

        assert_eq!(
            with,
            json!({"jsonrpc": "2.0", "method": "printer.gcode.script", "params": {"script": "G28"}, "id": 3})
        );
        assert_eq!(without, json!({"jsonrpc": "2.0", "method": "server.info", "id": 4}));
    }

    #[test]
    fn decodes_result_error_and_notification() {
        assert_eq!(
            decode(r#"{"jsonrpc":"2.0","result":"ok","id":1}"#),
            Some(Incoming::Response { id: 1, result: Ok(json!("ok")) })
        );
        assert_eq!(
            decode(r#"{"jsonrpc":"2.0","error":{"code":400,"message":"Must home axis first"},"id":2}"#),
            Some(Incoming::Response {
                id: 2,
                result: Err(RpcError { code: 400, message: "Must home axis first".to_owned() })
            })
        );
        assert_eq!(
            decode(r#"{"jsonrpc":"2.0","method":"notify_gcode_response","params":["ok"]}"#),
            Some(Incoming::Notification { method: "notify_gcode_response".to_owned(), params: json!(["ok"]) })
        );
    }

    #[test]
    fn ignores_garbage() {
        assert_eq!(decode("not json"), None);
        assert_eq!(decode(r#"{"jsonrpc":"2.0","id":"text"}"#), None);
    }
}
