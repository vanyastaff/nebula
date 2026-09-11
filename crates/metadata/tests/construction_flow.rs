//! Intrinsic catalog validity must hold at every public construction boundary.

use nebula_core::ActionKey;
use nebula_metadata::{
    BaseMetadata, DeprecationNotice, MaturityLevel, MetadataDraft, MetadataError, PluginManifest,
    RecordedBaseMetadata,
};
use nebula_schema::ValidSchema;
use semver::Version;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

fn leaf_json() -> Value {
    json!({
        "key": "example.action",
        "name": "Example",
        "description": "",
        "schema": ValidSchema::empty(),
    })
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, serde_json::Error> {
    serde_json::from_str(&value.to_string())
}

#[test]
fn leaf_deserialization_rejects_blank_names() {
    for name in ["", " \t\n", "\u{2003}"] {
        let mut value = leaf_json();
        value["name"] = json!(name);
        let error = decode::<RecordedBaseMetadata<ActionKey>>(value)
            .expect_err("a catalog display name must contain non-whitespace text");
        assert!(error.to_string().contains("name"), "{error}");
    }
}

#[test]
fn manifest_deserialization_rejects_the_same_blank_names_as_build() {
    for name in ["", " \t\n", "\u{2003}"] {
        let expected = PluginManifest::builder("example", name)
            .build()
            .expect_err("builder rejects blank names");
        let error = decode::<PluginManifest>(json!({
            "key": "example",
            "name": name,
        }))
        .expect_err("deserialization must enforce builder name validation");
        assert!(
            error.to_string().starts_with(&expected.to_string()),
            "{error}"
        );
    }
}

#[test]
fn manifest_deserialization_uses_builder_key_normalization() {
    let expected = PluginManifest::builder("HTTP Request", "HTTP Request")
        .build()
        .expect("valid normalized key");
    let decoded: PluginManifest = decode(json!({
        "key": "HTTP Request",
        "name": "HTTP Request",
    }))
    .expect("the builder and deserializer accept the same raw key");
    assert_eq!(decoded, expected);
}

#[test]
fn deserialization_rejects_invalid_typed_identity_and_version() {
    for (field, invalid) in [("key", "bad!key"), ("key", "a_"), ("version", "01.0.0")] {
        let mut leaf = leaf_json();
        leaf[field] = json!(invalid);
        decode::<RecordedBaseMetadata<ActionKey>>(leaf)
            .expect_err("leaf identity and version must retain their typed validation");

        let mut manifest = json!({ "key": "example", "name": "Example" });
        manifest[field] = json!(invalid);
        decode::<PluginManifest>(manifest)
            .expect_err("manifest identity and version must retain their typed validation");
    }
}

#[test]
fn manifest_deserialization_rejects_invalid_minimum_engine_version() {
    let error = decode::<PluginManifest>(json!({
        "key": "example",
        "name": "Example",
        "nebula_version": "not-a-version",
    }))
    .expect_err("the minimum engine version is a semver version too");
    assert!(error.to_string().contains("version"), "{error}");
}

#[test]
fn notice_takes_precedence_over_every_wire_maturity() {
    let notice = DeprecationNotice::new(Version::new(2, 0, 0)).reason("Superseded");
    for maturity in [
        None,
        Some("experimental"),
        Some("beta"),
        Some("stable"),
        Some("deprecated"),
    ] {
        let mut leaf = leaf_json();
        let mut manifest = json!({ "key": "example", "name": "Example" });
        for value in [&mut leaf, &mut manifest] {
            value["deprecation"] = json!(notice);
            if let Some(maturity) = maturity {
                value["maturity"] = json!(maturity);
            }
        }

        let recorded: RecordedBaseMetadata<ActionKey> = decode(leaf).expect("valid notice");
        let fresh = MetadataDraft::try_new(
            "example.action".parse::<ActionKey>().expect("valid key"),
            "Example",
            "",
        )
        .expect("valid metadata")
        .with_deprecation(notice.clone())
        .bind_schema(ValidSchema::empty());
        let leaf = recorded
            .readmit_against(&fresh)
            .expect("recorded notice matches fresh definition");
        assert_eq!(leaf.maturity(), MaturityLevel::Deprecated);
        assert_eq!(leaf.deprecation(), Some(&notice));
        let manifest: PluginManifest = decode(manifest).expect("valid notice");
        assert_eq!(manifest.maturity(), MaturityLevel::Deprecated);
        assert_eq!(manifest.deprecation(), Some(&notice));
    }
}

#[test]
fn leaf_deserialization_rejects_deprecated_without_notice() {
    let mut value = leaf_json();
    value["maturity"] = json!("deprecated");
    let error = decode::<RecordedBaseMetadata<ActionKey>>(value)
        .expect_err("Deprecated requires a deprecation notice");
    assert!(error.to_string().contains("deprecation"), "{error}");
}

#[test]
fn manifest_build_and_deserialization_reject_deprecated_without_notice() {
    let expected = PluginManifest::builder("example", "Example")
        .maturity(MaturityLevel::Deprecated)
        .build()
        .expect_err("Deprecated requires a deprecation notice");
    let error = decode::<PluginManifest>(json!({
        "key": "example",
        "name": "Example",
        "maturity": "deprecated",
    }))
    .expect_err("deserialization must enforce the builder lifecycle contract");
    assert!(
        error.to_string().starts_with(&expected.to_string()),
        "{error}"
    );
}

#[test]
fn recorded_leaf_accepts_valid_keys_from_owned_json_and_readers() {
    let value = leaf_json();
    let expected: RecordedBaseMetadata<ActionKey> = decode(value.clone()).expect("borrowed JSON");
    let owned: RecordedBaseMetadata<ActionKey> =
        serde_json::from_value(value.clone()).expect("owned JSON");
    assert_eq!(owned, expected);
    let reader: RecordedBaseMetadata<ActionKey> =
        serde_json::from_reader(value.to_string().as_bytes()).expect("JSON reader");
    assert_eq!(reader, expected);
}

#[test]
fn manifest_accepts_valid_dependency_keys_from_owned_json_and_readers() {
    let value = json!({
        "key": "example",
        "name": "Example",
        "dependencies": [{ "key": "dependency", "req": "^1.0.0" }],
    });
    let expected: PluginManifest = decode(value.clone()).expect("borrowed JSON");
    let owned: PluginManifest = serde_json::from_value(value.clone()).expect("owned JSON");
    assert_eq!(owned, expected);
    let reader: PluginManifest =
        serde_json::from_reader(value.to_string().as_bytes()).expect("JSON reader");
    assert_eq!(reader, expected);
}

#[test]
fn draft_constructor_rejects_blank_names_with_a_typed_error() {
    for name in ["", " \t\n", "\u{2003}"] {
        let error =
            MetadataDraft::try_new("example".parse::<ActionKey>().expect("valid key"), name, "")
                .expect_err("a catalog display name must contain non-whitespace text");
        assert_eq!(error, MetadataError::BlankName);
    }
}

#[test]
fn draft_lifecycle_setters_preserve_notice_precedence() {
    let draft = MetadataDraft::try_new(
        "example".parse::<ActionKey>().expect("valid key"),
        "Example",
        "",
    )
    .expect("valid metadata");

    let notice = DeprecationNotice::new(Version::new(2, 0, 0)).reason("Superseded");
    let deprecated = draft.with_deprecation(notice.clone());
    for updated in [
        deprecated.clone().mark_experimental(),
        deprecated.clone().mark_beta(),
        deprecated.mark_stable(),
    ] {
        let updated = updated.bind_schema(ValidSchema::empty());
        assert_eq!(updated.maturity(), MaturityLevel::Deprecated);
        assert_eq!(updated.deprecation(), Some(&notice));
    }
}

#[test]
fn active_maturity_can_change_before_schema_binding() {
    let metadata = MetadataDraft::try_new("example".to_owned(), "Example", "")
        .expect("valid metadata")
        .mark_beta()
        .mark_experimental()
        .mark_stable()
        .bind_schema(ValidSchema::empty());
    assert_eq!(metadata.maturity(), MaturityLevel::Stable);
    assert_eq!(metadata.deprecation(), None);
    let recorded: RecordedBaseMetadata<String> =
        serde_json::from_value(json!(metadata)).expect("active lifecycle is recorded");
    let restored = recorded
        .readmit_against(&metadata)
        .expect("record matches fresh definition");
    assert_eq!(restored, metadata);
}

#[test]
fn intrinsic_construction_accepts_semver_boundaries_and_unicode_names() {
    let name = "\u{0418}\u{043c}\u{044f}";
    for version in [
        Version::new(0, 0, 0),
        Version::new(u64::MAX, u64::MAX, u64::MAX),
        "1.2.3-beta.1+build.2".parse().expect("valid semver"),
    ] {
        let metadata = MetadataDraft::try_new("example".to_owned(), name, "")
            .expect("Unicode display name and empty description are valid")
            .with_version(version.clone())
            .bind_schema(ValidSchema::empty());
        assert_eq!(metadata.name(), name);
        assert_eq!(metadata.description(), "");
        assert_eq!(metadata.version(), &version);
        let recorded: RecordedBaseMetadata<String> =
            serde_json::from_value(json!(metadata)).expect("valid semver is recorded");
        let restored = recorded
            .readmit_against(&metadata)
            .expect("record matches fresh definition");
        assert_eq!(restored, metadata);
    }
}

#[test]
fn flattened_typed_metadata_validates_and_preserves_outer_fields() {
    #[derive(Debug, PartialEq, serde::Serialize)]
    struct Entity {
        #[serde(flatten)]
        base: BaseMetadata<ActionKey>,
        category: String,
    }

    #[derive(Debug, serde::Deserialize)]
    struct RecordedEntity {
        #[serde(flatten)]
        base: RecordedBaseMetadata<ActionKey>,
        category: String,
    }

    let original = Entity {
        base: MetadataDraft::try_new("example".parse().expect("valid key"), "Example", "")
            .expect("valid metadata")
            .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)))
            .bind_schema(ValidSchema::empty()),
        category: "network".to_owned(),
    };
    let mut value = json!(original);
    assert!(value.get("base").is_none());
    let recorded: RecordedEntity =
        serde_json::from_value(value.clone()).expect("flattened recorded JSON");
    let restored = Entity {
        base: recorded
            .base
            .readmit_against(&original.base)
            .expect("record matches fresh definition"),
        category: recorded.category,
    };
    assert_eq!(restored, original);
    value["name"] = json!(" ");
    serde_json::from_value::<RecordedEntity>(value)
        .expect_err("flattening cannot bypass validation");
}

#[test]
fn key_parser_errors_do_not_expose_submitted_text() {
    #[derive(Debug)]
    struct RejectedKey;

    impl std::str::FromStr for RejectedKey {
        type Err = String;

        fn from_str(value: &str) -> Result<Self, Self::Err> {
            Err(value.to_owned())
        }
    }

    let mut value = leaf_json();
    value["key"] = json!("private_key_payload");
    let error = serde_json::from_value::<RecordedBaseMetadata<RejectedKey>>(value)
        .expect_err("custom key parser rejects the identity");
    assert_eq!(error.to_string(), MetadataError::InvalidKey.to_string());
    assert!(!format!("{error:?}").contains("private_key_payload"));
}
