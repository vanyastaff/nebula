//! A deprecation notice implies Deprecated maturity through setters and serde.

use nebula_metadata::{
    DeprecationNotice, MaturityLevel, Metadata, PluginManifest, RecordedBaseDefinition,
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

fn key() -> LocalKey {
    LocalKey("k".to_owned())
}

fn empty_schema() -> ValidSchema {
    ValidSchema::empty()
}

#[test]
fn base_definition_deprecation_forces_deprecated_through_serde_round_trip() {
    let original = Metadata::try_new("n", "d")
        .expect("nonblank name")
        .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)).with_reason("superseded"))
        .bind(key(), empty_schema())
        .expect("valid bounded metadata");
    assert_eq!(original.maturity(), MaturityLevel::Deprecated);

    let json = serde_json::to_string(&original).expect("serializes");
    let recorded: RecordedBaseDefinition<LocalKey> =
        serde_json::from_str(&json).expect("recorded metadata deserializes");
    let decoded = recorded
        .readmit_against(&original)
        .expect("record matches fresh definition");

    assert_eq!(decoded.maturity(), MaturityLevel::Deprecated);
    assert_eq!(decoded, original);
}

#[test]
fn plugin_manifest_deprecation_forces_deprecated_through_serde_round_trip() {
    let original = PluginManifest::builder("legacy", "Legacy")
        .version(Version::new(2, 0, 0))
        .deprecation(DeprecationNotice::new(Version::new(2, 0, 0)).with_reason("superseded"))
        .build()
        .expect("valid manifest");
    assert_eq!(original.maturity(), MaturityLevel::Deprecated);

    let json = serde_json::to_string(&original).expect("serializes");
    let decoded: PluginManifest = serde_json::from_str(&json).expect("deserializes");

    assert_eq!(decoded.maturity(), MaturityLevel::Deprecated);
    assert_eq!(decoded, original);
}

/// Builder call order cannot discard the meaning of a deprecation notice.
#[test]
fn plugin_manifest_builder_stays_order_independent() {
    let manifest = PluginManifest::builder("legacy", "Legacy")
        .version(Version::new(2, 0, 0))
        .deprecation(DeprecationNotice::new(Version::new(2, 0, 0)))
        .maturity(MaturityLevel::Stable)
        .build()
        .expect("valid manifest");

    assert_eq!(manifest.maturity(), MaturityLevel::Deprecated);
}

/// A later maturity setter must preserve both the notice and its meaning.
#[test]
fn metadata_authoring_order_preserves_the_deprecation_invariant() {
    let metadata = Metadata::try_new("n", "d")
        .expect("nonblank name")
        .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)))
        .mark_stable()
        .bind(key(), empty_schema())
        .expect("valid bounded metadata");

    assert_eq!(
        metadata.maturity(),
        MaturityLevel::Deprecated,
        "a maturity setter cannot contradict an attached notice"
    );
    assert_eq!(
        metadata.deprecation(),
        Some(&DeprecationNotice::new(Version::new(1, 0, 0))),
        "changing maturity must preserve the notice"
    );
}

/// Wire input follows the same notice precedence as the builder API.
#[test]
fn adversarial_json_deprecation_with_explicit_stable_maturity_stays_deprecated() {
    let adversarial = serde_json::json!({
        "metadata_wire_version": 2,
        "key": "k",
        "name": "n",
        "description": "d",
        "schema": serde_json::to_value(empty_schema()).expect("schema serializes"),
        "maturity": "stable",
        "deprecation": { "since": "1.0.0" },
    });

    let recorded: RecordedBaseDefinition<LocalKey> =
        serde_json::from_value(adversarial).expect("deprecation determines maturity");
    let fresh_definition = Metadata::try_new("n", "d")
        .expect("nonblank name")
        .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)))
        .bind(key(), empty_schema())
        .expect("valid bounded metadata");
    let decoded = recorded
        .readmit_against(&fresh_definition)
        .expect("recorded notice matches fresh definition");

    assert_eq!(
        decoded.maturity(),
        MaturityLevel::Deprecated,
        "the notice must determine maturity on the wire too"
    );
    assert_eq!(
        decoded.deprecation(),
        Some(&DeprecationNotice::new(Version::new(1, 0, 0))),
        "the complete notice must survive deserialization"
    );
}
