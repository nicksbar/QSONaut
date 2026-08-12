//! Opt-in PSK Reporter IPFIX batching over a persistent UDP socket.

use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RECEIVER_TEMPLATE: &[u8] = &[
    0x00, 0x03, 0x00, 0x24, 0x99, 0x92, 0x00, 0x03, 0x00, 0x01, 0x80, 0x02, 0xFF, 0xFF, 0x00, 0x00,
    0x76, 0x8F, 0x80, 0x04, 0xFF, 0xFF, 0x00, 0x00, 0x76, 0x8F, 0x80, 0x08, 0xFF, 0xFF, 0x00, 0x00,
    0x76, 0x8F, 0x00, 0x00,
];
const SENDER_TEMPLATE: &[u8] = &[
    0x00, 0x02, 0x00, 0x3C, 0x99, 0x93, 0x00, 0x07, 0x80, 0x01, 0xFF, 0xFF, 0x00, 0x00, 0x76, 0x8F,
    0x80, 0x05, 0x00, 0x05, 0x00, 0x00, 0x76, 0x8F, 0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x76, 0x8F,
    0x80, 0x0A, 0xFF, 0xFF, 0x00, 0x00, 0x76, 0x8F, 0x80, 0x03, 0xFF, 0xFF, 0x00, 0x00, 0x76, 0x8F,
    0x80, 0x0B, 0x00, 0x01, 0x00, 0x00, 0x76, 0x8F, 0x00, 0x96, 0x00, 0x04,
];

#[derive(Debug, Clone)]
pub struct ReporterConfig {
    pub receiver_callsign: String,
    pub receiver_locator: String,
    pub decoder_software: String,
    pub destination: String,
}

impl ReporterConfig {
    pub fn production(callsign: impl Into<String>, locator: impl Into<String>) -> Self {
        Self {
            receiver_callsign: callsign.into(),
            receiver_locator: locator.into(),
            decoder_software: format!("QSONaut {}", env!("CARGO_PKG_VERSION")),
            destination: "report.pskreporter.info:4739".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceptionReport {
    pub sender_callsign: String,
    pub frequency_hz: u64,
    pub snr_db: i8,
    pub mode: String,
    pub sender_locator: String,
    pub received_at: u32,
}

#[derive(Debug, Clone, Default)]
pub struct ReporterStatus {
    pub queued: usize,
    pub sent: u64,
    pub last_error: Option<String>,
    pub active: bool,
}

enum Command {
    Report(ReceptionReport),
    Stop,
}

pub struct Reporter {
    tx: mpsc::Sender<Command>,
    status: Arc<Mutex<ReporterStatus>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Reporter {
    pub fn start(config: ReporterConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(ReporterStatus {
            active: true,
            ..Default::default()
        }));
        let worker_status = status.clone();
        let worker = thread::spawn(move || run_worker(config, rx, worker_status));
        Self {
            tx,
            status,
            worker: Some(worker),
        }
    }

    pub fn sender(&self) -> ReportSender {
        ReportSender(self.tx.clone())
    }

    pub fn status(&self) -> ReporterStatus {
        self.status.lock().expect("PSK status lock").clone()
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReportSender(mpsc::Sender<Command>);

impl ReportSender {
    pub fn submit(&self, report: ReceptionReport) {
        let _ = self.0.send(Command::Report(report));
    }
}

fn run_worker(
    config: ReporterConfig,
    rx: mpsc::Receiver<Command>,
    status: Arc<Mutex<ReporterStatus>>,
) {
    let socket = UdpSocket::bind("0.0.0.0:0").and_then(|socket| {
        socket.connect(&config.destination)?;
        Ok(socket)
    });
    let socket = match socket {
        Ok(socket) => socket,
        Err(error) => {
            let mut state = status.lock().expect("PSK status lock");
            state.active = false;
            state.last_error = Some(error.to_string());
            return;
        }
    };

    let session_id = session_identifier(&config.receiver_callsign);
    let interval = Duration::from_secs(300 + u64::from(session_id % 31));
    let mut deadline = Instant::now() + interval;
    let mut queue = Vec::new();
    let mut recent: HashMap<String, Instant> = HashMap::new();
    let mut sequence = 0u32;
    let mut packets = 0u32;

    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Command::Report(report)) => {
                let key = report.sender_callsign.to_ascii_uppercase();
                let now = Instant::now();
                if recent
                    .get(&key)
                    .is_none_or(|seen| now.duration_since(*seen) >= Duration::from_secs(300))
                {
                    recent.insert(key, now);
                    queue.push(report);
                    status.lock().expect("PSK status lock").queued = queue.len();
                }
                if queue.len() < 80 {
                    continue;
                }
            }
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if !queue.is_empty() {
            let packet = encode_packet(
                &config,
                &queue,
                sequence,
                session_id,
                unix_seconds(),
                packets < 3 || packets % 12 == 0,
            );
            let mut state = status.lock().expect("PSK status lock");
            match socket.send(&packet) {
                Ok(_) => {
                    sequence = sequence.wrapping_add(queue.len() as u32);
                    state.sent += queue.len() as u64;
                    state.last_error = None;
                    queue.clear();
                }
                Err(error) => state.last_error = Some(error.to_string()),
            }
            state.queued = queue.len();
            packets = packets.wrapping_add(1);
        }
        recent.retain(|_, seen| seen.elapsed() < Duration::from_secs(3_600));
        deadline = Instant::now() + interval;
    }
    status.lock().expect("PSK status lock").active = false;
}

pub fn encode_packet(
    config: &ReporterConfig,
    reports: &[ReceptionReport],
    sequence: u32,
    session_id: u32,
    export_time: u32,
    include_templates: bool,
) -> Vec<u8> {
    let mut packet = vec![0x00, 0x0A, 0, 0];
    packet.extend_from_slice(&export_time.to_be_bytes());
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&session_id.to_be_bytes());
    if include_templates {
        packet.extend_from_slice(RECEIVER_TEMPLATE);
        packet.extend_from_slice(SENDER_TEMPLATE);
    }

    let receiver_start = packet.len();
    packet.extend_from_slice(&[0x99, 0x92, 0, 0]);
    push_string(&mut packet, &config.receiver_callsign);
    push_string(&mut packet, &config.receiver_locator);
    push_string(&mut packet, &config.decoder_software);
    pad4(&mut packet);
    set_length(&mut packet, receiver_start);

    let sender_start = packet.len();
    packet.extend_from_slice(&[0x99, 0x93, 0, 0]);
    for report in reports {
        push_string(&mut packet, &report.sender_callsign);
        packet.push(((report.frequency_hz >> 32) & 0xFF) as u8);
        packet.extend_from_slice(&(report.frequency_hz as u32).to_be_bytes());
        packet.push(report.snr_db as u8);
        push_string(&mut packet, &report.mode);
        push_string(&mut packet, &report.sender_locator);
        packet.push(1);
        packet.extend_from_slice(&report.received_at.to_be_bytes());
    }
    pad4(&mut packet);
    set_length(&mut packet, sender_start);
    let total = u16::try_from(packet.len())
        .unwrap_or(u16::MAX)
        .to_be_bytes();
    packet[2..4].copy_from_slice(&total);
    packet
}

fn push_string(output: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    let len = bytes.len().min(254);
    output.push(len as u8);
    output.extend_from_slice(&bytes[..len]);
}

fn pad4(output: &mut Vec<u8>) {
    while !output.len().is_multiple_of(4) {
        output.push(0);
    }
}

fn set_length(output: &mut [u8], start: usize) {
    let length = u16::try_from(output.len() - start)
        .unwrap_or(u16::MAX)
        .to_be_bytes();
    output[start + 2..start + 4].copy_from_slice(&length);
}

fn unix_seconds() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as u32)
        .unwrap_or_default()
}

fn session_identifier(callsign: &str) -> u32 {
    callsign
        .bytes()
        .fold(std::process::id() ^ unix_seconds(), |hash, byte| {
            hash.rotate_left(5) ^ u32::from(byte)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_has_ipfix_header_templates_and_network_order_fields() {
        let config = ReporterConfig {
            receiver_callsign: "N1DQ".into(),
            receiver_locator: "FN42hn".into(),
            decoder_software: "QSONaut test".into(),
            destination: String::new(),
        };
        let report = ReceptionReport {
            sender_callsign: "KB1MBX".into(),
            frequency_hz: 14_070_987,
            snr_db: -12,
            mode: "FT8".into(),
            sender_locator: "FN42".into(),
            received_at: 1_200_960_104,
        };
        let packet = encode_packet(&config, &[report], 4, 7, 1_200_960_114, true);
        assert_eq!(&packet[..2], &[0, 10]);
        assert_eq!(
            u16::from_be_bytes([packet[2], packet[3]]) as usize,
            packet.len()
        );
        assert!(packet.windows(2).any(|window| window == [0x99, 0x92]));
        assert!(packet.windows(2).any(|window| window == [0x99, 0x93]));
        assert!(packet
            .windows(5)
            .any(|window| window == [0, 0x00, 0xD6, 0xB4, 0xCB]));
        assert!(packet.windows(2).any(|window| window == [0xF4, 3]));
        assert_eq!(packet.len() % 4, 0);
    }
}
