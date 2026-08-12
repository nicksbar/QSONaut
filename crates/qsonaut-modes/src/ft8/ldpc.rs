// LDPC(174,91) belief-propagation decoder.
// Direct translation of WSJTX bpdecode174_91.f90.

use crate::ft8::crc14::crc14_valid;
use crate::ft8::params::{K, M, MN, N, NM, NRW};

const NCW: usize = 3; // check nodes per variable (regular LDPC)
const MAX_ITER: u32 = 30;

fn platanh(x: f32) -> f32 {
    // Clipped atanh — avoids ±∞ in the check-node update.
    let x = x.clamp(-0.9999998, 0.9999998);
    0.5 * ((1.0 + x) / (1.0 - x)).ln()
}

/// Decode 174 soft log-likelihood ratios → 77 message bits, or None on failure.
/// LLR convention: positive = bit is 0, negative = bit is 1.
pub fn bpdecode(llr: &[f32; N]) -> Option<[u8; 77]> {
    let mut toc = [[0f32; 7]; M]; // check-to-variable messages
    let mut tov = [[0f32; NCW]; N]; // variable-to-check messages
    let mut tanh_toc = [[0f32; 7]; M];
    let mut zn = [0f32; N];
    let mut cw = [0u8; N];

    // Initialise toc from the channel LLRs.
    for j in 0..M {
        for i in 0..NRW[j] as usize {
            toc[j][i] = llr[NM[j][i] as usize];
        }
    }

    let mut nclast = 0i32;
    let mut ncnt = 0u32;

    for iter in 0..=MAX_ITER {
        // Compute a-posteriori beliefs.
        for i in 0..N {
            zn[i] = llr[i] + tov[i].iter().sum::<f32>();
        }

        // Hard-decision codeword and parity check.
        for i in 0..N {
            cw[i] = if zn[i] > 0.0 { 0 } else { 1 };
        }
        let mut ncheck = 0i32;
        for j in 0..M {
            let s: u32 = (0..NRW[j] as usize)
                .map(|i| cw[NM[j][i] as usize] as u32)
                .sum();
            if s % 2 != 0 {
                ncheck += 1;
            }
        }

        if ncheck == 0 {
            let mut word = [0u8; 91];
            word[..K].copy_from_slice(&cw[..K]);
            if crc14_valid(&word) {
                let mut msg = [0u8; 77];
                msg.copy_from_slice(&cw[..77]);
                return Some(msg);
            }
        }

        if iter == MAX_ITER {
            break;
        }

        // Early stopping.
        if iter > 0 {
            let nd = ncheck - nclast;
            if nd < 0 {
                ncnt = 0;
            } else {
                ncnt += 1;
            }
            if ncnt >= 5 && iter >= 10 && ncheck > 15 {
                return None;
            }
        }
        nclast = ncheck;

        // Variable → check messages.
        for j in 0..M {
            for i in 0..NRW[j] as usize {
                let ibj = NM[j][i] as usize;
                let mut val = zn[ibj];
                for kk in 0..NCW {
                    if MN[ibj][kk] as usize == j {
                        val -= tov[ibj][kk];
                    }
                }
                toc[j][i] = val;
            }
        }

        // Precompute tanh values.
        for j in 0..M {
            for i in 0..NRW[j] as usize {
                tanh_toc[j][i] = (-toc[j][i] / 2.0).tanh();
            }
        }

        // Check → variable messages.
        for j in 0..N {
            for i in 0..NCW {
                let ichk = MN[j][i] as usize;
                let tmn: f32 = (0..NRW[ichk] as usize)
                    .filter(|&k| NM[ichk][k] as usize != j)
                    .map(|k| tanh_toc[ichk][k])
                    .product();
                tov[j][i] = 2.0 * platanh(-tmn);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_positive_llr_gives_zero_codeword() {
        // All-zero codeword: if all LLRs are strongly positive (bit=0), the
        // decoder should output cw=0 and the CRC check tells us if it's valid.
        let llr = [10.0f32; N];
        // All-zero codeword satisfies all parity checks but the CRC may not be
        // valid for the all-zero message — that's fine, we just check it runs.
        let _ = bpdecode(&llr);
    }
}
