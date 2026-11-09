//! WebSocket connection to Moonraker: matches responses to their requests and forwards notifications.

use super::rpc::{self, Incoming, RpcError};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::{self, Message};

type Replies = HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>;
/// Open requests; `None` once the connection has ended.
type Pending = Arc<Mutex<Option<Replies>>>;

/// A rebooting host neither answers nor closes the socket; pings reveal that.
const KEEPALIVE: Keepalive = Keepalive {
    connect_timeout: Duration::from_secs(10),
    ping_interval: Duration::from_secs(5),
    idle_timeout: Duration::from_secs(15),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionState {
    #[default]
    Connecting,
    Connected,
    Disconnected,
    /// Connected, but Moonraker answers only after a login ([`PrinterService::log_in`]).
    LoginRequired,
}

#[derive(Debug, Clone, Copy)]
struct Keepalive {
    connect_timeout: Duration,
    ping_interval: Duration,
    /// Without any message (pongs included) for this long the connection counts as lost.
    idle_timeout: Duration,
}

#[derive(Debug, Error)]
pub enum CallError {
    #[error(transparent)]
    Rpc(#[from] RpcError),
    #[error("connection to Moonraker closed")]
    Closed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

/// A request that has been sent; its reply is awaited with [`PendingCall::response`].
pub struct PendingCall(oneshot::Receiver<Result<Value, RpcError>>);

impl PendingCall {
    pub async fn response(self) -> Result<Value, CallError> {
        self.0.await.map_err(|_| CallError::Closed)?.map_err(CallError::from)
    }
}

/// Handle to an open connection; cheap to clone.
#[derive(Clone)]
pub struct Connection {
    outgoing: mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}

impl Connection {
    /// Opens the connection; notifications arrive via the returned receiver,
    /// which ends as soon as the connection closes.
    pub async fn open(url: &str) -> Result<(Self, mpsc::UnboundedReceiver<Notification>), tungstenite::Error> {
        Self::open_with(url, KEEPALIVE).await
    }

    async fn open_with(
        url: &str,
        keepalive: Keepalive,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Notification>), tungstenite::Error> {
        let (socket, _) = tokio::time::timeout(keepalive.connect_timeout, tokio_tungstenite::connect_async(url))
            .await
            .map_err(|_| tungstenite::Error::Io(std::io::ErrorKind::TimedOut.into()))??;
        let (mut sink, mut stream) = socket.split();
        let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<String>();
        let (notifications, notifications_rx) = mpsc::unbounded_channel();
        let pending: Pending = Arc::new(Mutex::new(Some(Replies::new())));

        tokio::spawn(async move {
            let mut ping = tokio::time::interval(keepalive.ping_interval);
            loop {
                let message = tokio::select! {
                    text = outgoing_rx.recv() => match text {
                        Some(text) => Message::text(text),
                        None => break,
                    },
                    _ = ping.tick() => Message::Ping(Vec::new().into()),
                };
                if sink.send(message).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let reader_pending = Arc::clone(&pending);
        tokio::spawn(async move {
            while let Ok(Some(Ok(message))) = tokio::time::timeout(keepalive.idle_timeout, stream.next()).await {
                match message {
                    Message::Text(text) => dispatch(text.as_str(), &reader_pending, &notifications),
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            lock(&reader_pending).take();
        });

        let connection = Self { outgoing, pending, next_id: Arc::new(AtomicU64::new(1)) };
        Ok((connection, notifications_rx))
    }

    pub async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, CallError> {
        self.start_call(method, params)?.response().await
    }

    /// Puts the request on the wire right away; only the reply is awaited. Requests started one after another
    /// therefore reach Moonraker in that order, which matters for absolute values (heater targets) and for moves.
    pub fn start_call(&self, method: &str, params: Option<Value>) -> Result<PendingCall, CallError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, response) = oneshot::channel();
        lock(&self.pending).as_mut().ok_or(CallError::Closed)?.insert(id, reply);
        if self.outgoing.send(rpc::encode_request(id, method, params)).is_err() {
            if let Some(replies) = lock(&self.pending).as_mut() {
                replies.remove(&id);
            }
            return Err(CallError::Closed);
        }
        Ok(PendingCall(response))
    }

    pub fn is_closed(&self) -> bool {
        self.outgoing.is_closed() || lock(&self.pending).is_none()
    }
}

fn dispatch(text: &str, pending: &Pending, notifications: &mpsc::UnboundedSender<Notification>) {
    match rpc::decode(text) {
        Some(Incoming::Response { id, result }) => {
            if let Some(reply) = lock(pending).as_mut().and_then(|replies| replies.remove(&id)) {
                let _ = reply.send(result);
            }
        }
        Some(Incoming::Notification { method, params }) => {
            let _ = notifications.send(Notification { method, params });
        }
        None => log::debug!("ignoring unreadable message from Moonraker"),
    }
}

fn lock(pending: &Pending) -> std::sync::MutexGuard<'_, Option<Replies>> {
    pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    const SHORT: Keepalive = Keepalive {
        connect_timeout: Duration::from_secs(2),
        ping_interval: Duration::from_millis(50),
        idle_timeout: Duration::from_millis(300),
    };

    #[tokio::test]
    async fn detects_a_server_that_stops_answering() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/websocket", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            // Accept the handshake, then never read again: no pongs, no close, like a host that vanished.
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(socket);
        });

        let (connection, mut notifications) = Connection::open_with(&url, SHORT).await.unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(2), notifications.recv()).await;

        assert_eq!(ended.expect("connection loss not detected"), None);
        assert!(matches!(connection.call("server.info", None).await, Err(CallError::Closed)));
        server.abort();
    }

    #[tokio::test]
    async fn pongs_keep_a_quiet_connection_open() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/websocket", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            // Reading answers pings; the server itself never sends anything.
            while socket.next().await.is_some() {}
        });

        let (_connection, mut notifications) = Connection::open_with(&url, SHORT).await.unwrap();
        tokio::time::sleep(SHORT.idle_timeout * 3).await;

        assert_eq!(notifications.try_recv(), Err(mpsc::error::TryRecvError::Empty));
        server.abort();
    }
}
