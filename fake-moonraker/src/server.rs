//! HTTP and WebSocket server that behaves like Moonraker.

use crate::access::AccessConfig;
use crate::backend::Backend;
use crate::machine::MachineProfile;
use crate::session::Session;
use crate::tls::{TlsIdentity, TlsListener};
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;

const SIMULATION_STEP: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Speeds up heating and print progress; 1.0 means real time.
    pub time_scale: f64,
    pub status_interval: Duration,
    pub profile: MachineProfile,
    /// Wants a login like a Moonraker that does not trust the client; `None` trusts everyone.
    pub access: Option<AccessConfig>,
    /// Serves HTTPS and WSS with this certificate instead of plain HTTP.
    pub tls: Option<TlsIdentity>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            time_scale: 1.0,
            status_interval: Duration::from_millis(250),
            profile: MachineProfile::default(),
            access: None,
            tls: None,
        }
    }
}

#[derive(Clone)]
struct AppState {
    backend: Arc<Backend>,
    status_interval: Duration,
    next_connection_id: Arc<AtomicU64>,
}

pub async fn serve(listener: TcpListener, config: ServerConfig) -> io::Result<()> {
    let backend = Arc::new(Backend::with_profile(config.time_scale, config.profile).with_access(config.access));
    let simulation = tokio::spawn(simulate(Arc::clone(&backend)));
    let state =
        AppState { backend, status_interval: config.status_interval, next_connection_id: Arc::new(AtomicU64::new(1)) };

    let result = match &config.tls {
        Some(identity) => axum::serve(TlsListener::new(listener, identity)?, router(state)).await,
        None => axum::serve(listener, router(state)).await,
    };
    simulation.abort();
    result
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/websocket", get(websocket))
        .route("/server/files/gcodes/{*path}", get(thumbnail))
        .with_state(state)
}

async fn simulate(backend: Arc<Backend>) {
    let mut ticker = tokio::time::interval(SIMULATION_STEP);
    let mut last_tick = Instant::now();
    loop {
        ticker.tick().await;
        let now = Instant::now();
        backend.advance_simulation(now - last_tick);
        last_tick = now;
    }
}

async fn websocket(upgrade: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    let connection_id = state.next_connection_id.fetch_add(1, Ordering::Relaxed);
    upgrade.on_upgrade(move |socket| run_session(socket, state, connection_id))
}

async fn run_session(mut socket: WebSocket, state: AppState, connection_id: u64) {
    let mut session = Session::new(connection_id);
    let mut status_ticker = tokio::time::interval(state.status_interval);
    let mut notifications = state.backend.notifications();
    // Replies that wait for a dwell; the session keeps answering other requests meanwhile.
    let (delayed_tx, mut delayed) = tokio::sync::mpsc::unbounded_channel::<String>();

    loop {
        let outgoing = tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let reply = session.handle_text(&state.backend, text.as_str());
                    match (reply, session.take_reply_delay()) {
                        (Some(reply), Some(delay)) => {
                            let delayed_tx = delayed_tx.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(delay).await;
                                let _ = delayed_tx.send(reply);
                            });
                            None
                        }
                        (reply, _) => reply,
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => None,
            },
            Some(reply) = delayed.recv() => Some(reply),
            _ = status_ticker.tick() => session.status_notification(&state.backend),
            notification = notifications.recv() => match notification {
                Ok(message) => session.authorized(&state.backend).then_some(message),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
        };

        if let Some(message) = outgoing
            && socket.send(Message::Text(message.into())).await.is_err()
        {
            break;
        }
    }
}

/// A fake that wants a login serves files only for a one-time token (`?token=`) or the API key (`X-Api-Key`).
async fn thumbnail(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let api_key = headers.get("X-Api-Key").and_then(|value| value.to_str().ok());
    if !state.backend.access().allows_download(query.get("token").map(String::as_str), api_key) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.backend.catalog().thumbnail(&path) {
        Some(image) => ([(header::CONTENT_TYPE, image.content_type())], image.into_bytes()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
