use super::super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ft8SeqState {
    Idle,
    CqArmed,
    ReplyArmed,
    TxQueued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ft8TxQueuePolicy {
    Standard,
    ReplyAsap,
    NextSlotOnly,
}

#[derive(Debug)]
pub(crate) struct PendingManualFt8Reply {
    pub(crate) compose: String,
    pub(crate) target: String,
    pub(crate) session: QsoSession,
    pub(crate) freq_hz: u32,
    pub(crate) source_period: u64,
    pub(crate) move_tx_to_remote: bool,
}

impl Ft8SeqState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::CqArmed => "CQ ARMED",
            Self::ReplyArmed => "REPLY ARMED",
            Self::TxQueued => "TX QUEUED",
        }
    }
}
