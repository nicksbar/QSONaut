//! QSONaut's transport-neutral client for a QSONaut HostBridge.
//!
//! This crate owns WebSocket sessions and wire/media validation. GUI state,
//! device presentation, audio routing, and TX policy remain QSONaut concerns.

use anyhow::{anyhow, Context, Result};
use futures_util::{Sink, SinkExt, StreamExt};
use qsonaut_hostbridge_protocol::{
    AudioFormat, ClientHello, ClientMessage, HostHello, MediaDirection, MediaFrameHeader,
    ServerMessage,
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::warn;
use url::Url;

const MAX_MEDIA_PAYLOAD_BYTES: usize = 1_048_576;

#[derive(Debug, Clone)]
pub struct HostBridgeConfig {
    pub endpoint: String,
    pub client_name: String,
    pub access_key: String,
    pub password: String,
    pub reconnect_delay: Duration,
}

impl Default for HostBridgeConfig {
    fn default() -> Self {
        Self {
            endpoint: "ws://127.0.0.1:8765".into(),
            client_name: "QSONaut".into(),
            access_key: String::new(),
            password: String::new(),
            reconnect_delay: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone)]
pub enum HostBridgeEvent {
    Connected(HostHello),
    Server(ServerMessage),
    Media {
        header: MediaFrameHeader,
        payload: Vec<u8>,
    },
    Disconnected {
        reason: String,
    },
    Reconnecting,
    /// QSONaut must clear every local transmit arm/autosequence on this event.
    SafetyDisarmed {
        reason: String,
    },
}

#[derive(Debug)]
enum Command {
    Message(ClientMessage),
    Media(Vec<u8>),
    Shutdown,
}

pub struct HostBridgeClient {
    commands: mpsc::UnboundedSender<Command>,
    worker: Option<tokio::task::JoinHandle<()>>,
}

impl HostBridgeClient {
    #[must_use]
    pub fn spawn(config: HostBridgeConfig) -> (Self, mpsc::UnboundedReceiver<HostBridgeEvent>) {
        install_tls_crypto_provider();
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (events, event_rx) = mpsc::unbounded_channel();
        let worker = tokio::spawn(run(config, command_rx, events));
        (
            Self {
                commands,
                worker: Some(worker),
            },
            event_rx,
        )
    }

    pub fn select_radio(&self, device_id: impl Into<String>) -> Result<()> {
        self.send(ClientMessage::SelectRadio {
            request_id: None,
            device_id: device_id.into(),
        })
    }

    pub fn select_audio(
        &self,
        enabled: bool,
        source_id: impl Into<String>,
        format: AudioFormat,
    ) -> Result<()> {
        self.send(ClientMessage::SelectAudio {
            request_id: None,
            enabled,
            source_id: source_id.into(),
            format,
        })
    }

    pub fn select_audio_output(
        &self,
        enabled: bool,
        output_id: impl Into<String>,
        format: AudioFormat,
    ) -> Result<()> {
        self.send(ClientMessage::SelectAudioOutput {
            request_id: None,
            enabled,
            output_id: output_id.into(),
            format,
        })
    }

    pub fn set_frequency(&self, frequency_hz: u64) -> Result<()> {
        self.send(ClientMessage::SetFrequency {
            request_id: None,
            frequency_hz,
        })
    }

    pub fn set_mode(&self, mode: qsonaut_hostbridge_protocol::WireMode) -> Result<()> {
        self.send(ClientMessage::SetMode {
            request_id: None,
            mode,
        })
    }

    pub fn set_ptt(&self, enabled: bool) -> Result<()> {
        self.send(ClientMessage::SetPtt {
            request_id: None,
            enabled,
        })
    }

    pub fn get_state(&self) -> Result<()> {
        self.send(ClientMessage::GetState { request_id: None })
    }

    pub fn send_media(&self, header: MediaFrameHeader, payload: &[u8]) -> Result<()> {
        validate_outgoing_media(header, payload)?;
        let mut bytes = Vec::with_capacity(MediaFrameHeader::BYTES + payload.len());
        header.encode(&mut bytes);
        bytes.extend_from_slice(payload);
        self.commands
            .send(Command::Media(bytes))
            .map_err(|_| anyhow!("HostBridge client is stopped"))
    }

    pub fn shutdown(&self) -> Result<()> {
        self.commands
            .send(Command::Shutdown)
            .map_err(|_| anyhow!("HostBridge client is stopped"))
    }

    fn send(&self, message: ClientMessage) -> Result<()> {
        self.commands
            .send(Command::Message(message))
            .map_err(|_| anyhow!("HostBridge client is stopped"))
    }
}

impl Drop for HostBridgeClient {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            worker.abort();
        }
    }
}

async fn run(
    config: HostBridgeConfig,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<HostBridgeEvent>,
) {
    loop {
        match connect(&config, &mut commands, &events).await {
            Ok(ConnectionEnd::Shutdown) => return,
            Ok(ConnectionEnd::Disconnected(reason)) => {
                let _ = events.send(HostBridgeEvent::SafetyDisarmed {
                    reason: reason.clone(),
                });
                let _ = events.send(HostBridgeEvent::Disconnected { reason });
            }
            Err(error) => {
                let reason = error.to_string();
                warn!(%reason, "HostBridge client connection failed");
                let _ = events.send(HostBridgeEvent::SafetyDisarmed {
                    reason: reason.clone(),
                });
                let _ = events.send(HostBridgeEvent::Disconnected { reason });
            }
        }
        let _ = events.send(HostBridgeEvent::Reconnecting);
        tokio::select! {
            () = tokio::time::sleep(config.reconnect_delay) => {},
            command = commands.recv() => {
                if matches!(command, Some(Command::Shutdown) | None) { return; }
            }
        }
    }
}

enum ConnectionEnd {
    Shutdown,
    Disconnected(String),
}

async fn connect(
    config: &HostBridgeConfig,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    events: &mpsc::UnboundedSender<HostBridgeEvent>,
) -> Result<ConnectionEnd> {
    let endpoint = validate_endpoint(&config.endpoint)?;
    let (socket, _) =
        tokio::time::timeout(Duration::from_secs(10), connect_async(endpoint.as_str()))
            .await
            .context("HostBridge connection timed out")??;
    let (mut writer, mut reader) = socket.split();
    send_json(
        &mut writer,
        &ClientMessage::Hello(ClientHello {
            protocol_version: qsonaut_hostbridge_protocol::PROTOCOL_VERSION,
            client_name: config.client_name.clone(),
            access_key: config.access_key.clone(),
            password: config.password.clone(),
        }),
    )
    .await?;
    let hello = match reader.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<ServerMessage>(&text)? {
            ServerMessage::Hello(hello) => hello,
            other => anyhow::bail!("HostBridge sent {other:?} instead of hello"),
        },
        Some(Ok(_)) => anyhow::bail!("HostBridge sent a non-text hello"),
        Some(Err(error)) => return Err(error.into()),
        None => anyhow::bail!("HostBridge closed before hello"),
    };
    let _ = events.send(HostBridgeEvent::Connected(hello));
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                send_json(&mut writer, &ClientMessage::Ping { nonce: 0 }).await?;
            }
            command = commands.recv() => match command {
                Some(Command::Message(message)) => send_json(&mut writer, &message).await?,
                Some(Command::Media(bytes)) => writer.send(Message::Binary(bytes.into())).await?,
                Some(Command::Shutdown) | None => {
                    let _ = send_json(&mut writer, &ClientMessage::SetPtt { request_id: None, enabled: false }).await;
                    let _ = writer.send(Message::Close(None)).await;
                    return Ok(ConnectionEnd::Shutdown);
                }
            },
            message = reader.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    let message: ServerMessage = serde_json::from_str(&text)?;
                    if let ServerMessage::Ping { nonce } = message {
                        send_json(&mut writer, &ClientMessage::Pong { nonce }).await?;
                    }
                    let _ = events.send(HostBridgeEvent::Server(message));
                }
                Some(Ok(Message::Binary(bytes))) => {
                    let media = decode_media(&bytes)?;
                    let _ = events.send(HostBridgeEvent::Media { header: media.0, payload: media.1 });
                }
                Some(Ok(Message::Close(_))) | None => return Ok(ConnectionEnd::Disconnected("HostBridge closed the session".into())),
                Some(Ok(_)) => {},
                Some(Err(error)) => return Err(error.into()),
            }
        }
    }
}

async fn send_json<S>(writer: &mut S, message: &ClientMessage) -> Result<()>
where
    S: Sink<Message> + Unpin,
    S::Error: Into<anyhow::Error>,
{
    writer
        .send(Message::Text(serde_json::to_string(message)?.into()))
        .await
        .map_err(Into::into)
}

fn validate_endpoint(value: &str) -> Result<String> {
    let url = Url::parse(value.trim()).context("HostBridge endpoint is invalid")?;
    if !matches!(url.scheme(), "ws" | "wss") {
        anyhow::bail!("HostBridge endpoint must use ws:// or wss://")
    }
    Ok(url.to_string())
}

fn validate_outgoing_media(header: MediaFrameHeader, payload: &[u8]) -> Result<()> {
    if header.version != qsonaut_hostbridge_protocol::MEDIA_HEADER_VERSION {
        anyhow::bail!("unsupported media header version {}", header.version)
    }
    if header.direction != MediaDirection::ClientToHost {
        anyhow::bail!("outgoing media must use client_to_host direction")
    }
    if header.payload_bytes as usize != payload.len() {
        anyhow::bail!("media payload length does not match its header")
    }
    if payload.len() > MAX_MEDIA_PAYLOAD_BYTES {
        anyhow::bail!("media payload exceeds HostBridge limit")
    }
    Ok(())
}

fn decode_media(bytes: &[u8]) -> Result<(MediaFrameHeader, Vec<u8>)> {
    if bytes.len() < MediaFrameHeader::BYTES {
        anyhow::bail!("HostBridge media frame is shorter than its header")
    }
    let header = MediaFrameHeader::decode(bytes).ok_or_else(|| anyhow!("invalid media header"))?;
    if header.version != qsonaut_hostbridge_protocol::MEDIA_HEADER_VERSION
        || header.direction != MediaDirection::HostToClient
    {
        anyhow::bail!("invalid HostBridge media direction or version")
    }
    let payload = &bytes[MediaFrameHeader::BYTES..];
    if header.payload_bytes as usize != payload.len() || payload.len() > MAX_MEDIA_PAYLOAD_BYTES {
        anyhow::bail!("invalid HostBridge media payload length")
    }
    Ok((header, payload.to_vec()))
}

fn install_tls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsonaut_hostbridge_protocol::{AudioCodec, MediaDirection, MEDIA_HEADER_VERSION};

    #[test]
    fn endpoint_requires_websocket_scheme() {
        assert!(validate_endpoint("ws://127.0.0.1:8765").is_ok());
        assert!(validate_endpoint("https://example.test").is_err());
    }

    #[test]
    fn outgoing_media_requires_client_direction_and_exact_length() {
        let header = MediaFrameHeader {
            version: MEDIA_HEADER_VERSION,
            stream_id: 1,
            direction: MediaDirection::ClientToHost,
            codec: AudioCodec::PcmS16Le,
            sequence: 0,
            timestamp_samples: 0,
            sample_rate_hz: 48_000,
            channels: 1,
            payload_bytes: 2,
        };
        assert!(validate_outgoing_media(header, &[0, 0]).is_ok());
        assert!(validate_outgoing_media(header, &[0]).is_err());
    }
}
