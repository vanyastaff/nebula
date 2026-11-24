//! Round-trip and lifecycle laws for checked catalog definitions.

use nebula_core::ActionKey;
use nebula_metadata::{
    DeprecationNotice, MaturityLevel, Metadata, PluginManifest, RecordedBaseDefinition,
};
use nebula_schema::ValidSchema;
use proptest::prelude::*;
use semver::Version;
use serde_json::json;

proptest! {
    #[test]
    fn checked_catalogs_round_trip(
        key in "[a-z][a-z0-9]{0,12}(_[a-z][a-z0-9]{0,12})?",
        name_suffix in any::<String>(),
        description in any::<String>(),
        version in (any::<u64>(), any::<u64>(), any::<u64>()),
        deprecated in any::<bool>(),
    ) {
        let name = format!("Name {name_suffix}");
        let version = Version::new(version.0, version.1, version.2);
        let mut authored = Metadata::try_new(&name,
            &description,
        )
        .expect("generated nonblank name")
        .with_version(version.clone());
        let mut manifest = PluginManifest::builder(&key, &name)
            .description(&description)
            .version(version.clone())
            .nebula_version(version.clone());
        if deprecated {
            let notice = DeprecationNotice::new(version).with_reason(&description);
            authored = authored.with_deprecation(notice.clone());
            manifest = manifest.deprecation(notice);
        }
        let leaf = authored.bind(key.parse::<ActionKey>().expect("generated valid key"), ValidSchema::empty()).expect("valid bounded metadata");
        let manifest = manifest.build().expect("generated valid manifest");

        let recorded_leaf: RecordedBaseDefinition<ActionKey> = serde_json::from_value(json!(leaf))?;
        let restored_leaf = recorded_leaf.readmit_against(&leaf)?;
        prop_assert_eq!(restored_leaf, leaf);
        let decoded_manifest: PluginManifest = serde_json::from_value(json!(manifest))?;
        prop_assert_eq!(decoded_manifest, manifest);
    }

    #[test]
    fn name_checks_agree_with_the_nonblank_contract(name in any::<String>()) {
        let leaf = Metadata::try_new(&name,
            "",
        ).map(|authored| authored.bind("example".parse::<ActionKey>().expect("valid key"), ValidSchema::empty()).expect("valid bounded metadata"));
        let manifest = PluginManifest::builder("example", &name).build();
        let leaf_wire = serde_json::from_value::<RecordedBaseDefinition<ActionKey>>(json!({
            "metadata_wire_version": 2,
            "key": "example", "name": name, "description": "",
            "schema": ValidSchema::empty(),
        }));
        let manifest_wire = serde_json::from_value::<PluginManifest>(json!({
            "metadata_wire_version": 2,
            "key": "example", "name": name,
        }));

        if name.trim().is_empty() {
            prop_assert_eq!(leaf.expect_err("blank name"), nebula_metadata::MetadataError::BlankName);
            prop_assert_eq!(manifest.expect_err("blank name"), nebula_metadata::ManifestError::Metadata(nebula_metadata::MetadataError::BlankName));
            prop_assert!(leaf_wire.is_err());
            prop_assert!(manifest_wire.is_err());
        } else {
            let leaf = leaf?;
            let manifest = manifest?;
            prop_assert_eq!(leaf.name(), &name);
            prop_assert_eq!(manifest.name(), name.trim());
            prop_assert_eq!(leaf_wire?.readmit_against(&leaf)?, leaf);
            prop_assert_eq!(manifest_wire?, manifest);
        }
    }

    #[test]
    fn any_maturity_sequence_preserves_a_notice(
        maturities in prop::collection::vec(0_u8..3, 0..32),
    ) {
        let notice = DeprecationNotice::new(Version::new(1, 0, 0));
        let mut authored = Metadata::try_new("Example", "")?
            .with_deprecation(notice.clone());
        let mut manifest = PluginManifest::builder("example", "Example")
            .deprecation(notice.clone());
        for maturity in maturities {
            authored = match maturity {
                0 => authored.mark_experimental(),
                1 => authored.mark_beta(),
                _ => authored.mark_stable(),
            };
            manifest = manifest.maturity(match maturity {
                0 => MaturityLevel::Experimental,
                1 => MaturityLevel::Beta,
                _ => MaturityLevel::Stable,
            });
            let leaf = authored.clone().bind("example", ValidSchema::empty()).expect("valid bounded metadata");
            prop_assert_eq!(leaf.maturity(), MaturityLevel::Deprecated);
            prop_assert_eq!(leaf.deprecation(), Some(&notice));
        }
        let manifest = manifest.build()?;
        prop_assert_eq!(manifest.maturity(), MaturityLevel::Deprecated);
        prop_assert_eq!(manifest.deprecation(), Some(&notice));
    }
}
