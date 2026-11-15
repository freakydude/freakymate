//! End-to-end tests: real WebSocket client against the running Fake-Moonraker.

use fake_moonraker::machine::MachineProfile;
use fake_moonraker::server::{ServerConfig, serve};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const WAIT_LIMIT: Duration = Duration::from_secs(10);

async fn start_server() -> SocketAddr {
    start_server_with(MachineProfile::default()).await
}

async fn start_server_with(profile: MachineProfile) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = ServerConfig {
        time_scale: 1000.0,
        status_interval: Duration::from_millis(20),
        profile,
        ..ServerConfig::default()
    };
    tokio::spawn(serve(listener, config));
    address
}

struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl Client {
    async fn connect(address: SocketAddr) -> Self {
        let (socket, _) = connect_async(format!("ws://{address}/websocket")).await.unwrap();
        Self { socket, next_id: 1 }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id.to_string();
        self.next_id += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.socket.send(Message::text(request.to_string())).await.unwrap();

        loop {
            let message = self.next_message().await;
            if message["id"] == id {
                return message;
            }
        }
    }

    async fn next_message(&mut self) -> Value {
        loop {
            let frame = timeout(WAIT_LIMIT, self.socket.next())
                .await
                .expect("timed out waiting for message")
                .expect("connection closed")
                .unwrap();
            if let Message::Text(text) = frame {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    async fn wait_for_status_update(&mut self, accept: impl Fn(&Value) -> bool) -> Value {
        loop {
            let message = self.next_message().await;
            if message["method"] == "notify_status_update" && accept(&message["params"][0]) {
                return message["params"][0].clone();
            }
        }
    }
}

async fn http_get(address: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

#[tokio::test]
async fn subscribe_delivers_initial_status() {
    let mut client = Client::connect(start_server().await).await;

    let identify = client.request("server.connection.identify", json!({ "client_name": "test" })).await;
    let subscribe = client
        .request("printer.objects.subscribe", json!({ "objects": { "webhooks": null, "extruder": ["target"] } }))
        .await;

    assert!(identify["result"]["connection_id"].is_u64());
    assert_eq!(subscribe["result"]["status"]["webhooks"]["state"], "ready");
    assert_eq!(subscribe["result"]["status"]["extruder"], json!({ "target": 0.0 }));
}

#[tokio::test]
async fn heating_is_pushed_as_status_updates() {
    let mut client = Client::connect(start_server().await).await;
    client.request("printer.objects.subscribe", json!({ "objects": { "extruder": ["temperature", "target"] } })).await;

    let script = client.request("printer.gcode.script", json!({ "script": "M104 S50" })).await;
    let update = client.wait_for_status_update(|status| status["extruder"]["temperature"] == 50.0).await;

    assert_eq!(script["result"], "ok");
    assert_eq!(update["extruder"]["temperature"], 50.0);
}

#[tokio::test]
async fn print_job_progresses_to_complete() {
    let mut client = Client::connect(start_server().await).await;
    client.request("printer.objects.subscribe", json!({ "objects": { "print_stats": ["state"] } })).await;

    let start = client.request("printer.print.start", json!({ "filename": "calibration_cube.gcode" })).await;
    let update = client.wait_for_status_update(|status| status["print_stats"]["state"] == "complete").await;

    assert_eq!(start["result"], "ok");
    assert_eq!(update["print_stats"]["state"], "complete");
}

#[tokio::test]
async fn failing_gcode_returns_error_response() {
    let mut client = Client::connect(start_server().await).await;

    let reply = client.request("printer.gcode.script", json!({ "script": "G0 X10" })).await;

    assert_eq!(reply["error"]["code"], 400);
    assert_eq!(reply["error"]["message"], "Must home axis first");
}

#[tokio::test]
async fn file_list_and_metadata_are_available() {
    let mut client = Client::connect(start_server().await).await;

    let list = client.request("server.files.list", json!({})).await;
    let metadata = client.request("server.files.metadata", json!({ "filename": "benchy.gcode" })).await;

    assert_eq!(list["result"].as_array().unwrap().len(), 3);
    assert_eq!(metadata["result"]["layer_count"], 250);
}

#[tokio::test]
async fn thumbnail_from_metadata_is_served_over_http() {
    let address = start_server().await;
    let mut client = Client::connect(address).await;
    let metadata = client.request("server.files.metadata", json!({ "filename": "benchy.gcode" })).await;
    let relative_path = metadata["result"]["thumbnails"][0]["relative_path"].as_str().unwrap();

    let found = http_get(address, &format!("/server/files/gcodes/{relative_path}")).await;
    let missing = http_get(address, "/server/files/gcodes/.thumbs/missing.png").await;

    assert!(found.starts_with("HTTP/1.1 200"), "{found}");
    assert!(found.contains("image/png"));
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
}

#[tokio::test]
async fn happy_hare_profile_exposes_mmu_and_accepts_tool_changes() {
    let mut client = Client::connect(start_server_with(MachineProfile::happy_hare(4)).await).await;

    let list = client.request("printer.objects.list", json!({})).await;
    client.request("printer.objects.subscribe", json!({ "objects": { "mmu": ["tool", "filament"] } })).await;
    client.request("printer.gcode.script", json!({ "script": "MMU_HOME\nT2" })).await;
    let update = client.wait_for_status_update(|status| status["mmu"]["tool"] == 2).await;

    assert!(list["result"]["objects"].as_array().unwrap().contains(&json!("mmu")));
    assert_eq!(update["mmu"]["filament"], "Loaded");
}

#[tokio::test]
async fn basic_profile_hides_optional_modules() {
    let mut client = Client::connect(start_server_with(MachineProfile::basic()).await).await;

    let list = client.request("printer.objects.list", json!({})).await;
    let tool_change = client.request("printer.gcode.script", json!({ "script": "T1" })).await;

    let objects = list["result"]["objects"].as_array().unwrap();
    assert!(!objects.contains(&json!("mmu")));
    assert!(!objects.contains(&json!("quad_gantry_level")));
    assert_eq!(tool_change["error"]["code"], 400);
}
