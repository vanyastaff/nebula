use nebula_core::{
    CredentialId, ExecutablePlanRevisionId, ExecutionContractBundleFingerprint,
    ExecutionContractBundleId, OrgId, PluginSetId, WorkerFlavorRevisionId, WorkflowVersionId,
    WorkspaceId,
};
use nebula_error::Classify;
use nebula_execution::{
    ExecutionContractBundle, ExecutionContractBundleIntegrityError, ExecutionProfile,
    ExecutionRevisions, RecordedExecutionContractBundleV1,
};
use serde_json::Value;

const GOLDEN_FINGERPRINT_V1: &str =
    "6d5c1fe5c9c59616db4f0b47db1675ed6559bbdb2b9bcfffc973435a162ead08";

fn bundle_id(byte: u8) -> ExecutionContractBundleId {
    ExecutionContractBundleId::from_bytes([byte; 16])
}

fn org_id(byte: u8) -> OrgId {
    OrgId::from_bytes([byte; 16])
}

fn workspace_id(byte: u8) -> WorkspaceId {
    WorkspaceId::from_bytes([byte; 16])
}

fn workflow_version_id(byte: u8) -> WorkflowVersionId {
    WorkflowVersionId::from_bytes([byte; 16])
}

fn credential_id(byte: u8) -> CredentialId {
    CredentialId::from_bytes([byte; 16])
}

fn plan_revision(byte: u8) -> ExecutablePlanRevisionId {
    ExecutablePlanRevisionId::from_bytes([byte; 32])
}

fn plugin_set(byte: u8) -> PluginSetId {
    PluginSetId::from_bytes([byte; 32])
}

fn flavor_revision(byte: u8) -> WorkerFlavorRevisionId {
    WorkerFlavorRevisionId::from_bytes([byte; 32])
}

fn revisions(workflow: u8, flavor: u8) -> ExecutionRevisions {
    ExecutionRevisions::new(workflow_version_id(workflow), flavor_revision(flavor))
}

fn canonical_bundle(id_byte: u8) -> ExecutionContractBundle {
    ExecutionContractBundle::new_graph_v1(
        bundle_id(id_byte),
        org_id(2),
        workspace_id(3),
        plan_revision(4),
        plugin_set(5),
        revisions(6, 7),
        [credential_id(9), credential_id(8), credential_id(9)],
    )
}

fn recorded_from(value: Value) -> RecordedExecutionContractBundleV1 {
    serde_json::from_value(value).expect("recorded bundle fixture must deserialize")
}

#[test]
fn graph_profile_has_the_canonical_wire_discriminant() {
    assert_eq!(
        serde_json::to_string(&ExecutionProfile::Graph).expect("profile must serialize"),
        "\"graph\""
    );
}

#[test]
fn graph_v1_constructor_canonicalizes_credentials_and_exposes_complete_read_only_state() {
    let bundle = canonical_bundle(1);

    assert_eq!(bundle.bundle_id(), bundle_id(1));
    assert_eq!(bundle.org_id(), org_id(2));
    assert_eq!(bundle.workspace_id(), workspace_id(3));
    assert_eq!(bundle.profile(), ExecutionProfile::Graph);
    assert_eq!(bundle.executable_plan_revision_id(), plan_revision(4));
    assert_eq!(bundle.plugin_set_id(), plugin_set(5));
    assert_eq!(bundle.revisions(), revisions(6, 7));
    assert_eq!(
        bundle.authorized_credential_ids(),
        &[credential_id(8), credential_id(9)]
    );
    assert_eq!(bundle.schema_version(), 1);
    assert_eq!(bundle.durable_envelope_version(), 1);
    assert_eq!(bundle.fingerprint_version(), 1);
}

#[test]
fn fingerprint_v1_matches_the_independent_golden_vector() {
    assert_eq!(
        canonical_bundle(1).fingerprint().to_string(),
        GOLDEN_FINGERPRINT_V1
    );
}

#[test]
fn random_bundle_identity_is_excluded_from_the_semantic_fingerprint() {
    let first = canonical_bundle(1);
    let second = canonical_bundle(10);

    assert_ne!(first.bundle_id(), second.bundle_id());
    assert_eq!(first.fingerprint(), second.fingerprint());
}

#[test]
fn every_constructor_controlled_semantic_field_changes_the_fingerprint() {
    let id = bundle_id(1);
    let base = canonical_bundle(1).fingerprint();
    let changed = [
        ExecutionContractBundle::new_graph_v1(
            id,
            org_id(20),
            workspace_id(3),
            plan_revision(4),
            plugin_set(5),
            revisions(6, 7),
            [credential_id(8), credential_id(9)],
        )
        .fingerprint(),
        ExecutionContractBundle::new_graph_v1(
            id,
            org_id(2),
            workspace_id(30),
            plan_revision(4),
            plugin_set(5),
            revisions(6, 7),
            [credential_id(8), credential_id(9)],
        )
        .fingerprint(),
        ExecutionContractBundle::new_graph_v1(
            id,
            org_id(2),
            workspace_id(3),
            plan_revision(40),
            plugin_set(5),
            revisions(6, 7),
            [credential_id(8), credential_id(9)],
        )
        .fingerprint(),
        ExecutionContractBundle::new_graph_v1(
            id,
            org_id(2),
            workspace_id(3),
            plan_revision(4),
            plugin_set(50),
            revisions(6, 7),
            [credential_id(8), credential_id(9)],
        )
        .fingerprint(),
        ExecutionContractBundle::new_graph_v1(
            id,
            org_id(2),
            workspace_id(3),
            plan_revision(4),
            plugin_set(5),
            revisions(60, 7),
            [credential_id(8), credential_id(9)],
        )
        .fingerprint(),
        ExecutionContractBundle::new_graph_v1(
            id,
            org_id(2),
            workspace_id(3),
            plan_revision(4),
            plugin_set(5),
            revisions(6, 70),
            [credential_id(8), credential_id(9)],
        )
        .fingerprint(),
        ExecutionContractBundle::new_graph_v1(
            id,
            org_id(2),
            workspace_id(3),
            plan_revision(4),
            plugin_set(5),
            revisions(6, 7),
            [credential_id(8), credential_id(99)],
        )
        .fingerprint(),
    ];

    for fingerprint in changed {
        assert_ne!(fingerprint, base);
    }
}

#[test]
fn bundle_wire_round_trip_revalidates_integrity_and_preserves_getters() {
    let bundle = canonical_bundle(1);
    let encoded = serde_json::to_value(&bundle).expect("bundle must serialize");
    let decoded: ExecutionContractBundle =
        serde_json::from_value(encoded.clone()).expect("canonical bundle must deserialize");

    assert_eq!(decoded.bundle_id(), bundle.bundle_id());
    assert_eq!(decoded.org_id(), bundle.org_id());
    assert_eq!(decoded.workspace_id(), bundle.workspace_id());
    assert_eq!(decoded.profile(), bundle.profile());
    assert_eq!(
        decoded.executable_plan_revision_id(),
        bundle.executable_plan_revision_id()
    );
    assert_eq!(decoded.plugin_set_id(), bundle.plugin_set_id());
    assert_eq!(decoded.revisions(), bundle.revisions());
    assert_eq!(
        decoded.authorized_credential_ids(),
        bundle.authorized_credential_ids()
    );
    assert_eq!(decoded.fingerprint(), bundle.fingerprint());

    let recorded = recorded_from(encoded);
    let reconstructed = ExecutionContractBundle::try_from_recorded_v1(recorded)
        .expect("canonical recorded bundle must pass integrity validation");
    assert_eq!(reconstructed.fingerprint(), bundle.fingerprint());
}

#[test]
fn recorded_v1_denies_unknown_fields() {
    let mut encoded = serde_json::to_value(canonical_bundle(1)).expect("bundle must serialize");
    encoded
        .as_object_mut()
        .expect("bundle wire shape must be an object")
        .insert("latest".to_owned(), Value::Bool(true));

    assert!(serde_json::from_value::<ExecutionContractBundle>(encoded.clone()).is_err());
    assert!(serde_json::from_value::<RecordedExecutionContractBundleV1>(encoded).is_err());
}

#[test]
fn recorded_v1_rejects_each_unsupported_protocol_version() {
    let encoded = serde_json::to_value(canonical_bundle(1)).expect("bundle must serialize");

    let mut schema = encoded.clone();
    schema["schema_version"] = Value::from(2);
    assert!(serde_json::from_value::<ExecutionContractBundle>(schema.clone()).is_err());
    assert!(matches!(
        ExecutionContractBundle::try_from_recorded_v1(recorded_from(schema)),
        Err(ExecutionContractBundleIntegrityError::UnsupportedSchemaVersion { actual: 2 })
    ));

    let mut envelope = encoded.clone();
    envelope["durable_envelope_version"] = Value::from(2);
    assert!(serde_json::from_value::<ExecutionContractBundle>(envelope.clone()).is_err());
    assert!(matches!(
        ExecutionContractBundle::try_from_recorded_v1(recorded_from(envelope)),
        Err(ExecutionContractBundleIntegrityError::UnsupportedEnvelopeVersion { actual: 2 })
    ));

    let mut fingerprint = encoded;
    fingerprint["fingerprint_version"] = Value::from(2);
    assert!(serde_json::from_value::<ExecutionContractBundle>(fingerprint.clone()).is_err());
    assert!(matches!(
        ExecutionContractBundle::try_from_recorded_v1(recorded_from(fingerprint)),
        Err(ExecutionContractBundleIntegrityError::UnsupportedFingerprintVersion { actual: 2 })
    ));
}

#[test]
fn recorded_v1_rejects_unsupported_profile() {
    let mut encoded = serde_json::to_value(canonical_bundle(1)).expect("bundle must serialize");
    encoded["profile"] = Value::String("stream".to_owned());

    assert!(serde_json::from_value::<ExecutionContractBundle>(encoded.clone()).is_err());
    assert!(matches!(
        ExecutionContractBundle::try_from_recorded_v1(recorded_from(encoded)),
        Err(ExecutionContractBundleIntegrityError::UnsupportedProfile)
    ));
}

#[test]
fn recorded_v1_rejects_noncanonical_or_duplicate_credentials() {
    let encoded = serde_json::to_value(canonical_bundle(1)).expect("bundle must serialize");

    let mut descending = encoded.clone();
    descending["authorized_credential_ids"] =
        serde_json::json!([credential_id(9), credential_id(8)]);
    assert!(serde_json::from_value::<ExecutionContractBundle>(descending.clone()).is_err());
    assert!(matches!(
        ExecutionContractBundle::try_from_recorded_v1(recorded_from(descending)),
        Err(ExecutionContractBundleIntegrityError::NonCanonicalCredentialIds)
    ));

    let mut duplicate = encoded;
    duplicate["authorized_credential_ids"] =
        serde_json::json!([credential_id(8), credential_id(8)]);
    assert!(serde_json::from_value::<ExecutionContractBundle>(duplicate.clone()).is_err());
    assert!(matches!(
        ExecutionContractBundle::try_from_recorded_v1(recorded_from(duplicate)),
        Err(ExecutionContractBundleIntegrityError::NonCanonicalCredentialIds)
    ));
}

#[test]
fn recorded_v1_rejects_forged_fingerprint() {
    let mut encoded = serde_json::to_value(canonical_bundle(1)).expect("bundle must serialize");
    encoded["fingerprint"] = Value::String("00".repeat(32));

    assert!(serde_json::from_value::<ExecutionContractBundle>(encoded.clone()).is_err());
    assert!(matches!(
        ExecutionContractBundle::try_from_recorded_v1(recorded_from(encoded)),
        Err(ExecutionContractBundleIntegrityError::FingerprintMismatch { .. })
    ));
}

#[test]
fn structural_integrity_failures_have_stable_classification_codes() {
    let fingerprints = (
        ExecutionContractBundleFingerprint::from_bytes([1; 32]),
        ExecutionContractBundleFingerprint::from_bytes([2; 32]),
    );
    let errors = [
        (
            ExecutionContractBundleIntegrityError::UnsupportedSchemaVersion { actual: 2 },
            "EXECUTION_CONTRACT_BUNDLE:UNSUPPORTED_SCHEMA_VERSION",
        ),
        (
            ExecutionContractBundleIntegrityError::UnsupportedEnvelopeVersion { actual: 2 },
            "EXECUTION_CONTRACT_BUNDLE:UNSUPPORTED_ENVELOPE_VERSION",
        ),
        (
            ExecutionContractBundleIntegrityError::UnsupportedFingerprintVersion { actual: 2 },
            "EXECUTION_CONTRACT_BUNDLE:UNSUPPORTED_FINGERPRINT_VERSION",
        ),
        (
            ExecutionContractBundleIntegrityError::UnsupportedProfile,
            "EXECUTION_CONTRACT_BUNDLE:UNSUPPORTED_PROFILE",
        ),
        (
            ExecutionContractBundleIntegrityError::NonCanonicalCredentialIds,
            "EXECUTION_CONTRACT_BUNDLE:NON_CANONICAL_CREDENTIAL_IDS",
        ),
        (
            ExecutionContractBundleIntegrityError::FingerprintMismatch {
                claimed: fingerprints.0,
                computed: fingerprints.1,
            },
            "EXECUTION_CONTRACT_BUNDLE:FINGERPRINT_MISMATCH",
        ),
    ];

    for (error, expected_code) in errors {
        assert_eq!(error.category(), nebula_error::ErrorCategory::Validation);
        assert_eq!(error.code().as_str(), expected_code);
    }
}
