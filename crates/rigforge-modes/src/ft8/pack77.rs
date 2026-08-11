// 77-bit FT8 message pack and unpack.
// Covers Type 1 (standard exchange), Type 0.0 (free text), and the
// special callsign tokens (CQ, QRZ, DE, CQ_NNN).
// Reference: WSJTX lib/77bit/packjt77.f90

const NTOKENS: u32 = 2_063_592;
const MAX22: u32 = 4_194_304; // 2^22
const MAXGRID4: u32 = 32_400;  // 18×18×10×10

static C1: &[u8] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";  // 37 chars
static C2: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";   // 36 chars
static C3: &[u8] = b"0123456789";                              // 10 chars
static C4: &[u8] = b" ABCDEFGHIJKLMNOPQRSTUVWXYZ";            // 27 chars

/// Decoded FT8 message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ft8Message {
    /// Standard exchange: call1 call2 report_or_grid
    Standard {
        call1: String,
        call2: String,
        report: String,
        ir: bool,  // R prefix before report
    },
    /// Free text (up to 13 chars)
    FreeText(String),
    /// Unknown / unparsed type
    Raw { i3: u8, n3: u8, bits: [u8; 77] },
}

impl std::fmt::Display for Ft8Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Standard { call1, call2, report, ir } => {
                if *ir {
                    write!(f, "{call1} {call2} R{report}")
                } else {
                    write!(f, "{call1} {call2} {report}")
                }
            }
            Self::FreeText(s) => write!(f, "{s}"),
            Self::Raw { i3, n3, .. } => write!(f, "<type {i3}.{n3}>"),
        }
    }
}

// ─── bit helpers ────────────────────────────────────────────────────────────

fn bits_to_u32(bits: &[u8], start: usize, len: usize) -> u32 {
    let mut v = 0u32;
    for i in 0..len {
        v = (v << 1) | bits[start + i] as u32;
    }
    v
}

fn u32_to_bits(v: u32, len: usize) -> Vec<u8> {
    (0..len).map(|i| ((v >> (len - 1 - i)) & 1) as u8).collect()
}

// ─── callsign decode / encode ───────────────────────────────────────────────

/// Decode a 28-bit packed callsign token into a string.
pub fn unpack28(n28: u32) -> String {
    if n28 == 0 { return "DE".into(); }
    if n28 == 1 { return "QRZ".into(); }
    if n28 == 2 { return "CQ".into(); }
    if n28 <= 1002 { return format!("CQ_{:03}", n28 - 3); }
    if n28 < NTOKENS {
        // CQ_AAAA suffix (4 chars from C4 alphabet)
        let n = n28 - 1003;
        let i4 =  n % 27;
        let n  =  n / 27;
        let i3 =  n % 27;
        let n  =  n / 27;
        let i2 =  n % 27;
        let i1 =  n / 27;
        let s: String = [C4[i1 as usize], C4[i2 as usize], C4[i3 as usize], C4[i4 as usize]]
            .iter()
            .map(|&b| b as char)
            .collect::<String>()
            .trim()
            .to_string();
        return format!("CQ_{s}");
    }
    // 22-bit hash — return placeholder since we have no hash table in this lib.
    if n28 < NTOKENS + MAX22 {
        return format!("<{:06X}>", n28 - NTOKENS);
    }
    // Standard callsign
    let n  = n28 - NTOKENS - MAX22;
    let i6 = (n % 27) as usize;
    let n  =  n / 27;
    let i5 = (n % 27) as usize;
    let n  =  n / 27;
    let i4 = (n % 27) as usize;
    let n  =  n / 27;
    let i3 = (n % 10) as usize;
    let n  =  n / 10;
    let i2 = (n % 36) as usize;
    let i1 = (n / 36) as usize;
    let s: String = [C1[i1], C2[i2], C3[i3], C4[i4], C4[i5], C4[i6]]
        .iter()
        .map(|&b| b as char)
        .collect::<String>()
        .trim()
        .to_string();
    s
}

/// Encode a callsign string to a 28-bit token.
/// Returns None if the callsign is not a standard 1-6 char amateur call.
pub fn pack28(call: &str) -> Option<u32> {
    let call = call.trim().to_ascii_uppercase();
    // Special tokens
    match call.as_str() {
        "DE"  => return Some(0),
        "QRZ" => return Some(1),
        "CQ"  => return Some(2),
        _ => {}
    }
    if call.starts_with("CQ_") {
        // CQ_NNN
        if let Ok(n) = call[3..].parse::<u32>() {
            if n <= 999 { return Some(n + 3); }
        }
    }
    // Standard callsign — pad so digit falls at c3 position (index 2).
    let raw = call.trim_end().to_string();
    if raw.len() > 6 { return None; }
    // Find position of the digit character.
    let digit_idx = raw.bytes().position(|b| b.is_ascii_digit())?;
    if digit_idx > 2 { return None; }
    let leading = 2 - digit_idx;
    let mut b = [b' '; 6];
    for (i, c) in raw.bytes().enumerate() {
        if leading + i < 6 { b[leading + i] = c; }
    }
    let i1 = C1.iter().position(|&x| x == b[0])?;
    let i2 = C2.iter().position(|&x| x == b[1])?;
    let i3 = C3.iter().position(|&x| x == b[2])?;
    let i4 = C4.iter().position(|&x| x == b[3])?;
    let i5 = C4.iter().position(|&x| x == b[4])?;
    let i6 = C4.iter().position(|&x| x == b[5])?;
    let n = NTOKENS + MAX22
        + i1 as u32 * 36 * 10 * 27 * 27 * 27
        + i2 as u32 *      10 * 27 * 27 * 27
        + i3 as u32 *           27 * 27 * 27
        + i4 as u32 *                27 * 27
        + i5 as u32 *                     27
        + i6 as u32;
    Some(n)
}

// ─── grid / report decode ───────────────────────────────────────────────────

fn decode_grid4(igrid4: u32) -> String {
    let lon_minor = igrid4 % 10;
    let lat_minor = (igrid4 / 10) % 10;
    let lon_major = (igrid4 / 100) % 18;
    let lat_major = igrid4 / 100 / 18;
    format!(
        "{}{}{}{}",
        (b'A' + lat_major as u8) as char,
        (b'A' + lon_major as u8) as char,
        lat_minor,
        lon_minor,
    )
}

fn encode_grid4(grid: &str) -> Option<u32> {
    let g = grid.to_ascii_uppercase();
    let b = g.as_bytes();
    if b.len() != 4 { return None; }
    let lat_major = (b[0] as u32).checked_sub(b'A' as u32)?;
    let lon_major = (b[1] as u32).checked_sub(b'A' as u32)?;
    let lat_minor = (b[2] as u32).checked_sub(b'0' as u32)?;
    let lon_minor = (b[3] as u32).checked_sub(b'0' as u32)?;
    if lat_major > 17 || lon_major > 17 || lat_minor > 9 || lon_minor > 9 { return None; }
    Some(lat_major * 18 * 100 + lon_major * 100 + lat_minor * 10 + lon_minor)
}

fn decode_report(igrid4: u32) -> String {
    let irpt = igrid4 - MAXGRID4;
    match irpt {
        1 => String::new(),
        2 => "RRR".into(),
        3 => "RR73".into(),
        4 => "73".into(),
        n => {
            let snr = n as i32 - 35;
            let snr = if snr > 50 { snr - 101 } else { snr };
            if snr >= 0 { format!("+{snr:02}") } else { format!("{snr:03}") }
        }
    }
}

fn encode_report(report: &str) -> Option<u32> {
    match report.trim() {
        "RRR"  => return Some(MAXGRID4 + 2),
        "RR73" => return Some(MAXGRID4 + 3),
        "73"   => return Some(MAXGRID4 + 4),
        ""     => return Some(MAXGRID4 + 1),
        s => {
            let s = s.trim_start_matches('R');
            let snr: i32 = s.parse().ok()?;
            let snr = snr.clamp(-50, 50);
            let irpt = snr + 35;
            return Some(MAXGRID4 + irpt as u32);
        }
    }
}

// ─── free-text ──────────────────────────────────────────────────────────────

// Allowed chars for free text (71 bits, 13 chars from 42-char alphabet)
static FT_ALPHA: &[u8] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ+-./?";

fn unpack_text77(bits: &[u8; 71]) -> String {
    let mut n: u128 = 0;
    for &b in bits.iter() {
        n = (n << 1) | b as u128;
    }
    let mut chars = Vec::new();
    for _ in 0..13 {
        let idx = (n % 42) as usize;
        chars.push(FT_ALPHA[idx] as char);
        n /= 42;
    }
    chars.iter().rev().collect::<String>().trim().to_string()
}

fn pack_text77(text: &str) -> Option<[u8; 71]> {
    let mut n: u128 = 0;
    let padded = format!("{:<13}", &text[..text.len().min(13)]);
    for ch in padded.chars() {
        let pos = FT_ALPHA.iter().position(|&b| b as char == ch.to_ascii_uppercase())?;
        n = n * 42 + pos as u128;
    }
    let mut bits = [0u8; 71];
    for i in (0..71).rev() {
        bits[i] = (n & 1) as u8;
        n >>= 1;
    }
    Some(bits)
}

// ─── public API ─────────────────────────────────────────────────────────────

/// Decode a 77-bit message into an `Ft8Message`.
pub fn unpack77(bits: &[u8; 77]) -> Ft8Message {
    let i3 = bits_to_u32(bits, 74, 3) as u8;
    let n3 = bits_to_u32(bits, 71, 3) as u8;

    match (i3, n3) {
        (0, 0) => {
            // Free text: bits 0..70 (71 bits)
            let mut b71 = [0u8; 71];
            b71.copy_from_slice(&bits[..71]);
            Ft8Message::FreeText(unpack_text77(&b71))
        }
        (1, _) | (2, _) => {
            // Standard: 2(b28,b1),b1,b15,b3
            let n28a  = bits_to_u32(bits,  0, 28);
            let ipa   = bits[28];
            let n28b  = bits_to_u32(bits, 29, 28);
            let ipb   = bits[57];
            let ir    = bits[58];
            let ig    = bits_to_u32(bits, 59, 15);

            let suffix_a = if ipa == 1 {
                if i3 == 1 { "/R" } else { "/P" }
            } else { "" };
            let suffix_b = if ipb == 1 {
                if i3 == 1 { "/R" } else { "/P" }
            } else { "" };

            let call1 = format!("{}{}", unpack28(n28a), suffix_a);
            let call2 = format!("{}{}", unpack28(n28b), suffix_b);
            let report = if ig <= MAXGRID4 {
                decode_grid4(ig)
            } else {
                decode_report(ig)
            };
            Ft8Message::Standard { call1, call2, report, ir: ir == 1 }
        }
        _ => Ft8Message::Raw { i3, n3, bits: *bits },
    }
}

/// Pack an `Ft8Message` into 77 bits.
pub fn pack77(msg: &Ft8Message) -> Option<[u8; 77]> {
    let mut bits = [0u8; 77];
    match msg {
        Ft8Message::FreeText(text) => {
            let b71 = pack_text77(text)?;
            bits[..71].copy_from_slice(&b71);
            // i3=0, n3=0 → bits 71..77 stay 0
        }
        Ft8Message::Standard { call1, call2, report, ir } => {
            let call1_base = call1.trim_end_matches("/R").trim_end_matches("/P");
            let call2_base = call2.trim_end_matches("/R").trim_end_matches("/P");
            let ipa: u8 = (call1.ends_with("/R") || call1.ends_with("/P")) as u8;
            let ipb: u8 = (call2.ends_with("/R") || call2.ends_with("/P")) as u8;
            let i3: u32 = if ipa == 1 && call1.ends_with("/P") { 2 } else { 1 };

            let n28a = pack28(call1_base)?;
            let n28b = pack28(call2_base)?;
            let ig = encode_grid4(report)
                .or_else(|| encode_report(report))?;

            let a = u32_to_bits(n28a, 28);
            bits[..28].copy_from_slice(&a);
            bits[28] = ipa;
            let b = u32_to_bits(n28b, 28);
            bits[29..57].copy_from_slice(&b);
            bits[57] = ipb;
            bits[58] = *ir as u8;
            let r = u32_to_bits(ig, 15);
            bits[59..74].copy_from_slice(&r);
            // n3=0, i3=001 → bits 71..74 = 000, bits 74..77 = 001
            bits[74] = 0; bits[75] = 0; bits[76] = i3 as u8 & 1;
        }
        Ft8Message::Raw { bits: b, .. } => {
            bits.copy_from_slice(b);
        }
    }
    Some(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpack28_special_tokens() {
        assert_eq!(unpack28(0), "DE");
        assert_eq!(unpack28(1), "QRZ");
        assert_eq!(unpack28(2), "CQ");
        assert_eq!(unpack28(3), "CQ_000");
        assert_eq!(unpack28(12), "CQ_009");
    }

    #[test]
    fn grid4_roundtrip() {
        for g in &["FN42", "JO22", "IO91", "AA00", "RR99"] {
            let enc = encode_grid4(g).expect("encode");
            assert!(enc <= MAXGRID4, "igrid4 overflow");
            let dec = decode_grid4(enc);
            assert_eq!(&dec, g, "grid4 roundtrip failed for {g}");
        }
    }

    #[test]
    fn cq_pack_roundtrip() {
        let msg = Ft8Message::Standard {
            call1: "CQ".into(),
            call2: "W1AW".into(),
            report: "FN31".into(),
            ir: false,
        };
        let bits = pack77(&msg).expect("pack");
        let decoded = unpack77(&bits);
        assert_eq!(decoded.to_string(), msg.to_string());
    }
}
