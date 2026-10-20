//! KSUIDs (K-Sortable Unique Identifiers), as the TypeScript SDK encodes them.
//!
//! A KSUID is 20 bytes: a big-endian `u32` timestamp in seconds since the
//! KSUID epoch (2014-05-13T16:53:20Z), then 16 payload bytes. Its string form
//! is 27 base62 characters (`0-9A-Za-z`), left-padded with `0`.

use std::fmt;

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

/// Why a KSUID could not be built. `Display` gives the TS error messages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum KsuidError {
    /// The string form is not 27 UTF-16 code units long.
    EncodedLength,
    /// The decoded value needs more than 20 bytes.
    Decode {
        /// The byte count the TS decoder reports.
        expected_at_least: usize,
    },
    /// The time is outside the KSUID range.
    Time,
    /// The payload is not 16 bytes.
    Payload,
    /// The binary form is not 20 bytes.
    #[cfg(test)]
    Buffer,
}

impl fmt::Display for KsuidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EncodedLength => f.write_str("Valid encoded KSUIDs are 27 characters"),
            Self::Decode { expected_at_least } => write!(
                f,
                "Fixed length of {BYTE_LENGTH} is too small, expected at least {expected_at_least}"
            ),
            Self::Time => f.write_str(
                "Valid KSUID timestamps must be in milliseconds since 1970-01-01T00:00:00Z, \
                 no earlier than 2014-05-13T16:53:20Z and no later than 2150-06-19T23:21:35Z",
            ),
            Self::Payload => f.write_str("Valid KSUID payloads are 16 bytes"),
            #[cfg(test)]
            Self::Buffer => f.write_str("Valid KSUID buffers are 20 bytes"),
        }
    }
}

/// Ordered by bytes, so by time first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Ksuid([u8; BYTE_LENGTH]);

impl Ksuid {
    /// The string form of the smallest KSUID.
    #[cfg(test)]
    pub(crate) const MIN_STRING: &str = "000000000000000000000000000";
    /// The string form of the largest KSUID.
    #[cfg(test)]
    pub(crate) const MAX_STRING: &str = "aWgEPTl1tmebfsQzFP4bxwgy80V";

    /// Builds a KSUID from a Unix time in milliseconds and a 16-byte payload.
    pub(crate) fn from_parts(time_ms: u64, payload: &[u8]) -> Result<Self, KsuidError> {
        if !(EPOCH_MS..=MAX_TIME_MS).contains(&time_ms) {
            return Err(KsuidError::Time);
        }
        let payload: &[u8; 16] = payload.try_into().map_err(|_| KsuidError::Payload)?;
        let timestamp = u32::try_from((time_ms - EPOCH_MS) / 1_000).unwrap_or(u32::MAX);
        let mut bytes = [0u8; BYTE_LENGTH];
        bytes[..4].copy_from_slice(&timestamp.to_be_bytes());
        bytes[4..].copy_from_slice(payload);
        Ok(Self(bytes))
    }

    /// Wraps the 20-byte binary form.
    #[cfg(test)]
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, KsuidError> {
        bytes.try_into().map(Self).map_err(|_| KsuidError::Buffer)
    }

    /// A KSUID for the current time with a random payload.
    pub(crate) fn random() -> Self {
        let mut payload = [0u8; 16];
        fill_random(&mut payload);
        let now = crate::util::unix_millis();
        Self::from_parts(now.clamp(EPOCH_MS, MAX_TIME_MS), &payload).unwrap_or(Self([0; BYTE_LENGTH]))
    }

    /// A KSUID for the given Unix time in milliseconds with a random payload.
    #[cfg(test)]
    pub(crate) fn random_at(time_ms: u64) -> Result<Self, KsuidError> {
        let mut payload = [0u8; 16];
        fill_random(&mut payload);
        Self::from_parts(time_ms, &payload)
    }

    /// Seconds since the KSUID epoch.
    pub(crate) fn timestamp(&self) -> u32 {
        u32::from_be_bytes([self.0[0], self.0[1], self.0[2], self.0[3]])
    }

    /// Unix time in milliseconds.
    pub(crate) fn time_ms(&self) -> u64 {
        1_000 * u64::from(self.timestamp()) + EPOCH_MS
    }

    #[cfg(test)]
    pub(crate) fn payload(&self) -> [u8; 16] {
        let mut p = [0u8; 16];
        p.copy_from_slice(&self.0[4..]);
        p
    }

    /// The 20-byte binary form.
    #[cfg(test)]
    pub(crate) fn as_bytes(&self) -> &[u8; BYTE_LENGTH] {
        &self.0
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
                    // `acc < 62 * 256`, so `q < 256`.
                    quotients.push(u8::try_from(q).unwrap_or(u8::MAX));
                }
            }
            offset -= 1;
            if let (Some(slot), Some(&c)) = (out.get_mut(offset), ALPHABET.get(remainder as usize)) {
                *slot = c;
            }
            digits = quotients;
        }
        // Every byte of `out` comes from ALPHABET or is b'0', so it is ASCII.
        out.iter().map(|&b| b as char).collect()
    }

    /// Decodes the string form exactly as the TS decoder does: the length
    /// counts UTF-16 code units, and characters outside `0-9A-Za-z` are not
    /// rejected but decode to out-of-range digits. Fails only for a wrong
    /// length or a value over 20 bytes.
    pub(crate) fn parse(encoded: &str) -> Result<Self, KsuidError> {
        if encoded.encode_utf16().count() != STRING_LENGTH {
            return Err(KsuidError::EncodedLength);
        }
        // `Array.from` walks code points; `charCodeAt(0)` reads the first UTF-16 unit.
        // There is no alphabet check: other characters give out-of-range digits.
        let digits: Vec<i64> = encoded
            .chars()
            .map(|c| {
                let mut units = [0u16; 2];
                let code = c.encode_utf16(&mut units).first().map_or(0, |&u| i64::from(u));
                if code < 58 {
                    code - 48
                } else if code < 91 {
                    code - 55
                } else {
                    code - 61
                }
            })
            .collect();
        let mut out = [0i64; BYTE_LENGTH];
        let mut offset = BYTE_LENGTH;
        let mut input = digits.clone();
        while !input.is_empty() {
            let Some(next) = offset.checked_sub(1) else {
                return Err(KsuidError::Decode {
                    expected_at_least: max_length(digits.len()),
                });
            };
            offset = next;
            let mut quotients = Vec::with_capacity(input.len());
            let mut remainder: i64 = 0;
            for digit in &input {
                let acc = digit + remainder * 62;
                let q = acc.div_euclid(256); // Math.floor(acc / 256)
                remainder = acc % 256; // JS `%` keeps the dividend's sign
                if !quotients.is_empty() || q > 0 {
                    quotients.push(q);
                }
            }
            if let Some(slot) = out.get_mut(offset) {
                *slot = remainder;
            }
            input = quotients;
        }
        // Buffer.from(array) stores each value modulo 256.
        let mut bytes = [0u8; BYTE_LENGTH];
        for (b, v) in bytes.iter_mut().zip(out) {
            *b = u8::try_from(v.rem_euclid(256)).unwrap_or(0);
        }
        Ok(Self(bytes))
    }
}

/// `Math.ceil(n * Math.log2(62) / Math.log2(256))`: the byte count the TS
/// decoder reports for `n` base62 digits that overflow 20 bytes.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "n is at most 27, so the value is a small positive integer"
)]
fn max_length(n: usize) -> usize {
    (n as f64 * 62f64.log2() / 8.0).ceil() as usize
}

impl fmt::Display for Ksuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

impl fmt::Debug for Ksuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KSUID {{ {} }}", self.encode())
    }
}

#[cfg(test)]
impl serde::Serialize for Ksuid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.encode())
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
    let hash: [u8; 32] = Sha256::digest(input.as_bytes()).into();
    let offset = u64::from(u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]])) % ONE_YEAR_MS;
    // EPOCH_2024 plus at most u32::MAX ms is always inside the KSUID range.
    let ksuid = Ksuid::from_parts(EPOCH_2024_MS + offset, &hash[4..20]).unwrap_or(Ksuid([0; BYTE_LENGTH]));
    format!("ses_{}", ksuid.encode())
}

/// True only for a session ID this SDK issued: `ses_` plus 27 base62 chars.
pub(crate) fn is_valid_session_id(value: &str) -> bool {
    value.len() == 31
        && value.starts_with("ses_")
        && value
            .as_bytes()
            .get(4..)
            .is_some_and(|rest| rest.iter().all(u8::is_ascii_alphanumeric))
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
        for (b, h) in buf.iter_mut().zip(hash.iter().cycle()) {
            *b = *h;
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
            assert_eq!(Ksuid::parse(s).map(|k| k.encode()).as_deref(), Ok(s));
        }
    }

    #[test]
    fn rejects_bad_strings() {
        assert_eq!(Ksuid::parse("short"), Err(KsuidError::EncodedLength));
        // One above the maximum value overflows 20 bytes.
        assert_eq!(
            Ksuid::parse("aWgEPTl1tmebfsQzFP4bxwgy80W"),
            Err(KsuidError::Decode {
                expected_at_least: 21
            })
        );
        assert!(Ksuid::parse("zzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
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
        assert_eq!(Ksuid::from_parts(EPOCH_MS - 1, &[0; 16]), Err(KsuidError::Time));
        assert_eq!(
            Ksuid::from_parts(MAX_TIME_MS + 1, &[0; 16]),
            Err(KsuidError::Time)
        );
        assert!(Ksuid::from_parts(MAX_TIME_MS, &[0; 16]).is_ok());
    }
}
