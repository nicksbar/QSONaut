// CRC-14 computed via the LFSR algorithm from WSJTX get_crc14.f90.
// Polynomial: x^14 + x^13 + x^10 + x^9 + x^8 + x^6 + x^4 + x^2 + x + 1

const POLY: [u8; 15] = [1, 1, 0, 0, 1, 1, 1, 0, 1, 0, 1, 0, 1, 1, 1];

/// Run the LFSR over `mc` bits.  Passing `len=77` bits + 14 zero bits produces
/// the CRC to append.  Passing a full 91-bit received word returns 0 if valid.
pub fn crc14(mc: &[u8]) -> u16 {
    let len = mc.len();
    assert!(len >= 15, "crc14: input too short");
    let mut r = [0u8; 15];
    r.copy_from_slice(&mc[..15]);
    let steps = len - 14; // i=0 ..= len-15, inclusive
    for i in 0..steps {
        r[14] = mc.get(i + 14).copied().unwrap_or(0);
        if r[0] == 1 {
            for (rk, pk) in r.iter_mut().zip(POLY.iter()) {
                *rk ^= *pk;
            }
        }
        r.rotate_left(1);
    }
    let mut v: u16 = 0;
    for &bit in r.iter().take(14) {
        v = (v << 1) | bit as u16;
    }
    v
}

/// Compute the 14-bit CRC of a 77-bit message and return it as an array of bits.
pub fn crc14_bits(msg77: &[u8; 77]) -> [u8; 14] {
    let mut mc = [0u8; 91];
    mc[..77].copy_from_slice(msg77);
    let v = crc14(&mc);
    let mut out = [0u8; 14];
    for (i, bit) in out.iter_mut().enumerate() {
        *bit = ((v >> (13 - i)) & 1) as u8;
    }
    out
}

/// Check CRC validity of a decoded 91-bit word (77 msg + 14 CRC).
pub fn crc14_valid(decoded91: &[u8; 91]) -> bool {
    crc14(decoded91) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_crc14() {
        let msg = [0u8; 77];
        let check = crc14_bits(&msg);
        let mut word = [0u8; 91];
        word[..14].copy_from_slice(&check); // wrong — place at end
                                            // Place at positions 77..91
        let mut full = [0u8; 91];
        full[77..].copy_from_slice(&check);
        assert!(crc14_valid(&full));
    }

    #[test]
    fn known_zero_message_crc() {
        // For an all-zero message, get_crc14 on 77+14 zeros should return 0.
        let zeros = [0u8; 91];
        assert_eq!(crc14(&zeros), 0);
    }
}
