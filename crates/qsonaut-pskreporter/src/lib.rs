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

/// Tuning knobs for how reception reports are batched and sent to PSK Reporter.
///
/// These mirror the knobs WSJT-X exposes internally so operators can follow the
/// service's rules (or relax them) without editing code.
#[derive(Debug, Clone)]
pub struct ReporterTuning {
    /// Nominal batch interval in seconds. The actual interval is randomized
    /// around this value (up to +30 s) so bursts from many clients don't all
    /// land on the same wall-clock boundary. WSJT-X uses 300 s.
    pub batch_interval_secs: u64,
    /// Minimum time in seconds before the same callsign may be reported again.
    /// WSJT-X uses 300 s (5 minutes) to reduce load on the collector.
    pub repeat_cache_secs: u64,
    /// Maximum number of pending reports held before a batch is forced out.
    /// WSJT-X uses 2048; QSONaut's default is 80.
    pub max_pending: usize,
}

impl Default for ReporterTuning {
    fn default() -> Self {
        Self {
            batch_interval_secs: 300,
            repeat_cache_secs: 300,
            max_pending: 80,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReporterConfig {
    pub receiver_callsign: String,
    pub receiver_locator: String,
    pub decoder_software: String,
    pub destination: String,
    pub tuning: ReporterTuning,
}

impl ReporterConfig {
    pub fn production(callsign: impl Into<String>, locator: impl Into<String>) -> Self {
        Self {
            receiver_callsign: callsign.into(),
            receiver_locator: locator.into(),
            decoder_software: format!("QSONaut {}", env!("CARGO_PKG_VERSION")),
            destination: "report.pskreporter.info:4739".to_string(),
            tuning: ReporterTuning::default(),
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
        tracing::info!(
            callsign = %config.receiver_callsign,
            destination = %config.destination,
            batch_interval_secs = config.tuning.batch_interval_secs,
            max_pending = config.tuning.max_pending,
            "PSK Reporter worker starting"
        );
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
    pub fn submit(&self, report: ReceptionReport) -> bool {
        self.0.send(Command::Report(report)).is_ok()
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
            tracing::error!(error = %error, "PSK Reporter socket initialization failed");
            let mut state = status.lock().expect("PSK status lock");
            state.active = false;
            state.last_error = Some(error.to_string());
            return;
        }
    };

    let session_id = session_identifier(&config.receiver_callsign);
    let tuning = &config.tuning;
    let interval = Duration::from_secs(tuning.batch_interval_secs + u64::from(session_id % 31));
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
                if recent.get(&key).is_none_or(|seen| {
                    now.duration_since(*seen) >= Duration::from_secs(tuning.repeat_cache_secs)
                }) {
                    recent.insert(key, now);
                    queue.push(report);
                    status.lock().expect("PSK status lock").queued = queue.len();
                    tracing::debug!(queued = queue.len(), "PSK Reporter report queued");
                }
                if queue.len() < tuning.max_pending {
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
                packets < 3 || packets.is_multiple_of(12),
            );
            let mut state = status.lock().expect("PSK status lock");
            match socket.send(&packet) {
                Ok(_) => {
                    tracing::info!(reports = queue.len(), "PSK Reporter batch sent");
                    sequence = sequence.wrapping_add(queue.len() as u32);
                    state.sent += queue.len() as u64;
                    state.last_error = None;
                    queue.clear();
                }
                Err(error) => {
                    tracing::error!(
                        reports = queue.len(),
                        error = %error,
                        "PSK Reporter batch send failed"
                    );
                    state.last_error = Some(error.to_string())
                }
            }
            state.queued = queue.len();
            packets = packets.wrapping_add(1);
        }
        recent.retain(|_, seen| seen.elapsed() < Duration::from_secs(3_600));
        deadline = Instant::now() + interval;
    }
    status.lock().expect("PSK status lock").active = false;
    tracing::info!("PSK Reporter worker stopped");
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
            tuning: ReporterTuning::default(),
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

    #[test]
    fn production_config_and_tuning_defaults_are_operator_safe() {
        let config = ReporterConfig::production(" n1dq ", "FN42hn");
        assert_eq!(config.receiver_callsign, " n1dq ");
        assert_eq!(config.receiver_locator, "FN42hn");
        assert!(config.decoder_software.starts_with("QSONaut "));
        assert_eq!(config.destination, "report.pskreporter.info:4739");
        assert_eq!(config.tuning.batch_interval_secs, 300);
        assert_eq!(config.tuning.repeat_cache_secs, 300);
        assert_eq!(config.tuning.max_pending, 80);
    }

    #[test]
    fn packet_without_templates_still_has_valid_aligned_lengths() {
        let config = ReporterConfig::production("N0CALL", "AA00");
        let packet = encode_packet(&config, &[], 99, 123, 456, false);
        assert_eq!(&packet[..2], &[0, 10]);
        assert_eq!(
            u16::from_be_bytes([packet[2], packet[3]]) as usize,
            packet.len()
        );
        assert_eq!(packet.len() % 4, 0);
        assert_eq!(packet.len(), 52);
    }

    #[test]
    fn string_encoding_is_length_limited_and_padding_is_four_byte_aligned() {
        let mut output = Vec::new();
        push_string(&mut output, &"x".repeat(300));
        assert_eq!(output[0], 254);
        assert_eq!(output.len(), 255);
        pad4(&mut output);
        assert_eq!(output.len() % 4, 0);

        let start = output.len();
        output.extend_from_slice(&[0, 0, 0, 0]);
        set_length(&mut output, start);
        assert_eq!(
            u16::from_be_bytes([output[start + 2], output[start + 3]]),
            4
        );
    }

    #[test]
    fn report_sender_forwards_only_while_worker_channel_is_alive() {
        let (tx, rx) = mpsc::channel();
        let sender = ReportSender(tx);
        let report = ReceptionReport {
            sender_callsign: "K1ABC".into(),
            frequency_hz: 14_074_000,
            snr_db: -10,
            mode: "FT8".into(),
            sender_locator: "FN42".into(),
            received_at: 1,
        };
        assert!(sender.submit(report));
        assert!(matches!(rx.try_recv(), Ok(Command::Report(_))));
        drop(rx);
        assert!(!sender.submit(ReceptionReport {
            sender_callsign: "W1AW".into(),
            frequency_hz: 7_074_000,
            snr_db: 0,
            mode: "FT8".into(),
            sender_locator: "FN31".into(),
            received_at: 2,
        }));
    }

    #[test]
    fn encodes_multiple_reports_and_preserves_signed_snr_values() {
        let config = ReporterConfig::production("N0CALL", "AA00");
        let reports = [
            ReceptionReport {
                sender_callsign: "K1ABC".into(),
                frequency_hz: 1,
                snr_db: -128,
                mode: "FT8".into(),
                sender_locator: "FN42".into(),
                received_at: 10,
            },
            ReceptionReport {
                sender_callsign: "W1AW".into(),
                frequency_hz: u64::from(u32::MAX) + 1,
                snr_db: 127,
                mode: "FT4".into(),
                sender_locator: "FN31".into(),
                received_at: 20,
            },
        ];
        let packet = encode_packet(&config, &reports, u32::MAX, 9, 11, false);
        assert_eq!(
            u16::from_be_bytes([packet[2], packet[3]]) as usize,
            packet.len()
        );
        assert!(packet.windows(5).any(|window| window == [0, 0, 0, 0, 1]));
        assert!(packet.contains(&0x7F));
        assert!(packet.len().is_multiple_of(4));
    }

    #[test]
    fn reporter_marks_invalid_destination_as_inactive() {
        let reporter = Reporter::start(ReporterConfig {
            receiver_callsign: "N0CALL".into(),
            receiver_locator: "AA00".into(),
            decoder_software: "test".into(),
            destination: "not a valid destination".into(),
            tuning: ReporterTuning::default(),
        });
        for _ in 0..20 {
            if !reporter.status().active {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let status = reporter.status();
        assert!(!status.active);
        assert!(status.last_error.is_some());
    }

    #[test]
    fn session_identifiers_are_nonzero_and_callsign_sensitive() {
        let first = session_identifier("K1ABC");
        let second = session_identifier("W1AW");
        assert_ne!(first, 0);
        assert_ne!(first, second);
        assert!(unix_seconds() > 1_700_000_000);
    }
}
