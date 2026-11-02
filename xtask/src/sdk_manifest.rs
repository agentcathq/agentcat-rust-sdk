//! The SDK's own `serde_json` dependency must turn on `preserve_order`.
//!
//! Cargo unifies the features of a dependency across every package of one build, so a key-order
//! test passes as soon as any other package of the build (or a dev-dependency) turns the feature
//! on. A customer build has only the SDK, so this check reads the SDK's manifest instead of
//! trusting the built `serde_json`.
use std::path::Path;

/// The SDK manifest, relative to the workspace root.
pub const SDK_MANIFEST: &str = "agentcat-sdk/Cargo.toml";

/// Checks the text of `agentcat-sdk/Cargo.toml`: its `[dependencies]` table has a `serde_json`
/// entry that lists `preserve_order` in `features` and is not optional.
///
/// # Errors
///
/// Invalid TOML, or a message that names what is missing.
pub fn check_preserve_order(manifest: &str) -> Result<(), String> {
    let doc: toml::Table = toml::from_str(manifest).map_err(|e| format!("{SDK_MANIFEST}: {e}"))?;
    let entry = doc
        .get("dependencies")
        .and_then(|deps| deps.get("serde_json"))
        .ok_or_else(|| format!("{SDK_MANIFEST}: [dependencies] has no serde_json entry"))?;
    let missing = || {
        format!(
            "{SDK_MANIFEST}: the [dependencies] serde_json entry must have \
             features = [\"preserve_order\"] (mcp_session stays the first key)"
        )
    };
    let table = entry.as_table().ok_or_else(missing)?;
    if table.get("optional").and_then(toml::Value::as_bool) == Some(true) {
        return Err(format!(
            "{SDK_MANIFEST}: serde_json must not be optional (preserve_order is always on)"
        ));
    }
    let on = table
        .get("features")
        .and_then(toml::Value::as_array)
        .is_some_and(|features| {
            features
                .iter()
                .any(|f| f.as_str() == Some("preserve_order"))
        });
    if on { Ok(()) } else { Err(missing()) }
}

/// Reads `agentcat-sdk/Cargo.toml` under `root` and runs [`check_preserve_order`] on it.
///
/// # Errors
///
/// The file cannot be read, or the check fails.
pub fn check_repo(root: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(root.join(SDK_MANIFEST))
        .map_err(|e| format!("cannot read {SDK_MANIFEST}: {e}"))?;
    check_preserve_order(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inline_entry_with_the_feature_passes() {
        let text = "[dependencies]\n\
                    serde_json = { workspace = true, features = [\"std\", \"preserve_order\"] }\n";
        assert_eq!(check_preserve_order(text), Ok(()));
    }

    #[test]
    fn a_table_entry_with_the_feature_passes() {
        let text =
            "[dependencies.serde_json]\nversion = \"1\"\nfeatures = [\n  \"preserve_order\",\n]\n";
        assert_eq!(check_preserve_order(text), Ok(()));
    }

    #[test]
    fn an_entry_without_the_feature_fails() {
        for text in [
            "[dependencies]\nserde_json = { workspace = true }\n",
            "[dependencies]\nserde_json = \"1\"\n",
            "[dependencies]\nserde_json = { version = \"1\", features = [\"std\"] }\n",
        ] {
            let err = check_preserve_order(text).unwrap_err();
            assert!(err.contains("preserve_order"), "{text}: {err}");
        }
    }

    #[test]
    fn the_feature_on_a_dev_dependency_only_fails() {
        let text = "[dependencies]\nserde_json = { workspace = true }\n\n\
                    [dev-dependencies]\nserde_json = { workspace = true, features = [\"preserve_order\"] }\n";
        assert!(check_preserve_order(text).is_err());
        let text = "[dev-dependencies]\nserde_json = { workspace = true, features = [\"preserve_order\"] }\n";
        assert!(
            check_preserve_order(text)
                .unwrap_err()
                .contains("no serde_json")
        );
    }

    #[test]
    fn an_optional_entry_fails() {
        let text = "[dependencies]\n\
                    serde_json = { version = \"1\", optional = true, features = [\"preserve_order\"] }\n";
        assert!(check_preserve_order(text).unwrap_err().contains("optional"));
    }

    #[test]
    fn invalid_toml_fails() {
        assert!(check_preserve_order("[dependencies\n").is_err());
    }

    /// The real manifest: the SDK crate itself keeps the rule, whatever the workspace builds.
    #[test]
    fn the_sdk_manifest_turns_on_preserve_order() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        assert_eq!(check_repo(&root), Ok(()));
    }
}
