//! JSON-RPC session of a WebSocket connection.

use crate::access;
use crate::backend::Backend;
use crate::files::GcodeFile;
use crate::gcode::{self, CommandError};
use crate::printer::Printer;
use crate::subscription::Subscription;
use serde_json::{json, Value};
use std::time::Duration;

const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const PARSE_ERROR: i64 = -32700;
const REQUEST_FAILED: i64 = 400;
const NOT_FOUND: i64 = 404;

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(REQUEST_FAILED, message)
    }

    fn to_json(&self) -> Value {
        json!({ "code": self.code, "message": self.message })
    }
}

impl From<CommandError> for RpcError {
    fn from(error: CommandError) -> Self {
        Self::new(REQUEST_FAILED, error.message())
    }
}

pub struct Session {
    connection_id: u64,
    subscription: Subscription,
    reply_delay: Option<Duration>,
    authorization: Authorization,
}

/// What a connection showed; it only matters while the fake wants a login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authorization {
    Missing,
    ApiKey,
    /// Logged in, valid until the next logout (the backend counts them).
    Login {
        logouts: u64,
    },
}

impl Session {
    pub fn new(connection_id: u64) -> Self {
        Self {
            connection_id,
            subscription: Subscription::default(),
            reply_delay: None,
            authorization: Authorization::Missing,
        }
    }

    /// Moonraker sends notifications only to authorized connections.
    pub fn authorized(&self, backend: &Backend) -> bool {
        let access = backend.access();
        !access.required()
            || match self.authorization {
                Authorization::Missing => false,
                Authorization::ApiKey => true,
                Authorization::Login { logouts } => logouts == access.logouts(),
            }
    }

    /// Like Moonraker: a token or key must be valid even where none is needed, and without one the connection must
    /// already be authorized.
    fn identify(&mut self, backend: &Backend, params: &Value) -> Result<Value, RpcError> {
        let access = backend.access();
        if access.enabled() {
            if let Some(token) = params.get("access_token").and_then(Value::as_str) {
                if !access.accepts_token(token) {
                    return Err(RpcError::new(access::REFUSED, "Failed to decode JWT: unknown token"));
                }
                self.authorization = Authorization::Login { logouts: access.logouts() };
            } else if let Some(api_key) = params.get("api_key").and_then(Value::as_str) {
                if !access.accepts_api_key(api_key) {
                    return Err(RpcError::new(access::REFUSED, "Invalid API Key"));
                }
                self.authorization = Authorization::ApiKey;
            } else if !self.authorized(backend) {
                return Err(unauthorized());
            }
        }
        Ok(json!({ "connection_id": self.connection_id }))
    }

    fn log_in(&mut self, backend: &Backend, params: &Value) -> Result<Value, RpcError> {
        let text = |name: &str| params.get(name).and_then(Value::as_str).unwrap_or_default();
        let login = backend
            .access()
            .log_in(text("username"), text("password"))
            .map_err(|message| RpcError::new(access::REFUSED, message))?;
        self.authorization = Authorization::Login { logouts: backend.access().logouts() };
        Ok(login)
    }

    fn log_out(backend: &Backend) -> Result<Value, RpcError> {
        let username = backend.access().log_out().ok_or_else(|| RpcError::invalid_params("No user logged in"))?;
        backend.notify("notify_user_logged_out", json!([{ "username": username }]));
        Ok(json!({ "username": username, "action": "user_logged_out" }))
    }

    /// How long the reply to the last message waits (a script with `G4`); the server sends it later.
    pub fn take_reply_delay(&mut self) -> Option<Duration> {
        self.reply_delay.take()
    }

    /// Handles an incoming message; `None` if no reply is due.
    pub fn handle_text(&mut self, backend: &Backend, text: &str) -> Option<String> {
        match serde_json::from_str::<Value>(text) {
            Ok(Value::Array(requests)) => {
                let responses: Vec<Value> =
                    requests.iter().filter_map(|request| self.handle_request(backend, request)).collect();
                (!responses.is_empty()).then(|| Value::Array(responses).to_string())
            }
            Ok(request) => self.handle_request(backend, &request).map(|response| response.to_string()),
            Err(_) => Some(failure(Value::Null, &RpcError::new(PARSE_ERROR, "Parse error")).to_string()),
        }
    }

    /// Changes of the subscribed objects since the last report as `notify_status_update`.
    pub fn status_notification(&mut self, backend: &Backend) -> Option<String> {
        if !self.authorized(backend) {
            return None;
        }
        let changes = self.subscription.changes(&backend.status())?;
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notify_status_update",
            "params": [changes, backend.eventtime()],
        });
        Some(notification.to_string())
    }

    fn handle_request(&mut self, backend: &Backend, request: &Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let outcome = match request.get("method").and_then(Value::as_str) {
            Some(method) => {
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                self.dispatch(backend, method, &params)
            }
            None => Err(RpcError::new(INVALID_REQUEST, "Invalid Request")),
        };
        let id = id?;
        Some(match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "result": result, "id": id }),
            Err(error) => failure(id, &error),
        })
    }

    fn dispatch(&mut self, backend: &Backend, method: &str, params: &Value) -> Result<Value, RpcError> {
        if !access::OPEN_METHODS.contains(&method) && !self.authorized(backend) {
            return Err(unauthorized());
        }
        match method {
            "server.connection.identify" => self.identify(backend, params),
            "access.info" => Ok(backend.access().info()),
            "access.login" => self.log_in(backend, params),
            "access.refresh_jwt" => backend
                .access()
                .refresh(params.get("refresh_token").and_then(Value::as_str).unwrap_or_default())
                .map_err(|message| RpcError::new(access::REFUSED, message)),
            "access.logout" => Self::log_out(backend),
            "access.oneshot_token" => Ok(json!(backend.access().oneshot_token())),
            "server.info" => Ok(server_info(backend)),
            "printer.info" => Ok(printer_info(backend)),
            "printer.gcode.help" => Ok(gcode_help(backend)),
            "server.gcode_store" => Ok(backend.with_host(|host| {
                host.gcode_store(params.get("count").and_then(Value::as_u64).map(|count| count as usize))
            })),
            "machine.device_power.devices" => Ok(backend.with_host(|host| host.power_devices())),
            "machine.device_power.post_device" => set_power(backend, params),
            "machine.update.status" | "machine.update.refresh" => Ok(backend.with_host(|host| host.update_status())),
            "machine.update.upgrade" => backend
                .with_host(|host| host.upgrade(params.get("name").and_then(Value::as_str)))
                .map(|()| ok())
                .map_err(|message| RpcError::new(METHOD_NOT_FOUND, message)),
            "machine.services.restart" | "machine.reboot" | "machine.shutdown" => Ok(ok()),
            "machine.system_info" => Ok(backend.with_host(|host| host.system_info())),
            "server.temperature_store" => Ok(backend.with_printer(|printer| printer.temperature_store())),
            "printer.objects.subscribe" => self.subscribe(backend, params),
            "printer.objects.list" => Ok(json!({ "objects": backend.object_names() })),
            "printer.objects.query" => query_objects(backend, params),
            "printer.gcode.script" => run_script(backend, params).map(|(reply, dwell)| {
                self.reply_delay = Some(dwell).filter(|dwell| !dwell.is_zero());
                reply
            }),
            "printer.print.start" => start_print(backend, params),
            "printer.print.pause" => control_printer(backend, Printer::pause_print),
            "printer.print.resume" => control_printer(backend, Printer::resume_print),
            "printer.print.cancel" => control_printer(backend, Printer::cancel_print),
            "printer.emergency_stop" => {
                backend.with_printer(Printer::emergency_stop);
                backend.respond("!! Shutdown due to M112 command");
                backend.notify("notify_klippy_shutdown", Value::Null);
                Ok(ok())
            }
            "printer.restart" | "printer.firmware_restart" => {
                backend.with_printer(Printer::restart);
                backend.respond("// Klipper state: Ready");
                backend.notify("notify_klippy_ready", Value::Null);
                Ok(ok())
            }
            "server.files.list" => Ok(backend.catalog().list()),
            "server.files.metadata" => Ok(find_file(backend, params)?.metadata()),
            "server.history.list" => Ok(backend.history()),
            "server.files.delete_file" => delete_file(backend, params),
            "server.files.thumbnails" => Ok(find_file(backend, params)?.thumbnails()),
            "server.spoolman.status" if backend.spoolman_enabled() => {
                Ok(backend.with_spoolman(|spoolman| spoolman.status()))
            }
            "server.spoolman.post_spool_id" if backend.spoolman_enabled() => set_active_spool(backend, params),
            "server.spoolman.proxy" if backend.spoolman_enabled() => Ok(spoolman_proxy(backend, params)),
            "machine.wled.strips" if backend.wled_enabled() => Ok(backend.with_wled(|wled| wled.strips())),
            "machine.wled.post_strip" if backend.wled_enabled() => post_strip(backend, params),
            _ => Err(RpcError::new(METHOD_NOT_FOUND, format!("Method not found: {method}"))),
        }
    }

    fn subscribe(&mut self, backend: &Backend, params: &Value) -> Result<Value, RpcError> {
        let objects = params.get("objects").ok_or_else(|| RpcError::invalid_params("missing parameter 'objects'"))?;
        self.subscription.replace_with(objects).map_err(RpcError::invalid_params)?;
        let status = self.subscription.snapshot(&backend.status());
        Ok(json!({ "eventtime": backend.eventtime(), "status": status }))
    }
}

fn query_objects(backend: &Backend, params: &Value) -> Result<Value, RpcError> {
    let objects = params.get("objects").ok_or_else(|| RpcError::invalid_params("missing parameter 'objects'"))?;
    let mut query = Subscription::default();
    query.replace_with(objects).map_err(RpcError::invalid_params)?;
    Ok(json!({ "eventtime": backend.eventtime(), "status": query.snapshot(&backend.status()) }))
}

/// Like Klipper's HELP: one response with a line per command, which Moonraker stores and forwards as a single message.
const HELP_TEXT: &str = "Available commands:\n\
    G0, G1: Move to a position\n\
    G28: Home axes\n\
    G90, G91: Absolute or relative positioning\n\
    M104, M140: Set extruder or bed target\n\
    M106, M107: Set fan speed\n\
    M220, M221: Set speed or extrude factor\n\
    SET_GCODE_OFFSET: Change the Z homing origin\n\
    HELP: Show this list";

/// Runs a script; also returns how long its dwells (`G4`) take.
fn run_script(backend: &Backend, params: &Value) -> Result<(Value, Duration), RpcError> {
    let script = required_str(params, "script")?;
    backend.record_command(script);
    let outcome = gcode::parse_script(script)
        .inspect_err(|_| {
            // A script that does not parse is a rejected command as well (`gcode:command_error`).
            backend.with_printer(|printer| printer.note_command_error());
        })
        .and_then(|commands| {
            let help = commands.contains(&gcode::Command::Help);
            let dwell = commands
                .iter()
                .map(|command| match command {
                    gcode::Command::Dwell(duration) => *duration,
                    _ => Duration::ZERO,
                })
                .sum();
            backend.with_printer(|printer| commands.into_iter().try_for_each(|command| printer.apply(command)))?;
            if help {
                backend.respond(HELP_TEXT);
            }
            Ok(dwell)
        });
    if let Err(error) = &outcome {
        backend.respond(&format!("!! {}", error.message()));
    }
    let dwell = outcome?;
    follow_selected_gate(backend);
    Ok((ok(), dwell))
}

/// Happy Hare makes the spool of the selected gate the active one in Moonraker.
fn follow_selected_gate(backend: &Backend) {
    if !backend.spoolman_enabled() || !backend.with_printer(|printer| printer.has_mmu()) {
        return;
    }
    let spool = backend.with_printer(|printer| printer.selected_gate_spool());
    if backend.with_spoolman(|spoolman| spoolman.set_active(spool)) == Ok(true) {
        backend.notify("notify_active_spool_set", json!([{ "spool_id": spool }]));
    }
}

/// `server.spoolman.post_spool_id`; `null` unsets the active spool.
fn set_active_spool(backend: &Backend, params: &Value) -> Result<Value, RpcError> {
    let spool_id = params.get("spool_id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok());
    let changed = backend.with_spoolman(|spoolman| spoolman.set_active(spool_id)).map_err(RpcError::invalid_params)?;
    if changed {
        backend.notify("notify_active_spool_set", json!([{ "spool_id": spool_id }]));
    }
    Ok(json!({ "spool_id": spool_id }))
}

/// `server.spoolman.proxy` with the version 2 response.
fn spoolman_proxy(backend: &Backend, params: &Value) -> Value {
    let text = |field: &str| params.get(field).and_then(Value::as_str).unwrap_or_default().to_owned();
    let body = params.get("body").cloned().unwrap_or(Value::Null);
    match backend.with_spoolman(|spoolman| spoolman.proxy(&text("request_method"), &text("path"), &body)) {
        Ok(response) => json!({ "response": response, "error": null }),
        Err((status_code, message)) => {
            json!({ "response": null, "error": { "status_code": status_code, "message": message } })
        }
    }
}

fn server_info(backend: &Backend) -> Value {
    let state = backend.with_printer(|printer| printer.klippy_state());
    let mut components = vec!["file_manager", "machine", "data_store", "update_manager", "power", "history"];
    if backend.spoolman_enabled() {
        components.push("spoolman");
    }
    if backend.wled_enabled() {
        components.push("wled");
    }
    json!({
        "klippy_connected": true,
        "klippy_state": state,
        "components": components,
        "failed_components": [],
        "registered_directories": ["gcodes", "config"],
        "warnings": [],
        "websocket_count": 1,
        "moonraker_version": "v0.9.3-fake",
        "api_version": [1, 5, 0],
        "api_version_string": "1.5.0",
    })
}

fn printer_info(backend: &Backend) -> Value {
    let (state, message) = backend.with_printer(|printer| (printer.klippy_state(), printer.state_message()));
    json!({
        "state": state,
        "state_message": message,
        "hostname": "fake-printer",
        "software_version": "v0.13.0-fake",
    })
}

/// Commands AFC registers for its lanes, its map and its errors (the `T<n>` of the lanes come from `AFC.maps`).
const AFC_COMMANDS: [&str; 11] = [
    "CHANGE_TOOL",
    "TOOL_LOAD",
    "TOOL_UNLOAD",
    "LANE_UNLOAD",
    "SET_SPOOL_ID",
    "SET_COLOR",
    "SET_MATERIAL",
    "SET_MAP",
    "RESET_AFC_MAPPING",
    "RESET_FAILURE",
    "AFC_LANE_RESET",
];
/// AFC's tool changer adds `AFC_UNSELECT_TOOL` (`AFC_Toolchanger.py` L51); with `show_macros` (the default) Klipper
/// lists it as the macro of that name.
const AFC_TOOLCHANGER_PREFIX: &str = "AFC_Toolchanger ";
const AFC_TOOLCHANGER_COMMANDS: [&str; 1] = ["AFC_UNSELECT_TOOL"];

/// Klipper's command list: a few of its own commands, every macro, and Happy Hare's tool commands (one per gate).
fn gcode_help(backend: &Backend) -> Value {
    let status = backend.with_printer(|printer| printer.status());
    let mut help = serde_json::Map::new();
    for command in ["G1", "G28", "M104", "M140", "SET_HEATER_TEMPERATURE"] {
        help.insert(command.to_owned(), json!("Klipper command"));
    }
    let macros = status.as_object().into_iter().flat_map(|objects| objects.keys());
    for name in macros.filter_map(|key| key.strip_prefix("gcode_macro ")) {
        help.insert(name.to_ascii_uppercase(), json!("G-Code macro"));
    }
    for command in backend.with_printer(|printer| printer.toolchanger_commands()) {
        help.insert(command.to_owned(), json!("Klipper command"));
    }
    let gates = status.pointer("/mmu/num_gates").and_then(Value::as_u64).unwrap_or(0);
    for tool in 0..gates {
        help.insert(format!("T{tool}"), json!("Change to tool"));
    }
    if let Some(maps) = status.pointer("/AFC/maps").and_then(Value::as_array) {
        for command in AFC_COMMANDS {
            help.insert(command.to_owned(), json!("Klipper command"));
        }
        for map in maps.iter().filter_map(Value::as_str) {
            help.insert(map.to_owned(), json!("Change to tool"));
        }
    }
    let objects = status.as_object().into_iter().flat_map(|objects| objects.keys());
    if objects.into_iter().any(|key| key.starts_with(AFC_TOOLCHANGER_PREFIX)) {
        for command in AFC_TOOLCHANGER_COMMANDS {
            help.insert(command.to_owned(), json!("G-Code macro"));
        }
    }
    Value::Object(help)
}

fn set_power(backend: &Backend, params: &Value) -> Result<Value, RpcError> {
    let device = required_str(params, "device")?;
    let action = required_str(params, "action")?;
    let printing = backend.with_printer(|printer| printer.print_state() == crate::printer::PrintState::Printing);
    let result =
        backend.with_host(|host| host.set_power(device, action, printing)).map_err(RpcError::invalid_params)?;
    if let Some(status) = backend.with_host(|host| host.device_notification(device)) {
        backend.notify("notify_power_changed", json!([status]));
    }
    Ok(result)
}

fn post_strip(backend: &Backend, params: &Value) -> Result<Value, RpcError> {
    let strip = required_str(params, "strip")?;
    let action = required_str(params, "action")?;
    let brightness = params.get("brightness").and_then(Value::as_i64).unwrap_or(-1);
    backend.with_wled(|wled| wled.post_strip(strip, action, brightness)).map_err(RpcError::invalid_params)
}

fn start_print(backend: &Backend, params: &Value) -> Result<Value, RpcError> {
    let file = find_file(backend, params)?;
    backend.with_printer(|printer| printer.start_print(&file))?;
    Ok(ok())
}

fn control_printer(
    backend: &Backend,
    action: impl FnOnce(&mut Printer) -> Result<(), CommandError>,
) -> Result<Value, RpcError> {
    backend.with_printer(action)?;
    Ok(ok())
}

fn find_file(backend: &Backend, params: &Value) -> Result<GcodeFile, RpcError> {
    let filename = required_str(params, "filename")?;
    backend
        .catalog()
        .find(filename)
        .cloned()
        .ok_or_else(|| RpcError::new(NOT_FOUND, format!("File does not exist: {filename}")))
}

/// `path` is `gcodes/<file>`, as Moonraker takes it; the file that is being printed stays.
fn delete_file(backend: &Backend, params: &Value) -> Result<Value, RpcError> {
    let path = required_str(params, "path")?;
    let relative = path
        .strip_prefix("gcodes/")
        .ok_or_else(|| RpcError::invalid_params(format!("unknown root in path '{path}'")))?;
    if backend.with_printer(|printer| printer.is_printing(relative)) {
        return Err(RpcError::new(INVALID_REQUEST, format!("File is loaded, cannot delete: {relative}")));
    }
    if !backend.catalog().remove(relative) {
        return Err(RpcError::new(NOT_FOUND, format!("File does not exist: {path}")));
    }
    backend.notify(
        "notify_filelist_changed",
        json!([{ "action": "delete_file", "item": { "root": "gcodes", "path": relative } }]),
    );
    Ok(json!({ "item": { "path": relative, "root": "gcodes" }, "action": "delete_file" }))
}

fn required_str<'a>(params: &'a Value, name: &str) -> Result<&'a str, RpcError> {
    params
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params(format!("missing parameter '{name}'")))
}

fn ok() -> Value {
    Value::String("ok".to_owned())
}

fn failure(id: Value, error: &RpcError) -> Value {
    json!({ "jsonrpc": "2.0", "error": error.to_json(), "id": id })
}

fn unauthorized() -> RpcError {
    RpcError::new(access::REFUSED, "Unauthorized")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::AccessConfig;
    use crate::machine::MachineProfile;

    fn guarded() -> Backend {
        Backend::new(1.0).with_access(Some(AccessConfig::new("test", "secret")))
    }

    fn error_code(reply: &Value) -> Option<i64> {
        reply["error"]["code"].as_i64()
    }

    #[test]
    fn without_login_only_the_access_methods_answer() {
        let backend = guarded();
        let mut session = Session::new(1);

        assert_eq!(error_code(&exchange(&mut session, &backend, call("server.info", json!({})))), Some(-32602));
        let identify = exchange(&mut session, &backend, call("server.connection.identify", json!({})));
        assert_eq!(identify["error"]["message"], "Unauthorized");
        assert_eq!(exchange(&mut session, &backend, call("access.info", json!({})))["result"]["trusted"], false);
        assert!(session.status_notification(&backend).is_none());
    }

    #[test]
    fn a_login_authorizes_the_connection_until_the_logout() {
        let backend = guarded();
        let mut session = Session::new(1);
        let wrong =
            exchange(&mut session, &backend, call("access.login", json!({"username": "test", "password": "x"})));
        assert_eq!(error_code(&wrong), Some(-32602));

        let login =
            exchange(&mut session, &backend, call("access.login", json!({"username": "test", "password": "secret"})));
        let token = login["result"]["token"].clone();
        let mut other = Session::new(2);
        let identify =
            exchange(&mut other, &backend, call("server.connection.identify", json!({"access_token": token})));

        assert!(identify["result"]["connection_id"].is_number());
        assert!(exchange(&mut session, &backend, call("server.info", json!({})))["result"].is_object());
        exchange(&mut session, &backend, call("access.logout", json!({})));
        assert_eq!(error_code(&exchange(&mut other, &backend, call("server.info", json!({})))), Some(-32602));
    }

    #[test]
    fn the_api_key_authorizes_and_a_wrong_one_is_refused() {
        let backend = guarded();
        let mut session = Session::new(1);

        let wrong = exchange(&mut session, &backend, call("server.connection.identify", json!({"api_key": "guess"})));
        let right = exchange(
            &mut session,
            &backend,
            call("server.connection.identify", json!({"api_key": AccessConfig::API_KEY})),
        );

        assert_eq!(wrong["error"]["message"], "Invalid API Key");
        assert!(right["result"].is_object());
        assert!(exchange(&mut session, &backend, call("server.info", json!({})))["result"].is_object());
    }

    fn exchange(session: &mut Session, backend: &Backend, request: Value) -> Value {
        let reply = session.handle_text(backend, &request.to_string()).expect("reply expected");
        serde_json::from_str(&reply).unwrap()
    }

    fn call(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": "7", "method": method, "params": params })
    }

    fn setup() -> (Session, Backend) {
        (Session::new(1), Backend::new(1.0))
    }

    #[test]
    fn identify_returns_connection_id_and_echoes_request_id() {
        let (mut session, backend) = setup();

        let reply = exchange(&mut session, &backend, call("server.connection.identify", json!({})));

        assert_eq!(reply["id"], "7");
        assert_eq!(reply["result"]["connection_id"], 1);
    }

    #[test]
    fn subscribe_returns_initial_status() {
        let (mut session, backend) = setup();

        let reply = exchange(
            &mut session,
            &backend,
            call("printer.objects.subscribe", json!({ "objects": { "webhooks": ["state"] } })),
        );

        assert_eq!(reply["result"]["status"], json!({ "webhooks": { "state": "ready" } }));
    }

    #[test]
    fn status_notification_reports_changes_after_gcode() {
        let (mut session, backend) = setup();
        exchange(
            &mut session,
            &backend,
            call("printer.objects.subscribe", json!({ "objects": { "extruder": ["target"] } })),
        );
        assert!(session.status_notification(&backend).is_none());

        let reply = exchange(&mut session, &backend, call("printer.gcode.script", json!({ "script": "M104 S100" })));
        let notification: Value = serde_json::from_str(&session.status_notification(&backend).unwrap()).unwrap();

        assert_eq!(reply["result"], "ok");
        assert_eq!(notification["method"], "notify_status_update");
        assert_eq!(notification["params"][0], json!({ "extruder": { "target": 100.0 } }));
    }

    #[test]
    fn a_dwell_delays_the_reply_of_its_script() {
        let (mut session, backend) = setup();

        let reply = exchange(&mut session, &backend, call("printer.gcode.script", json!({ "script": "G4 P1200" })));

        assert_eq!(reply["result"], "ok");
        assert_eq!(session.take_reply_delay(), Some(Duration::from_millis(1200)));
        exchange(&mut session, &backend, call("printer.gcode.script", json!({ "script": "M104 S100" })));
        assert_eq!(session.take_reply_delay(), None);
    }

    #[test]
    fn help_is_stored_as_one_multi_line_response() {
        let (mut session, backend) = setup();

        let reply = exchange(&mut session, &backend, call("printer.gcode.script", json!({ "script": "HELP" })));
        let store = backend.with_host(|host| host.gcode_store(Some(1)));
        let entry = &store["gcode_store"][0];

        assert_eq!(reply["result"], "ok");
        assert_eq!(entry["type"], "response");
        assert_eq!(entry["message"].as_str().unwrap().lines().count(), 9);
        assert!(entry["message"].as_str().unwrap().starts_with("Available commands:\nG0, G1"));
    }

    #[test]
    fn toolchanger_profile_lists_its_commands_in_gcode_help() {
        let backend = Backend::with_profile(1.0, MachineProfile::toolchanger());
        let mut session = Session::new(1);

        let reply = exchange(&mut session, &backend, call("printer.gcode.help", json!({})));

        for command in ["T0", "T1", "T2", "SELECT_TOOL", "UNSELECT_TOOL", "SET_GCODE_VARIABLE", "SAVE_VARIABLE"] {
            assert!(reply["result"].get(command).is_some(), "missing {command}");
        }
    }

    #[test]
    fn toolchanger_profiles_list_initialize_and_unselect_by_require_tool_present() {
        for (profile, has_unselect) in
            [(MachineProfile::toolchanger(), true), (MachineProfile::toolchanger_required(), false)]
        {
            let backend = Backend::with_profile(1.0, profile);
            let mut session = Session::new(1);

            let reply = exchange(&mut session, &backend, call("printer.gcode.help", json!({})));

            assert!(reply["result"].get("INITIALIZE_TOOLCHANGER").is_some());
            assert!(reply["result"].get("SELECT_TOOL").is_some());
            assert_eq!(reply["result"].get("UNSELECT_TOOL").is_some(), has_unselect);
        }
    }

    #[test]
    fn afc_profiles_list_their_commands_in_gcode_help() {
        for (profile, has_unselect) in [(MachineProfile::afc(), false), (MachineProfile::afc_toolchanger(), true)] {
            let backend = Backend::with_profile(1.0, profile);
            let mut session = Session::new(1);

            let reply = exchange(&mut session, &backend, call("printer.gcode.help", json!({})));

            for command in [
                "T0",
                "T3",
                "CHANGE_TOOL",
                "TOOL_LOAD",
                "SET_SPOOL_ID",
                "SET_COLOR",
                "SET_MAP",
                "LANE_UNLOAD",
                "RESET_AFC_MAPPING",
                "RESET_FAILURE",
                "AFC_LANE_RESET",
            ] {
                assert!(reply["result"].get(command).is_some(), "missing {command}");
            }
            assert!(reply["result"].get("T4").is_none());
            assert!(reply["result"].get("SELECT_TOOL").is_none());
            assert_eq!(reply["result"].get("AFC_UNSELECT_TOOL").is_some(), has_unselect);
        }
    }

    #[test]
    fn happy_hare_units_list_a_tool_command_per_gate_in_gcode_help() {
        let backend = Backend::with_profile(1.0, MachineProfile::happy_hare_units());
        let mut session = Session::new(1);

        let reply = exchange(&mut session, &backend, call("printer.gcode.help", json!({})));

        assert!(reply["result"].get("T7").is_some());
        assert!(reply["result"].get("T8").is_none());
    }

    #[test]
    fn an_afc_tool_change_sets_the_active_spool_of_moonraker_and_unloading_clears_it() {
        let backend = Backend::with_profile(1.0, MachineProfile::afc());
        let mut session = Session::new(1);
        let script = |text: &str| call("printer.gcode.script", json!({ "script": text }));

        let reply = exchange(&mut session, &backend, script("G28\nT2"));
        assert_eq!(reply["result"], "ok");
        let status = exchange(&mut session, &backend, call("server.spoolman.status", json!({})));
        assert_eq!(status["result"]["spool_id"], 3);

        exchange(&mut session, &backend, script("TOOL_UNLOAD"));
        let status = exchange(&mut session, &backend, call("server.spoolman.status", json!({})));
        assert_eq!(status["result"]["spool_id"], Value::Null);
        let queried =
            exchange(&mut session, &backend, call("printer.objects.query", json!({ "objects": { "AFC": null } })));
        assert_eq!(queried["result"]["status"]["AFC"]["current_load"], Value::Null);
    }

    #[test]
    fn afc_state_changes_reach_subscribers() {
        let backend = Backend::with_profile(1.0, MachineProfile::afc_toolchanger());
        let mut session = Session::new(1);
        let objects = json!({ "objects": { "AFC": ["current_load"], "toolhead": ["extruder"] } });

        let reply = exchange(&mut session, &backend, call("printer.objects.subscribe", objects));
        assert_eq!(reply["result"]["status"]["AFC"]["current_load"], "e1");
        assert_eq!(reply["result"]["status"]["toolhead"]["extruder"], "extruder1");

        exchange(&mut session, &backend, call("printer.gcode.script", json!({ "script": "G28\nT0" })));
        let update: Value = serde_json::from_str(&session.status_notification(&backend).unwrap()).unwrap();
        assert_eq!(update["method"], "notify_status_update");
        assert_eq!(update["params"][0]["AFC"]["current_load"], "e0");
        assert_eq!(update["params"][0]["toolhead"]["extruder"], "extruder");
    }

    #[test]
    fn gcode_errors_are_reported_as_rpc_errors() {
        let (mut session, backend) = setup();

        let reply = exchange(&mut session, &backend, call("printer.gcode.script", json!({ "script": "G0 X10" })));

        assert_eq!(reply["error"]["code"], 400);
        assert_eq!(reply["error"]["message"], "Must home axis first");
    }

    #[test]
    fn unknown_method_is_reported() {
        let (mut session, backend) = setup();

        let reply = exchange(&mut session, &backend, call("printer.unknown", json!({})));

        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn starting_a_missing_file_fails_with_not_found() {
        let (mut session, backend) = setup();

        let reply = exchange(&mut session, &backend, call("printer.print.start", json!({ "filename": "nope.gcode" })));

        assert_eq!(reply["error"]["code"], 404);
    }

    #[test]
    fn notifications_without_id_get_no_reply() {
        let (mut session, backend) = setup();
        let notification =
            json!({ "jsonrpc": "2.0", "method": "printer.gcode.script", "params": { "script": "M104 S50" } });

        assert!(session.handle_text(&backend, &notification.to_string()).is_none());
        assert_eq!(backend.status()["extruder"]["target"], 50.0);
    }

    #[test]
    fn batch_requests_get_batch_responses() {
        let (mut session, backend) = setup();
        let batch = json!([call("server.connection.identify", json!({})), call("server.files.list", json!({})),]);

        let reply: Value = serde_json::from_str(&session.handle_text(&backend, &batch.to_string()).unwrap()).unwrap();

        assert_eq!(reply.as_array().unwrap().len(), 2);
    }

    #[test]
    fn invalid_json_yields_parse_error() {
        let (mut session, backend) = setup();

        let reply: Value = serde_json::from_str(&session.handle_text(&backend, "{").unwrap()).unwrap();

        assert_eq!(reply["error"]["code"], -32700);
        assert_eq!(reply["id"], Value::Null);
    }

    #[test]
    fn objects_list_names_the_configured_modules() {
        let mut session = Session::new(1);
        let backend = Backend::with_profile(1.0, MachineProfile::happy_hare(4));

        let reply = exchange(&mut session, &backend, call("printer.objects.list", json!({})));

        let names: Vec<&str> =
            reply["result"]["objects"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(names.contains(&"mmu"));
        assert!(names.contains(&"quad_gantry_level"));
    }

    #[test]
    fn objects_query_returns_selected_fields_without_subscribing() {
        let (mut session, backend) = setup();

        let reply = exchange(
            &mut session,
            &backend,
            call("printer.objects.query", json!({ "objects": { "configfile": ["save_config_pending"] } })),
        );

        assert_eq!(reply["result"]["status"], json!({ "configfile": { "save_config_pending": false } }));
        assert!(session.status_notification(&backend).is_none());
    }
}
