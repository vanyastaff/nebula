use nebula_core::{
    CredentialId, CredentialKey, ExecutablePlanRevisionId, ExecutionContractBundleId, NodeKey,
    OrgId, PluginSetId, ResourceId, ResourceKey, WorkerFlavorRevisionId, WorkflowVersionId,
    WorkspaceId,
};
use nebula_execution::{
    BindingContractVersion, CredentialBindingContractV2, CredentialCapability,
    ExecutionBindingEntryV2, ExecutionBindingManifestV2, ExecutionBindingSiteV2,
    ExecutionBindingTargetV2, ExecutionContractBundleV2, ExecutionRevisions,
    RecordedExecutionContractBundleV1, RecordedExecutionContractBundleV2,
    ResourceBindingContractV2,
};
use serde_json::Value;

fn credential_entry(node: u8, slot: &str, credential: u8) -> ExecutionBindingEntryV2 {
    ExecutionBindingEntryV2::new(
        ExecutionBindingSiteV2::Node(NodeKey::new(format!("node-{node}")).unwrap()),
        slot,
        ExecutionBindingTargetV2::Credential {
            credential_id: CredentialId::from_bytes([credential; 16]),
            contract: CredentialBindingContractV2::new(
                CredentialKey::new("demo.api_key").unwrap(),
                BindingContractVersion::parse("1.2.3").unwrap(),
                [
                    CredentialCapability::Refreshable,
                    CredentialCapability::Revocable,
                ],
            ),
        },
    )
    .unwrap()
}

fn resource_entry(trigger: u8, slot: &str, resource: u8) -> ExecutionBindingEntryV2 {
    ExecutionBindingEntryV2::new(
        ExecutionBindingSiteV2::Trigger(NodeKey::new(format!("hook-{trigger}")).unwrap()),
        slot,
        ExecutionBindingTargetV2::Resource {
            resource_id: ResourceId::from_bytes([resource; 16]),
            contract: ResourceBindingContractV2::new(
                ResourceKey::new("demo.client").unwrap(),
                BindingContractVersion::parse("2.0.0").unwrap(),
            ),
        },
    )
    .unwrap()
}

fn bundle(entries: impl IntoIterator<Item = ExecutionBindingEntryV2>) -> ExecutionContractBundleV2 {
    ExecutionContractBundleV2::new_graph_v2(
        ExecutionContractBundleId::from_bytes([1; 16]),
        OrgId::from_bytes([2; 16]),
        WorkspaceId::from_bytes([3; 16]),
        ExecutablePlanRevisionId::from_bytes([4; 32]),
        PluginSetId::from_bytes([5; 32]),
        ExecutionRevisions::new(
            WorkflowVersionId::from_bytes([6; 16]),
            WorkerFlavorRevisionId::from_bytes([7; 32]),
        ),
        ExecutionBindingManifestV2::new(entries).unwrap(),
    )
}

#[test]
fn v2_manifest_is_site_qualified_canonical_and_round_trips() {
    let bundle = bundle([
        resource_entry(2, "client", 9),
        credential_entry(1, "auth", 8),
    ]);
    let encoded = serde_json::to_value(&bundle).unwrap();
    let decoded: ExecutionContractBundleV2 = serde_json::from_value(encoded).unwrap();

    assert_eq!(decoded, bundle);
    assert_eq!(decoded.schema_version(), 2);
    assert_eq!(decoded.durable_envelope_version(), 2);
    assert_eq!(decoded.fingerprint_version(), 2);
    assert_eq!(decoded.binding_manifest().entries().len(), 2);
    assert_eq!(decoded.binding_manifest().entries()[0].slot_key(), "auth");
}

#[test]
fn fingerprint_v2_matches_the_independent_golden_vector() {
    assert_eq!(
        bundle([credential_entry(1, "auth", 8)])
            .fingerprint()
            .to_string(),
        "97b86fe4e2eded16bd78a7ec341b186fa1481e10d97b9075a02b74830825e000"
    );
}

#[test]
fn fingerprint_binds_tenant_revisions_and_the_full_site_slot_mapping() {
    let encoded = serde_json::to_value(bundle([credential_entry(1, "auth", 8)])).unwrap();
    let mutations: Vec<fn(&mut Value)> = vec![
        |value| {
            value["org_id"] = serde_json::to_value(OrgId::from_bytes([20; 16])).unwrap();
        },
        |value| {
            value["workspace_id"] =
                serde_json::to_value(WorkspaceId::from_bytes([30; 16])).unwrap();
        },
        |value| {
            value["executable_plan_revision_id"] =
                serde_json::to_value(ExecutablePlanRevisionId::from_bytes([40; 32])).unwrap();
        },
        |value| {
            value["plugin_set_id"] =
                serde_json::to_value(PluginSetId::from_bytes([50; 32])).unwrap();
        },
        |value| {
            value["revisions"]["workflow"] =
                serde_json::to_value(WorkflowVersionId::from_bytes([60; 16])).unwrap();
        },
        |value| {
            value["revisions"]["worker_flavor"] =
                serde_json::to_value(WorkerFlavorRevisionId::from_bytes([70; 32])).unwrap();
        },
        |value| {
            value["binding_manifest"][0]["site"]["key"] = Value::String("node-9".to_owned());
        },
        |value| {
            value["binding_manifest"][0]["slot_key"] = Value::String("secondary_auth".to_owned());
        },
        |value| {
            value["binding_manifest"][0]["credential_id"] =
                serde_json::to_value(CredentialId::from_bytes([80; 16])).unwrap();
        },
        |value| {
            value["binding_manifest"][0]["contract"]["key"] =
                Value::String("demo.oauth".to_owned());
        },
        |value| {
            value["binding_manifest"][0]["contract"]["version"] = Value::String("1.2.4".to_owned());
        },
        |value| {
            value["binding_manifest"][0]["contract"]["required_capabilities"] =
                serde_json::json!(["refreshable", "testable"]);
        },
    ];

    for mutate in mutations {
        let mut tampered = encoded.clone();
        mutate(&mut tampered);
        assert!(
            serde_json::from_value::<ExecutionContractBundleV2>(tampered).is_err(),
            "every tenant, revision, site, slot, target, and contract field must be fingerprinted"
        );
    }
}

#[test]
fn tampering_with_selected_credential_is_detected() {
    let bundle = bundle([credential_entry(1, "auth", 8)]);
    let mut encoded = serde_json::to_value(bundle).unwrap();
    encoded["binding_manifest"][0]["credential_id"] =
        serde_json::to_value(CredentialId::from_bytes([99; 16])).unwrap();

    assert!(serde_json::from_value::<ExecutionContractBundleV2>(encoded).is_err());
}

#[test]
fn moving_a_binding_to_another_site_is_detected() {
    let bundle = bundle([credential_entry(1, "auth", 8)]);
    let mut encoded = serde_json::to_value(bundle).unwrap();
    encoded["binding_manifest"][0]["site"]["key"] = Value::String("node-2".to_owned());

    assert!(serde_json::from_value::<ExecutionContractBundleV2>(encoded).is_err());
}

#[test]
fn duplicate_site_slot_is_rejected_even_for_different_targets() {
    let first = credential_entry(1, "auth", 8);
    let second = credential_entry(1, "auth", 9);

    assert!(ExecutionBindingManifestV2::new([first, second]).is_err());
}

#[test]
fn v1_and_v2_recorded_envelopes_remain_independently_decodable() {
    let v1_fixture = serde_json::json!({
        "bundle_id": ExecutionContractBundleId::from_bytes([1; 16]),
        "org_id": OrgId::from_bytes([2; 16]),
        "workspace_id": WorkspaceId::from_bytes([3; 16]),
        "profile": "graph",
        "executable_plan_revision_id": ExecutablePlanRevisionId::from_bytes([4; 32]),
        "plugin_set_id": PluginSetId::from_bytes([5; 32]),
        "revisions": {
            "workflow": WorkflowVersionId::from_bytes([6; 16]),
            "worker_flavor": WorkerFlavorRevisionId::from_bytes([7; 32]),
        },
        "authorized_credential_ids": [],
        "schema_version": 1,
        "durable_envelope_version": 1,
        "fingerprint_version": 1,
        "fingerprint": "00".repeat(32),
    });
    let v2_fixture = serde_json::to_value(bundle([credential_entry(1, "auth", 8)])).unwrap();

    assert!(serde_json::from_value::<RecordedExecutionContractBundleV1>(v1_fixture).is_ok());
    assert!(serde_json::from_value::<RecordedExecutionContractBundleV2>(v2_fixture).is_ok());
}

#[test]
fn noncanonical_nested_contract_data_is_rejected_on_decode() {
    let bundle = bundle([credential_entry(1, "auth", 8)]);
    let mut encoded = serde_json::to_value(bundle).unwrap();
    encoded["binding_manifest"][0]["contract"]["required_capabilities"] =
        serde_json::json!(["revocable", "refreshable"]);

    assert!(serde_json::from_value::<RecordedExecutionContractBundleV2>(encoded).is_err());
}
