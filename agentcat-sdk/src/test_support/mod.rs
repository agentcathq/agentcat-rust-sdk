//! Helpers shared by the crate's unit tests: the JS oracle, the markers of the strict JSON
//! encoding, the vector loader and the proptest strategies.
pub(crate) mod markers;
pub(crate) mod oracle;
pub(crate) mod strategies;
pub(crate) mod vectors;

mod smoke;

use proptest::test_runner::Config;

/// The proptest settings for property tests: 256 cases unless `PROPTEST_CASES` says otherwise
/// (proptest reads that variable itself).
pub(crate) fn config() -> Config {
    Config::default()
}

/// [`config`] with at most `max` cases, for a property test whose cases are slow.
pub(crate) fn config_capped(max: u32) -> Config {
    Config::with_cases(config().cases.min(max))
}

mod tests {
    use proptest::test_runner::Config;

    use super::{config, config_capped};

    #[test]
    fn config_is_the_proptest_default() {
        assert_eq!(config().cases, Config::default().cases);
    }

    #[test]
    fn config_capped_takes_the_smaller_case_count() {
        assert_eq!(config_capped(1).cases, 1);
        assert_eq!(config_capped(u32::MAX).cases, Config::default().cases);
    }
}
