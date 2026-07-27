use std::sync::Arc;

use nebula_core::{ArtifactSetDigest, WorkerFlavorRevisionId};
use nebula_plugin::{
    Plugin, PluginManifest, PluginRegistry, RecordedWorkerFlavorRevisionV1, ResolvedPlugin,
    RuntimeContractVersion, WorkerFlavorIntegrityError, WorkerFlavorRevision,
};
use serde_json::json;

#[derive(Debug)]
struct TestPlugin(PluginManifest);

impl Plugin for TestPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.0
    }
}

fn frozen_registry() -> nebula_plugin::FrozenPluginRegistry {
    let manifest = PluginManifest::builder("record-fixture", "Record fixture")
        .build()
        .expect("test plugin manifest must be valid");
    let resolved = ResolvedPlugin::from(TestPlugin(manifest))
        .expect("test plugin must resolve without components");
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(resolved))
        .expect("test plugin key must be unique");
    registry
        .freeze(
            ArtifactSetDigest::from_bytes([0x5a; 32]),
            "1.2.3"
                .parse::<RuntimeContractVersion>()
                .expect("test runtime contract version must be valid"),
        )
        .expect("test registry must freeze")
}

fn recorded_revision() -> (
    RecordedWorkerFlavorRevisionV1,
    WorkerFlavorRevisionId,
    ArtifactSetDigest,
) {
    let frozen = frozen_registry();
    (
        RecordedWorkerFlavorRevisionV1::from(frozen.revision()),
        frozen.revision().id(),
        frozen.revision().artifact_set_digest(),
    )
}

#[test]
fn recorded_worker_flavor_round_trips_through_checked_loading() {
    let frozen = frozen_registry();
    let encoded = serde_json::to_vec(&RecordedWorkerFlavorRevisionV1::from(frozen.revision()))
        .expect("recorded worker flavor must serialize");
    let recorded: RecordedWorkerFlavorRevisionV1 =
        serde_json::from_slice(&encoded).expect("recorded worker flavor must deserialize");

    let reloaded = WorkerFlavorRevision::try_from_recorded_v1(recorded)
        .expect("canonical worker-flavor record must pass integrity checking");

    assert_eq!(reloaded.id(), frozen.revision().id());
    assert_eq!(reloaded.plugin_set_id(), frozen.revision().plugin_set_id());
    assert_eq!(
        reloaded.runtime_contract_version(),
        frozen.revision().runtime_contract_version()
    );
    assert_eq!(
        reloaded.artifact_set_digest(),
        frozen.revision().artifact_set_digest()
    );
}

#[test]
fn standard_try_from_rejects_a_forged_revision_identity() {
    let (recorded, expected_id, _) = recorded_revision();
    let forged_id = WorkerFlavorRevisionId::from_bytes([0xa5; 32]);
    let mut encoded = serde_json::to_value(recorded).expect("record must serialize");
    encoded["claimed_id"] = json!(forged_id);
    let forged: RecordedWorkerFlavorRevisionV1 =
        serde_json::from_value(encoded).expect("forged record remains valid wire data");

    let error = WorkerFlavorRevision::try_from(forged)
        .expect_err("a claimed identity must not bypass canonical derivation");

    assert!(matches!(
        error,
        WorkerFlavorIntegrityError::RevisionIdMismatch { claimed, computed }
            if claimed == forged_id && computed == expected_id
    ));
}

#[test]
fn recorded_worker_flavor_rejects_unknown_fields_during_deserialization() {
    let (recorded, _, _) = recorded_revision();
    let mut encoded = serde_json::to_value(recorded).expect("record must serialize");
    encoded["unexpected_authority"] = json!(true);

    let error = serde_json::from_value::<RecordedWorkerFlavorRevisionV1>(encoded)
        .expect_err("unknown fields must fail closed");

    assert!(
        error.to_string().contains("unknown field"),
        "serde diagnostic must identify the closed-record violation: {error}"
    );
}

#[test]
fn checked_loading_rejects_an_unknown_record_version() {
    let (recorded, _, _) = recorded_revision();
    let mut encoded = serde_json::to_value(recorded).expect("record must serialize");
    encoded["record_version"] = json!(2);
    let unsupported: RecordedWorkerFlavorRevisionV1 =
        serde_json::from_value(encoded).expect("version dispatch belongs to checked loading");

    let error = WorkerFlavorRevision::try_from_recorded_v1(unsupported)
        .expect_err("unknown record versions must fail closed");

    assert!(matches!(
        error,
        WorkerFlavorIntegrityError::UnsupportedRecordVersion { found: 2 }
    ));
}

#[test]
fn checked_loading_rejects_an_unknown_canonical_hash_version() {
    let (recorded, _, _) = recorded_revision();
    let mut encoded = serde_json::to_value(recorded).expect("record must serialize");
    encoded["canonical_hash_version"] = json!(2);
    let unsupported: RecordedWorkerFlavorRevisionV1 =
        serde_json::from_value(encoded).expect("version dispatch belongs to checked loading");

    let error = WorkerFlavorRevision::try_from_recorded_v1(unsupported)
        .expect_err("unknown canonical hash versions must fail closed");

    assert!(matches!(
        error,
        WorkerFlavorIntegrityError::UnsupportedCanonicalHashVersion { found: 2 }
    ));
}

#[test]
fn recorded_worker_flavor_debug_omits_artifact_provenance() {
    let (recorded, expected_id, artifact_set_digest) = recorded_revision();

    let rendered = format!("{recorded:?}");

    assert!(rendered.contains("RecordedWorkerFlavorRevisionV1"));
    assert!(rendered.contains(&expected_id.to_string()));
    assert!(!rendered.contains("artifact_set_digest"));
    assert!(!rendered.contains(&artifact_set_digest.to_string()));
}
