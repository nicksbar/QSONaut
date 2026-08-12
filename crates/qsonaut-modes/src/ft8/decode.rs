// FT8 decode pipeline: LLR array → LDPC decode → unpack77 → Ft8Message.

use crate::ft8::ldpc::bpdecode;
use crate::ft8::pack77::{unpack77, Ft8Message};
use crate::ft8::params::{COL_ORDER, GRAY_INV, N, N_SYM};

/// A successfully decoded FT8 frame.
#[derive(Debug, Clone)]
pub struct Ft8Decoded {
    pub message: Ft8Message,
    /// Audio baseband frequency of this signal (Hz).
    pub freq_hz: f32,
    /// Timing offset from the 15-second period start (seconds).
    pub time_offset_s: f32,
    /// Signal-to-noise ratio estimate (dB).
    pub snr_db: f32,
}

/// Decode 174 soft LLR values to a message.
///
/// `llr[i]` is the log-likelihood ratio for the i-th channel bit in
/// *transmission order* (data symbols only, excluding sync).
/// Positive LLR ⇒ bit is 0; negative ⇒ bit is 1.
pub fn decode_llr(channel_llr: &[f32; N]) -> Option<Ft8Decoded> {
    decode_llr_with_meta(channel_llr, 0.0, 0.0, 0.0)
}

/// Decode with frequency / timing metadata attached.
pub fn decode_llr_with_meta(
    channel_llr: &[f32; N],
    freq_hz: f32,
    time_offset_s: f32,
    snr_db: f32,
) -> Option<Ft8Decoded> {
    // Permute channel LLRs → codeword LLRs using inverse colorder.
    let mut codeword_llr = [0f32; N];
    for i in 0..N {
        codeword_llr[COL_ORDER[i] as usize] = channel_llr[i];
    }

    let bits77 = bpdecode(&codeword_llr)?;
    let message = unpack77(&bits77);
    Some(Ft8Decoded {
        message,
        freq_hz,
        time_offset_s,
        snr_db,
    })
}

/// Convert per-symbol soft magnitudes (8 tones × 79 symbols) to 174 channel LLRs.
///
/// `s8[sym][tone]` is the non-negative power/magnitude at that tone.
/// Only data symbol positions (excluding the 3 Costas blocks) are processed.
pub fn symbol_magnitudes_to_llr(s8: &[[f32; 8]; N_SYM]) -> [f32; N] {
    let mut llr = [0f32; N];
    let mut data_idx = 0usize;
    for sym in 0..N_SYM {
        let is_sync = sym < 7 || (sym >= 36 && sym < 43) || sym >= 72;
        if is_sync {
            continue;
        }

        // For each of the 3 channel bits in this symbol, compute a soft LLR.
        // LLR for bit b: sum of mag over all tones where bit b=0, minus sum where bit b=1.
        for bit in 0..3 {
            let mut sum0 = 0f32;
            let mut sum1 = 0f32;
            for tone in 0..8u8 {
                // Map tone back to codeword 3-bit pattern via inverse Gray code.
                let code = GRAY_INV[tone as usize];
                let this_bit = (code >> (2 - bit)) & 1;
                if this_bit == 0 {
                    sum0 += s8[sym][tone as usize];
                } else {
                    sum1 += s8[sym][tone as usize];
                }
            }
            let ratio = (sum0 + 1e-9) / (sum1 + 1e-9);
            llr[data_idx * 3 + bit] = ratio.ln();
        }
        data_idx += 1;
    }
    llr
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ft8::encode::{codeword_to_tones, pack_and_encode};
    use crate::ft8::pack77::Ft8Message;

    #[test]
    fn encode_decode_roundtrip() {
        let msg = Ft8Message::Standard {
            call1: "W1AW".into(),
            call2: "K1JT".into(),
            report: "FN20".into(),
            ir: false,
        };
        let (codeword, _) = pack_and_encode(&msg).expect("encode");
        let tones = codeword_to_tones(&codeword);

        // Build perfect soft symbol magnitudes (spike on the correct tone).
        let mut s8 = [[0f32; 8]; N_SYM];
        for (sym, &tone) in tones.iter().enumerate() {
            s8[sym][tone as usize] = 100.0;
        }

        let channel_llr = symbol_magnitudes_to_llr(&s8);
        let decoded = decode_llr(&channel_llr).expect("decode");
        assert_eq!(decoded.message.to_string(), msg.to_string());
    }
}
