use super::*;
use nebula_core::{
    ExecutablePlanRevisionId, PluginSetId, WorkerFlavorRevisionId, WorkflowId, WorkflowVersionId,
};
use nebula_schema::{ModeField, ObjectField, Schema, SecretField, ValuePath, field_key};
use serde_json::{Value, json};
use std::assert_matches;

const SECRET_PAYLOAD: &str = "credential-value-that-must-not-leak";
const ACTUAL_CONTRACT_DETAIL: &str = "registered-contract-v2";

fn diagnostic(
    code: &str,
    path: &str,
    expected: &str,
    actual: &str,
    remediation: &str,
) -> ActivationDiagnostic {
    ActivationDiagnostic::new(code, path, expected, actual, remediation)
        .expect("the fixture uses non-empty diagnostic fields")
}

#[test]
fn activation_diagnostics_reject_empty_fields_sort_dedupe_and_redact() {
    assert!(ActivationDiagnostic::new("", "graph.node", "expected", "actual", "fix").is_none());
    assert!(ActivationDiagnostic::new("E002", "graph.node", "expected", "", "fix").is_none());

    let later = diagnostic(
        "E002",
        "graph.node[2]",
        "registered action",
        ACTUAL_CONTRACT_DETAIL,
        "install the action",
    );
    let earlier = diagnostic(
        "E001",
        "graph.node[1]",
        "compatible schema",
        ACTUAL_CONTRACT_DETAIL,
        "update the parameter",
    );
    let error = PlanCompilationError::new(vec![later, earlier.clone(), earlier.clone()])
        .expect("the fixture has diagnostics");

    assert_eq!(
        error.diagnostics(),
        &[
            earlier.clone(),
            diagnostic(
                "E002",
                "graph.node[2]",
                "registered action",
                ACTUAL_CONTRACT_DETAIL,
                "install the action",
            )
        ]
    );
    assert!(!format!("{error}").contains(ACTUAL_CONTRACT_DETAIL));
    assert!(!format!("{error:?}").contains(ACTUAL_CONTRACT_DETAIL));
    assert!(!format!("{earlier}").contains(ACTUAL_CONTRACT_DETAIL));
    assert!(!format!("{earlier:?}").contains(ACTUAL_CONTRACT_DETAIL));
    assert!(PlanCompilationError::new(Vec::new()).is_none());
}

/// Every integrity rejection reports all five activation-diagnostic fields.
///
/// The list is exhaustive by construction: `activation_diagnostics` matches
/// the enum without a wildcard, so a new variant fails to compile there
/// before it can reach a caller with a missing field.
#[test]
fn every_plan_integrity_rejection_reports_all_five_fields() {
    use nebula_error::ActivationDiagnostics;

    let rejections = [
        ExecutablePlanIntegrityError::UnsupportedFormat,
        ExecutablePlanIntegrityError::NonCanonical {
            section: "bindings",
        },
        ExecutablePlanIntegrityError::ConvertersUnsupported,
        ExecutablePlanIntegrityError::UnknownCredentialCapability,
        ExecutablePlanIntegrityError::CanonicalEncoding,
        ExecutablePlanIntegrityError::RevisionIdMismatch {
            claimed: ExecutablePlanRevisionId::from_bytes([0x11; 32]),
            computed: ExecutablePlanRevisionId::from_bytes([0x22; 32]),
        },
    ];

    for rejection in &rejections {
        let diagnostics = rejection.activation_diagnostics();
        assert!(
            !diagnostics.is_empty(),
            "a rejection with nothing to report is not actionable"
        );
        for reported in diagnostics {
            for field in [
                reported.code(),
                reported.path(),
                reported.expected(),
                reported.actual(),
                reported.remediation(),
            ] {
                assert!(
                    !field.trim().is_empty(),
                    "activation diagnostics require all five fields"
                );
            }
        }
    }
}

/// A compilation rejection hands its diagnostics through already canonical,
/// so two runs over the same workflow report the same sequence.
#[test]
fn compilation_diagnostics_come_through_canonically_ordered() {
    use nebula_error::ActivationDiagnostics;

    let later = diagnostic("E002", "/nodes/b", "a contract", "second", "fix it");
    let earlier = diagnostic("E001", "/nodes/a", "a contract", "first", "fix it");
    let error = PlanCompilationError::new(vec![later.clone(), earlier.clone(), earlier.clone()])
        .expect("the fixture has diagnostics");

    assert_eq!(error.activation_diagnostics(), vec![earlier, later]);
}

fn recorded_semver(major: u64, minor: u64, patch: u64) -> RecordedSemverV1 {
    RecordedSemverV1 {
        major,
        minor,
        patch,
        pre: String::new(),
        build: String::new(),
    }
}

fn recorded_schema(schema: ValidSchema) -> RecordedSchemaV1 {
    RecordedSchemaV1 {
        schema_wire_version: SCHEMA_WIRE_VERSION_GRAPH_V1,
        schema,
    }
}

fn empty_dependencies() -> RecordedDependenciesV1 {
    RecordedDependenciesV1 {
        credentials: Vec::new().into_boxed_slice(),
        resources: Vec::new().into_boxed_slice(),
        slots: Vec::new().into_boxed_slice(),
    }
}

fn minimal_action(dependencies: RecordedDependenciesV1) -> RecordedActionV1 {
    RecordedActionV1 {
        effect_contract: None,
        key: "demo.echo".into(),
        plugin_key: "demo".into(),
        version: recorded_semver(1, 0, 0),
        kind: RecordedActionKindV1::Stateless,
        isolation: RecordedIsolationV1::None,
        checkpoint_policy: RecordedCheckpointPolicyV1::Inherit,
        max_concurrent: None,
        inputs: vec![RecordedInputPortV1::Flow { key: "in".into() }].into_boxed_slice(),
        outputs: vec![RecordedOutputPortV1::Flow {
            key: "out".into(),
            flow_kind: RecordedFlowKindV1::Main,
        }]
        .into_boxed_slice(),
        input_schema: recorded_schema(ValidSchema::empty()),
        output_schema: recorded_schema(ValidSchema::empty()),
        dependencies,
    }
}

fn minimal_node(id: &str) -> RecordedNodeV1 {
    RecordedNodeV1 {
        id: id.into(),
        plugin_key: "demo".into(),
        action_key: "demo.echo".into(),
        action_version: recorded_semver(1, 0, 0),
        parameters: Vec::new().into_boxed_slice(),
        retry_policy: None,
        timeout: None,
        rate_limit: None,
        enabled: true,
    }
}

fn workflow_config() -> RecordedWorkflowConfigV1 {
    RecordedWorkflowConfigV1 {
        timeout: None,
        max_parallel_nodes: 1,
        checkpointing: RecordedCheckpointingV1 {
            enabled: true,
            interval: None,
        },
        retry_policy: None,
        error_strategy: RecordedErrorStrategyV1::FailFast,
    }
}

fn resource_binding(slot_key: &str, selector: &str) -> RecordedBindingV1 {
    RecordedBindingV1 {
        site: RecordedBindingSiteV1::Node("fetch".into()),
        slot_key: slot_key.into(),
        selector: selector.into(),
        contract: RecordedBindingContractV1::Resource {
            key: "demo.client".into(),
            version: recorded_semver(1, 0, 0),
        },
        required: true,
        lazy: false,
    }
}

fn credential_binding(slot_key: &str, selector: &str, capability_bits: u8) -> RecordedBindingV1 {
    RecordedBindingV1 {
        site: RecordedBindingSiteV1::Node("fetch".into()),
        slot_key: slot_key.into(),
        selector: selector.into(),
        contract: RecordedBindingContractV1::Credential {
            key: "demo.oauth".into(),
            version: recorded_semver(2, 1, 0),
            required_capability_bits: capability_bits,
        },
        required: true,
        lazy: true,
    }
}

fn reseal(record: &mut RecordedExecutablePlanRevisionV1) {
    record.claimed_id = record
        .recomputed_id()
        .expect("the fixture is canonical and hashable");
}

fn fixture_record() -> RecordedExecutablePlanRevisionV1 {
    let mut record = RecordedExecutablePlanRevisionV1 {
        record_version: RECORD_VERSION_V1,
        compiler_version: COMPILER_VERSION_GRAPH_V1,
        canonical_hash_version: CANONICAL_HASH_VERSION_V1,
        profile: RecordedPlanProfileV1::GraphV1,
        claimed_id: ExecutablePlanRevisionId::from_bytes([0; 32]),
        workflow_version_id: WorkflowVersionId::from_bytes([1; 16]),
        plugin_set_id: PluginSetId::from_bytes([2; 32]),
        worker_flavor_revision_id: WorkerFlavorRevisionId::from_bytes([3; 32]),
        manifest: RecordedPlanManifestV1 {
            workflow_definition_schema_version: nebula_workflow::CURRENT_SCHEMA_VERSION,
            workflow_id: WorkflowId::from_bytes([4; 16]),
            workflow_semantic_version: RecordedWorkflowVersionV1 {
                major: 1,
                minor: 2,
                patch: 3,
                pre: None,
                build: None,
            },
        },
        content: RecordedGraphContentV1 {
            plugins: vec![RecordedPluginV1 {
                key: "demo".into(),
                version: recorded_semver(1, 0, 0),
            }]
            .into_boxed_slice(),
            nodes: vec![minimal_node("fetch")].into_boxed_slice(),
            connections: Vec::new().into_boxed_slice(),
            actions: vec![minimal_action(empty_dependencies())].into_boxed_slice(),
            resources: Vec::new().into_boxed_slice(),
            credentials: Vec::new().into_boxed_slice(),
            triggers: Vec::new().into_boxed_slice(),
            variables: Vec::new().into_boxed_slice(),
            workflow_config: workflow_config(),
            converters: Vec::new().into_boxed_slice(),
        },
        bindings: Vec::new().into_boxed_slice(),
    };
    reseal(&mut record);
    record
}

fn resource_binding_record(selector: &str) -> RecordedExecutablePlanRevisionV1 {
    let mut record = fixture_record();
    record.content.resources = vec![RecordedResourceV1 {
        key: "demo.client".into(),
        plugin_key: "demo".into(),
        version: recorded_semver(1, 0, 0),
        configuration_schema: recorded_schema(ValidSchema::empty()),
        dependencies: empty_dependencies(),
    }]
    .into_boxed_slice();
    record.content.actions[0].dependencies.slots = vec![RecordedSlotV1::Resource {
        slot_key: "client".into(),
        default_selector: "primary".into(),
        contract_key: "demo.client".into(),
        required: true,
        lazy: false,
    }]
    .into_boxed_slice();
    record.bindings = vec![resource_binding("client", selector)].into_boxed_slice();
    reseal(&mut record);
    record
}

fn credential_binding_record(
    selector: &str,
    required_capability_bits: u8,
) -> RecordedExecutablePlanRevisionV1 {
    let mut record = fixture_record();
    record.content.credentials = vec![RecordedCredentialV1 {
        key: "demo.oauth".into(),
        plugin_key: "demo".into(),
        version: recorded_semver(2, 1, 0),
        pattern: RecordedAuthPatternV1::OAuth2,
        properties_schema: recorded_schema(ValidSchema::empty()),
        capability_bits: Capabilities::REFRESHABLE.bits(),
    }]
    .into_boxed_slice();
    record.content.actions[0].dependencies.slots = vec![RecordedSlotV1::Credential {
        slot_key: "auth".into(),
        default_selector: "primary".into(),
        contract_key: "demo.oauth".into(),
        required: true,
        lazy: true,
    }]
    .into_boxed_slice();
    record.bindings = vec![credential_binding(
        "auth",
        selector,
        required_capability_bits,
    )]
    .into_boxed_slice();
    reseal(&mut record);
    record
}

fn object_with_secret_default(default: Value) -> ValidSchema {
    Schema::builder()
        .add(
            ObjectField::new(field_key!("auth"))
                .add(SecretField::new(field_key!("token")))
                .default(default),
        )
        .build()
        .expect("the object default is accepted by the general schema contract")
}

fn mode_with_secret_default(default: Value) -> ValidSchema {
    Schema::builder()
        .add(
            ModeField::new(field_key!("auth"))
                .variant("token", "Token", SecretField::new(field_key!("token")))
                .default(default),
        )
        .build()
        .expect("the mode default is accepted by the general schema contract")
}

#[test]
fn checked_record_rejects_forged_id_and_unknown_fields() {
    let record = fixture_record();
    let mut forged = record.clone();
    forged.claimed_id = ExecutablePlanRevisionId::from_bytes([9; 32]);

    assert!(matches!(
        ExecutablePlanRevision::try_from_recorded_v1(forged),
        Err(ExecutablePlanIntegrityError::RevisionIdMismatch { .. })
    ));

    let mut encoded = serde_json::to_value(record).expect("the record serializes");
    encoded
        .as_object_mut()
        .expect("a record is a JSON object")
        .insert("future_field".into(), json!(true));
    let error = serde_json::from_value::<RecordedExecutablePlanRevisionV1>(encoded)
        .expect_err("unknown top-level record fields must fail closed");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn minimal_typed_record_is_integrity_valid() {
    let record = fixture_record();
    let plan = ExecutablePlanRevision::try_from(record)
        .expect("the fixture is a fully closed Graph-v1 record");
    assert!(plan.bindings().is_empty());
}

#[test]
fn effect_lookup_distinguishes_legacy_declarations_from_unknown_actions() {
    let plan = ExecutablePlanRevision::try_from(fixture_record())
        .expect("the fixture is a fully closed Graph-v1 record");

    assert_eq!(
        plan.action_effect_contract(&ActionKey::new("demo.echo").unwrap())
            .unwrap(),
        PlanActionEffectContract::LegacyUndeclared
    );
    assert_eq!(
        plan.action_effect_contract(&ActionKey::new("demo.missing").unwrap())
            .unwrap(),
        PlanActionEffectContract::UnknownAction
    );
}

#[test]
fn graph_v1_hash_matches_literal_golden_and_independent_record_projection() {
    let record = fixture_record();
    assert_eq!(record.compiler_version, COMPILER_VERSION_GRAPH_V1);
    assert_eq!(
        ExecutablePlanRevision::try_from(record.clone())
            .unwrap()
            .id(),
        record.claimed_id
    );
    assert_eq!(
        record.claimed_id.to_string(),
        "f1e5fa3021749835b3bea5848df1d517d405e5a26b95d5ec597f872fd1ae8f79"
    );

    let mut projected = serde_json::to_value(&record).expect("record serializes");
    projected
        .as_object_mut()
        .expect("record is an object")
        .remove("claimed_id");
    let canonical = canonical_json_v1(&projected).expect("record projection is canonical");
    let mut independent = Sha256::new();
    independent.update([1]);
    independent.update((EXECUTABLE_PLAN_GRAPH_V1_DOMAIN.len() as u64).to_be_bytes());
    independent.update(EXECUTABLE_PLAN_GRAPH_V1_DOMAIN);
    independent.update([2]);
    independent.update((canonical.len() as u64).to_be_bytes());
    independent.update(canonical);
    let digest: [u8; 32] = independent.finalize().into();
    assert_eq!(
        record.claimed_id,
        ExecutablePlanRevisionId::from_bytes(digest)
    );
}

#[test]
fn compiler_effect_tuples_are_closed_and_legacy_fields_stay_absent() {
    for compiler in [
        COMPILER_VERSION_GRAPH_V1,
        COMPILER_VERSION_GRAPH_V3,
        COMPILER_VERSION_GRAPH_V4,
    ] {
        for hash in [CANONICAL_HASH_VERSION_V1, CANONICAL_HASH_VERSION_V2] {
            for declared in [false, true] {
                let mut record = fixture_record();
                record.compiler_version = compiler;
                record.canonical_hash_version = hash;
                record.content.actions[0].effect_contract = declared
                    .then_some(crate::plan_effect::RecordedActionEffectV1::NoExternalEffects);
                reseal(&mut record);
                let expected = if compiler == COMPILER_VERSION_GRAPH_V1 {
                    hash == CANONICAL_HASH_VERSION_V1 && !declared
                } else {
                    hash == CANONICAL_HASH_VERSION_V2 && declared
                };
                assert_eq!(
                    ExecutablePlanRevision::try_from(record.clone()).is_ok(),
                    expected,
                    "compiler={compiler}, hash={hash}, declared={declared}"
                );
                if !declared {
                    assert!(
                        !serde_json::to_string(&record)
                            .unwrap()
                            .contains("effect_contract")
                    );
                }
            }
        }
    }
}

#[test]
fn scalar_aware_compiler_epoch_preserves_legacy_schema_bytes() {
    let mut record = fixture_record();
    record.compiler_version = COMPILER_VERSION_GRAPH_V4;
    record.canonical_hash_version = CANONICAL_HASH_VERSION_V2;
    record.content.actions[0].effect_contract =
        Some(crate::plan_effect::RecordedActionEffectV1::NoExternalEffects);
    reseal(&mut record);
    let encoded = serde_json::to_vec(&record).unwrap();
    let checked = ExecutablePlanRevision::try_from(record.clone()).unwrap();
    assert_eq!(checked.id(), record.claimed_id);
    assert_eq!(
        serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&checked)).unwrap(),
        encoded
    );
    assert_eq!(
        serde_json::to_value(&record.content.actions[0].input_schema).unwrap(),
        json!({"schema_wire_version": 1, "schema": {"fields": []}})
    );

    let current_id = record.claimed_id;
    record.compiler_version = COMPILER_VERSION_GRAPH_V3;
    reseal(&mut record);
    assert_ne!(current_id, record.claimed_id);
    ExecutablePlanRevision::try_from(record).unwrap();
}

#[test]
fn legacy_empty_record_never_decodes_as_null() {
    for (compiler, hash, effect) in [
        (COMPILER_VERSION_GRAPH_V1, CANONICAL_HASH_VERSION_V1, None),
        (
            COMPILER_VERSION_GRAPH_V3,
            CANONICAL_HASH_VERSION_V2,
            Some(crate::plan_effect::RecordedActionEffectV1::NoExternalEffects),
        ),
    ] {
        let mut record = fixture_record();
        record.compiler_version = compiler;
        record.canonical_hash_version = hash;
        record.content.actions[0].effect_contract = effect;
        reseal(&mut record);
        let encoded = serde_json::to_vec(&record).unwrap();
        let decoded: RecordedExecutablePlanRevisionV1 = serde_json::from_slice(&encoded).unwrap();
        let schema = &decoded.content.actions[0].input_schema.schema;
        assert_eq!(schema.kind(), SchemaKind::Record);
        let resolved = schema
            .validate(AuthoredValue::from_data(json!({})).unwrap())
            .unwrap()
            .resolve_data()
            .unwrap();
        assert_eq!(resolved.into_typed::<Value>().unwrap(), json!({}));
        assert!(
            schema
                .validate(AuthoredValue::from_data(Value::Null).unwrap())
                .is_err()
        );
        let checked = ExecutablePlanRevision::try_from(decoded).unwrap();
        assert_eq!(
            serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&checked)).unwrap(),
            encoded
        );
    }
}

#[test]
fn scalar_schema_envelopes_require_the_scalar_compiler_epoch_at_every_contract_site() {
    let scalar = serde_json::from_value::<ValidSchema>(json!({
        "kind": "scalar", "scalar": {"version": 1, "type": "null"}
    }))
    .unwrap();
    for site in ["input", "output", "resource", "credential"] {
        for compiler in [1, 3, 4] {
            for wire_version in [1, 2, 3] {
                let mut record = match site {
                    "resource" => resource_binding_record("primary"),
                    "credential" => {
                        credential_binding_record("primary", Capabilities::REFRESHABLE.bits())
                    },
                    _ => fixture_record(),
                };
                record.compiler_version = compiler;
                record.canonical_hash_version = if compiler == 1 { 1 } else { 2 };
                record.content.actions[0].effect_contract = (compiler != 1)
                    .then_some(crate::plan_effect::RecordedActionEffectV1::NoExternalEffects);
                let contract = match site {
                    "input" => &mut record.content.actions[0].input_schema,
                    "output" => &mut record.content.actions[0].output_schema,
                    "resource" => &mut record.content.resources[0].configuration_schema,
                    "credential" => &mut record.content.credentials[0].properties_schema,
                    _ => unreachable!("the fixture enumerates all contract sites"),
                };
                contract.schema = scalar.clone();
                contract.schema_wire_version = wire_version;
                reseal(&mut record);
                let encoded = serde_json::to_vec(&record).unwrap();
                let decoded = serde_json::from_slice(&encoded).unwrap();
                let checked = ExecutablePlanRevision::try_from_recorded_v1(decoded);
                assert_eq!(
                    checked.is_ok(),
                    compiler == 4 && wire_version == 2,
                    "site={site}, compiler={compiler}, schema_wire={wire_version}: {checked:?}"
                );
                if let Ok(plan) = checked {
                    assert_eq!(
                        serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&plan)).unwrap(),
                        encoded
                    );
                }
            }
        }
    }
}

#[test]
fn scalar_schema_descriptor_versions_and_mixed_shapes_fail_closed() {
    for invalid in [
        json!({"kind": "scalar", "scalar": {"version": 2, "type": "null"}}),
        json!({"kind": "scalar", "scalar": {"type": "null"}}),
        json!({"kind": "scalar", "scalar": {"version": 1, "type": "future"}}),
        json!({"kind": "scalar", "scalar": {"version": 1, "type": "null"}, "fields": []}),
        json!({"kind": "scalar", "scalar": {"version": 1, "type": "null"}, "root_rules": []}),
        json!({"fields": [], "scalar": {"version": 1, "type": "null"}}),
        json!({"kind": "scalar", "scalar": {"version": 1, "type": "string", "unexpected": SECRET_PAYLOAD}}),
    ] {
        let error = serde_json::from_value::<RecordedSchemaV1>(json!({
            "schema_wire_version": 2, "schema": invalid,
        }))
        .err()
        .expect("a mixed or unsupported scalar wire must fail closed");
        assert!(!error.to_string().contains(SECRET_PAYLOAD));
        assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));
    }
}

#[test]
fn scalar_compiler_does_not_relabel_legacy_record_any_or_union_schema_wire() {
    let union = ValidSchema::union(
        Field::mode(field_key!("choice")).variant(
            "text",
            "Text",
            Field::string(field_key!("text")),
        ),
        nebula_schema::SerdeTagging::External,
    )
    .unwrap();
    for schema in [ValidSchema::empty(), ValidSchema::any(), union] {
        for epoch in [PlanEpoch::GraphV1, PlanEpoch::GraphV3, PlanEpoch::GraphV4] {
            let mut record = fixture_record();
            record.compiler_version = epoch.compiler_version();
            record.canonical_hash_version = epoch.canonical_hash_version();
            record.content.actions[0].effect_contract = epoch
                .records_effect_contract()
                .then_some(crate::plan_effect::RecordedActionEffectV1::NoExternalEffects);
            record.content.actions[0].output_schema = RecordedSchemaV1::new(schema.clone());
            assert_eq!(
                record.content.actions[0].output_schema.schema_wire_version,
                1
            );
            reseal(&mut record);
            let encoded = serde_json::to_vec(&record).unwrap();
            let loaded = ExecutablePlanRevision::try_from_recorded_v1(
                serde_json::from_slice(&encoded).unwrap(),
            )
            .unwrap();
            assert_eq!(
                serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&loaded)).unwrap(),
                encoded
            );
            record.content.actions[0].output_schema.schema_wire_version =
                SCHEMA_WIRE_VERSION_SCALAR_V2;
            reseal(&mut record);
            std::assert_matches!(
                ExecutablePlanRevision::try_from_recorded_v1(record),
                Err(ExecutablePlanIntegrityError::NonCanonical {
                    section: "actions.output_schema"
                })
            );
        }
    }
}

#[test]
fn trigger_root_configuration_preserves_legacy_normalization_without_scalar_coercion() {
    let mut trigger = minimal_action(empty_dependencies());
    trigger.kind = RecordedActionKindV1::Trigger;
    for epoch in [PlanEpoch::GraphV1, PlanEpoch::GraphV3] {
        validate_trigger_configuration(&Value::Null, &trigger, epoch).unwrap();
        validate_trigger_configuration(&json!({}), &trigger, epoch).unwrap();
    }
    assert!(validate_trigger_configuration(&Value::Null, &trigger, PlanEpoch::GraphV4).is_err());
    validate_trigger_configuration(&json!({}), &trigger, PlanEpoch::GraphV4).unwrap();

    trigger.input_schema = RecordedSchemaV1::new(nebula_schema::schema_of::<()>().unwrap());
    validate_trigger_configuration(&Value::Null, &trigger, PlanEpoch::GraphV4).unwrap();
    assert!(validate_trigger_configuration(&json!({}), &trigger, PlanEpoch::GraphV4).is_err());
    trigger.input_schema = RecordedSchemaV1::new(nebula_schema::schema_of::<u8>().unwrap());
    validate_trigger_configuration(&json!(42), &trigger, PlanEpoch::GraphV4).unwrap();
    for invalid in [json!(256), json!(-1), json!("42"), json!({}), Value::Null] {
        assert!(validate_trigger_configuration(&invalid, &trigger, PlanEpoch::GraphV4).is_err());
    }
}

#[test]
fn effect_plan_hash_uses_new_domain_and_complete_record_projection() {
    let mut record = fixture_record();
    record.compiler_version = COMPILER_VERSION_GRAPH_V3;
    record.canonical_hash_version = CANONICAL_HASH_VERSION_V2;
    record.content.actions[0].effect_contract =
        Some(crate::plan_effect::RecordedActionEffectV1::NoExternalEffects);
    reseal(&mut record);
    let mut projected = serde_json::to_value(&record).unwrap();
    projected.as_object_mut().unwrap().remove("claimed_id");
    let canonical = canonical_json_v1(&projected).unwrap();
    let mut independent = Sha256::new();
    independent.update([1]);
    independent.update((EXECUTABLE_PLAN_GRAPH_V2_DOMAIN.len() as u64).to_be_bytes());
    independent.update(EXECUTABLE_PLAN_GRAPH_V2_DOMAIN);
    independent.update([2]);
    independent.update((canonical.len() as u64).to_be_bytes());
    independent.update(canonical);
    let digest: [u8; 32] = independent.finalize().into();
    assert_eq!(
        record.claimed_id,
        ExecutablePlanRevisionId::from_bytes(digest)
    );
    ExecutablePlanRevision::try_from(record).unwrap();
}

#[test]
fn intrinsic_error_edges_require_the_effect_aware_compiler() {
    let mut record = fixture_record();
    record.content.nodes = vec![minimal_node("source"), minimal_node("target")].into_boxed_slice();
    record.content.connections = vec![RecordedConnectionV1 {
        from_node: "source".into(),
        from_port: "error".into(),
        to_node: "target".into(),
        to_port: None,
    }]
    .into_boxed_slice();
    reseal(&mut record);
    assert!(matches!(
        ExecutablePlanRevision::try_from(record.clone()),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "connections.from_port"
        })
    ));
    record.compiler_version = COMPILER_VERSION_GRAPH_V3;
    record.canonical_hash_version = CANONICAL_HASH_VERSION_V2;
    record.content.actions[0].effect_contract =
        Some(crate::plan_effect::RecordedActionEffectV1::NoExternalEffects);
    reseal(&mut record);
    let plan = ExecutablePlanRevision::try_from(record).expect("intrinsic error edge is certified");
    assert_eq!(
        plan.execution_graph().unwrap().connections()[0]
            .from_port
            .as_ref()
            .unwrap()
            .as_str(),
        "error"
    );
}

#[test]
fn resealed_error_references_cannot_read_success_only_fields() {
    let mut record = fixture_record();
    record.compiler_version = COMPILER_VERSION_GRAPH_V3;
    record.canonical_hash_version = CANONICAL_HASH_VERSION_V2;
    record.content.actions[0].effect_contract =
        Some(crate::plan_effect::RecordedActionEffectV1::NoExternalEffects);
    record.content.actions[0].input_schema.schema =
        nebula_schema::schema_of::<nebula_workflow::ErrorPortPayload>()
            .expect("valid test catalog definition");
    record.content.actions[0].output_schema.schema = Schema::builder()
        .add(Field::string(nebula_schema::field_key!("success_only")).required())
        .build()
        .unwrap();
    let nodes = ["source", "target"].map(|id| {
        let mut node = minimal_node(id);
        node.parameters = vec![
            RecordedParameterV1 {
                key: "error".into(),
                value: if id == "target" {
                    RecordedParameterValueV1::Reference {
                        node_key: "source".into(),
                        output_path: ValuePath::from_pointer("/error").unwrap(),
                    }
                } else {
                    RecordedParameterValueV1::Literal {
                        value: serde_json::json!("fixture"),
                    }
                },
            },
            RecordedParameterV1 {
                key: "node_id".into(),
                value: RecordedParameterValueV1::Literal {
                    value: serde_json::json!("source"),
                },
            },
        ]
        .into_boxed_slice();
        node
    });
    record.content.nodes = nodes.into();
    record.content.connections = vec![RecordedConnectionV1 {
        from_node: "source".into(),
        from_port: "error".into(),
        to_node: "target".into(),
        to_port: None,
    }]
    .into_boxed_slice();
    reseal(&mut record);
    ExecutablePlanRevision::try_from(record.clone()).expect("intrinsic error field is valid");
    record.content.nodes[1].parameters[0].value = RecordedParameterValueV1::Reference {
        node_key: "source".into(),
        output_path: ValuePath::from_pointer("/success_only").unwrap(),
    };
    reseal(&mut record);
    assert!(matches!(
        ExecutablePlanRevision::try_from(record),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.reference.path"
        })
    ));
}

#[test]
fn json_object_order_is_invariant_but_array_order_and_included_fields_are_not() {
    let mut object_a_then_b = fixture_record();
    object_a_then_b.content.variables = vec![RecordedVariableV1 {
        name: "value".into(),
        value: serde_json::from_str(r#"{"a":1,"b":2}"#).expect("fixture JSON is valid"),
    }]
    .into_boxed_slice();
    reseal(&mut object_a_then_b);

    let mut object_b_then_a = fixture_record();
    object_b_then_a.content.variables = vec![RecordedVariableV1 {
        name: "value".into(),
        value: serde_json::from_str(r#"{"b":2,"a":1}"#).expect("fixture JSON is valid"),
    }]
    .into_boxed_slice();
    reseal(&mut object_b_then_a);
    assert_eq!(object_a_then_b.claimed_id, object_b_then_a.claimed_id);

    let mut array_a_then_b = fixture_record();
    array_a_then_b.content.variables = vec![RecordedVariableV1 {
        name: "value".into(),
        value: json!(["a", "b"]),
    }]
    .into_boxed_slice();
    reseal(&mut array_a_then_b);
    let mut array_b_then_a = fixture_record();
    array_b_then_a.content.variables = vec![RecordedVariableV1 {
        name: "value".into(),
        value: json!(["b", "a"]),
    }]
    .into_boxed_slice();
    reseal(&mut array_b_then_a);
    assert_ne!(array_a_then_b.claimed_id, array_b_then_a.claimed_id);

    let mut changed_schema_version = fixture_record();
    changed_schema_version
        .manifest
        .workflow_definition_schema_version += 1;
    reseal(&mut changed_schema_version);
    assert_ne!(
        fixture_record().claimed_id,
        changed_schema_version.claimed_id
    );
}

#[test]
fn unsupported_versions_and_profile_fail_closed() {
    for mutate in [
        |record: &mut RecordedExecutablePlanRevisionV1| record.record_version += 1,
        |record: &mut RecordedExecutablePlanRevisionV1| {
            record.compiler_version = COMPILER_VERSION_GRAPH_V4 + 1;
        },
        |record: &mut RecordedExecutablePlanRevisionV1| record.canonical_hash_version += 1,
    ] {
        let mut record = fixture_record();
        mutate(&mut record);
        assert!(matches!(
            ExecutablePlanRevision::try_from(record),
            Err(ExecutablePlanIntegrityError::UnsupportedFormat)
        ));
    }

    let mut encoded = serde_json::to_value(fixture_record()).expect("record serializes");
    encoded
        .as_object_mut()
        .expect("record is an object")
        .insert("profile".into(), json!("future-profile"));
    assert!(
        serde_json::from_value::<RecordedExecutablePlanRevisionV1>(encoded).is_err(),
        "an unknown execution profile must fail during record decoding"
    );
}

#[test]
fn collection_and_converter_canonicality_fail_closed() {
    let mut unsorted = fixture_record();
    unsorted.content.nodes = vec![minimal_node("b"), minimal_node("a")].into_boxed_slice();
    assert!(matches!(
        ExecutablePlanRevision::try_from(unsorted),
        Err(ExecutablePlanIntegrityError::NonCanonical { section: "nodes" })
    ));

    let mut duplicate = fixture_record();
    duplicate.content.nodes = vec![minimal_node("a"), minimal_node("a")].into_boxed_slice();
    assert!(matches!(
        ExecutablePlanRevision::try_from(duplicate),
        Err(ExecutablePlanIntegrityError::NonCanonical { section: "nodes" })
    ));

    let mut converter = fixture_record();
    converter.content.converters = vec![RecordedConverterV1 {
        key: "implicit".into(),
    }]
    .into_boxed_slice();
    assert!(matches!(
        ExecutablePlanRevision::try_from(converter),
        Err(ExecutablePlanIntegrityError::ConvertersUnsupported)
    ));
}

#[test]
fn explicit_literals_are_not_reclassified_as_expressions() {
    let mut literal = fixture_record();
    literal.content.actions[0].input_schema = recorded_schema(
        Schema::builder()
            .add(Field::string(field_key!("value")).no_expression())
            .build()
            .expect("fixture schema is valid"),
    );
    literal.content.nodes[0].parameters = vec![RecordedParameterV1 {
        key: "value".into(),
        value: RecordedParameterValueV1::Literal {
            value: json!("{{ $workflow.input }}"),
        },
    }]
    .into_boxed_slice();
    reseal(&mut literal);
    ExecutablePlanRevision::try_from(literal.clone())
        .expect("the tagged literal must remain a literal");

    literal.content.nodes[0].parameters[0].value = RecordedParameterValueV1::Expression {
        expression: "{{ $workflow.input }}".into(),
    };
    reseal(&mut literal);
    assert!(matches!(
        ExecutablePlanRevision::try_from(literal),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.schema"
        })
    ));
}

#[test]
fn parameter_validation_rejects_invalid_nested_values_and_secret_literals() {
    let mut nested = fixture_record();
    nested.content.actions[0].input_schema = recorded_schema(
        Schema::builder()
            .add(
                ObjectField::new(field_key!("config"))
                    .add(Field::string(field_key!("name")).required()),
            )
            .build()
            .expect("fixture schema is valid"),
    );
    nested.content.nodes[0].parameters = vec![RecordedParameterV1 {
        key: "config".into(),
        value: RecordedParameterValueV1::Literal { value: json!({}) },
    }]
    .into_boxed_slice();
    reseal(&mut nested);
    assert!(matches!(
        ExecutablePlanRevision::try_from(nested),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.schema"
        })
    ));

    let mut secret = fixture_record();
    secret.content.actions[0].input_schema = recorded_schema(
        Schema::builder()
            .add(ObjectField::new(field_key!("auth")).add(SecretField::new(field_key!("token"))))
            .build()
            .expect("fixture schema is valid"),
    );
    secret.content.nodes[0].parameters = vec![RecordedParameterV1 {
        key: "auth".into(),
        value: RecordedParameterValueV1::Literal {
            value: json!({"token": SECRET_PAYLOAD}),
        },
    }]
    .into_boxed_slice();
    reseal(&mut secret);
    let error = ExecutablePlanRevision::try_from(secret)
        .expect_err("an executable plan cannot persist credential material");
    assert!(matches!(
        error,
        ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.secret"
        }
    ));
    assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));
}

#[rstest::rstest]
#[case::secret_alias(json!({"nested": {"legacy_token": SECRET_PAYLOAD}}))]
#[case::container_alias(json!({"legacy_nested": {"token": SECRET_PAYLOAD}}))]
#[case::shadowed_alias(json!({"nested": {}, "legacy_nested": {"legacy_token": SECRET_PAYLOAD}}))]
fn recorded_literals_reject_secret_read_aliases(#[case] value: Value) {
    let mut record = fixture_record();
    record.content.actions[0].input_schema = recorded_schema(
        Schema::builder()
            .add(
                ObjectField::new(field_key!("auth")).add(
                    ObjectField::new(field_key!("nested"))
                        .read_alias(field_key!("legacy_nested"))
                        .unwrap()
                        .add(
                            SecretField::new(field_key!("token"))
                                .read_alias(field_key!("legacy_token"))
                                .unwrap(),
                        ),
                ),
            )
            .build()
            .unwrap(),
    );
    record.content.nodes[0].parameters = vec![RecordedParameterV1 {
        key: "auth".into(),
        value: RecordedParameterValueV1::Literal { value },
    }]
    .into_boxed_slice();
    reseal(&mut record);
    let error = ExecutablePlanRevision::try_from(record)
        .expect_err("recorded aliases cannot retain secrets");
    assert_matches!(
        error,
        ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.secret"
        }
    );
    assert!(!format!("{error:?}: {error}").contains(SECRET_PAYLOAD));
}

#[test]
fn trigger_configuration_rejects_a_secret_read_alias() {
    let mut action = minimal_action(empty_dependencies());
    action.input_schema = recorded_schema(
        Schema::builder()
            .add(
                SecretField::new(field_key!("token"))
                    .read_alias(field_key!("legacy_token"))
                    .unwrap(),
            )
            .build()
            .unwrap(),
    );
    let error = validate_trigger_configuration(
        &json!({"legacy_token": SECRET_PAYLOAD}),
        &action,
        PlanEpoch::CURRENT,
    )
    .expect_err("trigger aliases cannot retain secrets");
    assert_matches!(
        error,
        ExecutablePlanIntegrityError::NonCanonical {
            section: "triggers.configuration.secret"
        }
    );
    assert!(!format!("{error:?}: {error}").contains(SECRET_PAYLOAD));
}

#[test]
fn typed_literal_keeps_arbitrary_keys_and_program_shaped_data() {
    let raw = json!({"a/b": [{"": {"$expr": "{{ 1 }}"}}], "0": "{{ 2 }}"});
    let typed = typed_literal(raw.clone()).unwrap();
    let path = nebula_schema::ValuePath::from_pointer("/a~1b/0//$expr").unwrap();
    assert_eq!(typed.get_path(&path).unwrap().as_str(), Some("{{ 1 }}"));
    assert_eq!(typed.get("0").unwrap().as_str(), Some("{{ 2 }}"));
    assert_eq!(typed.to_json(), raw);
}

#[test]
fn node_parameter_set_cannot_bypass_required_fields_or_root_rules() {
    let mut missing = fixture_record();
    missing.content.actions[0].input_schema = recorded_schema(
        Schema::builder()
            .add(Field::string(field_key!("name")).required())
            .build()
            .expect("fixture schema is valid"),
    );
    reseal(&mut missing);
    assert!(matches!(
        ExecutablePlanRevision::try_from(missing),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.required"
        })
    ));

    let mut root_rules = fixture_record();
    root_rules.content.actions[0].input_schema = recorded_schema(
        Schema::builder()
            .add(Field::string(field_key!("name")))
            .root_rule(
                nebula_schema::Rule::predicate(
                    nebula_schema::Predicate::eq("name", json!("expected"))
                        .expect("fixture predicate is valid"),
                )
                .unwrap(),
            )
            .build()
            .expect("fixture schema is valid"),
    );
    reseal(&mut root_rules);
    assert!(matches!(
        ExecutablePlanRevision::try_from(root_rules),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.root_rules"
        })
    ));
}

#[test]
fn dynamic_source_paths_have_one_canonical_spelling() {
    for path in ["", "value", "items.0.name", "184467440737095516160"] {
        assert!(is_canonical_reference_path(path), "{path:?} is canonical");
    }
    for alias in [
        "$",
        "$.value",
        ".value",
        "value.",
        "value..name",
        "items.00",
    ] {
        assert!(
            !is_canonical_reference_path(alias),
            "{alias:?} must be normalized before persistence"
        );
    }
}

fn root_rule_record(
    epoch: PlanEpoch,
    rule: nebula_schema::Rule,
    value: Option<RecordedParameterValueV1>,
) -> RecordedExecutablePlanRevisionV1 {
    let mut record = fixture_record();
    record.compiler_version = epoch.compiler_version();
    record.canonical_hash_version = epoch.canonical_hash_version();
    record.content.actions[0].effect_contract = epoch
        .records_effect_contract()
        .then_some(crate::plan_effect::RecordedActionEffectV1::NoExternalEffects);
    record.content.actions[0].input_schema = recorded_schema(
        Schema::builder()
            .add(
                Field::list(field_key!("data"))
                    .item(Field::dynamic(field_key!("item")))
                    .expression_mode(nebula_schema::ExpressionMode::Allowed),
            )
            .root_rule(rule)
            .build()
            .unwrap(),
    );
    record.content.nodes[0].parameters = value
        .map(|value| RecordedParameterV1 {
            key: "data".into(),
            value,
        })
        .into_iter()
        .collect();
    reseal(&mut record);
    record
}

#[test]
fn static_root_rules_are_proved_in_current_plans_without_changing_legacy_epochs() {
    let path = nebula_schema::ValuePath::root().push("data");
    let rule = nebula_schema::Rule::any([
        nebula_schema::Rule::predicate(nebula_schema::Predicate::Set(path.clone())).unwrap(),
        nebula_schema::Rule::predicate(nebula_schema::Predicate::Eq(path, json!([]))).unwrap(),
    ])
    .unwrap();
    for epoch in [PlanEpoch::GraphV1, PlanEpoch::GraphV3, PlanEpoch::GraphV4] {
        for value in [json!([]), json!([1])] {
            let record = root_rule_record(
                epoch,
                rule.clone(),
                Some(RecordedParameterValueV1::Literal { value }),
            );
            let encoded = serde_json::to_vec(&record).unwrap();
            let checked = ExecutablePlanRevision::try_from_recorded_v1(
                serde_json::from_slice(&encoded).unwrap(),
            );
            if epoch == PlanEpoch::GraphV4 {
                let checked = checked.expect("current plans prove pure root presence predicates");
                assert_eq!(checked.id(), record.claimed_id);
                assert_eq!(
                    serde_json::to_vec(&RecordedExecutablePlanRevisionV1::from(&checked)).unwrap(),
                    encoded
                );
            } else {
                assert_matches!(
                    checked,
                    Err(ExecutablePlanIntegrityError::NonCanonical {
                        section: "nodes.parameters.root_rules"
                    })
                );
            }
        }
    }
}

#[test]
fn current_root_rules_are_rechecked_after_resealed_parameter_changes() {
    let rule = nebula_schema::Rule::predicate(nebula_schema::Predicate::Eq(
        nebula_schema::ValuePath::root().push("data"),
        json!([]),
    ))
    .unwrap();
    let record = root_rule_record(
        PlanEpoch::GraphV4,
        rule,
        Some(RecordedParameterValueV1::Literal { value: json!([]) }),
    );
    let checked = ExecutablePlanRevision::try_from(record.clone()).unwrap();
    assert_eq!(checked.id(), record.claimed_id);
    for value in [None, Some(json!([1])), Some(Value::Null)] {
        let mut tampered = record.clone();
        tampered.content.nodes[0].parameters = value
            .map(|value| RecordedParameterV1 {
                key: "data".into(),
                value: RecordedParameterValueV1::Literal { value },
            })
            .into_iter()
            .collect();
        reseal(&mut tampered);
        assert_matches!(
            ExecutablePlanRevision::try_from(tampered),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "nodes.parameters.schema"
            })
        );
    }
}

#[test]
fn current_root_rules_cannot_discard_unresolved_parameter_obligations() {
    let presence = nebula_schema::Rule::predicate(nebula_schema::Predicate::Set(
        nebula_schema::ValuePath::root().push("data"),
    ))
    .unwrap();
    for value in [
        RecordedParameterValueV1::Expression {
            expression: "[]".into(),
        },
        RecordedParameterValueV1::Template {
            template: "{{ [] }}".into(),
        },
        RecordedParameterValueV1::Reference {
            node_key: "fetch".into(),
            output_path: ValuePath::root(),
        },
    ] {
        let record = root_rule_record(PlanEpoch::GraphV4, presence.clone(), Some(value));
        assert_matches!(
            ExecutablePlanRevision::try_from(record),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "nodes.parameters.root_rules"
            })
        );
    }
    let custom = root_rule_record(
        PlanEpoch::GraphV4,
        nebula_schema::Rule::custom("runtime_only").unwrap(),
        Some(RecordedParameterValueV1::Literal { value: json!([]) }),
    );
    assert_matches!(
        ExecutablePlanRevision::try_from(custom.clone()),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.root_rules"
        })
    );
    let mut scalar = custom;
    scalar.content.actions[0].input_schema = RecordedSchemaV1::new(
        ValidSchema::scalar(
            nebula_schema::ScalarSchema::string()
                .root_rule(nebula_schema::Rule::custom("runtime_only").unwrap()),
        )
        .unwrap(),
    );
    scalar.content.nodes[0].parameters = Box::default();
    reseal(&mut scalar);
    assert_matches!(
        ExecutablePlanRevision::try_from(scalar),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.root_rules"
        })
    );
}

#[test]
fn current_root_rules_do_not_prove_unknown_passthrough_against_empty_objects() {
    let rule = nebula_schema::Rule::not(
        nebula_schema::Rule::predicate(nebula_schema::Predicate::Set(
            nebula_schema::ValuePath::root().push("data"),
        ))
        .unwrap(),
    )
    .unwrap();
    let record = root_rule_record(PlanEpoch::GraphV4, rule, None);
    let prepared = record.content.actions[0]
        .input_schema
        .schema
        .validate(AuthoredValue::object())
        .unwrap();
    assert!(
        prepared.pending().is_empty(),
        "the synthetic empty object satisfies this rule"
    );
    assert_matches!(
        ExecutablePlanRevision::try_from(record),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "nodes.parameters.root_rules"
        })
    );
}

#[test]
fn graph_cycle_check_is_iterative_for_deep_dags() {
    let nodes = (0..10_000)
        .map(|index| index.to_string())
        .collect::<Vec<_>>();
    let mut adjacency = HashMap::with_capacity(nodes.len());
    for pair in nodes.windows(2) {
        adjacency.insert(pair[0].as_str(), vec![pair[1].as_str()]);
    }
    assert!(!graph_has_cycle(
        nodes.iter().map(String::as_str),
        &adjacency
    ));
    adjacency
        .entry(nodes.last().expect("the fixture is not empty").as_str())
        .or_default()
        .push(nodes[0].as_str());
    assert!(graph_has_cycle(
        nodes.iter().map(String::as_str),
        &adjacency
    ));
}

#[test]
fn only_replayable_connection_contracts_are_certified() {
    let mut dynamic_declaration = fixture_record();
    dynamic_declaration.content.actions[0].outputs = vec![
        RecordedOutputPortV1::Dynamic {
            key: "branch".into(),
            source_field: "route".into(),
            label_field: None,
            include_fallback: true,
        },
        RecordedOutputPortV1::Flow {
            key: "out".into(),
            flow_kind: RecordedFlowKindV1::Main,
        },
    ]
    .into_boxed_slice();
    reseal(&mut dynamic_declaration);
    ExecutablePlanRevision::try_from(dynamic_declaration.clone())
        .expect("an unused dynamic declaration does not claim routing semantics");

    dynamic_declaration.content.nodes =
        vec![minimal_node("source"), minimal_node("target")].into_boxed_slice();
    dynamic_declaration.content.connections = vec![RecordedConnectionV1 {
        from_node: "source".into(),
        from_port: "branch".into(),
        to_node: "target".into(),
        to_port: None,
    }]
    .into_boxed_slice();
    reseal(&mut dynamic_declaration);
    assert!(matches!(
        ExecutablePlanRevision::try_from(dynamic_declaration),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "connections.from_port"
        })
    ));

    let mut error_flow = fixture_record();
    error_flow.content.actions[0].outputs[0] = RecordedOutputPortV1::Flow {
        key: "out".into(),
        flow_kind: RecordedFlowKindV1::Error,
    };
    error_flow.content.nodes =
        vec![minimal_node("source"), minimal_node("target")].into_boxed_slice();
    error_flow.content.connections = vec![RecordedConnectionV1 {
        from_node: "source".into(),
        from_port: "out".into(),
        to_node: "target".into(),
        to_port: None,
    }]
    .into_boxed_slice();
    reseal(&mut error_flow);
    assert!(matches!(
        ExecutablePlanRevision::try_from(error_flow),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "connections.from_port"
        })
    ));
}

#[test]
fn tag_filtered_support_ports_are_recordable_but_not_certifiable_edges() {
    let mut record = fixture_record();
    record.content.actions[0].inputs = vec![
        RecordedInputPortV1::Flow { key: "in".into() },
        RecordedInputPortV1::Support {
            key: "model".into(),
            required: false,
            multi: false,
            allowed_node_types: None,
            allowed_tags: Some(vec!["llm".into()].into_boxed_slice()),
        },
    ]
    .into_boxed_slice();
    reseal(&mut record);
    ExecutablePlanRevision::try_from(record.clone())
        .expect("unused tag-filter declarations remain an exact contract fact");

    record.content.nodes = vec![minimal_node("source"), minimal_node("target")].into_boxed_slice();
    record.content.connections = vec![RecordedConnectionV1 {
        from_node: "source".into(),
        from_port: "out".into(),
        to_node: "target".into(),
        to_port: Some("model".into()),
    }]
    .into_boxed_slice();
    reseal(&mut record);
    assert!(matches!(
        ExecutablePlanRevision::try_from(record),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "connections.to_port.tag_filter"
        })
    ));
}

#[test]
fn binding_integrity_rejects_unknown_bits_and_duplicate_site_slot() {
    let unknown_bits = credential_binding_record("primary", 0b1000_0000);
    assert!(matches!(
        ExecutablePlanRevision::try_from(unknown_bits),
        Err(ExecutablePlanIntegrityError::UnknownCredentialCapability)
    ));

    let mut duplicate_site_slot = resource_binding_record("primary");
    duplicate_site_slot.bindings = vec![
        resource_binding("client", "primary"),
        resource_binding("client", "secondary"),
    ]
    .into_boxed_slice();
    assert!(matches!(
        ExecutablePlanRevision::try_from(duplicate_site_slot),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "bindings"
        })
    ));
}

#[test]
fn typed_schema_rejects_unknown_fields_and_any_secret_bearing_default() {
    let mut unknown_wire = serde_json::to_value(fixture_record()).expect("record serializes");
    let schema = unknown_wire
        .pointer_mut("/content/actions/0/input_schema/schema")
        .expect("fixture action input schema exists");
    schema.as_object_mut().expect("schema is an object").insert(
        "fields".into(),
        json!([{
            "type": "future_secret",
            "key": "future",
            "payload": SECRET_PAYLOAD
        }]),
    );
    let mut unknown = serde_json::from_value::<RecordedExecutablePlanRevisionV1>(unknown_wire)
        .expect("unknown schema field kinds are preserved by the schema wire");
    reseal(&mut unknown);
    let error = ExecutablePlanRevision::try_from(unknown)
        .expect_err("Graph-v1 must reject opaque future schema fields");
    assert!(matches!(
        error,
        ExecutablePlanIntegrityError::NonCanonical {
            section: "actions.input_schema"
        }
    ));
    assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));

    for secret_schema in [
        object_with_secret_default(json!({"token": SECRET_PAYLOAD})),
        object_with_secret_default(Value::Null),
        mode_with_secret_default(json!({"mode": "token", "value": SECRET_PAYLOAD})),
        mode_with_secret_default(Value::Null),
    ] {
        let mut record = fixture_record();
        record.content.actions[0].input_schema = recorded_schema(secret_schema);
        reseal(&mut record);
        assert!(matches!(
            ExecutablePlanRevision::try_from(record),
            Err(ExecutablePlanIntegrityError::NonCanonical {
                section: "actions.input_schema"
            })
        ));
    }
}

#[test]
fn malformed_schema_decode_is_secret_free_and_fail_closed() {
    let mut encoded = serde_json::to_value(fixture_record()).expect("record serializes");
    let schema = encoded
        .pointer_mut("/content/actions/0/input_schema/schema")
        .expect("fixture action input schema exists");
    *schema = json!({
        "fields": [{
            "type": "string",
            "key": format!("{SECRET_PAYLOAD} invalid")
        }]
    });

    let error = serde_json::from_value::<RecordedExecutablePlanRevisionV1>(encoded)
        .expect_err("a malformed typed schema must fail during decoding");
    assert!(error.to_string().contains("invalid Graph-v1 schema wire"));
    assert!(!error.to_string().contains(SECRET_PAYLOAD));
    assert!(!format!("{error:?}").contains(SECRET_PAYLOAD));
}

#[test]
fn exact_component_build_metadata_is_kept_but_plugin_build_metadata_is_rejected() {
    let mut component = fixture_record();
    component.content.actions[0].version.build = "linux.1".into();
    component.content.nodes[0].action_version.build = "linux.1".into();
    reseal(&mut component);
    ExecutablePlanRevision::try_from(component)
        .expect("exact component versions retain valid build metadata");

    let mut plugin = fixture_record();
    plugin.content.plugins[0].version.build = "linux.1".into();
    reseal(&mut plugin);
    assert!(matches!(
        ExecutablePlanRevision::try_from(plugin),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "plugins.version"
        })
    ));
}

#[test]
fn empty_reference_path_means_whole_output_and_requires_a_durable_edge() {
    let mut record = fixture_record();
    record.content.actions[0].input_schema = recorded_schema(
        Schema::builder()
            .add(ObjectField::new(field_key!("payload")).add(Field::string(field_key!("value"))))
            .build()
            .expect("fixture consumer schema is valid"),
    );
    record.content.actions[0].output_schema = recorded_schema(
        Schema::builder()
            .add(Field::string(field_key!("value")))
            .build()
            .expect("fixture producer schema is valid"),
    );
    record.content.actions[0].inputs = vec![
        RecordedInputPortV1::Flow { key: "in".into() },
        RecordedInputPortV1::Support {
            key: "support".into(),
            required: false,
            multi: false,
            allowed_node_types: None,
            allowed_tags: None,
        },
    ]
    .into_boxed_slice();
    record.content.nodes = vec![minimal_node("source"), minimal_node("target")].into_boxed_slice();
    record.content.nodes[1].parameters = vec![RecordedParameterV1 {
        key: "payload".into(),
        value: RecordedParameterValueV1::Reference {
            node_key: "source".into(),
            output_path: ValuePath::root(),
        },
    }]
    .into_boxed_slice();
    record.content.connections = vec![RecordedConnectionV1 {
        from_node: "source".into(),
        from_port: "out".into(),
        to_node: "target".into(),
        to_port: Some("support".into()),
    }]
    .into_boxed_slice();
    reseal(&mut record);
    ExecutablePlanRevision::try_from(record)
        .expect("an empty reference path selects the producer's whole output");
}

#[test]
fn component_closure_and_binding_references_fail_closed() {
    let mut unused = fixture_record();
    unused.content.credentials = vec![RecordedCredentialV1 {
        key: "demo.oauth".into(),
        plugin_key: "demo".into(),
        version: recorded_semver(1, 0, 0),
        pattern: RecordedAuthPatternV1::OAuth2,
        properties_schema: recorded_schema(ValidSchema::empty()),
        capability_bits: Capabilities::REFRESHABLE.bits(),
    }]
    .into_boxed_slice();
    reseal(&mut unused);
    assert!(matches!(
        ExecutablePlanRevision::try_from(unused),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "components.unused"
        })
    ));

    let mut dangling = resource_binding_record("primary");
    dangling.bindings[0].site = RecordedBindingSiteV1::Node("missing".into());
    reseal(&mut dangling);
    assert!(matches!(
        ExecutablePlanRevision::try_from(dangling),
        Err(ExecutablePlanIntegrityError::NonCanonical {
            section: "bindings.site"
        })
    ));
}

#[test]
fn nested_unknown_fields_fail_closed() {
    let record = resource_binding_record("primary");
    let mut encoded = serde_json::to_value(record).expect("record serializes");
    encoded
        .get_mut("bindings")
        .and_then(Value::as_array_mut)
        .and_then(|bindings| bindings.first_mut())
        .and_then(Value::as_object_mut)
        .expect("fixture binding is an object")
        .insert("future_field".into(), json!(true));
    let wire = serde_json::to_string(&encoded).expect("mutated record serializes");
    let error = serde_json::from_str::<RecordedExecutablePlanRevisionV1>(&wire)
        .expect_err("unknown nested record fields must fail closed");
    assert!(
        error.to_string().contains("unknown field"),
        "unexpected nested decode error: {error}"
    );
}

#[test]
fn checked_plan_roundtrips_record_and_redacts_debug_surfaces() {
    let record = credential_binding_record(SECRET_PAYLOAD, Capabilities::REFRESHABLE.bits());
    assert!(!format!("{record:?}").contains(SECRET_PAYLOAD));

    let wire_value = serde_json::to_value(&record).expect("record serializes");
    let decoded = serde_json::from_value::<RecordedExecutablePlanRevisionV1>(wire_value)
        .expect("record deserializes from an owned JSON value");
    let plan = ExecutablePlanRevision::try_from(decoded).expect("record is integrity-valid");
    assert_eq!(plan.bindings().len(), 1);
    assert_eq!(plan.bindings()[0].selector(), SECRET_PAYLOAD);
    assert!(!format!("{:?}", plan.bindings()[0]).contains(SECRET_PAYLOAD));
    assert!(!format!("{plan:?}").contains(SECRET_PAYLOAD));

    let graph = plan.execution_graph().expect("checked plan projects");
    assert_eq!(graph.bindings(), plan.bindings());
    assert_eq!(graph.bindings()[0].selector(), SECRET_PAYLOAD);
    assert!(
        graph
            .nodes()
            .iter()
            .all(|node| node.slot_bindings.is_empty())
    );
    assert!(!format!("{graph:?}").contains(SECRET_PAYLOAD));

    let projected = RecordedExecutablePlanRevisionV1::from(&plan);
    let reloaded = ExecutablePlanRevision::try_from(projected).expect("roundtrip remains valid");
    assert_eq!(reloaded.id(), plan.id());
    assert_eq!(reloaded.workflow_version_id(), plan.workflow_version_id());
    assert_eq!(reloaded.plugin_set_id(), plan.plugin_set_id());
    assert_eq!(
        reloaded.worker_flavor_revision_id(),
        plan.worker_flavor_revision_id()
    );
}
