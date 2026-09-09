use qsonaut_core::{LanDiscoveryConfig, N3fjpEndpointConfig, ThirdPartyConfig};
use qsonaut_log::QsoRecord;
use qsonaut_n3fjp::{
    api::{Client as ApiClient, Command, CommandKind, Entry},
    network::{Client as NetworkClient, Message, Transaction},
    udp::{Broadcaster, Config as UdpConfig, Format, Qso as UdpQso},
    Config as ProtocolConfig,
};
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
    let mut api = connect_api(&config.n3fjp_api);
    let mut network = connect_network(&config.station_network, &station_callsign);
    let lan = bind_lan(&config.lan_discovery);
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
                let _ = client.disconnect();
                api = None;
            }
        }
        while let Ok(command) = chat_rx.try_recv() {
            if let Some(client) = network.as_mut() {
                match command {
                    ThirdPartyChatCommand::Send { to, text } => {
                        if let Err(error) = client.send(&Message::Chat {
                            to,
                            from: station_callsign.clone(),
                            text,
                        }) {
                            warn!(error = %error, "N3FJP station-network chat send failed");
                        }
                    }
                }
            }
        }

        if let Some(socket) = lan.as_ref() {
            if last_lan_beacon.elapsed() >= Duration::from_secs(15) {
                broadcast_lan(socket, &station_callsign, config.lan_discovery.port);
                last_lan_beacon = Instant::now();
            }
            poll_lan(socket, &station_callsign, &chat_event_tx);
        }

        if let Some(client) = network.as_mut() {
            if last_network_check.elapsed() >= Duration::from_secs(30) {
                if let Err(error) = client.heartbeat(Duration::from_secs(30)) {
                    warn!(error = %error, "N3FJP station-network heartbeat failed");
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
                                    let _ = chat_event_tx
                                        .send(ThirdPartyChatEvent::Message { from, text });
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
    let payload = format!("QSONAUT/1|{station}");
    if let Err(error) = socket.send_to(payload.as_bytes(), ("255.255.255.255", port)) {
        debug!(%error, "LAN discovery beacon failed");
    }
}

fn poll_lan(socket: &UdpSocket, station: &str, events: &Sender<ThirdPartyChatEvent>) {
    let mut buffer = [0_u8; 256];
    for _ in 0..16 {
        match socket.recv_from(&mut buffer) {
            Ok((size, _peer)) => {
                let Ok(payload) = std::str::from_utf8(&buffer[..size]) else {
                    continue;
                };
                let Some(callsign) = payload.strip_prefix("QSONAUT/1|") else {
                    continue;
                };
                let callsign = callsign.trim();
                if !callsign.is_empty() && !callsign.eq_ignore_ascii_case(station) {
                    let _ = events.send(ThirdPartyChatEvent::Users {
                        source: ThirdPartyUserSource::Lan,
                        users: vec![callsign.to_string()],
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

fn connect_api(endpoint: &N3fjpEndpointConfig) -> Option<ApiClient> {
    if !endpoint.enabled {
        return None;
    }
    match ApiClient::connect(&protocol_config(endpoint, 1024 * 1024)) {
        Ok(Some(mut client)) => {
            if let Err(error) = client.send(&Command::new(CommandKind::Program)) {
                warn!(error = %error, "N3FJP API discovery failed");
                return None;
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while client.api_version().is_none() && Instant::now() < deadline {
                match client.poll() {
                    Ok(_) => {}
                    Err(error) => {
                        warn!(error = %error, "N3FJP API discovery polling failed");
                        return None;
                    }
                }
            }
            if client.api_version().is_none() {
                warn!("N3FJP API version discovery timed out");
                return None;
            }
            info!(host = %endpoint.host, port = endpoint.port, "N3FJP API connected");
            Some(client)
        }
        Ok(None) => None,
        Err(error) => {
            warn!(error = %error, "N3FJP API connection failed");
            None
        }
    }
}

fn connect_network(endpoint: &N3fjpEndpointConfig, station: &str) -> Option<NetworkClient> {
    if !endpoint.enabled {
        return None;
    }
    match NetworkClient::connect(&protocol_config(endpoint, 1024 * 1024)) {
        Ok(Some(mut client)) => {
            if let Err(error) = client.open_session(station, "", "") {
                warn!(error = %error, "N3FJP station-network handshake failed");
                return None;
            }
            info!(host = %endpoint.host, port = endpoint.port, "N3FJP station network connected");
            Some(client)
        }
        Ok(None) => None,
        Err(error) => {
            warn!(error = %error, "N3FJP station-network connection failed");
            None
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
    use super::udp_qso;
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
}
