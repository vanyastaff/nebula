//! `PluginManifest` build/serde flow: build, serde round-trip, dependency
//! declarations, key normalization, and a `try_new()`-shaped constructor
//! mirroring `CorePlugin::try_new` (`crates/plugin-core/src/plugin.rs:47`)
//! that must propagate `ManifestError` unchanged through `?` — an
//! `Err`-path a success-only test cannot prove.

use nebula_metadata::{
    DeprecationNotice, ManifestError, MaturityLevel, PluginDependency, PluginManifest,
};
use pretty_assertions::assert_eq;
use semver::{Version, VersionReq};

/// Local stand-in for a first-party plugin constructor, mirroring
/// `CorePlugin::try_new`: builds a manifest and propagates any
/// `ManifestError` unchanged through `?`.
#[derive(Debug)]
struct FixturePlugin {
    manifest: PluginManifest,
}

impl FixturePlugin {
    fn try_new(key: &str, name: &str) -> Result<Self, ManifestError> {
        let manifest = PluginManifest::builder(key, name)
            .description("fixture plugin for integration tests")
            .build()?;
        Ok(Self { manifest })
    }
}

#[test]
fn try_new_succeeds_for_a_valid_key_and_name() {
    let plugin = FixturePlugin::try_new("fixture", "Fixture").expect("valid manifest");
    assert_eq!(plugin.manifest.key().as_str(), "fixture");
    assert_eq!(plugin.manifest.name(), "Fixture");
}

#[test]
fn try_new_propagates_invalid_key_unchanged() {
    let err = FixturePlugin::try_new("", "Fixture").expect_err("empty key must be rejected");
    assert!(matches!(err, ManifestError::InvalidKey(_)), "got: {err:?}");
}

#[test]
fn try_new_propagates_missing_name_unchanged() {
    let err = FixturePlugin::try_new("fixture", "   ").expect_err("blank name must be rejected");
    assert_eq!(err, ManifestError::MissingRequiredField { field: "name" });
}

#[test]
fn build_full_manifest_round_trips_through_json() {
    let manifest = PluginManifest::builder("slack_notify", "Slack Notify")
        .description("Send Slack notifications")
        .version(Version::new(2, 3, 0))
        .group(vec!["messaging".into(), "notifications".into()])
        .inline_icon("slack")
        .color("#4A154B")
        .tags(vec!["chat".into()])
        .author("Acme Corp")
        .license("Apache-2.0")
        .homepage("https://example.com")
        .repository("https://github.com/acme/slack-notify")
        .nebula_version("0.5.0")
        .maturity(MaturityLevel::Beta)
        .dependency(PluginDependency::new(
            "auth".parse().expect("valid key"),
            "^1.0.0".parse::<VersionReq>().expect("valid req"),
        ))
        .build()
        .expect("valid manifest");

    let json = serde_json::to_string(&manifest).expect("serializes");
    let decoded: PluginManifest = serde_json::from_str(&json).expect("deserializes");

    assert_eq!(decoded, manifest);
    assert_eq!(decoded.dependencies().len(), 1);
    assert_eq!(decoded.dependencies()[0].key().as_str(), "auth");
}

#[test]
fn key_normalization_lowercases_and_replaces_spaces() {
    let manifest = PluginManifest::builder("Slack Notify", "Slack Notify")
        .build()
        .expect("valid manifest");
    assert_eq!(manifest.key().as_str(), "slack_notify");
}

#[test]
fn dependency_declarations_preserve_order() {
    let auth = PluginDependency::new(
        "auth".parse().expect("valid key"),
        "^1".parse().expect("valid req"),
    );
    let http_client = PluginDependency::new(
        "http_client".parse().expect("valid key"),
        ">=2.0.0".parse().expect("valid req"),
    );
    let manifest = PluginManifest::builder("consumer", "Consumer")
        .dependency(auth)
        .dependency(http_client)
        .build()
        .expect("valid manifest");

    assert_eq!(manifest.dependencies().len(), 2);
    assert_eq!(manifest.dependencies()[0].key().as_str(), "auth");
    assert_eq!(manifest.dependencies()[1].key().as_str(), "http_client");
}

#[test]
fn deprecation_notice_forces_deprecated_maturity_end_to_end() {
    let manifest = PluginManifest::builder("legacy", "Legacy")
        .deprecation(DeprecationNotice::new(Version::new(1, 0, 0)).reason("superseded"))
        .build()
        .expect("valid manifest");
    assert_eq!(manifest.maturity(), MaturityLevel::Deprecated);
}
