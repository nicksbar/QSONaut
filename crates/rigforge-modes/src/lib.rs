//! Digital mode engines (FT8/FT4/CW/etc.) live here.
//! M0 note: placeholder until DSP pipeline milestones are complete.

#[derive(Debug, Clone, Copy)]
pub enum ModeEngine {
    Ft8,
    Ft4,
    Cw,
}
