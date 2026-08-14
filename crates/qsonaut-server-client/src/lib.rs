//! Optional QSONaut Server connection over proxy-friendly WebSockets.

use std::{
    collections::{BTreeMap, VecDeque},
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAutomationEvent {
    pub kind: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug)]
enum Command {
    Presence(Box<Presence>),
    Log(Value),
    Sync,
    ChannelMessage { channel: String, message: String },
    Shutdown,
}

pub struct ServerClient {
    commands: mpsc::UnboundedSender<Command>,
    status: Arc<Mutex<ConnectionStatus>>,
    automation_events: Arc<Mutex<VecDeque<ServerAutomationEvent>>>,
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
        let automation_events = Arc::new(Mutex::new(VecDeque::with_capacity(256)));
        let worker_events = Arc::clone(&automation_events);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => runtime.block_on(run(
                    config,
                    receiver,
                    worker_status,
                    worker_events,
                    worker_stop,
                )),
                Err(error) => {
                    set_error(&worker_status, ConnectionState::Stopped, error.to_string())
                }
            }
        });
        Self {
            commands,
            status,
            automation_events,
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

    pub fn publish_channel_message(&self, channel: impl Into<String>, message: impl Into<String>) {
        let _ = self.commands.send(Command::ChannelMessage {
            channel: channel.into(),
            message: message.into(),
        });
    }

    #[must_use]
    pub fn drain_automation_events(&self) -> Vec<ServerAutomationEvent> {
        self.automation_events
            .lock()
            .expect("server automation event lock poisoned")
            .drain(..)
            .collect()
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
    automation_events: Arc<Mutex<VecDeque<ServerAutomationEvent>>>,
    stop: Arc<AtomicBool>,
) {
    let mut delay = Duration::from_secs(1);
    loop {
        if stop.load(Ordering::Acquire) {
            set_state(&status, ConnectionState::Stopped);
            return;
        }
        set_state(&status, ConnectionState::Connecting);
        match connect(&config, &mut commands, &status, &automation_events).await {
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
    automation_events: &Arc<Mutex<VecDeque<ServerAutomationEvent>>>,
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
                Some(Command::ChannelMessage { channel, message }) => {
                    send(
                        &mut writer,
                        ClientMessage::ChannelMessage(ChannelMessageInput {
                            event_id: None,
                            channel,
                            message,
                            metadata: serde_json::json!({ "source": "qsonaut-automation" }),
                        }),
                    ).await?;
                }
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
                Some(Ok(Message::Text(text))) => receive(&text, status, automation_events)?,
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

fn receive(
    text: &str,
    status: &Arc<Mutex<ConnectionStatus>>,
    automation_events: &Arc<Mutex<VecDeque<ServerAutomationEvent>>>,
) -> Result<()> {
    let envelope: ServerEnvelope = serde_json::from_str(text).context("invalid server message")?;
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(anyhow!("unsupported server protocol version"));
    }
    match envelope.message {
        ServerMessage::Welcome { user } => {
            status
                .lock()
                .expect("server status lock poisoned")
                .operator_callsign = Some(user.callsign.clone());
            push_automation_event(
                automation_events,
                "connected",
                [("callsign", user.callsign)],
            );
        }
        ServerMessage::Snapshot {
            events,
            contest_templates,
            channel_messages,
        } => {
            let active_event_count = events
                .iter()
                .filter(|event| event.status == "active")
                .count();
            let catalog_size = contest_templates.len();
            let message_count = channel_messages.len();
            let mut current = status.lock().expect("server status lock poisoned");
            current.active_event_count = active_event_count;
            current.catalog_size = catalog_size;
            drop(current);
            push_automation_event(
                automation_events,
                "snapshot",
                [
                    ("active_events", active_event_count.to_string()),
                    ("catalog_size", catalog_size.to_string()),
                    ("message_count", message_count.to_string()),
                ],
            );
            for message in channel_messages {
                push_channel_event(automation_events, "channel_history", message);
            }
        }
        ServerMessage::Error { message } => {
            status
                .lock()
                .expect("server status lock poisoned")
                .last_error = Some(message.clone());
            push_automation_event(automation_events, "error", [("message", message)]);
        }
        ServerMessage::ChannelMessagePublished(message) => {
            push_channel_event(automation_events, "channel_message", message);
        }
        ServerMessage::ChannelMessageAccepted(message) => {
            push_channel_event(automation_events, "message_accepted", message);
        }
        ServerMessage::PresenceAccepted(value) | ServerMessage::LogAccepted(value) => {
            drop(value);
        }
        ServerMessage::Ack | ServerMessage::Pong => {}
    }
    Ok(())
}

fn push_channel_event(
    events: &Arc<Mutex<VecDeque<ServerAutomationEvent>>>,
    kind: &str,
    message: ChannelMessage,
) {
    push_automation_event(
        events,
        kind,
        [
            ("id", message.id),
            ("author", message.author_callsign),
            ("channel", message.channel),
            ("message", message.message),
            ("created_at", message.created_at),
        ],
    );
}

fn push_automation_event<const N: usize>(
    events: &Arc<Mutex<VecDeque<ServerAutomationEvent>>>,
    kind: impl Into<String>,
    fields: [(&str, String); N],
) {
    let mut queue = events
        .lock()
        .expect("server automation event lock poisoned");
    if queue.len() == 256 {
        queue.pop_front();
    }
    queue.push_back(ServerAutomationEvent {
        kind: kind.into(),
        fields: fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    });
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
    ChannelMessage(ChannelMessageInput),
    Ping,
}

#[derive(Debug, Serialize)]
struct ChannelMessageInput {
    event_id: Option<Uuid>,
    channel: String,
    message: String,
    metadata: Value,
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
        channel_messages: Vec<ChannelMessage>,
    },
    PresenceAccepted(Value),
    LogAccepted(Value),
    ChannelMessageAccepted(ChannelMessage),
    ChannelMessagePublished(ChannelMessage),
    Ack,
    Pong,
    Error {
        message: String,
    },
}

#[derive(Debug, Deserialize)]
struct ChannelMessage {
    id: String,
    author_callsign: String,
    channel: String,
    message: String,
    created_at: String,
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

    #[test]
    fn shared_channel_messages_use_the_stable_server_contract() {
        let envelope = ClientEnvelope {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            event_id: Uuid::nil(),
            message: ClientMessage::ChannelMessage(ChannelMessageInput {
                event_id: None,
                channel: "ops".to_string(),
                message: "Band opening on 20m".to_string(),
                metadata: serde_json::json!({ "source": "test" }),
            }),
        };
        let json = serde_json::to_value(envelope).unwrap();
        assert_eq!(json["type"], "channel_message");
        assert_eq!(json["payload"]["channel"], "ops");
        assert_eq!(json["payload"]["message"], "Band opening on 20m");
    }

    #[test]
    fn published_channel_message_becomes_an_automation_event() {
        let status = Arc::new(Mutex::new(ConnectionStatus::default()));
        let events = Arc::new(Mutex::new(VecDeque::new()));
        receive(
            r#"{
                "protocol_version":"v1",
                "event_id":"00000000-0000-0000-0000-000000000000",
                "type":"channel_message_published",
                "payload":{
                    "id":"3ecb975c-bd24-47fd-b230-5a79c0d5cad3",
                    "author_callsign":"W1AW",
                    "channel":"ops",
                    "message":"Band opening on 20m",
                    "created_at":"2026-08-14T12:00:00Z"
                }
            }"#,
            &status,
            &events,
        )
        .unwrap();
        let event = events.lock().unwrap().pop_front().unwrap();
        assert_eq!(event.kind, "channel_message");
        assert_eq!(event.fields["author"], "W1AW");
        assert_eq!(event.fields["channel"], "ops");
    }
}
