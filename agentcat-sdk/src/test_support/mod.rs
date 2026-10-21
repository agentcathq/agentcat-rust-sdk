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

/// The most shrink steps of a [`config_capped`] test: each step reruns the test, oracle call
/// included, on an input that can be several hundred kilobytes.
const SHRINK_ITERS: u32 = 64;

/// The most time, in milliseconds, a [`config_capped`] test spends shrinking.
const SHRINK_MS: u32 = 60_000;

/// [`config`] with at most `max` cases, for a property test whose cases are slow or large (the
/// size-guard tests). Shrinking stops after [`SHRINK_ITERS`] steps or [`SHRINK_MS`]
/// milliseconds, so a failure reports a partly shrunk input in bounded time; a smaller limit
/// from `PROPTEST_MAX_SHRINK_ITERS` or `PROPTEST_MAX_SHRINK_TIME` still applies.
pub(crate) fn config_capped(max: u32) -> Config {
    let mut config = Config::with_cases(config().cases.min(max));
    config.max_shrink_iters = config.max_shrink_iters().min(SHRINK_ITERS);
    config.max_shrink_time = match config.max_shrink_time {
        0 => SHRINK_MS,
        ms => ms.min(SHRINK_MS),
    };
    config
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

    #[test]
    fn config_capped_caps_shrinking() {
        let config = config_capped(1_000);
        assert!(
            config.max_shrink_iters() <= 64,
            "{}",
            config.max_shrink_iters()
        );
        assert!(
            (1..=60_000).contains(&config.max_shrink_time),
            "{}",
            config.max_shrink_time
        );
    }
}
