use qsonaut_core::{LanDiscoveryConfig, N3fjpEndpointConfig, ThirdPartyConfig};
use qsonaut_log::QsoRecord;
use qsonaut_n3fjp::{
    api::{Client as ApiClient, Command, CommandKind, Entry},
    network::{Client as NetworkClient, Message, Transaction},
    udp::{Broadcaster, Config as UdpConfig, Format, Qso as UdpQso},
    Config as ProtocolConfig,
};
use serde::{Deserialize, Serialize};
use std::{
    net::{SocketAddr, UdpSocket},
    sync::mpsc::{self, Receiver, Sender},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tracing::{debug, info, warn};

pub(crate) struct ThirdPartyBridge {
    tx: Sender<WorkerCommand>,
    chat_events: Receiver<ThirdPartyChatEvent>,
    chat_tx: Sender<ThirdPartyChatCommand>,
    stop: Option<Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
    status: Arc<Mutex<ThirdPartyStatus>>,
}

#[derive(Debug)]
enum WorkerCommand {
    Qso(QsoRecord),
}

#[derive(Debug, Clone)]
pub(crate) enum ThirdPartyChatEvent {
    Message {
        source: ThirdPartyUserSource,
        from: String,
        text: String,
    },
    Users {
        source: ThirdPartyUserSource,
        users: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ThirdPartyUserSource {
    N3fjp,
    Lan,
}

#[derive(Debug)]
pub(crate) enum ThirdPartyChatCommand {
    Send { to: String, text: String },
    SendLan { to: String, text: String },
    SetLanTrust { callsign: String, trusted: bool },
    ClearLanTrust { callsign: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanPacket {
    version: String,
    kind: String,
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    text: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ThirdPartyStatus {
    pub(crate) api: String,
    pub(crate) network: String,
    pub(crate) udp: String,
    pub(crate) lan: String,
    pub(crate) published: u64,
    pub(crate) last_error: Option<String>,
}

impl ThirdPartyBridge {
    pub(crate) fn spawn(config: &ThirdPartyConfig, station_callsign: &str) -> Option<Self> {
        if !config.n3fjp_api.enabled
            && !config.station_network.enabled
            && !config.udp_logging.enabled
            && !config.lan_discovery.enabled
        {
            return None;
        }

        let (tx, rx) = mpsc::channel();
        let (chat_tx, chat_rx) = mpsc::channel();
        let (chat_event_tx, chat_events) = mpsc::channel();
        let (stop, stop_rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(ThirdPartyStatus {
            api: enabled_label(config.n3fjp_api.enabled),
            network: enabled_label(config.station_network.enabled),
            udp: enabled_label(config.udp_logging.enabled),
            lan: enabled_label(config.lan_discovery.enabled),
            ..ThirdPartyStatus::default()
        }));
        let config = config.clone();
        let station_callsign = station_callsign.to_string();
        let worker_status = Arc::clone(&status);
        let worker = thread::Builder::new()
            .name("qsonaut-third-party".to_string())
            .spawn(move || {
                run_worker(
                    config,
                    station_callsign,
                    rx,
                    stop_rx,
                    chat_rx,
                    chat_event_tx,
                    worker_status,
                )
            })
            .ok()?;
        Some(Self {
            tx,
            chat_events,
            chat_tx,
            stop: Some(stop),
            worker: Some(worker),
            status,
        })
    }

    pub(crate) fn publish(&self, record: QsoRecord) {
        if let Err(error) = self.tx.send(WorkerCommand::Qso(record)) {
            warn!(error = %error, "third-party QSO worker is unavailable");
        }
    }

    pub(crate) fn send_chat(&self, to: impl Into<String>, text: impl Into<String>) {
        if let Err(error) = self.chat_tx.send(ThirdPartyChatCommand::Send {
            to: to.into(),
            text: text.into(),
        }) {
            warn!(error = %error, "third-party chat worker is unavailable");
        }
    }

    pub(crate) fn send_lan_chat(&self, to: impl Into<String>, text: impl Into<String>) {
        if let Err(error) = self.chat_tx.send(ThirdPartyChatCommand::SendLan {
            to: to.into(),
            text: text.into(),
        }) {
            warn!(error = %error, "LAN chat worker is unavailable");
        }
    }

    pub(crate) fn set_lan_trust(&self, callsign: impl Into<String>, trusted: bool) {
        let _ = self.chat_tx.send(ThirdPartyChatCommand::SetLanTrust {
            callsign: callsign.into(),
            trusted,
        });
    }

    pub(crate) fn clear_lan_trust(&self, callsign: impl Into<String>) {
        let _ = self.chat_tx.send(ThirdPartyChatCommand::ClearLanTrust {
            callsign: callsign.into(),
        });
    }

    pub(crate) fn poll_chat_events(&self) -> Vec<ThirdPartyChatEvent> {
        self.chat_events.try_iter().collect()
    }

    pub(crate) fn status(&self) -> ThirdPartyStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }
}

impl Drop for ThirdPartyBridge {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    config: ThirdPartyConfig,
    station_callsign: String,
    rx: Receiver<WorkerCommand>,
    stop_rx: Receiver<()>,
    chat_rx: Receiver<ThirdPartyChatCommand>,
    chat_event_tx: Sender<ThirdPartyChatEvent>,
    status: Arc<Mutex<ThirdPartyStatus>>,
) {
    let udp = build_udp(&config);
    let (mut api, api_error) = connect_api(&config.n3fjp_api);
    let (mut network, network_error) = connect_network(&config.station_network, &station_callsign);
    let lan = bind_lan(&config.lan_discovery);
    let mut trusted_lan = config.lan_discovery.trusted_callsigns.clone();
    let mut blocked_lan = config.lan_discovery.blocked_callsigns.clone();
    set_status(&status, |current| {
        current.api = if api.is_some() {
            "CONNECTED".to_string()
        } else {
            enabled_label(config.n3fjp_api.enabled)
        };
        current.network = if network.is_some() {
            "CONNECTED".to_string()
        } else {
            enabled_label(config.station_network.enabled)
        };
        current.udp = if udp.is_some() {
            "READY".to_string()
        } else {
            enabled_label(config.udp_logging.enabled)
        };
        current.lan = if lan.is_some() {
            "READY".to_string()
        } else {
            enabled_label(config.lan_discovery.enabled)
        };
        current.last_error = if api_error.is_some() {
            api_error
        } else if network_error.is_some() {
            network_error
        } else if config.lan_discovery.enabled && lan.is_none() {
            Some(format!(
                "Could not bind LAN discovery port {}",
                config.lan_discovery.port
            ))
        } else {
            None
        };
    });
    let mut last_network_check = Instant::now();
    let mut last_lan_beacon = Instant::now() - Duration::from_secs(30);

    loop {
        if stop_rx.try_recv().is_ok() {
            if let Some(client) = api.as_mut() {
                let _ = client.disconnect();
            }
            if let Some(client) = network.as_mut() {
                client.disconnect();
            }
            return;
        }

        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(WorkerCommand::Qso(record)) => {
                if let Some(broadcaster) = udp.as_ref() {
                    broadcast_udp(
                        broadcaster,
                        &station_callsign,
                        &record,
                        &config.udp_logging.format,
                    );
                }
                if let Some(client) = api.as_mut() {
                    submit_api(client, &record);
                }
                if let Some(client) = network.as_mut() {
                    send_network(client, &station_callsign, &record);
                }
                set_status(&status, |current| {
                    current.published = current.published.saturating_add(1)
                });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if let Some(client) = api.as_mut() {
            if let Err(error) = drain_api(client) {
                warn!(error = %error, "N3FJP API polling failed; disconnecting");
                set_status(&status, |current| {
                    current.last_error = Some(format!("N3FJP API polling failed: {error}"));
                    current.api = "DISCONNECTED".to_string();
                });
                let _ = client.disconnect();
                api = None;
            }
        }
        while let Ok(command) = chat_rx.try_recv() {
            match command {
                ThirdPartyChatCommand::Send { to, text } => {
                    if let Some(client) = network.as_mut() {
                        if let Err(error) = client.send(&Message::Chat {
                            to,
                            from: station_callsign.clone(),
                            text,
                        }) {
                            warn!(error = %error, "N3FJP station-network chat send failed");
                            set_status(&status, |current| {
                                current.last_error =
                                    Some(format!("N3FJP chat send failed: {error}"));
                            });
                        }
                    }
                }
                ThirdPartyChatCommand::SendLan { to, text } => {
                    if let Some(socket) = lan.as_ref() {
                        broadcast_lan_chat(
                            socket,
                            &station_callsign,
                            &to,
                            &text,
                            config.lan_discovery.port,
                        );
                    }
                }
                ThirdPartyChatCommand::SetLanTrust { callsign, trusted } => {
                    trusted_lan.retain(|peer| !peer.eq_ignore_ascii_case(&callsign));
                    blocked_lan.retain(|peer| !peer.eq_ignore_ascii_case(&callsign));
                    if trusted {
                        trusted_lan.push(callsign);
                    } else {
                        blocked_lan.push(callsign);
                    }
                }
                ThirdPartyChatCommand::ClearLanTrust { callsign } => {
                    trusted_lan.retain(|peer| !peer.eq_ignore_ascii_case(&callsign));
                    blocked_lan.retain(|peer| !peer.eq_ignore_ascii_case(&callsign));
                }
            }
        }

        if let Some(socket) = lan.as_ref() {
            if last_lan_beacon.elapsed() >= Duration::from_secs(15) {
                broadcast_lan(socket, &station_callsign, config.lan_discovery.port);
                last_lan_beacon = Instant::now();
            }
            poll_lan(
                socket,
                &station_callsign,
                &trusted_lan,
                &blocked_lan,
                &chat_event_tx,
            );
        }

        if let Some(client) = network.as_mut() {
            if last_network_check.elapsed() >= Duration::from_secs(30) {
                if let Err(error) = client.heartbeat(Duration::from_secs(30)) {
                    warn!(error = %error, "N3FJP station-network heartbeat failed");
                    set_status(&status, |current| {
                        current.last_error =
                            Some(format!("N3FJP station-network heartbeat failed: {error}"));
                        current.network = "DISCONNECTED".to_string();
                    });
                    client.disconnect();
                    network = None;
                }
                last_network_check = Instant::now();
            }
            if let Some(client) = network.as_mut() {
                match client.poll() {
                    Ok(messages) => {
                        for message in messages {
                            match message {
                                Message::Chat { from, text, .. } => {
                                    let _ = chat_event_tx.send(ThirdPartyChatEvent::Message {
                                        source: ThirdPartyUserSource::N3fjp,
                                        from,
                                        text,
                                    });
                                }
                                Message::Who(users) => {
                                    let _ = chat_event_tx.send(ThirdPartyChatEvent::Users {
                                        source: ThirdPartyUserSource::N3fjp,
                                        users,
                                    });
                                }
                                message => debug!(?message, "N3FJP station-network message"),
                            }
                        }
                    }
                    Err(error) => {
                        warn!(error = %error, "N3FJP station-network polling failed");
                        set_status(&status, |current| {
                            current.last_error =
                                Some(format!("N3FJP station-network polling failed: {error}"));
                            current.network = "DISCONNECTED".to_string();
                        });
                        client.disconnect();
                        network = None;
                    }
                }
            }
        }
    }
}

fn enabled_label(enabled: bool) -> String {
    if enabled {
        "STARTING".to_string()
    } else {
        "DISABLED".to_string()
    }
}

fn bind_lan(config: &LanDiscoveryConfig) -> Option<UdpSocket> {
    if !config.enabled {
        return None;
    }
    let socket = UdpSocket::bind(("0.0.0.0", config.port)).ok()?;
    socket.set_broadcast(true).ok()?;
    socket.set_nonblocking(true).ok()?;
    Some(socket)
}

fn broadcast_lan(socket: &UdpSocket, station: &str, port: u16) {
    let payload = serde_json::to_vec(&LanPacket {
        version: "QSONAUT/1".to_string(),
        kind: "hello".to_string(),
        from: station.to_string(),
        to: String::new(),
        text: String::new(),
    })
    .unwrap_or_default();
    if let Err(error) = socket.send_to(&payload, ("255.255.255.255", port)) {
        debug!(%error, "LAN discovery beacon failed");
    }
}

fn broadcast_lan_chat(socket: &UdpSocket, station: &str, to: &str, text: &str, port: u16) {
    let Some(packet) = lan_chat_packet(station, to, text) else {
        return;
    };
    let Ok(payload) = serde_json::to_vec(&packet) else {
        return;
    };
    if let Err(error) = socket.send_to(&payload, ("255.255.255.255", port)) {
        debug!(%error, "LAN chat broadcast failed");
    }
}

fn lan_chat_packet(station: &str, to: &str, text: &str) -> Option<LanPacket> {
    if text.len() > 1024 || to.len() > 32 {
        return None;
    }
    Some(LanPacket {
        version: "QSONAUT/1".to_string(),
        kind: "chat".to_string(),
        from: station.to_string(),
        to: to.to_string(),
        text: text.to_string(),
    })
}

fn accepts_lan_chat(
    payload: &LanPacket,
    station: &str,
    trusted_lan: &[String],
    blocked_lan: &[String],
) -> bool {
    let callsign = payload.from.trim();
    payload.version == "QSONAUT/1"
        && payload.kind == "chat"
        && !callsign.is_empty()
        && callsign.len() <= 32
        && !callsign.eq_ignore_ascii_case(station)
        && (payload.to.is_empty() || payload.to.eq_ignore_ascii_case(station))
        && trusted_lan
            .iter()
            .any(|trusted| trusted.eq_ignore_ascii_case(callsign))
        && !blocked_lan
            .iter()
            .any(|blocked| blocked.eq_ignore_ascii_case(callsign))
}

fn poll_lan(
    socket: &UdpSocket,
    station: &str,
    trusted_lan: &[String],
    blocked_lan: &[String],
    events: &Sender<ThirdPartyChatEvent>,
) {
    let mut buffer = [0_u8; 2048];
    for _ in 0..16 {
        match socket.recv_from(&mut buffer) {
            Ok((size, _peer)) => {
                let Ok(payload) = serde_json::from_slice::<LanPacket>(&buffer[..size]) else {
                    continue;
                };
                if payload.version != "QSONAUT/1" {
                    continue;
                }
                let callsign = payload.from.trim();
                if callsign.is_empty() || callsign.len() > 32 {
                    continue;
                }
                if payload.kind == "hello" && !callsign.eq_ignore_ascii_case(station) {
                    let _ = events.send(ThirdPartyChatEvent::Users {
                        source: ThirdPartyUserSource::Lan,
                        users: vec![callsign.to_string()],
                    });
                }
                if accepts_lan_chat(&payload, station, trusted_lan, blocked_lan) {
                    let _ = events.send(ThirdPartyChatEvent::Message {
                        source: ThirdPartyUserSource::Lan,
                        from: callsign.to_string(),
                        text: payload.text,
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => {
                debug!(%error, "LAN discovery receive failed");
                break;
            }
        }
    }
}

fn set_status(status: &Arc<Mutex<ThirdPartyStatus>>, update: impl FnOnce(&mut ThirdPartyStatus)) {
    if let Ok(mut status) = status.lock() {
        update(&mut status);
    }
}

fn protocol_config(endpoint: &N3fjpEndpointConfig, max_frame_bytes: usize) -> ProtocolConfig {
    ProtocolConfig {
        enabled: endpoint.enabled,
        host: endpoint.host.clone(),
        port: endpoint.port,
        max_frame_bytes,
        ..ProtocolConfig::default()
    }
}

fn connect_api(endpoint: &N3fjpEndpointConfig) -> (Option<ApiClient>, Option<String>) {
    if !endpoint.enabled {
        return (None, None);
    }
    match ApiClient::connect(&protocol_config(endpoint, 1024 * 1024)) {
        Ok(Some(mut client)) => {
            if let Err(error) = client.send(&Command::new(CommandKind::Program)) {
                warn!(error = %error, "N3FJP API discovery failed");
                return (None, Some(format!("N3FJP API discovery failed: {error}")));
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while client.api_version().is_none() && Instant::now() < deadline {
                match client.poll() {
                    Ok(_) => {}
                    Err(error) => {
                        warn!(error = %error, "N3FJP API discovery polling failed");
                        return (
                            None,
                            Some(format!("N3FJP API discovery polling failed: {error}")),
                        );
                    }
                }
            }
            if client.api_version().is_none() {
                warn!("N3FJP API version discovery timed out");
                return (
                    None,
                    Some("N3FJP API version discovery timed out".to_string()),
                );
            }
            info!(host = %endpoint.host, port = endpoint.port, "N3FJP API connected");
            (Some(client), None)
        }
        Ok(None) => (
            None,
            Some(format!(
                "N3FJP API did not accept a connection at {}:{}",
                endpoint.host, endpoint.port
            )),
        ),
        Err(error) => {
            warn!(error = %error, "N3FJP API connection failed");
            (None, Some(format!("N3FJP API connection failed: {error}")))
        }
    }
}

fn connect_network(
    endpoint: &N3fjpEndpointConfig,
    station: &str,
) -> (Option<NetworkClient>, Option<String>) {
    if !endpoint.enabled {
        return (None, None);
    }
    match NetworkClient::connect(&protocol_config(endpoint, 1024 * 1024)) {
        Ok(Some(mut client)) => {
            if let Err(error) = client.open_session(station, "", "") {
                warn!(error = %error, "N3FJP station-network handshake failed");
                return (
                    None,
                    Some(format!("N3FJP station-network handshake failed: {error}")),
                );
            }
            info!(host = %endpoint.host, port = endpoint.port, "N3FJP station network connected");
            (Some(client), None)
        }
        Ok(None) => (
            None,
            Some(format!(
                "N3FJP station network did not accept a connection at {}:{}",
                endpoint.host, endpoint.port
            )),
        ),
        Err(error) => {
            warn!(error = %error, "N3FJP station-network connection failed");
            (
                None,
                Some(format!("N3FJP station-network connection failed: {error}")),
            )
        }
    }
}

fn build_udp(config: &ThirdPartyConfig) -> Option<Broadcaster> {
    let settings = &config.udp_logging;
    if !settings.enabled {
        return None;
    }
    let destinations: Vec<SocketAddr> = settings
        .destinations
        .iter()
        .filter_map(|destination| match destination.parse::<SocketAddr>() {
            Ok(address) => Some(address),
            Err(error) => {
                warn!(destination, %error, "invalid UDP logging destination");
                None
            }
        })
        .collect();
    let udp_config = UdpConfig {
        enabled: true,
        destinations,
        ..UdpConfig::default()
    };
    match Broadcaster::bind(udp_config) {
        Ok(broadcaster) => Some(broadcaster),
        Err(error) => {
            warn!(error = %error, "UDP logging broadcaster disabled");
            None
        }
    }
}

fn broadcast_udp(
    broadcaster: &Broadcaster,
    station_callsign: &str,
    record: &QsoRecord,
    format: &str,
) {
    let qso = udp_qso(station_callsign, record);
    let format = if format.eq_ignore_ascii_case("adif") {
        Format::Adif
    } else {
        Format::N1mmContactInfo
    };
    if let Err(error) = broadcaster.broadcast(&qso, format) {
        warn!(error = %error, callsign = %record.callsign, "UDP QSO broadcast failed");
    }
}

fn udp_qso(station_callsign: &str, record: &QsoRecord) -> UdpQso {
    UdpQso {
        call: record.callsign.clone(),
        my_call: station_callsign.to_string(),
        band: record.band.clone(),
        mode: record.mode.clone(),
        timestamp: format!("{} {}", record.qso_date, record.time_on),
        qso_date: record.qso_date.clone(),
        time_on: record.time_on.clone(),
        rst_sent: record.report_sent.clone(),
        rst_received: record.report_received.clone(),
        grid: record.grid.clone(),
        exchange: format!(
            "{} {}",
            record.contest_exchange_sent, record.contest_exchange_received
        )
        .trim()
        .to_string(),
        comment: record.notes.clone(),
        station_name: "QSONaut".to_string(),
        id: record.id.to_string(),
    }
}

fn submit_api(client: &mut ApiClient, record: &QsoRecord) {
    let entry = Entry {
        call: record.callsign.clone(),
        band: record.band.clone(),
        mode: record.mode.clone(),
        frequency_mhz: (record.frequency_hz > 0)
            .then(|| format!("{:.6}", record.frequency_hz as f64 / 1_000_000.0)),
        controls: Vec::new(),
    };
    if let Err(error) = client.submit_entry(&entry) {
        warn!(error = %error, callsign = %record.callsign, "N3FJP API QSO submission failed");
    }
}

fn drain_api(client: &mut ApiClient) -> std::io::Result<()> {
    for packet in client.poll()? {
        if packet.id() == "ENTERRESPONSE" {
            debug!(?packet, "N3FJP API entry response");
        } else {
            debug!(id = packet.id(), "N3FJP API event");
        }
    }
    Ok(())
}

fn send_network(client: &mut NetworkClient, station: &str, record: &QsoRecord) {
    let fields = vec![
        ("CALL".to_string(), record.callsign.clone()),
        ("BAND".to_string(), record.band.clone()),
        ("MODE".to_string(), record.mode.clone()),
        ("QSO_DATE".to_string(), record.qso_date.clone()),
        ("TIME_ON".to_string(), record.time_on.clone()),
        ("RST_SENT".to_string(), record.report_sent.clone()),
        ("RST_RCVD".to_string(), record.report_received.clone()),
        ("GRIDSQUARE".to_string(), record.grid.clone()),
    ];
    let message = Message::Transaction {
        from: station.to_string(),
        kind: Transaction::Add,
        fields,
    };
    if let Err(error) = client.send(&message) {
        warn!(error = %error, callsign = %record.callsign, "N3FJP station-network QSO send failed");
    }
}

#[cfg(test)]
mod tests {
    use super::{accepts_lan_chat, lan_chat_packet, udp_qso};
    use qsonaut_log::QsoRecord;

    #[test]
    fn maps_qso_record_to_udp_payload_model() {
        let record = QsoRecord::new("w1aw", "ft8", "20m", 14_074_000, 1, 2);
        let qso = udp_qso("K7TEST", &record);
        assert_eq!(qso.call, "W1AW");
        assert_eq!(qso.mode, "FT8");
        assert_eq!(qso.band, "20m");
        assert_eq!(qso.my_call, "K7TEST");
        assert_eq!(qso.id, record.id.to_string());
    }

    #[test]
    fn empty_lan_recipient_creates_broadcast_packet() {
        let packet = lan_chat_packet("N7UF", "", "hello").expect("valid packet");
        assert_eq!(packet.kind, "chat");
        assert!(packet.to.is_empty());
        assert_eq!(packet.text, "hello");
    }

    #[test]
    fn broadcast_is_accepted_only_from_trusted_unblocked_peer() {
        let packet = lan_chat_packet("W1AW", "", "hello").expect("valid packet");
        let trusted = vec!["w1aw".to_string()];
        assert!(accepts_lan_chat(&packet, "N7UF", &trusted, &[]));
        assert!(!accepts_lan_chat(
            &packet,
            "N7UF",
            &trusted,
            &["W1AW".to_string()]
        ));
        assert!(!accepts_lan_chat(&packet, "N7UF", &[], &[]));
    }

    #[test]
    fn targeted_lan_packet_is_only_for_matching_station() {
        let packet = lan_chat_packet("W1AW", "N7UF", "hello").expect("valid packet");
        let trusted = vec!["W1AW".to_string()];
        assert!(accepts_lan_chat(&packet, "N7UF", &trusted, &[]));
        assert!(!accepts_lan_chat(&packet, "K1ABC", &trusted, &[]));
    }
}
