//! Prefixed KSUIDs (`ses_...`, `evt_...`), equal to `KSUID.withPrefix(prefix)` of the TS SDK
//! (`src/thirdparty/ksuid`). A KSUID is 20 bytes, a 4-byte big-endian count of seconds since
//! `EPOCH_MS` and a 16-byte payload, written as 27 base62 characters padded on the left with `0`.
//! `tests/vectors/ksuid.json` pins the encoding; `ksuid/tests.rs` checks it.
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
mod tests;

/// The KSUID epoch (2014-05-13T16:53:20Z) in milliseconds since the Unix epoch.
pub(crate) const EPOCH_MS: u64 = 1_400_000_000_000;

/// The latest time a KSUID can hold, in milliseconds since the Unix epoch.
pub(crate) const MAX_TIME_MS: u64 = 1_000 * 4_294_967_295 + EPOCH_MS;

/// The base62 digits, in value order.
const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// The length of the encoded part of a KSUID.
const ENCODED_LEN: usize = 27;

/// Base62 digits of 20 big-endian bytes, 27 of them, most significant first.
fn base62(bytes: &[u8; 20]) -> [u8; ENCODED_LEN] {
    let mut words = [0_u32; 5];
    let (chunks, _) = bytes.as_chunks::<4>();
    for (word, chunk) in words.iter_mut().zip(chunks) {
        *word = u32::from_be_bytes(*chunk);
    }
    let mut digits = [b'0'; ENCODED_LEN];
    for digit in digits.iter_mut().rev() {
        // Long division of the 160-bit number by 62; the remainder is the next digit.
        let mut remainder = 0_u64;
        for word in &mut words {
            let value = (remainder << 32) | u64::from(*word);
            *word = u32::try_from(value / 62).unwrap_or(0);
            remainder = value % 62;
        }
        *digit = ALPHABET[usize::try_from(remainder).unwrap_or(0)];
    }
    digits
}

/// `<prefix>_` and the 27-character KSUID of `time_ms` and `payload`; `None` when `time_ms` is
/// outside `EPOCH_MS..=MAX_TIME_MS`, where TS throws.
pub(crate) fn from_parts(prefix: &str, time_ms: u64, payload: &[u8; 16]) -> Option<String> {
    if !(EPOCH_MS..=MAX_TIME_MS).contains(&time_ms) {
        return None;
    }
    let seconds = u32::try_from((time_ms - EPOCH_MS) / 1_000).ok()?;
    let mut bytes = [0_u8; 20];
    bytes[..4].copy_from_slice(&seconds.to_be_bytes());
    bytes[4..].copy_from_slice(payload);
    let mut id = String::with_capacity(prefix.len() + 1 + ENCODED_LEN);
    id.push_str(prefix);
    id.push('_');
    id.extend(base62(&bytes).iter().map(|&b| char::from(b)));
    Some(id)
}

/// A new ID for the current time with 16 random bytes; `None` when the clock is outside the KSUID
/// range or the system has no randomness, where TS throws.
pub(crate) fn random(prefix: &str) -> Option<String> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok());
    random_at(prefix, now_ms, |buf| getrandom::fill(buf))
}

/// `random` with the clock (`None` = before the Unix epoch) and the random source passed in.
fn random_at(
    prefix: &str,
    now_ms: Option<u64>,
    fill: impl FnOnce(&mut [u8; 16]) -> Result<(), getrandom::Error>,
) -> Option<String> {
    let now_ms = now_ms?;
    let mut payload = [0_u8; 16];
    fill(&mut payload).ok()?;
    from_parts(prefix, now_ms, &payload)
}
