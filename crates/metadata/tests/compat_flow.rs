//! `validate_base_compat` against a *composed* entity, not bare
//! `BaseMetadata`. All seven unit tests in `compat.rs` compose nothing but
//! `BaseMetadata<TestKey>` directly; this file proves the same three
//! `BaseCompatError` variants plus the ok paths through a locally composed
//! metadata shape carrying an entity-specific field the compat check must
//! ignore.

use nebula_metadata::{
    BaseCompatError, BaseMetadata, Metadata, MetadataDraft, validate_base_compat,
};
use nebula_schema::{FieldCollector, Schema, ValidSchema, field_key};
use pretty_assertions::assert_eq;
use rstest::rstest;
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalKey(String);

impl std::str::FromStr for LocalKey {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(value.to_owned()))
    }
}

impl std::fmt::Display for LocalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn key(s: &str) -> LocalKey {
    LocalKey(s.to_owned())
}

fn empty_schema() -> ValidSchema {
    ValidSchema::empty()
}

fn schema_with_field() -> ValidSchema {
    Schema::builder()
        .string(field_key!("token"), |s| s)
        .build()
        .expect("single-string schema always valid")
}

/// Composed metadata carrying an entity-specific field (`retry_budget`)
/// alongside the nested `BaseMetadata` prefix — the shape
/// `validate_base_compat` is meant to be called against in production
/// (through `Metadata::base()`), not the bare `BaseMetadata` every unit
/// test in `compat.rs` uses.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct ComposedMetadata {
    base: BaseMetadata<LocalKey>,
    retry_budget: u32,
}

impl Metadata for ComposedMetadata {
    type Key = LocalKey;
    fn base(&self) -> &BaseMetadata<Self::Key> {
        &self.base
    }
}

fn composed(
    k: &str,
    major: u64,
    minor: u64,
    schema: ValidSchema,
    retry_budget: u32,
) -> ComposedMetadata {
    ComposedMetadata {
        base: MetadataDraft::try_new(key(k), "n", "d")
            .expect("nonblank name")
            .with_version(Version::new(major, minor, 0))
            .bind_schema(schema)
            .expect("valid bounded metadata"),
        retry_budget,
    }
}

// Every case below gives `prev` and `curr` a *different* `retry_budget`
// (1 vs 99). To be precise about what that does and does not demonstrate:
// `validate_base_compat` takes `&BaseMetadata<K>`, and line ~115 passes
// `curr.base()`/`prev.base()` — so `retry_budget` structurally cannot
// reach the function at all; the type signature guarantees that on its
// own, before any assertion runs. The varying value does not add proof of
// field-ignoring beyond that signature; it exists only so a reader
// scanning the cases sees `retry_budget` differ and does not mistake it
// for a value the outcome depends on.
#[rstest]
#[case::minor_bump_same_schema_ok(
    composed("k", 1, 0, empty_schema(), 1),
    composed("k", 1, 1, empty_schema(), 99),
    Ok(())
)]
#[case::major_bump_schema_change_ok(
    composed("k", 1, 0, empty_schema(), 1),
    composed("k", 2, 0, schema_with_field(), 99),
    Ok(())
)]
#[case::key_changed(
    composed("old", 1, 0, empty_schema(), 1),
    composed("new", 1, 0, empty_schema(), 99),
    Err(BaseCompatError::KeyChanged { previous: key("old"), current: key("new") })
)]
#[case::version_regressed(
    composed("k", 2, 1, empty_schema(), 1),
    composed("k", 2, 0, empty_schema(), 99),
    Err(BaseCompatError::VersionRegressed {
        previous: Version::new(2, 1, 0),
        current: Version::new(2, 0, 0),
    })
)]
#[case::schema_change_without_major_bump(
    composed("k", 1, 0, empty_schema(), 1),
    composed("k", 1, 1, schema_with_field(), 99),
    Err(BaseCompatError::SchemaChangeWithoutMajorBump)
)]
fn compat_ignores_entity_specific_fields(
    #[case] prev: ComposedMetadata,
    #[case] curr: ComposedMetadata,
    #[case] expected: Result<(), BaseCompatError<LocalKey>>,
) {
    let result = validate_base_compat(curr.base(), prev.base());
    assert_eq!(result, expected, "prev={prev:?} curr={curr:?}");
}

#[test]
fn build_metadata_does_not_change_revision_precedence() {
    let previous = MetadataDraft::try_new(key("k"), "Name", "")
        .expect("nonblank name")
        .with_version("1.2.3+z".parse().expect("valid version"))
        .bind_schema(empty_schema())
        .expect("valid bounded metadata");
    let current = MetadataDraft::try_new(key("k"), "Name", "")
        .expect("nonblank name")
        .with_version("1.2.3+a".parse().expect("valid version"))
        .bind_schema(empty_schema())
        .expect("valid bounded metadata");
    assert_eq!(validate_base_compat(&current, &previous), Ok(()));
    assert_eq!(validate_base_compat(&previous, &current), Ok(()));
}

#[test]
fn prerelease_regression_is_still_a_revision_error() {
    let previous = MetadataDraft::try_new(key("k"), "Name", "")
        .expect("nonblank name")
        .with_version("1.2.3-beta.2".parse().expect("valid version"))
        .bind_schema(empty_schema())
        .expect("valid bounded metadata");
    let current = MetadataDraft::try_new(key("k"), "Name", "")
        .expect("nonblank name")
        .with_version("1.2.3-beta.1+new-build".parse().expect("valid version"))
        .bind_schema(empty_schema())
        .expect("valid bounded metadata");
    assert_eq!(
        validate_base_compat(&current, &previous),
        Err(BaseCompatError::VersionRegressed {
            previous: previous.version().clone(),
            current: current.version().clone(),
        })
    );
}
