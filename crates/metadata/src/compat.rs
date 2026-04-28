//! Generic compatibility rules shared by every catalog citizen.
//!
//! Entity-specific rules (action ports, credential auth pattern, future
//! plugin-manifest contents) compose *on top of* the shared checks here.
//! Keeping the base rules here prevents action/credential/resource from
//! copy-drifting `key immutable / version monotonic / schema-break-requires-
//! major-bump` three times with slightly different spellings.

use semver::Version;

use crate::BaseMetadata;

/// Entity-agnostic compatibility errors reported by
/// [`validate_base_compat`].
///
/// `K` is the concrete catalog-entity key type (e.g. `ActionKey`,
/// `CredentialKey`, `ResourceKey`). It must be `Display` so the error
/// message can name the keys in each variant and `Debug + Clone + Eq` so
/// the error composes inside a `thiserror`-derived parent enum.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BaseCompatError<K>
where
    K: std::fmt::Debug + std::fmt::Display + Clone + PartialEq + Eq,
{
    /// Typed entity key was replaced across revisions — identity is
    /// immutable across a version history.
    #[error("metadata key changed from `{previous}` to `{current}`")]
    KeyChanged {
        /// Previous key value.
        previous: K,
        /// Current (rejected) key value.
        current: K,
    },

    /// Interface version went backwards.
    #[error("interface version regressed from {previous} to {current}")]
    VersionRegressed {
        /// Previous version.
        previous: Version,
        /// Current (rejected) version.
        current: Version,
    },

    /// Input schema changed shape but the major version was not bumped.
    #[error("breaking schema change detected without a major version bump")]
    SchemaChangeWithoutMajorBump,
}

/// Validate that `current` is a backwards-compatible revision of `previous`
/// with respect to the shared `BaseMetadata` prefix.
///
/// Rules:
/// - `key` must be equal.
/// - `version` must be `>= previous.version` (full `semver::Version` ordering, including
///   pre-release tags; build metadata is ignored per the SemVer 2.0 spec).
/// - If `schema` changed, `version.major` must exceed `previous.version.major`.
///
/// Entity-specific rules (ports, auth pattern) are **not** checked here —
/// each concrete metadata type layers its own rules on top.
pub fn validate_base_compat<K>(
    current: &BaseMetadata<K>,
    previous: &BaseMetadata<K>,
) -> Result<(), BaseCompatError<K>>
where
    K: std::fmt::Debug + std::fmt::Display + Clone + PartialEq + Eq,
{
    if current.key != previous.key {
        return Err(BaseCompatError::KeyChanged {
            previous: previous.key.clone(),
            current: current.key.clone(),
        });
    }
    if current.version < previous.version {
        return Err(BaseCompatError::VersionRegressed {
            previous: previous.version.clone(),
            current: current.version.clone(),
        });
    }
    if current.schema != previous.schema && current.version.major == previous.version.major {
        return Err(BaseCompatError::SchemaChangeWithoutMajorBump);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nebula_schema::{FieldCollector, Schema, ValidSchema, field_key};
    use semver::Version;

    use super::{BaseCompatError, validate_base_compat};
    use crate::BaseMetadata;

    fn empty_schema() -> ValidSchema {
        Schema::builder()
            .build()
            .expect("empty schema always valid")
    }

    fn schema_with_one_field() -> ValidSchema {
        Schema::builder()
            .string(field_key!("extra"), |s| s)
            .build()
            .expect("single-string schema always valid")
    }

    // Tests use a tiny newtype key so we don't pull nebula-core here.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestKey(&'static str);

    impl std::fmt::Display for TestKey {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    fn md(key: &'static str, major: u64, minor: u64) -> BaseMetadata<TestKey> {
        BaseMetadata::new(TestKey(key), "n", "d", empty_schema())
            .with_version(Version::new(major, minor, 0))
    }

    #[test]
    fn minor_bump_same_schema_ok() {
        let prev = md("k", 1, 0);
        let next = md("k", 1, 1);
        assert!(validate_base_compat(&next, &prev).is_ok());
    }

    #[test]
    fn major_bump_same_schema_ok() {
        let prev = md("k", 1, 0);
        let next = md("k", 2, 0);
        assert!(validate_base_compat(&next, &prev).is_ok());
    }

    #[test]
    fn key_change_rejected() {
        let prev = md("old", 1, 0);
        let next = md("new", 1, 0);
        let err = validate_base_compat(&next, &prev).unwrap_err();
        assert!(matches!(err, BaseCompatError::KeyChanged { .. }));
    }

    #[test]
    fn version_regression_rejected() {
        let prev = md("k", 2, 1);
        let next = md("k", 2, 0);
        let err = validate_base_compat(&next, &prev).unwrap_err();
        assert!(matches!(err, BaseCompatError::VersionRegressed { .. }));
    }

    #[test]
    fn schema_change_without_major_rejected() {
        let prev = md("k", 1, 0);
        let next_base = BaseMetadata::new(TestKey("k"), "n", "d", schema_with_one_field())
            .with_version(Version::new(1, 1, 0));
        let err = validate_base_compat(&next_base, &prev).unwrap_err();
        assert_eq!(err, BaseCompatError::SchemaChangeWithoutMajorBump);
    }

    #[test]
    fn schema_change_with_major_accepted() {
        let prev = md("k", 1, 0);
        let next_base = BaseMetadata::new(TestKey("k"), "n", "d", schema_with_one_field())
            .with_version(Version::new(2, 0, 0));
        assert!(validate_base_compat(&next_base, &prev).is_ok());
    }

    #[test]
    fn display_includes_keys() {
        let prev = md("old", 1, 0);
        let next = md("new", 1, 0);
        let err = validate_base_compat(&next, &prev).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("old") && msg.contains("new"), "got: {msg}");
    }
}
