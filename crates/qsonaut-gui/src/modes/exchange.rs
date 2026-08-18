use serde::{Deserialize, Serialize};

pub const SLOT_SECONDS: f64 = 15.0;
pub const AUDIO_START_SECONDS: f64 = 0.5;
pub const DEFAULT_PTT_LEAD_SECONDS: f64 = 0.20;
// Match WSJT-X's practical next-slot reply window.
pub const REPLY_DEADLINE_SECONDS: f64 = 2.0;
pub const MAX_ATTEMPTS_PER_EXCHANGE: u8 = 6;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoTxStopPolicy {
    #[default]
    Continuous,
    AfterNextTx,
    AfterCurrentQso,
}

impl AutoTxStopPolicy {
    pub const ALL: [Self; 3] = [Self::Continuous, Self::AfterNextTx, Self::AfterCurrentQso];

    pub fn label(self) -> &'static str {
        match self {
            Self::Continuous => "Keep running",
            Self::AfterNextTx => "Stop after next TX",
            Self::AfterCurrentQso => "Stop after current QSO",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoReplyPolicy {
    #[default]
    First,
    Strongest,
    Weakest,
    Closest,
}

impl AutoReplyPolicy {
    pub const ALL: [Self; 4] = [Self::First, Self::Strongest, Self::Weakest, Self::Closest];

    pub fn label(self) -> &'static str {
        match self {
            Self::First => "First decoded",
            Self::Strongest => "Strongest",
            Self::Weakest => "Weakest",
            Self::Closest => "Closest to RX",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exchange {
    Grid(String),
    Report(i8),
    RogerReport(i8),
    Roger,
    Roger73,
    SeventyThree,
    Other(String),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMessage {
    pub raw: String,
    pub from: String,
    pub to: Option<String>,
    pub is_cq: bool,
    pub exchange: Exchange,
}

impl ParsedMessage {
    pub fn directed_to(&self, callsign: &str) -> bool {
        self.to
            .as_deref()
            .is_some_and(|to| callsign_eq(to, callsign))
    }

    pub fn directed_away_from(&self, callsign: &str) -> bool {
        !self.is_cq
            && self
                .to
                .as_deref()
                .is_some_and(|to| !callsign_eq(to, callsign))
    }
}

pub fn parse_message(message: &str) -> Option<ParsedMessage> {
    let raw = message.trim().to_ascii_uppercase();
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let first = *tokens.first()?;

    if first == "CQ" || first == "QRZ" {
        let from = tokens
            .iter()
            .skip(1)
            .copied()
            .find(|token| is_probable_callsign(token))?
            .to_string();
        let exchange = tokens
            .last()
            .filter(|token| **token != from)
            .map_or(Exchange::None, |token| parse_exchange(token));
        return Some(ParsedMessage {
            raw,
            from,
            to: None,
            is_cq: true,
            exchange,
        });
    }

    if tokens.len() < 2 || !is_probable_callsign(tokens[0]) || !is_probable_callsign(tokens[1]) {
        return None;
    }

    // WSJT standard messages are DESTINATION SOURCE EXCHANGE.
    let from = tokens[1].to_string();
    let to = tokens[0].to_string();
    let exchange = tokens
        .get(2)
        .map_or(Exchange::None, |token| parse_exchange(token));
    Some(ParsedMessage {
        raw,
        from,
        to: Some(to),
        is_cq: false,
        exchange,
    })
}

fn parse_exchange(token: &str) -> Exchange {
    let token = token.trim().to_ascii_uppercase();
    match token.as_str() {
        "RRR" => Exchange::Roger,
        "RR73" => Exchange::Roger73,
        "73" => Exchange::SeventyThree,
        _ if is_grid(&token) => Exchange::Grid(token),
        _ if token.starts_with('R') => token[1..]
            .parse::<i8>()
            .map(Exchange::RogerReport)
            .unwrap_or(Exchange::Other(token)),
        _ => token
            .parse::<i8>()
            .map(Exchange::Report)
            .unwrap_or(Exchange::Other(token)),
    }
}

pub fn callsign_eq(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        value
            .trim_matches(|c| c == '<' || c == '>')
            .to_ascii_uppercase()
    };
    normalize(left) == normalize(right)
}

pub fn is_probable_callsign(token: &str) -> bool {
    let token = token
        .trim_matches(|c| c == '<' || c == '>')
        .to_ascii_uppercase();
    if token.len() < 3 || is_grid(&token) {
        return false;
    }
    if matches!(
        token.as_str(),
        "DX" | "TEST" | "POTA" | "SOTA" | "NA" | "EU" | "AS" | "AF" | "SA" | "OC"
    ) {
        return false;
    }
    token.chars().all(|c| c.is_ascii_alphanumeric() || c == '/')
        && token.chars().any(|c| c.is_ascii_alphabetic())
        && token.chars().any(|c| c.is_ascii_digit())
}

fn is_grid(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.len(), 4 | 6)
        && matches!(bytes[0], b'A'..=b'R')
        && matches!(bytes[1], b'A'..=b'R')
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && (bytes.len() == 4
            || (matches!(bytes[4], b'A'..=b'X') && matches!(bytes[5], b'A'..=b'X')))
}

#[derive(Debug, Clone)]
pub struct ReplyCandidate {
    pub index: usize,
    pub snr_db: i8,
    pub freq_hz: u32,
    pub parsed: ParsedMessage,
}

pub fn select_candidate(
    candidates: impl IntoIterator<Item = ReplyCandidate>,
    policy: AutoReplyPolicy,
    rx_tone_hz: u32,
) -> Option<ReplyCandidate> {
    let mut candidates: Vec<_> = candidates.into_iter().collect();
    candidates.sort_by(|left, right| match policy {
        AutoReplyPolicy::First => left.index.cmp(&right.index),
        AutoReplyPolicy::Strongest => right
            .snr_db
            .cmp(&left.snr_db)
            .then_with(|| left.index.cmp(&right.index)),
        AutoReplyPolicy::Weakest => left
            .snr_db
            .cmp(&right.snr_db)
            .then_with(|| left.index.cmp(&right.index)),
        AutoReplyPolicy::Closest => left
            .freq_hz
            .abs_diff(rx_tone_hz)
            .cmp(&right.freq_hz.abs_diff(rx_tone_hz))
            .then_with(|| left.index.cmp(&right.index)),
    });
    candidates.into_iter().next()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QsoStage {
    Calling,
    GridSent,
    ReportSent,
    RogerReportSent,
    FinalSent,
    Complete,
}

pub fn should_finalize_after_tx(stage: QsoStage) -> bool {
    stage == QsoStage::FinalSent
}

#[derive(Debug, Clone)]
pub struct QsoSession {
    pub target: String,
    pub stage: QsoStage,
    pub tx_parity: u8,
    pub started_period: u64,
    pub last_rx_period: u64,
    pub remote_grid: Option<String>,
    pub report_sent: Option<i8>,
    pub report_received: Option<i8>,
    pub tx_attempts: u8,
}

impl QsoSession {
    pub fn start(target: String, rx_period: u64) -> Self {
        Self {
            target,
            stage: QsoStage::Calling,
            tx_parity: ((rx_period + 1) % 2) as u8,
            started_period: rx_period,
            last_rx_period: rx_period,
            remote_grid: None,
            report_sent: None,
            report_received: None,
            tx_attempts: 0,
        }
    }

    pub fn response_to(
        &mut self,
        parsed: &ParsedMessage,
        my_call: &str,
        my_grid: &str,
        received_snr: i8,
        rx_period: u64,
    ) -> Option<String> {
        if self.stage == QsoStage::Complete {
            return None;
        }
        if !callsign_eq(&parsed.from, &self.target)
            || (!parsed.is_cq && !parsed.directed_to(my_call))
        {
            return None;
        }

        self.last_rx_period = rx_period;
        self.tx_parity = ((rx_period + 1) % 2) as u8;
        let target = self.target.to_ascii_uppercase();
        let my_call = my_call.trim().to_ascii_uppercase();
        let snr = received_snr.clamp(-50, 49);

        match &parsed.exchange {
            Exchange::Grid(grid) => self.remote_grid = Some(grid.clone()),
            Exchange::Report(report) | Exchange::RogerReport(report) => {
                self.report_received = Some(*report)
            }
            _ => {}
        }

        let previous_stage = self.stage;
        let (message, next_stage) = match parsed.exchange {
            Exchange::Grid(_) | Exchange::None if parsed.is_cq => (
                format!("{target} {my_call} {}", my_grid.trim().to_ascii_uppercase()),
                QsoStage::GridSent,
            ),
            Exchange::Grid(_) | Exchange::None => (
                {
                    self.report_sent = Some(snr);
                    format!("{target} {my_call} {snr:+03}")
                },
                QsoStage::ReportSent,
            ),
            Exchange::Report(_) => (
                {
                    self.report_sent = Some(snr);
                    format!("{target} {my_call} R{snr:+03}")
                },
                QsoStage::RogerReportSent,
            ),
            Exchange::RogerReport(_) => (format!("{target} {my_call} RR73"), QsoStage::FinalSent),
            Exchange::Roger => (format!("{target} {my_call} 73"), QsoStage::FinalSent),
            Exchange::Roger73 | Exchange::SeventyThree => {
                self.stage = QsoStage::Complete;
                return None;
            }
            Exchange::Other(_) => return None,
        };
        if next_stage != previous_stage {
            self.tx_attempts = 0;
        }
        self.stage = next_stage;
        Some(message)
    }
}

pub fn next_reply_period(now_seconds: f64, source_period: u64, ptt_lead_seconds: f64) -> u64 {
    let current = (now_seconds / SLOT_SECONDS).floor() as u64;
    let period_position = now_seconds - current as f64 * SLOT_SECONDS;
    let reply_parity = (source_period + 1) % 2;
    if current == source_period + 1
        && current % 2 == reply_parity
        && period_position <= REPLY_DEADLINE_SECONDS
    {
        current
    } else {
        next_tx_period(now_seconds, Some(reply_parity as u8), ptt_lead_seconds)
    }
}

pub fn next_tx_period(now_seconds: f64, required_parity: Option<u8>, ptt_lead_seconds: f64) -> u64 {
    let current = (now_seconds / SLOT_SECONDS).floor() as u64;
    for candidate in current..=current + 4 {
        if required_parity.is_some_and(|parity| candidate % 2 != u64::from(parity % 2)) {
            continue;
        }
        let ptt_start = candidate as f64 * SLOT_SECONDS + AUDIO_START_SECONDS - ptt_lead_seconds;
        if now_seconds < ptt_start {
            return candidate;
        }
    }
    current + 5
}

pub fn should_retry_after_decode(last_tx_period: Option<u64>, decoded_period: u64) -> bool {
    last_tx_period.is_some_and(|last_tx| decoded_period == last_tx.saturating_add(1))
}

pub fn should_repeat_cq(
    auto_sequence: bool,
    last_tx_was_cq: bool,
    last_tx_period: Option<u64>,
    completed_rx_period: Option<u64>,
) -> bool {
    auto_sequence
        && last_tx_was_cq
        && completed_rx_period
            .is_some_and(|period| should_retry_after_decode(last_tx_period, period))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(index: usize, snr_db: i8, freq_hz: u32, text: &str) -> ReplyCandidate {
        ReplyCandidate {
            index,
            snr_db,
            freq_hz,
            parsed: parse_message(text).expect("parsed message"),
        }
    }

    #[test]
    fn parses_standard_message_roles_and_cq_qualifiers() {
        let directed = parse_message("N7UF K1ABC -10").unwrap();
        assert_eq!(directed.to.as_deref(), Some("N7UF"));
        assert_eq!(directed.from, "K1ABC");
        assert!(directed.directed_to("N7UF"));
        assert!(!directed.directed_away_from("N7UF"));

        let other = parse_message("W9XYZ K1ABC -10").unwrap();
        assert!(other.directed_away_from("N7UF"));

        let cq = parse_message("CQ POTA K1ABC FN42").unwrap();
        assert!(cq.is_cq);
        assert_eq!(cq.from, "K1ABC");
        assert_eq!(cq.exchange, Exchange::Grid("FN42".into()));
    }

    #[test]
    fn ranks_reply_candidates_deterministically() {
        let candidates = || {
            vec![
                candidate(3, -12, 800, "N7UF K1AAA CN84"),
                candidate(4, -3, 1600, "N7UF K2BBB CN85"),
                candidate(5, -20, 1510, "N7UF K3CCC CN86"),
            ]
        };
        assert_eq!(
            select_candidate(candidates(), AutoReplyPolicy::First, 1500)
                .unwrap()
                .parsed
                .from,
            "K1AAA"
        );
        assert_eq!(
            select_candidate(candidates(), AutoReplyPolicy::Strongest, 1500)
                .unwrap()
                .parsed
                .from,
            "K2BBB"
        );
        assert_eq!(
            select_candidate(candidates(), AutoReplyPolicy::Weakest, 1500)
                .unwrap()
                .parsed
                .from,
            "K3CCC"
        );
        assert_eq!(
            select_candidate(candidates(), AutoReplyPolicy::Closest, 1500)
                .unwrap()
                .parsed
                .from,
            "K3CCC"
        );
    }

    #[test]
    fn preserves_reply_parity_after_missing_immediate_slot() {
        // Decode came from even period 100, so replies must use odd periods.
        assert_eq!(
            next_tx_period(101.0 * 15.0 + 0.10, Some(1), DEFAULT_PTT_LEAD_SECONDS),
            101
        );
        assert_eq!(
            next_tx_period(101.0 * 15.0 + 1.00, Some(1), DEFAULT_PTT_LEAD_SECONDS),
            103
        );
    }

    #[test]
    fn scheduler_honors_long_ptt_lead() {
        assert_eq!(next_tx_period(100.0 * 15.0 + 13.0, Some(1), 1.5), 101);
        assert_eq!(next_tx_period(100.0 * 15.0 + 14.2, Some(1), 1.5), 103);
    }

    #[test]
    fn retries_only_after_the_receive_slot_following_tx() {
        assert!(!should_retry_after_decode(Some(101), 101));
        assert!(should_retry_after_decode(Some(101), 102));
        assert!(!should_retry_after_decode(Some(101), 103));
        assert!(!should_retry_after_decode(None, 102));
    }

    #[test]
    fn cq_repeats_only_after_its_opposite_receive_period() {
        assert!(should_repeat_cq(true, true, Some(100), Some(101)));
        assert!(!should_repeat_cq(false, true, Some(100), Some(101)));
        assert!(!should_repeat_cq(true, false, Some(100), Some(101)));
        assert!(!should_repeat_cq(true, true, Some(100), Some(102)));
        assert!(!should_repeat_cq(true, true, None, Some(101)));
    }

    #[test]
    fn reply_uses_current_slot_after_strict_ptt_deadline() {
        let source_period = 100;
        assert_eq!(
            next_reply_period(
                101.0 * SLOT_SECONDS + 0.85,
                source_period,
                DEFAULT_PTT_LEAD_SECONDS,
            ),
            101
        );
        assert_eq!(
            next_reply_period(
                101.0 * SLOT_SECONDS + 1.75,
                source_period,
                DEFAULT_PTT_LEAD_SECONDS,
            ),
            101
        );
        assert_eq!(
            next_reply_period(
                101.0 * SLOT_SECONDS + 2.00,
                source_period,
                DEFAULT_PTT_LEAD_SECONDS,
            ),
            101
        );
    }

    #[test]
    fn reply_deadline_boundary_is_inclusive_then_rolls_forward() {
        let source_period = 220;
        let eps = 1e-6;
        assert_eq!(
            next_reply_period(
                221.0 * SLOT_SECONDS + (REPLY_DEADLINE_SECONDS - eps),
                source_period,
                DEFAULT_PTT_LEAD_SECONDS,
            ),
            221
        );
        assert_eq!(
            next_reply_period(
                221.0 * SLOT_SECONDS + REPLY_DEADLINE_SECONDS + 0.001,
                source_period,
                DEFAULT_PTT_LEAD_SECONDS,
            ),
            223
        );
    }

    #[test]
    fn next_tx_period_rejects_candidate_once_ptt_window_opens() {
        let candidate = 301u64;
        let ptt_lead = 0.25;
        let ptt_start = candidate as f64 * SLOT_SECONDS + AUDIO_START_SECONDS - ptt_lead;

        assert_eq!(
            next_tx_period(ptt_start - 0.001, Some((candidate % 2) as u8), ptt_lead),
            candidate
        );
        assert_eq!(
            next_tx_period(ptt_start, Some((candidate % 2) as u8), ptt_lead),
            candidate + 2
        );
    }

    #[test]
    fn retry_guard_is_saturating_and_never_wraps_period_math() {
        assert!(should_retry_after_decode(Some(u64::MAX), u64::MAX));
        assert!(!should_retry_after_decode(Some(u64::MAX), u64::MAX - 1));
    }

    #[test]
    fn advances_standard_qso_exchange() {
        let mut session = QsoSession::start("K1ABC".into(), 100);
        let grid = parse_message("N7UF K1ABC FN42").unwrap();
        assert_eq!(
            session
                .response_to(&grid, "N7UF", "CN84", -7, 100)
                .as_deref(),
            Some("K1ABC N7UF -07")
        );
        let report = parse_message("N7UF K1ABC -12").unwrap();
        assert_eq!(
            session
                .response_to(&report, "N7UF", "CN84", -9, 102)
                .as_deref(),
            Some("K1ABC N7UF R-09")
        );
        let roger = parse_message("N7UF K1ABC R-08").unwrap();
        assert_eq!(
            session
                .response_to(&roger, "N7UF", "CN84", -5, 104)
                .as_deref(),
            Some("K1ABC N7UF RR73")
        );
        let done = parse_message("N7UF K1ABC RR73").unwrap();
        assert_eq!(session.response_to(&done, "N7UF", "CN84", -5, 106), None);
        assert_eq!(session.stage, QsoStage::Complete);
        assert_eq!(session.started_period, 100);
        assert_eq!(session.last_rx_period, 106);
        assert_eq!(session.remote_grid.as_deref(), Some("FN42"));
        assert_eq!(session.report_sent, Some(-9));
        assert_eq!(session.report_received, Some(-8));
        assert_eq!(session.tx_attempts, 0);
    }

    #[test]
    fn new_exchange_step_resets_attempt_counter_but_repeats_do_not() {
        let mut session = QsoSession::start("K1ABC".into(), 100);
        let grid = parse_message("N7UF K1ABC FN42").unwrap();
        session.response_to(&grid, "N7UF", "CN84", -7, 100);
        session.tx_attempts = 5;

        session.response_to(&grid, "N7UF", "CN84", -8, 102);
        assert_eq!(session.tx_attempts, 5);

        let report = parse_message("N7UF K1ABC -12").unwrap();
        session.response_to(&report, "N7UF", "CN84", -9, 104);
        assert_eq!(session.tx_attempts, 0);
    }

    #[test]
    fn final_outbound_exchange_completes_the_local_qso() {
        assert!(should_finalize_after_tx(QsoStage::FinalSent));
        assert!(!should_finalize_after_tx(QsoStage::RogerReportSent));
        assert!(!should_finalize_after_tx(QsoStage::Complete));
    }
}
