//! KSUIDs (K-Sortable Unique IDentifiers), as the TypeScript SDK encodes them.
//!
//! A KSUID is 20 bytes: a big-endian `u32` timestamp in seconds since the
//! KSUID epoch (2014-05-13T16:53:20Z), then 16 payload bytes. Its string form
//! is 27 base62 characters (`0-9A-Za-z`), left-padded with `0`.

use sha2::{Digest, Sha256};

/// KSUID epoch in Unix milliseconds (14e11).
const EPOCH_MS: u64 = 1_400_000_000_000;
/// Largest timestamp a KSUID can carry, in Unix milliseconds.
const MAX_TIME_MS: u64 = 1_000 * (u32::MAX as u64) + EPOCH_MS;
const BYTE_LENGTH: usize = 20;
const STRING_LENGTH: usize = 27;
const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Unix epoch of 2024-01-01T00:00:00Z in milliseconds, the base of derived
/// session IDs.
const EPOCH_2024_MS: u64 = 1_704_067_200_000;
/// One year in milliseconds. The derivation offset is taken modulo this, which
/// never applies because a `u32` is smaller (kept for cross-SDK identity).
const ONE_YEAR_MS: u64 = 365 * 24 * 60 * 60 * 1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Ksuid([u8; BYTE_LENGTH]);

impl Ksuid {
    /// Builds a KSUID from a Unix time in milliseconds and a payload. Returns
    /// `None` when the time is outside the KSUID range.
    pub(crate) fn from_parts(time_ms: u64, payload: [u8; 16]) -> Option<Self> {
        if !(EPOCH_MS..=MAX_TIME_MS).contains(&time_ms) {
            return None;
        }
        let timestamp = ((time_ms - EPOCH_MS) / 1_000) as u32;
        let mut bytes = [0u8; BYTE_LENGTH];
        bytes[..4].copy_from_slice(&timestamp.to_be_bytes());
        bytes[4..].copy_from_slice(&payload);
        Some(Self(bytes))
    }

    /// A KSUID for the current time with a random payload.
    pub(crate) fn random() -> Self {
        let mut payload = [0u8; 16];
        fill_random(&mut payload);
        let now = crate::util::unix_millis();
        Self::from_parts(now.clamp(EPOCH_MS, MAX_TIME_MS), payload).unwrap_or(Self([0; BYTE_LENGTH]))
    }

    /// Seconds since the KSUID epoch.
    pub(crate) fn timestamp(&self) -> u32 {
        u32::from_be_bytes([self.0[0], self.0[1], self.0[2], self.0[3]])
    }

    /// Unix time in milliseconds.
    pub(crate) fn time_ms(&self) -> u64 {
        1_000 * u64::from(self.timestamp()) + EPOCH_MS
    }

    pub(crate) fn payload(&self) -> [u8; 16] {
        let mut p = [0u8; 16];
        p.copy_from_slice(&self.0[4..]);
        p
    }

    /// The 27-character base62 form.
    pub(crate) fn encode(&self) -> String {
        // Repeated division of the base-256 digits by 62, most significant
        // digit first: the same algorithm as the TS base-convert helper.
        let mut digits: Vec<u8> = self.0.to_vec();
        let mut out = [b'0'; STRING_LENGTH];
        let mut offset = STRING_LENGTH;
        while !digits.is_empty() && offset > 0 {
            let mut quotients = Vec::with_capacity(digits.len());
            let mut remainder: u32 = 0;
            for &digit in &digits {
                let acc = u32::from(digit) + remainder * 256;
                let q = acc / 62;
                remainder = acc % 62;
                if !quotients.is_empty() || q > 0 {
                    quotients.push(q as u8);
                }
            }
            offset -= 1;
            out[offset] = ALPHABET[remainder as usize];
            digits = quotients;
        }
        // Every byte of `out` comes from ALPHABET or is b'0', so it is ASCII.
        out.iter().map(|&b| b as char).collect()
    }

    /// Parses the 27-character base62 form. Returns `None` for a wrong
    /// length, a character outside `0-9A-Za-z`, or a value over 20 bytes.
    pub(crate) fn parse(s: &str) -> Option<Self> {
        if s.len() != STRING_LENGTH {
            return None;
        }
        let mut digits = Vec::with_capacity(STRING_LENGTH);
        for b in s.bytes() {
            digits.push(match b {
                b'0'..=b'9' => b - b'0',
                b'A'..=b'Z' => b - b'A' + 10,
                b'a'..=b'z' => b - b'a' + 36,
                _ => return None,
            });
        }
        let mut out = [0u8; BYTE_LENGTH];
        let mut offset = BYTE_LENGTH;
        while !digits.is_empty() {
            if offset == 0 {
                return None; // overflow: more than 20 bytes
            }
            let mut quotients = Vec::with_capacity(digits.len());
            let mut remainder: u32 = 0;
            for &digit in &digits {
                let acc = u32::from(digit) + remainder * 62;
                let q = acc / 256;
                remainder = acc % 256;
                if !quotients.is_empty() || q > 0 {
                    quotients.push(q as u8);
                }
            }
            offset -= 1;
            out[offset] = remainder as u8;
            digits = quotients;
        }
        Some(Self(out))
    }
}

/// A fresh random ID with the given prefix, for example `evt_…`.
pub(crate) fn new_prefixed(prefix: &str) -> String {
    format!("{prefix}_{}", Ksuid::random().encode())
}

/// A fresh random session ID (`ses_…`).
pub(crate) fn new_session_id() -> String {
    new_prefixed("ses")
}

/// A fresh random event ID (`evt_…`).
pub(crate) fn new_event_id() -> String {
    new_prefixed("evt")
}

/// Derives a session ID from a customer-supplied identifier. The same
/// identifier and project always give the same `ses_` KSUID, across
/// processes, restarts and AgentCat SDK languages.
pub fn derive_session_id(id: &str, project_id: Option<&str>) -> String {
    let input = match project_id {
        Some(p) if !p.is_empty() => format!("{id}:{p}"),
        _ => id.to_owned(),
    };
    let hash = Sha256::digest(input.as_bytes());
    let offset = u64::from(u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]])) % ONE_YEAR_MS;
    let mut payload = [0u8; 16];
    payload.copy_from_slice(&hash[4..20]);
    // EPOCH_2024 plus at most u32::MAX ms is always inside the KSUID range.
    let ksuid = Ksuid::from_parts(EPOCH_2024_MS + offset, payload).unwrap_or(Ksuid([0; BYTE_LENGTH]));
    format!("ses_{}", ksuid.encode())
}

/// True only for a session ID this SDK issued: `ses_` plus 27 base62 chars.
pub(crate) fn is_valid_session_id(value: &str) -> bool {
    value.len() == 31
        && value.starts_with("ses_")
        && value.as_bytes()[4..].iter().all(u8::is_ascii_alphanumeric)
}

pub(crate) fn fill_random(buf: &mut [u8]) {
    if getrandom::fill(buf).is_err() {
        // No OS randomness: fall back to hashing the clock and a counter.
        // Uniqueness matters here, not secrecy.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seed = format!(
            "{}:{}:{}",
            crate::util::unix_nanos(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
            std::process::id()
        );
        let hash = Sha256::digest(seed.as_bytes());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = hash[i % hash.len()];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_known_strings() {
        for s in [
            "0ujtsYcgvSTl8PAuAdqWYSMnLOv",
            "000000000000000000000000000",
            "aWgEPTl1tmebfsQzFP4bxwgy80V",
        ] {
            assert_eq!(Ksuid::parse(s).map(|k| k.encode()).as_deref(), Some(s));
        }
    }

    #[test]
    fn rejects_bad_strings() {
        assert!(Ksuid::parse("short").is_none());
        assert!(Ksuid::parse("0ujtsYcgvSTl8PAuAdqWYSMnLO-").is_none());
        // One above the maximum value overflows 20 bytes.
        assert!(Ksuid::parse("aWgEPTl1tmebfsQzFP4bxwgy80W").is_none());
        assert!(Ksuid::parse("zzzzzzzzzzzzzzzzzzzzzzzzzzz").is_none());
    }

    #[test]
    fn random_ids_are_valid_and_distinct() {
        let a = new_session_id();
        let b = new_session_id();
        assert!(is_valid_session_id(&a), "{a}");
        assert_ne!(a, b);
        assert!(new_event_id().starts_with("evt_"));
        let k = Ksuid::parse(&a[4..]).expect("parses");
        let now = crate::util::unix_millis();
        assert!(k.time_ms() <= now && now - k.time_ms() < 5_000);
    }

    #[test]
    fn from_parts_rejects_out_of_range_times() {
        assert!(Ksuid::from_parts(EPOCH_MS - 1, [0; 16]).is_none());
        assert!(Ksuid::from_parts(MAX_TIME_MS + 1, [0; 16]).is_none());
        assert!(Ksuid::from_parts(MAX_TIME_MS, [0; 16]).is_some());
    }
}
