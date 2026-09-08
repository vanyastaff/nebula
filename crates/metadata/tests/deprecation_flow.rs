//! Deprecation-forces-`Deprecated` through a full serde round trip, for
//! both `BaseMetadata<K>` and `PluginManifest`, plus two adversarial pins.
//!
//! "`deprecation` implies `maturity = Deprecated`" holds only at the moment
//! `with_deprecation()`/`deprecate()` (on `BaseMetadata`) or `.deprecation()`
//! (on `PluginManifestBuilder`) is called — it is not a standing invariant
//! of either type. `PluginManifestBuilder::build()` re-derives `maturity`
//! from `deprecation` at build time, so a later `.maturity(Stable)` call on
//! that builder cannot win (see
//! `plugin_manifest_deprecation_forces_deprecated_through_serde_round_trip`
//! and `manifest.rs`'s own
//! `deprecation_forces_deprecated_maturity_regardless_of_order`).
//! `BaseMetadata` has no such build step, so the invariant can be broken in
//! at least three ways, two of which are pinned here (the third — all ten
//! `BaseMetadata` fields being `pub`, so a caller can set `maturity` and
//! `deprecation` independently — is direct field assignment, not a
//! dedicated code path worth its own test):
//! - deserializing hand-written JSON that carries `deprecation` alongside
//!   an explicit non-`Deprecated` `maturity` (`maturity`/`deprecation` are
//!   independent fields with no `Deserialize`-time invariant);
//! - calling `.with_maturity(..)` *after* `.with_deprecation(..)` —
//!   `with_maturity` unconditionally overwrites `self.maturity` and does
//!   not re-check `self.deprecation`.

use nebula_metadata::{BaseMetadata, DeprecationNotice, MaturityLevel, PluginManifest};
use nebula_schema::ValidSchema;
use pretty_assertions::assert_eq;
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalKey(String);

fn key() -> LocalKey {
    LocalKey("k".to_owned())
}

fn empty_schema() -> ValidSchema {
    ValidSchema::empty()
}

#[test]
fn base_metadata_deprecation_forces_deprecated_through_serde_round_trip() {
    let original = BaseMetadata::new(key(), "n", "d", empty_schema())
        .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)).reason("superseded"));
    assert_eq!(original.maturity, MaturityLevel::Deprecated);

    let json = serde_json::to_string(&original).expect("serializes");
    let decoded: BaseMetadata<LocalKey> = serde_json::from_str(&json).expect("deserializes");

    assert_eq!(decoded.maturity, MaturityLevel::Deprecated);
    assert_eq!(decoded, original);
}

#[test]
fn plugin_manifest_deprecation_forces_deprecated_through_serde_round_trip() {
    let original = PluginManifest::builder("legacy", "Legacy")
        .deprecation(DeprecationNotice::new(Version::new(2, 0, 0)).reason("superseded"))
        .build()
        .expect("valid manifest");
    assert_eq!(original.maturity(), MaturityLevel::Deprecated);

    let json = serde_json::to_string(&original).expect("serializes");
    let decoded: PluginManifest = serde_json::from_str(&json).expect("deserializes");

    assert_eq!(decoded.maturity(), MaturityLevel::Deprecated);
    assert_eq!(decoded, original);
}

/// Contrasting case: `PluginManifestBuilder::build()` re-derives `maturity`
/// from `deprecation` at build time, so a `.maturity(Stable)` call *after*
/// `.deprecation(..)` cannot win — order-independent by construction. This
/// is the asymmetry with `BaseMetadata` (pinned below in
/// `base_metadata_builder_order_bypasses_the_deprecation_invariant`) made
/// concrete by a test, not left to prose.
#[test]
fn plugin_manifest_builder_stays_order_independent() {
    let manifest = PluginManifest::builder("legacy", "Legacy")
        .deprecation(DeprecationNotice::new(Version::new(2, 0, 0)))
        .maturity(MaturityLevel::Stable)
        .build()
        .expect("valid manifest");

    assert_eq!(manifest.maturity(), MaturityLevel::Deprecated);
}

/// Adversarial pin (bypass b): calling `.with_maturity(..)` *after*
/// `.with_deprecation(..)` on `BaseMetadata` leaves `deprecation: Some(..)`
/// with `maturity: Stable` — `with_maturity` unconditionally overwrites and
/// `BaseMetadata` has no build step to re-derive it afterward, unlike
/// `PluginManifestBuilder::build()` (see
/// `plugin_manifest_builder_stays_order_independent` above). This pins the
/// actual current behavior; it does not assert an invariant that does not
/// hold.
#[test]
fn base_metadata_builder_order_bypasses_the_deprecation_invariant() {
    let metadata = BaseMetadata::new(key(), "n", "d", empty_schema())
        .with_deprecation(DeprecationNotice::new(Version::new(1, 0, 0)))
        .with_maturity(MaturityLevel::Stable);

    assert_eq!(
        metadata.maturity,
        MaturityLevel::Stable,
        "with_maturity unconditionally overwrites, even after with_deprecation"
    );
    assert_eq!(
        metadata.deprecation,
        Some(DeprecationNotice::new(Version::new(1, 0, 0))),
        "deprecation stays set — nothing clears it when maturity is overwritten"
    );
}

/// Adversarial pin: `deprecation` set alongside an explicit
/// `maturity: "stable"` deserializes into an inconsistent `BaseMetadata` —
/// `deprecation` is `Some(..)` while `maturity` stays `Stable`, because the
/// derived `Deserialize` on `BaseMetadata` enforces no cross-field
/// invariant. Only the construction paths (`BaseMetadata::with_deprecation`,
/// `PluginManifestBuilder::build`) enforce "deprecation implies
/// `Deprecated`" — this pins the deserialize-time gap so a future
/// invariant-adding change must consciously update this assertion, not
/// discover the behavior by surprise.
#[test]
fn adversarial_json_deprecation_with_explicit_stable_maturity_deserializes_inconsistently() {
    let adversarial = serde_json::json!({
        "key": "k",
        "name": "n",
        "description": "d",
        "schema": serde_json::to_value(empty_schema()).expect("schema serializes"),
        "maturity": "stable",
        "deprecation": { "since": "1.0.0" },
    });

    let decoded: BaseMetadata<LocalKey> =
        serde_json::from_value(adversarial).expect("deserializes despite the inconsistency");

    assert_eq!(
        decoded.maturity,
        MaturityLevel::Stable,
        "maturity is taken verbatim from the wire, not derived from `deprecation`"
    );
    assert_eq!(
        decoded.deprecation,
        Some(DeprecationNotice::new(Version::new(1, 0, 0))),
        "deprecation is taken verbatim from the wire too, independent of maturity"
    );
}
