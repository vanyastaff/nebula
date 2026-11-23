//! `BaseDefinition<K>` composition contract, plus direct serde coverage of
//! every `skip_serializing_if` path on `BaseDefinition` itself.
//!
//! Three locally-defined composed metadata shapes stand in for the
//! production consumers: zero extra fields (mirrors
//! `nebula-resource::ResourceDefinition`), one scalar extra field (mirrors
//! `nebula-credential::CredentialMetadata::pattern`), and a
//! multi-field-with-collection shape (mirrors
//! `nebula-action::ActionMetadata`'s `category`/`isolation_level`/`inputs`).
//!
//! Nested `base` placement keeps strict shared-field decoding independent of
//! leaf-specific fields. These fixtures exercise that contract across shapes
//! and retain direct coverage of shared default-field omission.

use nebula_metadata::{
    BaseDefinition, Definition, DeprecationNotice, MaturityLevel, Metadata, RecordedBaseDefinition,
};
use nebula_schema::ValidSchema;
use pretty_assertions::assert_eq;
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

fn key(s: &str) -> LocalKey {
    LocalKey(s.to_owned())
}

fn empty_schema() -> ValidSchema {
    ValidSchema::empty()
}

fn base(name: &str) -> BaseDefinition<LocalKey> {
    authored(name)
        .bind(key(name), empty_schema())
        .expect("valid bounded metadata")
}

fn authored(name: &str) -> Metadata {
    Metadata::try_new(name, "desc").expect("nonblank name")
}

// --- Shape 1: zero extra fields ---

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ZeroExtraMetadata {
    base: BaseDefinition<LocalKey>,
}

#[derive(Debug, Deserialize)]
struct RecordedZeroExtraMetadata {
    base: RecordedBaseDefinition<LocalKey>,
}

impl Definition for ZeroExtraMetadata {
    type Key = LocalKey;
    fn base(&self) -> &BaseDefinition<Self::Key> {
        &self.base
    }
}

// --- Shape 2: one scalar extra field ---

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ScalarExtraMetadata {
    base: BaseDefinition<LocalKey>,
    pattern: String,
}

#[derive(Debug, Deserialize)]
struct RecordedScalarExtraMetadata {
    base: RecordedBaseDefinition<LocalKey>,
    pattern: String,
}

impl Definition for ScalarExtraMetadata {
    type Key = LocalKey;
    fn base(&self) -> &BaseDefinition<Self::Key> {
        &self.base
    }
}

// --- Shape 3: multi-field-with-collection ---

#[derive(Debug, Clone, PartialEq, Serialize)]
struct MultiFieldMetadata {
    base: BaseDefinition<LocalKey>,
    category: String,
    priority: u32,
    capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RecordedMultiFieldMetadata {
    base: RecordedBaseDefinition<LocalKey>,
    category: String,
    priority: u32,
    capabilities: Vec<String>,
}

impl Definition for MultiFieldMetadata {
    type Key = LocalKey;
    fn base(&self) -> &BaseDefinition<Self::Key> {
        &self.base
    }
}

#[test]
fn zero_extra_fields_nests_and_round_trips() {
    let original = ZeroExtraMetadata { base: base("noop") };

    let json = serde_json::to_string(&original).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert!(
        value.get("base").is_some(),
        "shared metadata must be nested under `base`"
    );
    assert_eq!(
        value["base"].get("key").and_then(serde_json::Value::as_str),
        Some("noop")
    );

    let recorded: RecordedZeroExtraMetadata =
        serde_json::from_str(&json).expect("recorded metadata deserializes");
    let restored = ZeroExtraMetadata {
        base: recorded
            .base
            .readmit_against(&original.base)
            .expect("record matches fresh definition"),
    };
    assert_eq!(restored, original);
}

#[test]
fn scalar_extra_field_nests_alongside_base() {
    let original = ScalarExtraMetadata {
        base: base("token_auth"),
        pattern: "secret_token".to_owned(),
    };

    let json = serde_json::to_string(&original).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert!(
        value.get("base").is_some(),
        "shared metadata must be nested under `base`"
    );
    assert_eq!(
        value["base"].get("key").and_then(serde_json::Value::as_str),
        Some("token_auth")
    );
    assert_eq!(
        value.get("pattern").and_then(serde_json::Value::as_str),
        Some("secret_token")
    );

    let recorded: RecordedScalarExtraMetadata =
        serde_json::from_str(&json).expect("recorded metadata deserializes");
    let restored = ScalarExtraMetadata {
        base: recorded
            .base
            .readmit_against(&original.base)
            .expect("record matches fresh definition"),
        pattern: recorded.pattern,
    };
    assert_eq!(restored, original);
}

#[test]
fn multi_field_with_collection_nests_alongside_base() {
    let original = MultiFieldMetadata {
        base: authored("http.request")
            .with_tags(["network"])
            .bind(key("http.request"), empty_schema())
            .expect("valid bounded metadata"),
        category: "integration".to_owned(),
        priority: 3,
        capabilities: vec!["http".to_owned(), "retryable".to_owned()],
    };

    let json = serde_json::to_string(&original).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert!(
        value.get("base").is_some(),
        "shared metadata must be nested under `base`"
    );
    assert_eq!(
        value["base"].get("key").and_then(serde_json::Value::as_str),
        Some("http.request")
    );
    assert_eq!(
        value.get("category").and_then(serde_json::Value::as_str),
        Some("integration")
    );
    assert_eq!(
        value.get("priority").and_then(serde_json::Value::as_u64),
        Some(3)
    );
    assert_eq!(
        value
            .get("capabilities")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );

    let recorded: RecordedMultiFieldMetadata =
        serde_json::from_str(&json).expect("recorded metadata deserializes");
    let restored = MultiFieldMetadata {
        base: recorded
            .base
            .readmit_against(&original.base)
            .expect("record matches fresh definition"),
        category: recorded.category,
        priority: recorded.priority,
        capabilities: recorded.capabilities,
    };
    assert_eq!(restored, original);
}

#[test]
fn definition_trait_delegates_through_a_composed_shape() {
    let metadata = ScalarExtraMetadata {
        base: authored("token_auth")
            .mark_beta()
            .bind(key("token_auth"), empty_schema())
            .expect("valid bounded metadata"),
        pattern: "secret_token".to_owned(),
    };

    assert_eq!(metadata.key(), &key("token_auth"));
    assert_eq!(metadata.name(), "token_auth");
    assert_eq!(metadata.maturity(), MaturityLevel::Beta);
}

// --- Direct `BaseDefinition` `skip_serializing_if` matrix ---

#[test]
fn default_version_is_omitted_and_explicit_version_is_present() {
    let default_value = serde_json::to_value(base("k")).expect("serializes");
    assert!(default_value.get("version").is_none());

    let explicit = authored("k")
        .with_version(Version::new(2, 0, 0))
        .bind(key("k"), empty_schema())
        .expect("valid bounded metadata");
    let explicit_value = serde_json::to_value(&explicit).expect("serializes");
    assert_eq!(
        explicit_value
            .get("version")
            .and_then(serde_json::Value::as_str),
        Some("2.0.0")
    );
}

#[test]
fn icon_none_is_omitted_and_set_icon_is_present() {
    let default_value = serde_json::to_value(base("k")).expect("serializes");
    assert!(default_value.get("icon").is_none());

    let with_icon = authored("k")
        .with_inline_icon("github")
        .bind(key("k"), empty_schema())
        .expect("valid bounded metadata");
    let value = serde_json::to_value(&with_icon).expect("serializes");
    assert_eq!(
        value.get("icon").and_then(serde_json::Value::as_str),
        Some("github")
    );
}

#[test]
fn empty_tags_is_omitted_and_nonempty_tags_is_present() {
    let default_value = serde_json::to_value(base("k")).expect("serializes");
    assert!(default_value.get("tags").is_none());

    let with_tags = authored("k")
        .with_tags(["a", "b"])
        .bind(key("k"), empty_schema())
        .expect("valid bounded metadata");
    let value = serde_json::to_value(&with_tags).expect("serializes");
    assert_eq!(
        value
            .get("tags")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn default_maturity_is_omitted_and_nondefault_maturity_is_present() {
    let default_value = serde_json::to_value(base("k")).expect("serializes");
    assert!(default_value.get("maturity").is_none());

    let experimental = authored("k")
        .mark_experimental()
        .bind(key("k"), empty_schema())
        .expect("valid bounded metadata");
    let value = serde_json::to_value(&experimental).expect("serializes");
    assert_eq!(
        value.get("maturity").and_then(serde_json::Value::as_str),
        Some("experimental")
    );
}

#[test]
fn documentation_url_none_is_omitted_and_set_url_is_present() {
    let default_value = serde_json::to_value(base("k")).expect("serializes");
    assert!(default_value.get("links").is_none());

    let with_url = authored("k")
        .with_documentation_url("https://example.com/docs")
        .bind(key("k"), empty_schema())
        .expect("valid bounded metadata");
    let value = serde_json::to_value(&with_url).expect("serializes");
    assert_eq!(
        value
            .pointer("/links/0/target")
            .and_then(serde_json::Value::as_str),
        Some("https://example.com/docs")
    );
}

#[test]
fn deprecation_none_is_omitted_and_set_notice_is_present() {
    let default_value = serde_json::to_value(base("k")).expect("serializes");
    assert!(default_value.get("deprecation").is_none());

    let deprecated = authored("k")
        .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)))
        .bind(key("k"), empty_schema())
        .expect("valid bounded metadata");
    let value = serde_json::to_value(&deprecated).expect("serializes");
    assert_eq!(
        value
            .get("deprecation")
            .and_then(|d| d.get("since"))
            .and_then(serde_json::Value::as_str),
        Some("1.0.0")
    );
}
