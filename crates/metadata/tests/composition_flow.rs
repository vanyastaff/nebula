//! `BaseMetadata<K>` composition contract, plus direct serde coverage of
//! every `skip_serializing_if` path on `BaseMetadata` itself.
//!
//! Three locally-defined composed metadata shapes stand in for the
//! production consumers: zero extra fields (mirrors
//! `nebula-resource::ResourceMetadata`), one scalar extra field (mirrors
//! `nebula-credential::CredentialMetadata::pattern`), and a
//! multi-field-with-collection shape (mirrors
//! `nebula-action::ActionMetadata`'s `category`/`isolation_level`/`inputs`).
//!
//! The flatten-placement assertion (`value.get("base").is_none()`) is not
//! new coverage on its own — `nebula-credential`
//! (`crates/credential/src/metadata.rs`) and `nebula-resource`
//! (`crates/resource/src/resource.rs`) already assert it on their real
//! composed types. What's missing everywhere, and what this file adds, is
//! exercising the same contract generically across representative shapes,
//! and direct `serde_json` coverage of `BaseMetadata` — `base.rs` has zero
//! `serde_json` calls in its own unit tests today.

use nebula_metadata::{BaseMetadata, DeprecationNotice, MaturityLevel, Metadata};
use nebula_schema::ValidSchema;
use pretty_assertions::assert_eq;
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalKey(String);

fn key(s: &str) -> LocalKey {
    LocalKey(s.to_owned())
}

fn empty_schema() -> ValidSchema {
    ValidSchema::empty()
}

fn base(name: &str) -> BaseMetadata<LocalKey> {
    BaseMetadata::new(key(name), name, "desc", empty_schema())
}

// --- Shape 1: zero extra fields ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ZeroExtraMetadata {
    #[serde(flatten)]
    base: BaseMetadata<LocalKey>,
}

impl Metadata for ZeroExtraMetadata {
    type Key = LocalKey;
    fn base(&self) -> &BaseMetadata<Self::Key> {
        &self.base
    }
}

// --- Shape 2: one scalar extra field ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ScalarExtraMetadata {
    #[serde(flatten)]
    base: BaseMetadata<LocalKey>,
    pattern: String,
}

impl Metadata for ScalarExtraMetadata {
    type Key = LocalKey;
    fn base(&self) -> &BaseMetadata<Self::Key> {
        &self.base
    }
}

// --- Shape 3: multi-field-with-collection ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MultiFieldMetadata {
    #[serde(flatten)]
    base: BaseMetadata<LocalKey>,
    category: String,
    priority: u32,
    capabilities: Vec<String>,
}

impl Metadata for MultiFieldMetadata {
    type Key = LocalKey;
    fn base(&self) -> &BaseMetadata<Self::Key> {
        &self.base
    }
}

#[test]
fn zero_extra_fields_flattens_and_round_trips() {
    let original = ZeroExtraMetadata { base: base("noop") };

    let json = serde_json::to_string(&original).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert!(
        value.get("base").is_none(),
        "shared metadata must stay flattened, not nested under `base`"
    );
    assert_eq!(
        value.get("key").and_then(serde_json::Value::as_str),
        Some("noop")
    );

    let decoded: ZeroExtraMetadata = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(decoded, original);
}

#[test]
fn scalar_extra_field_flattens_alongside_base() {
    let original = ScalarExtraMetadata {
        base: base("token_auth"),
        pattern: "secret_token".to_owned(),
    };

    let json = serde_json::to_string(&original).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert!(
        value.get("base").is_none(),
        "shared metadata must stay flattened, not nested under `base`"
    );
    assert_eq!(
        value.get("key").and_then(serde_json::Value::as_str),
        Some("token_auth")
    );
    assert_eq!(
        value.get("pattern").and_then(serde_json::Value::as_str),
        Some("secret_token")
    );

    let decoded: ScalarExtraMetadata = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(decoded, original);
}

#[test]
fn multi_field_with_collection_flattens_alongside_base() {
    let original = MultiFieldMetadata {
        base: base("http.request").with_tags(["network"]),
        category: "integration".to_owned(),
        priority: 3,
        capabilities: vec!["http".to_owned(), "retryable".to_owned()],
    };

    let json = serde_json::to_string(&original).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert!(
        value.get("base").is_none(),
        "shared metadata must stay flattened, not nested under `base`"
    );
    assert_eq!(
        value.get("key").and_then(serde_json::Value::as_str),
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

    let decoded: MultiFieldMetadata = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(decoded, original);
}

#[test]
fn metadata_trait_delegates_through_a_composed_shape() {
    let metadata = ScalarExtraMetadata {
        base: base("token_auth").mark_beta(),
        pattern: "secret_token".to_owned(),
    };

    assert_eq!(metadata.key(), &key("token_auth"));
    assert_eq!(metadata.name(), "token_auth");
    assert_eq!(metadata.maturity(), MaturityLevel::Beta);
}

// --- Direct `BaseMetadata` `skip_serializing_if` matrix ---

#[test]
fn default_version_is_omitted_and_explicit_version_is_present() {
    let default_value = serde_json::to_value(base("k")).expect("serializes");
    assert!(default_value.get("version").is_none());

    let explicit = base("k").with_version(Version::new(2, 0, 0));
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

    let with_icon = base("k").with_inline_icon("github");
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

    let with_tags = base("k").with_tags(["a", "b"]);
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

    let experimental = base("k").mark_experimental();
    let value = serde_json::to_value(&experimental).expect("serializes");
    assert_eq!(
        value.get("maturity").and_then(serde_json::Value::as_str),
        Some("experimental")
    );
}

#[test]
fn documentation_url_none_is_omitted_and_set_url_is_present() {
    let default_value = serde_json::to_value(base("k")).expect("serializes");
    assert!(default_value.get("documentation_url").is_none());

    let with_url = base("k").with_documentation_url("https://example.com/docs");
    let value = serde_json::to_value(&with_url).expect("serializes");
    assert_eq!(
        value
            .get("documentation_url")
            .and_then(serde_json::Value::as_str),
        Some("https://example.com/docs")
    );
}

#[test]
fn deprecation_none_is_omitted_and_set_notice_is_present() {
    let default_value = serde_json::to_value(base("k")).expect("serializes");
    assert!(default_value.get("deprecation").is_none());

    let deprecated = base("k").with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)));
    let value = serde_json::to_value(&deprecated).expect("serializes");
    assert_eq!(
        value
            .get("deprecation")
            .and_then(|d| d.get("since"))
            .and_then(serde_json::Value::as_str),
        Some("1.0.0")
    );
}
