//! Rules that several unit tests check, written apart from the code they test: the longest cut of
//! a string within a number of UTF-16 units, and the 20 bytes of a base62 KSUID.
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

/// Checks that `kept` is the longest prefix of `text` of at most `units` UTF-16 units: a prefix,
/// within the units, and either all of `text` or one character short of going over.
///
/// # Errors
///
/// A [`TestCaseError`] that names the broken part of the rule.
pub(crate) fn check_cut(text: &str, kept: &str, units: usize) -> Result<(), TestCaseError> {
    prop_assert!(
        text.starts_with(kept),
        "{:?} is not a prefix of {:?}",
        kept,
        text
    );
    let kept_units = kept.encode_utf16().count();
    prop_assert!(kept_units <= units, "{:?} is over {} units", kept, units);
    if let Some(next) = text[kept.len()..].chars().next() {
        prop_assert!(
            kept_units + next.len_utf16() > units,
            "{:?} could keep {:?} within {} units",
            kept,
            next,
            units
        );
    }
    Ok(())
}

/// The 20 bytes that the 27 base62 characters `text` stand for, big-endian, read with the
/// alphabet `0-9`, `A-Z`, `a-z`.
///
/// # Panics
///
/// On a character outside the alphabet, or a number over 20 bytes.
pub(crate) fn base62_bytes(text: &str) -> [u8; 20] {
    let mut bytes = [0_u8; 20];
    for c in text.chars() {
        let digit = match c {
            '0'..='9' => u32::from(c) - u32::from('0'),
            'A'..='Z' => u32::from(c) - u32::from('A') + 10,
            'a'..='z' => u32::from(c) - u32::from('a') + 36,
            _ => panic!("{c:?} is not a base62 digit"),
        };
        // bytes = bytes * 62 + digit, from the last byte up.
        let mut carry = digit;
        for byte in bytes.iter_mut().rev() {
            let value = u32::from(*byte) * 62 + carry;
            *byte = u8::try_from(value & 0xff).unwrap();
            carry = value >> 8;
        }
        assert_eq!(carry, 0, "{text} is more than 20 bytes");
    }
    bytes
}

mod tests {
    use proptest::test_runner::TestCaseError;

    use super::{base62_bytes, check_cut};

    #[test]
    fn check_cut_takes_only_the_longest_prefix_within_the_units() {
        let ok = |text: &str, kept: &str, units: usize| check_cut(text, kept, units).is_ok();
        assert!(ok("abc", "ab", 2));
        assert!(ok("abc", "abc", 5));
        // An astral character is 2 units: at 3 units, "a😀" fits and "a😀b" does not.
        assert!(ok("a😀b", "a😀", 3));
        // At 2 units the astral character does not fit, so "a" is the longest prefix.
        assert!(ok("a😀b", "a", 2));
        assert!(!ok("abc", "a", 2), "a shorter prefix than the units allow");
        assert!(!ok("abc", "abc", 2), "over the units");
        assert!(!ok("abc", "bc", 2), "not a prefix");
        let error: Result<(), TestCaseError> = check_cut("abc", "x", 1);
        assert!(error.is_err());
    }

    #[test]
    fn base62_bytes_reads_the_smallest_and_the_largest_ksuid() {
        assert_eq!(base62_bytes("000000000000000000000000000"), [0; 20]);
        assert_eq!(base62_bytes("aWgEPTl1tmebfsQzFP4bxwgy80V"), [0xff; 20]);
        assert_eq!(base62_bytes("000000000000000000000000001")[19], 1);
        assert_eq!(base62_bytes("00000000000000000000000000z")[19], 61);
    }
}
