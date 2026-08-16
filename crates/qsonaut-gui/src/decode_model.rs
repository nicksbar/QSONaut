use std::collections::{HashMap, HashSet, VecDeque};

use super::{modes::exchange::parse_message, WorkspaceMode};

#[cfg(test)]
use super::FT8_EARLY_DECODE_S;

/// Gates a slot-based decoder until a complete receive period has been observed.
#[derive(Debug, Default)]
pub(super) struct Ft8SlotGate {
    observed_period: Option<u64>,
    ready_after_boundary: bool,
    decoded_period: Option<u64>,
}

impl Ft8SlotGate {
    #[cfg(test)]
    pub(super) fn observe(
        &mut self,
        period: u64,
        slot_position_s: f64,
        buffer_ready: bool,
    ) -> bool {
        self.observe_at(period, slot_position_s, FT8_EARLY_DECODE_S, buffer_ready)
    }

    pub(super) fn observe_at(
        &mut self,
        period: u64,
        slot_position_s: f64,
        decode_at_s: f64,
        buffer_ready: bool,
    ) -> bool {
        match self.observed_period {
            None => {
                self.observed_period = Some(period);
                false
            }
            Some(observed) if observed != period => {
                self.observed_period = Some(period);
                self.ready_after_boundary = true;
                self.decoded_period = None;
                false
            }
            Some(_)
                if self.ready_after_boundary
                    && self.decoded_period != Some(period)
                    && slot_position_s >= decode_at_s
                    && buffer_ready =>
            {
                self.decoded_period = Some(period);
                true
            }
            Some(_) => false,
        }
    }

    pub(super) fn reset(&mut self) {
        self.observed_period = None;
        self.ready_after_boundary = false;
        self.decoded_period = None;
    }

    pub(super) fn skip(&mut self, period: u64) {
        if self.observed_period == Some(period) && self.ready_after_boundary {
            self.decoded_period = Some(period);
        }
    }
}

/// Gates modes that decode only after a full period boundary.
#[derive(Debug, Default)]
pub(super) struct DigitalSlotGate {
    observed_period: Option<u64>,
}

impl DigitalSlotGate {
    pub(super) fn boundary(&mut self, period: u64, buffer_ready: bool) -> bool {
        match self.observed_period {
            None => {
                self.observed_period = Some(period);
                false
            }
            Some(observed) if observed != period => {
                self.observed_period = Some(period);
                buffer_ready
            }
            Some(_) => false,
        }
    }

    pub(super) fn reset(&mut self) {
        self.observed_period = None;
    }
}

/// An immutable FT8 decode event. `audio_frequency_hz` is a baseband offset,
/// not a persistent channel or an absolute RF frequency.
#[derive(Debug, Clone)]
pub(super) struct Ft8DecodeEntry {
    pub(super) period: u64,
    pub(super) utc: String,
    pub(super) snr_db: i8,
    pub(super) dt_s: f32,
    pub(super) freq_hz: u32,
    pub(super) message: String,
    pub(super) is_cq: bool,
}

/// An immutable native-mode decode event using mfsk-core's common result data.
#[derive(Debug, Clone)]
pub(super) struct DigitalDecodeEntry {
    pub(super) mode: WorkspaceMode,
    pub(super) period: u64,
    pub(super) utc: String,
    pub(super) snr_db: f32,
    pub(super) dt_s: f32,
    pub(super) freq_hz: u32,
    pub(super) message: String,
}

#[derive(Debug)]
pub(super) struct PendingFt8Decode {
    pub(super) samples: Vec<f32>,
    pub(super) utc: String,
    pub(super) period: u64,
    pub(super) deep_decode: bool,
    pub(super) alignment_s: f32,
}

#[derive(Debug, Default)]
pub(super) struct DecodeActivityStats {
    pub(super) latest_cycle: usize,
    pub(super) average_per_cycle: f32,
    pub(super) cq_this_cycle: usize,
    pub(super) unique_stations: usize,
    pub(super) most_heard: Option<(String, usize)>,
    pub(super) median_snr: Option<i8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperatorCallHit {
    DirectedToMe,
    Mentioned,
}

pub(super) fn operator_call_hit(message: &str, callsign: &str) -> Option<OperatorCallHit> {
    let callsign = callsign
        .trim_matches(|c| c == '<' || c == '>')
        .trim()
        .to_ascii_uppercase();
    if callsign.is_empty() || callsign == "N0CALL" {
        return None;
    }
    if parse_message(message).is_some_and(|parsed| parsed.directed_to(&callsign)) {
        return Some(OperatorCallHit::DirectedToMe);
    }
    message
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|c: char| c == '<' || c == '>' || c.is_ascii_punctuation())
                .to_ascii_uppercase()
        })
        .any(|token| token == callsign)
        .then_some(OperatorCallHit::Mentioned)
}

pub(super) fn ft8_activity_stats(log: &[Ft8DecodeEntry]) -> DecodeActivityStats {
    activity_stats(log.iter().map(|entry| {
        (
            entry.period,
            entry.snr_db,
            entry.is_cq,
            entry.message.as_str(),
        )
    }))
}

pub(super) fn digital_activity_stats(
    log: &VecDeque<DigitalDecodeEntry>,
    mode: WorkspaceMode,
) -> DecodeActivityStats {
    activity_stats(log.iter().filter(|entry| entry.mode == mode).map(|entry| {
        (
            entry.period,
            entry.snr_db.round() as i8,
            entry.message.starts_with("CQ "),
            entry.message.as_str(),
        )
    }))
}

fn activity_stats<'a>(
    entries: impl IntoIterator<Item = (u64, i8, bool, &'a str)>,
) -> DecodeActivityStats {
    let entries: Vec<_> = entries.into_iter().collect();
    let mut per_cycle: HashMap<u64, usize> = HashMap::new();
    let mut station_counts: HashMap<String, usize> = HashMap::new();
    let mut stations = HashSet::new();
    let mut snrs = Vec::with_capacity(entries.len());

    for (period, snr_db, _, message) in &entries {
        *per_cycle.entry(*period).or_default() += 1;
        snrs.push(*snr_db);
        if let Some(message) = parse_message(message) {
            stations.insert(message.from.clone());
            *station_counts.entry(message.from).or_default() += 1;
        }
    }

    let latest_period = entries.iter().map(|entry| entry.0).max();
    let latest_cycle = latest_period
        .and_then(|period| per_cycle.get(&period).copied())
        .unwrap_or_default();
    let cq_this_cycle = latest_period.map_or(0, |period| {
        entries
            .iter()
            .filter(|entry| entry.0 == period && entry.2)
            .count()
    });
    let average_per_cycle = if per_cycle.is_empty() {
        0.0
    } else {
        entries.len() as f32 / per_cycle.len() as f32
    };
    let most_heard = station_counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)));
    snrs.sort_unstable();
    let median_snr = snrs.get(snrs.len() / 2).copied();

    DecodeActivityStats {
        latest_cycle,
        average_per_cycle,
        cq_this_cycle,
        unique_stations: stations.len(),
        most_heard,
        median_snr,
    }
}
