//! Optional QSONaut Server connection over proxy-friendly WebSockets.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{
            header::{AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL},
            HeaderValue,
        },
        Message,
    },
};
use url::Url;
use uuid::Uuid;

const PROTOCOL_VERSION: &str = "v1";

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub server_url: String,
    pub device_token: String,
    pub client_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    pub instance_id: String,
    pub station_label: String,
    pub radio_manufacturer: Option<String>,
    pub radio_model: Option<String>,
    pub frequency_hz: Option<i64>,
    pub band: Option<String>,
    pub mode: Option<String>,
    pub qsonaut_version: String,
    pub platform: String,
    pub status: String,
    pub metadata: Value,
}

#[must_use]
pub fn new_instance_id() -> String {
    Uuid::new_v4().to_string()
}

#[must_use]
pub fn log_idempotency_key(local_record_id: u64) -> String {
    Uuid::from_u128(0x7173_6f6e_6175_7400_0000_0000_0000_0000 | u128::from(local_record_id))
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionState {
    #[default]
    Disabled,
    Connecting,
    Connected,
    Reconnecting,
    Stopped,
}

#[derive(Debug, Clone, Default)]
pub struct ConnectionStatus {
    pub state: ConnectionState,
    pub operator_callsign: Option<String>,
    pub active_event_count: usize,
    pub catalog_size: usize,
    pub last_error: Option<String>,
}

#[derive(Debug)]
enum Command {
    Presence(Box<Presence>),
    Log(Value),
    Sync,
    Shutdown,
}

pub struct ServerClient {
    commands: mpsc::UnboundedSender<Command>,
    status: Arc<Mutex<ConnectionStatus>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ServerClient {
    #[must_use]
    pub fn spawn(config: ConnectionConfig) -> Self {
        let (commands, receiver) = mpsc::unbounded_channel();
        let status = Arc::new(Mutex::new(ConnectionStatus {
            state: ConnectionState::Connecting,
            ..ConnectionStatus::default()
        }));
        let worker_status = Arc::clone(&status);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => runtime.block_on(run(config, receiver, worker_status, worker_stop)),
                Err(error) => {
                    set_error(&worker_status, ConnectionState::Stopped, error.to_string())
                }
            }
        });
        Self {
            commands,
            status,
            stop,
            worker: Some(worker),
        }
    }

    pub fn publish_presence(&self, presence: Presence) {
        let _ = self.commands.send(Command::Presence(Box::new(presence)));
    }

    pub fn publish_log(&self, log: Value) {
        let _ = self.commands.send(Command::Log(log));
    }

    pub fn request_sync(&self) {
        let _ = self.commands.send(Command::Sync);
    }

    #[must_use]
    pub fn status(&self) -> ConnectionStatus {
        self.status
            .lock()
            .expect("server status lock poisoned")
            .clone()
    }
}

impl Drop for ServerClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

async fn run(
    config: ConnectionConfig,
    mut commands: mpsc::UnboundedReceiver<Command>,
    status: Arc<Mutex<ConnectionStatus>>,
    stop: Arc<AtomicBool>,
) {
    let mut delay = Duration::from_secs(1);
    loop {
        if stop.load(Ordering::Acquire) {
            set_state(&status, ConnectionState::Stopped);
            return;
        }
        set_state(&status, ConnectionState::Connecting);
        match connect(&config, &mut commands, &status).await {
            Ok(ConnectionEnd::Shutdown) => {
                set_state(&status, ConnectionState::Stopped);
                return;
            }
            Ok(ConnectionEnd::Disconnected) => {
                set_error(
                    &status,
                    ConnectionState::Reconnecting,
                    "server disconnected".to_owned(),
                );
            }
            Err(error) => set_error(&status, ConnectionState::Reconnecting, error.to_string()),
        }
        let deadline = tokio::time::Instant::now() + delay;
        while tokio::time::Instant::now() < deadline {
            if stop.load(Ordering::Acquire) {
                set_state(&status, ConnectionState::Stopped);
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

enum ConnectionEnd {
    Disconnected,
    Shutdown,
}

async fn connect(
    config: &ConnectionConfig,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    status: &Arc<Mutex<ConnectionStatus>>,
) -> Result<ConnectionEnd> {
    let socket_url = websocket_url(&config.server_url)?;
    let mut request = socket_url
        .as_str()
        .into_client_request()
        .context("invalid WebSocket request")?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", config.device_token))
            .context("invalid device token")?,
    );
    request.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("qsonaut.v1"),
    );
    let (socket, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(request))
        .await
        .context("QSONaut Server connection timed out")?
        .context("could not connect to QSONaut Server")?;
    let (mut writer, mut reader) = socket.split();
    set_state(status, ConnectionState::Connected);
    clear_error(status);
    send(
        &mut writer,
        ClientMessage::Hello {
            client_version: config.client_version.clone(),
        },
    )
    .await?;
    send(&mut writer, ClientMessage::Sync).await?;

    let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
    let mut last_presence: Option<Presence> = None;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => send(&mut writer, ClientMessage::Ping).await?,
            command = commands.recv() => match command {
                Some(Command::Presence(presence)) => {
                    last_presence = Some((*presence).clone());
                    send(&mut writer, ClientMessage::Presence(presence)).await?;
                }
                Some(Command::Log(log)) => send(&mut writer, ClientMessage::Log(log)).await?,
                Some(Command::Sync) => send(&mut writer, ClientMessage::Sync).await?,
                Some(Command::Shutdown) | None => {
                    if let Some(mut presence) = last_presence {
                        presence.status = "offline".to_owned();
                        let _ = send(&mut writer, ClientMessage::Presence(Box::new(presence))).await;
                    }
                    let _ = writer.send(Message::Close(None)).await;
                    return Ok(ConnectionEnd::Shutdown);
                }
            },
            message = reader.next() => match message {
                Some(Ok(Message::Text(text))) => receive(&text, status)?,
                Some(Ok(Message::Close(_))) | None => return Ok(ConnectionEnd::Disconnected),
                Some(Err(error)) => return Err(error).context("WebSocket receive failed"),
                _ => {}
            }
        }
    }
}

fn websocket_url(server_url: &str) -> Result<Url> {
    let mut url = Url::parse(server_url.trim()).context("server URL is invalid")?;
    let scheme = match url.scheme() {
        "https" | "wss" => "wss",
        "http" | "ws" => "ws",
        _ => return Err(anyhow!("server URL must use https, http, wss, or ws")),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow!("server URL scheme cannot be changed"))?;
    url.set_path("/api/v1/ws");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

async fn send<S>(writer: &mut S, message: ClientMessage) -> Result<()>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let envelope = ClientEnvelope {
        protocol_version: PROTOCOL_VERSION.to_owned(),
        event_id: Uuid::new_v4(),
        message,
    };
    let text = serde_json::to_string(&envelope)?;
    writer.send(Message::Text(text.into())).await?;
    Ok(())
}

fn receive(text: &str, status: &Arc<Mutex<ConnectionStatus>>) -> Result<()> {
    let envelope: ServerEnvelope = serde_json::from_str(text).context("invalid server message")?;
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(anyhow!("unsupported server protocol version"));
    }
    let mut current = status.lock().expect("server status lock poisoned");
    match envelope.message {
        ServerMessage::Welcome { user } => current.operator_callsign = Some(user.callsign),
        ServerMessage::Snapshot {
            events,
            contest_templates,
        } => {
            current.active_event_count = events
                .iter()
                .filter(|event| event.status == "active")
                .count();
            current.catalog_size = contest_templates.len();
        }
        ServerMessage::Error { message } => current.last_error = Some(message),
        ServerMessage::PresenceAccepted(value) | ServerMessage::LogAccepted(value) => {
            drop(value);
        }
        ServerMessage::Ack | ServerMessage::Pong => {}
    }
    Ok(())
}

fn set_state(status: &Arc<Mutex<ConnectionStatus>>, state: ConnectionState) {
    status.lock().expect("server status lock poisoned").state = state;
}

fn clear_error(status: &Arc<Mutex<ConnectionStatus>>) {
    status
        .lock()
        .expect("server status lock poisoned")
        .last_error = None;
}

fn set_error(status: &Arc<Mutex<ConnectionStatus>>, state: ConnectionState, error: String) {
    let mut current = status.lock().expect("server status lock poisoned");
    current.state = state;
    current.last_error = Some(error);
}

#[derive(Debug, Serialize)]
struct ClientEnvelope {
    protocol_version: String,
    event_id: Uuid,
    #[serde(flatten)]
    message: ClientMessage,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum ClientMessage {
    Hello { client_version: String },
    Sync,
    Presence(Box<Presence>),
    Log(Value),
    Ping,
}

#[derive(Debug, Deserialize)]
struct ServerEnvelope {
    protocol_version: String,
    #[allow(dead_code)]
    event_id: Uuid,
    #[serde(flatten)]
    message: ServerMessage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum ServerMessage {
    Welcome {
        user: User,
    },
    Snapshot {
        events: Vec<Event>,
        contest_templates: Vec<Value>,
    },
    PresenceAccepted(Value),
    LogAccepted(Value),
    Ack,
    Pong,
    Error {
        message: String,
    },
}

#[derive(Debug, Deserialize)]
struct User {
    callsign: String,
}

#[derive(Debug, Deserialize)]
struct Event {
    status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_url_uses_same_origin_wss_path() {
        assert_eq!(
            websocket_url("https://radio.example.net/base?old=true")
                .unwrap()
                .as_str(),
            "wss://radio.example.net/api/v1/ws"
        );
        assert_eq!(
            websocket_url("http://127.0.0.1:8080").unwrap().as_str(),
            "ws://127.0.0.1:8080/api/v1/ws"
        );
    }

    #[test]
    fn messages_use_versioned_stable_envelopes() {
        let envelope = ClientEnvelope {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            event_id: Uuid::nil(),
            message: ClientMessage::Sync,
        };
        let json = serde_json::to_value(envelope).unwrap();
        assert_eq!(json["protocol_version"], "v1");
        assert_eq!(json["type"], "sync");
    }
}
