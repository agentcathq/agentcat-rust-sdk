//! Helpers shared by integration tests.
pub(crate) mod oracle;

/// The proptest settings for property tests: 256 cases unless `PROPTEST_CASES` says otherwise
/// (proptest reads that variable itself).
pub(crate) fn proptest_config() -> proptest::test_runner::Config {
    proptest::test_runner::Config::default()
}
