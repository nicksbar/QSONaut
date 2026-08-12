// FT8 encoder: message → 79 channel tones.
// Pipeline: pack77 → add CRC14 → LDPC parity → colorder permute → tone map.

use crate::ft8::crc14::crc14_bits;
use crate::ft8::pack77::{pack77, Ft8Message};
use crate::ft8::params::{build_generator, COL_ORDER, COSTAS, GRAY_MAP, K, M, N, N_SYM};
use std::sync::OnceLock;

static GENERATOR: OnceLock<[[u8; K]; M]> = OnceLock::new();

fn generator() -> &'static [[u8; K]; M] {
    GENERATOR.get_or_init(build_generator)
}

/// Encode 91 information bits (77 msg + 14 CRC) to a 174-bit LDPC codeword.
pub fn encode174(info91: &[u8; 91]) -> [u8; N] {
    let gen = generator();
    let mut cw = [0u8; N];
    cw[..K].copy_from_slice(info91);
    for i in 0..M {
        let mut p = 0u8;
        for j in 0..K {
            p ^= info91[j] & gen[i][j];
        }
        cw[K + i] = p;
    }
    cw
}

/// Map a 174-bit codeword to 79 channel tones via colorder + Gray code + Costas sync.
pub fn codeword_to_tones(codeword: &[u8; N]) -> [u8; N_SYM] {
    // Apply column-order permutation: channel_bit[i] = codeword[colorder[i]]
    let mut channel = [0u8; N];
    for i in 0..N {
        channel[i] = codeword[COL_ORDER[i] as usize];
    }

    let mut tones = [0u8; N_SYM];
    // Symbol positions: 0..6 sync, 7..35 data(0..28), 36..42 sync, 43..71 data(29..57), 72..78 sync
    let mut data_idx = 0usize;
    for (sym, tone) in tones.iter_mut().enumerate().take(N_SYM) {
        let is_sync = sym < 7 || (36..43).contains(&sym) || sym >= 72;
        if is_sync {
            let costas_idx = if sym < 7 {
                sym
            } else if sym < 43 {
                sym - 36
            } else {
                sym - 72
            };
            *tone = COSTAS[costas_idx];
        } else {
            let b = data_idx * 3;
            let idx = (channel[b] as usize) << 2
                | (channel[b + 1] as usize) << 1
                | channel[b + 2] as usize;
            *tone = GRAY_MAP[idx];
            data_idx += 1;
        }
    }
    tones
}

/// Full encode: FT8 message → 79 channel tones.
pub fn message_to_tones(msg: &Ft8Message) -> Option<[u8; N_SYM]> {
    let bits77 = pack77(msg)?;
    let (cw, _) = encode_bits77(&bits77);
    Some(codeword_to_tones(&cw))
}

/// Encode 77 message bits: add CRC → 91 bits → LDPC → 174-bit codeword.
/// Returns (codeword, info91).
pub fn pack_and_encode(msg: &Ft8Message) -> Option<([u8; N], [u8; 91])> {
    let bits77 = pack77(msg)?;
    let (cw, info91) = encode_bits77(&bits77);
    Some((cw, info91))
}

pub fn encode_bits77(bits77: &[u8; 77]) -> ([u8; N], [u8; 91]) {
    let crc = crc14_bits(bits77);
    let mut info91 = [0u8; 91];
    info91[..77].copy_from_slice(bits77);
    info91[77..].copy_from_slice(&crc);
    (encode174(&info91), info91)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tone_count_and_sync_positions() {
        let msg = Ft8Message::Standard {
            call1: "CQ".into(),
            call2: "N0CALL".into(),
            report: "EN40".into(),
            ir: false,
        };
        if let Some(tones) = message_to_tones(&msg) {
            assert_eq!(tones.len(), N_SYM);
            // Verify Costas positions
            for (i, &t) in tones.iter().enumerate() {
                if i < 7 || (36..43).contains(&i) || i >= 72 {
                    let ci = if i < 7 {
                        i
                    } else if i < 43 {
                        i - 36
                    } else {
                        i - 72
                    };
                    assert_eq!(t, COSTAS[ci], "sync mismatch at sym {i}");
                }
                assert!(t < 8, "tone out of range at {i}");
            }
        }
    }

    #[test]
    fn parity_check_satisfied() {
        use crate::ft8::params::{M, NM, NRW};
        let msg = Ft8Message::FreeText("HELLO WORLD  ".into());
        if let Some((cw, _)) = pack_and_encode(&msg) {
            // Every parity check must be satisfied.
            for j in 0..M {
                let s: u32 = (0..NRW[j] as usize)
                    .map(|i| cw[NM[j][i] as usize] as u32)
                    .sum();
                assert_eq!(s % 2, 0, "parity check {j} failed");
            }
        }
    }
}
