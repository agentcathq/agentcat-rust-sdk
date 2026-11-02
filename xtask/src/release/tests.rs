use super::*;

fn v(text: &str) -> Version {
    Version::parse(text).expect("a valid version")
}

const CHANGELOG_TEXT: &str = "# Changelog

Intro.

## [Unreleased]

### Added

- A feature.

## [1.0.0] - 2026-11-03

### Added

- The first release.

[Unreleased]: https://github.com/agentcathq/agentcat-rust-sdk/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/agentcathq/agentcat-rust-sdk/releases/tag/v1.0.0
";

#[test]
fn versions_parse_only_as_major_minor_patch() {
    assert_eq!(v("1.20.3").to_string(), "1.20.3");
    assert_eq!(v("0.0.0").to_string(), "0.0.0");
    for bad in [
        "",
        "1",
        "1.2",
        "1.2.3.4",
        "01.2.3",
        "1.2.x",
        "v1.2.3",
        "1.2.3-rc.1",
        "1..3",
    ] {
        assert_eq!(Version::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn the_newest_tag_ignores_tags_that_are_not_versions() {
    let tags = ["v1.0.0", "v1.10.0", "v1.9.9", "attempt-1", "v2.0", "1.99.0"].map(str::to_owned);
    assert_eq!(newest_tag(&tags), Some(v("1.10.0")));
    assert_eq!(newest_tag(&["attempt-1".to_owned()]), None);
}

#[test]
fn words_count_from_the_newest_tag() {
    let newest = Some(v("1.2.3"));
    let current = v("1.2.3");
    assert_eq!(next_version("patch", current, newest), Ok(v("1.2.4")));
    assert_eq!(next_version("minor", current, newest), Ok(v("1.3.0")));
    assert_eq!(next_version("major", current, newest), Ok(v("2.0.0")));
    assert_eq!(next_version("1.5.0", current, newest), Ok(v("1.5.0")));
}

#[test]
fn an_exact_version_must_be_newer_than_the_newest_tag() {
    let newest = Some(v("1.2.3"));
    let error = next_version("1.2.3", v("1.2.3"), newest).expect_err("not newer");
    assert_eq!(error, "1.2.3 is not newer than the newest tag, v1.2.3");
    assert!(next_version("1.0.9", v("1.2.3"), newest).is_err());
    let error = next_version("next", v("1.2.3"), newest).expect_err("not a word");
    assert_eq!(
        error,
        "\"next\" is not patch, minor, major or a version X.Y.Z"
    );
}

#[test]
fn the_first_release_is_the_manifest_version() {
    for word in ["patch", "minor", "major", "1.0.0"] {
        assert_eq!(
            next_version(word, v("1.0.0"), None),
            Ok(v("1.0.0")),
            "{word}"
        );
    }
    let error = next_version("1.1.0", v("1.0.0"), None).expect_err("another version");
    assert!(
        error.contains("the first release is the version of agentcat-sdk/Cargo.toml, 1.0.0"),
        "{error}"
    );
}

#[test]
fn the_manifest_version_is_the_one_of_the_package_table() {
    let manifest = "[package]\nname = \"agentcat-sdk\"\nversion = \"1.0.0\"\n\n[dependencies]\nrmcp = { version = \"3.0.0\" }\n";
    assert_eq!(manifest_version(manifest), Some(v("1.0.0")));
    let updated = set_manifest_version(manifest, v("1.0.1")).expect("updated");
    assert_eq!(
        updated,
        manifest.replace("version = \"1.0.0\"", "version = \"1.0.1\"")
    );
    assert!(set_manifest_version("[dependencies]\nversion = \"1.0.0\"\n", v("1.0.1")).is_err());
}

#[test]
fn dating_moves_the_entries_under_the_version_and_links_both_sections() {
    let dated =
        date_changelog(CHANGELOG_TEXT, v("1.1.0"), "2026-12-01", Some(v("1.0.0"))).expect("dated");
    let expected = CHANGELOG_TEXT
        .replace(
            "## [Unreleased]\n",
            "## [Unreleased]\n\n## [1.1.0] - 2026-12-01\n",
        )
        .replace(
            "[Unreleased]: https://github.com/agentcathq/agentcat-rust-sdk/compare/v1.0.0...HEAD\n",
            "[Unreleased]: https://github.com/agentcathq/agentcat-rust-sdk/compare/v1.1.0...HEAD\n\
             [1.1.0]: https://github.com/agentcathq/agentcat-rust-sdk/compare/v1.0.0...v1.1.0\n",
        );
    assert_eq!(dated, expected);
    assert_eq!(
        dated_section(&dated, v("1.1.0")),
        Some((
            "2026-12-01".to_owned(),
            "### Added\n\n- A feature.".to_owned()
        ))
    );
}

#[test]
fn the_first_release_links_its_tag() {
    let first = "## [Unreleased]\n\n- One.\n\n[Unreleased]: https://github.com/agentcathq/agentcat-rust-sdk/commits/main\n";
    let dated = date_changelog(first, v("1.0.0"), "2026-11-03", None).expect("dated");
    assert_eq!(
        dated,
        "## [Unreleased]\n\n## [1.0.0] - 2026-11-03\n\n- One.\n\n\
         [Unreleased]: https://github.com/agentcathq/agentcat-rust-sdk/compare/v1.0.0...HEAD\n\
         [1.0.0]: https://github.com/agentcathq/agentcat-rust-sdk/releases/tag/v1.0.0\n"
    );
}

#[test]
fn dating_refuses_an_empty_section_a_bad_date_and_a_missing_section() {
    let empty =
        "## [Unreleased]\n\n### Added\n\n## [1.0.0] - 2026-11-03\n\n- One.\n\n[Unreleased]: x\n";
    let error =
        date_changelog(empty, v("1.0.1"), "2026-12-01", Some(v("1.0.0"))).expect_err("empty");
    assert!(error.contains("has no entry"), "{error}");
    assert!(date_changelog(CHANGELOG_TEXT, v("1.1.0"), "2026-12-1", None).is_err());
    assert!(date_changelog("# Changelog\n", v("1.1.0"), "2026-12-01", None).is_err());
}

#[test]
fn a_section_without_a_valid_date_is_not_dated() {
    assert_eq!(
        dated_section(CHANGELOG_TEXT, v("1.0.0")).map(|(date, _)| date),
        Some("2026-11-03".to_owned())
    );
    assert_eq!(
        dated_section("## [1.0.0] - soon\n- One.\n", v("1.0.0")),
        None
    );
    assert_eq!(dated_section(CHANGELOG_TEXT, v("9.9.9")), None);
}

#[test]
fn run_rejects_unknown_arguments_with_the_usage() {
    let root = Path::new(".");
    let (code, out, problems) = run(&["publish".to_owned()], root);
    assert_eq!((code, out.len()), (2, 0));
    assert!(problems[0].starts_with("usage: "), "{problems:?}");
}
