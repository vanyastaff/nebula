//! Raw observations from actual admission boundaries, separate from variant fixtures.

use axum::{body::Body, http::Request};
use nebula_api::{ApiConfig, app};
use nebula_core::{WorkflowId, WorkflowVersionId};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use super::common;
#[path = "fixtures.rs"]
mod fixtures;
#[path = "validation.rs"]
mod validation;

const CANARY: &str = "activation-private-description-canary";
const SCENARIOS: &[(&str, &str)] = &[
    ("missing_action", "MISSING_ACTION"),
    ("missing_plugin", "MISSING_PLUGIN"),
    ("unsupported_schema", "UNSUPPORTED_WORKFLOW_SCHEMA"),
    ("duplicate_node", "DUPLICATE_NODE"),
    ("missing_endpoint", "MISSING_CONNECTION_ENDPOINT"),
    ("duplicate_connection", "DUPLICATE_CONNECTION"),
    ("graph_cycle", "GRAPH_CYCLE"),
    ("undeclared_effects", "UNDECLARED_EFFECTS"),
    ("unsupported_effect_kind", "UNSUPPORTED_EFFECT_KIND"),
    ("unsupported_node_kind", "UNSUPPORTED_NODE_KIND"),
    ("action_version_mismatch", "ACTION_VERSION_MISMATCH"),
    ("disabled_node_edge", "DISABLED_NODE_EDGE"),
    ("unsupported_source_port", "UNSUPPORTED_SOURCE_PORT"),
    ("unsupported_target_port", "UNSUPPORTED_TARGET_PORT"),
    ("trigger_kind_mismatch", "TRIGGER_KIND_MISMATCH"),
    ("duplicate_trigger", "DUPLICATE_TRIGGER"),
    ("invalid_reference_path", "INVALID_REFERENCE_PATH"),
    ("unknown_slot_override", "UNKNOWN_SLOT_OVERRIDE"),
    ("invalid_parameter_contract", "INVALID_PARAMETER_CONTRACT"),
    ("invalid_reference_contract", "INVALID_REFERENCE_CONTRACT"),
    (
        "invalid_trigger_configuration",
        "INVALID_TRIGGER_CONFIGURATION",
    ),
    ("missing_resource_contract", "MISSING_RESOURCE_CONTRACT"),
    ("missing_credential_contract", "MISSING_CREDENTIAL_CONTRACT"),
    ("empty_selector", "EMPTY_SELECTOR"),
    ("slot_kind_mismatch", "SLOT_KIND_MISMATCH"),
    ("tag_filter", "UNSUPPORTED_TAG_FILTER"),
    ("node_filter", "NODE_TYPE_FILTER_REJECTED"),
    ("support_required", "SUPPORT_CARDINALITY"),
    ("invalid_projection", "UNSUPPORTED_CONTRACT_PROJECTION"),
    ("wrong_dependency_type", "DEPENDENCY_TYPE_MISMATCH"),
    ("undeclared_dependency", "UNDECLARED_PLUGIN_DEPENDENCY"),
    ("schema_incompatible", "SCHEMA_INCOMPATIBLE"),
    ("compiled_record_depth", "INVALID_COMPILED_RECORD"),
];

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Boundary {
    Compiler,
    HttpActivation,
    PlanIntegrity,
    FlavorIntegrity,
    WorkflowValidation,
    RegistryCompatibility,
}

#[derive(Serialize)]
struct DiagnosticObservation {
    sequence: usize,
    kind: &'static str,
    boundary: Boundary,
    http_status: Option<u16>,
    code: String,
    path: String,
    expected: String,
    actual: String,
    remediation: String,
}

#[derive(Serialize)]
struct ScenarioObservation {
    scenario: String,
    input_sha256: String,
    events: Vec<DiagnosticObservation>,
}

#[derive(Serialize)]
struct Fragment {
    producer_version: u16,
    contract: &'static str,
    scenario_inventory_version: u16,
    excluded_diagnostics: &'static [ExcludedDiagnostic],
    scenarios: Vec<ScenarioObservation>,
}

#[derive(Serialize)]
struct ExcludedDiagnostic {
    code: &'static str,
    reason: &'static str,
}

// Ratified inventory exclusions, not evidence that either rejection was executed.
// Their five-field formatting remains exercised by the separate variant tests.
const EXCLUDED_DIAGNOSTICS: &[ExcludedDiagnostic] = &[ExcludedDiagnostic {
    code: "WORKFLOW:GRAPH_ERROR",
    reason: "No reachable producer beyond error conversion plumbing in the supported workflow validator.",
}];

fn fixture(scenario: &str, workflow: WorkflowId) -> Value {
    let mut definition = common::make_valid_workflow_definition(&workflow);
    definition["description"] = json!(CANARY);
    match scenario {
        "compiled_record_depth" => {
            let mut nested = json!(0);
            for _ in 0..62 {
                nested = json!([nested]);
            }
            definition["variables"] = json!({"nested":nested});
        },
        "wrong_dependency_type" | "undeclared_dependency" => {
            definition["nodes"][0]["action_key"] = json!(scenario);
        },
        "schema_incompatible" => {
            let mut target = definition["nodes"][0].clone();
            target["id"] = json!("step_b");
            target["action_key"] = json!("string_input");
            target["parameters"] = json!({"value":{"type":"literal", "value":"fixture"}});
            definition["nodes"].as_array_mut().unwrap().push(target);
            definition["connections"] =
                json!([{"from_node":"step_a", "from_port":"error", "to_node":"step_b"}]);
        },
        "support_required" | "invalid_projection" => {
            definition["nodes"][0]["action_key"] = json!(scenario);
        },
        "tag_filter" | "node_filter" => {
            let mut target = definition["nodes"][0].clone();
            target["id"] = json!("step_b");
            target["action_key"] = json!(scenario);
            definition["nodes"].as_array_mut().unwrap().push(target);
            definition["connections"] =
                json!([{"from_node":"step_a", "to_node":"step_b", "to_port":"support"}]);
        },
        "invalid_parameter_contract" | "invalid_reference_contract" => {
            definition["nodes"][0]["action_key"] = json!("string_input");
            definition["nodes"][0]["parameters"] = if scenario == "invalid_parameter_contract" {
                json!({"value":{"type":"literal", "value":123}})
            } else {
                json!({"value":{"type":"reference", "node_key":"absent", "output_path":"value"}})
            };
        },
        "invalid_trigger_configuration" => {
            definition["trigger_bindings"] = json!([{"id":"start", "plugin_key":"core", "action_key":"secret_trigger", "config":{"token":CANARY}}]);
        },
        "missing_resource_contract"
        | "missing_credential_contract"
        | "empty_selector"
        | "slot_kind_mismatch" => {
            definition["nodes"][0]["action_key"] =
                json!(if scenario == "missing_credential_contract" {
                    "credential_slot"
                } else {
                    "resource_slot"
                });
            if scenario == "empty_selector" {
                definition["nodes"][0]["slot_bindings"] =
                    json!({"binding":{"kind":"resource_id", "id":" "}});
            }
            if scenario == "slot_kind_mismatch" {
                definition["nodes"][0]["slot_bindings"] =
                    json!({"binding":{"kind":"credential_id", "id":"fixture"}});
            }
        },
        "missing_action" => definition["nodes"][0]["action_key"] = json!("missing"),
        "missing_plugin" => definition["nodes"][0]["plugin_key"] = json!("missing"),
        "unsupported_schema" => definition["schema_version"] = json!(99),
        "undeclared_effects" => definition["nodes"][0]["action_key"] = json!("undeclared"),
        "unsupported_effect_kind" => definition["nodes"][0]["action_key"] = json!("remote_control"),
        "unsupported_node_kind" => definition["nodes"][0]["action_key"] = json!("trigger"),
        "action_version_mismatch" => definition["nodes"][0]["interface_version"] = json!("99.0.0"),
        "invalid_reference_path" => {
            definition["nodes"][0]["parameters"] = json!({"input":{"type":"reference", "node_key":"absent", "output_path":"$[?(@.secret)]"}});
        },
        "unknown_slot_override" => {
            definition["nodes"][0]["slot_bindings"] =
                json!({"absent":{"kind":"resource_id", "id":"fixture"}});
        },
        "trigger_kind_mismatch" | "duplicate_trigger" => {
            let trigger = json!({"id":"start", "plugin_key":"core", "action_key":"echo"});
            definition["trigger_bindings"] = if scenario == "duplicate_trigger" {
                json!([trigger, trigger])
            } else {
                json!([trigger])
            };
        },
        "duplicate_node" => {
            let duplicate = definition["nodes"][0].clone();
            definition["nodes"].as_array_mut().unwrap().push(duplicate);
        },
        "missing_endpoint" => {
            definition["connections"] = json!([{"from_node":"step_a","to_node":"absent"}]);
        },
        "duplicate_connection"
        | "graph_cycle"
        | "disabled_node_edge"
        | "unsupported_source_port"
        | "unsupported_target_port" => {
            let mut second = definition["nodes"][0].clone();
            second["id"] = json!("step_b");
            definition["nodes"].as_array_mut().unwrap().push(second);
            let first = json!({"from_node":"step_a","to_node":"step_b"});
            let second = if scenario == "graph_cycle" {
                json!({"from_node":"step_b","to_node":"step_a"})
            } else {
                first.clone()
            };
            definition["connections"] = json!([first, second]);
            if !matches!(scenario, "duplicate_connection" | "graph_cycle") {
                definition["connections"]
                    .as_array_mut()
                    .unwrap()
                    .truncate(1);
                match scenario {
                    "disabled_node_edge" => definition["nodes"][0]["enabled"] = json!(false),
                    "unsupported_source_port" => {
                        definition["connections"][0]["from_port"] = json!("absent");
                    },
                    "unsupported_target_port" => {
                        definition["connections"][0]["to_port"] = json!("absent");
                    },
                    _ => unreachable!(),
                }
            }
        },
        _ => panic!("inventory contains an undefined fixture"),
    }
    definition
}

fn bounded_field(value: &str) -> String {
    assert!(!value.trim().is_empty());
    assert!(value.len() <= 4096);
    assert!(!value.contains(CANARY));
    value.to_owned()
}

#[tokio::test]
async fn records_actual_compiler_and_http_diagnostics() {
    let mut scenarios = Vec::new();
    for &(scenario, expected_code) in SCENARIOS {
        let (mut state, stores) = common::create_state_with_port_handles().await;
        let registry = fixtures::registry();
        state.workflow_activation = Some(std::sync::Arc::new(
            nebula_engine::WorkflowActivationService::new(
                state.workflow_store.clone(),
                state.workflow_version_store.clone(),
                registry.clone(),
                nebula_engine::PlanFlavorRevisionInstaller::new(std::sync::Arc::new(
                    nebula_storage::InMemoryExecutionStore::new().plan_flavor_catalog(),
                )),
                std::sync::Arc::new(nebula_core::accessor::SystemClock),
            ),
        ));
        let workflow = WorkflowId::new();
        let definition = fixture(scenario, workflow);
        let input = serde_json::to_vec(&definition).unwrap();
        let input_sha256 = hex::encode(Sha256::digest(&input));
        let parsed = serde_json::from_slice(&input).unwrap();
        let rejection = registry
            .compile_graph_v1(WorkflowVersionId::new(), &parsed)
            .expect_err(scenario);
        let mut events = Vec::new();
        for diagnostic in rejection.diagnostics() {
            events.push(DiagnosticObservation {
                sequence: events.len(),
                kind: "diagnostic_raised",
                boundary: Boundary::Compiler,
                http_status: None,
                code: bounded_field(diagnostic.code()),
                path: bounded_field(diagnostic.path()),
                expected: bounded_field(diagnostic.expected()),
                actual: bounded_field(diagnostic.actual()),
                remediation: bounded_field(diagnostic.remediation()),
            });
        }
        let expected_code = format!("PLUGIN_PLAN_GRAPH_V1:{expected_code}");
        assert!(
            events.iter().any(|event| event.code == expected_code),
            "{scenario}: {}",
            serde_json::to_string(&events).unwrap()
        );
        let compiler_count = events.len();
        stores.seed_workflow(workflow, definition).await;
        let response = app::build_app(state, &ApiConfig::for_test())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(common::ws_path(&format!("/workflows/{workflow}/activate")))
                    .header(
                        "authorization",
                        format!("Bearer {}", common::create_test_jwt()),
                    )
                    .header("x-csrf-token", common::TEST_CSRF_TOKEN)
                    .header("cookie", common::TEST_CSRF_COOKIE)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        assert_eq!(status, 422, "{scenario}");
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(CANARY));
        let problem: Value = serde_json::from_slice(&bytes).unwrap();
        for diagnostic in problem["errors"].as_array().unwrap() {
            events.push(DiagnosticObservation {
                sequence: events.len(),
                kind: "diagnostic_raised",
                boundary: Boundary::HttpActivation,
                http_status: Some(status),
                code: bounded_field(diagnostic["code"].as_str().unwrap()),
                path: bounded_field(
                    diagnostic
                        .get("pointer")
                        .or_else(|| diagnostic.get("path"))
                        .unwrap()
                        .as_str()
                        .unwrap(),
                ),
                expected: bounded_field(diagnostic["expected"].as_str().unwrap()),
                actual: bounded_field(diagnostic["actual"].as_str().unwrap()),
                remediation: bounded_field(diagnostic["remediation"].as_str().unwrap()),
            });
        }
        assert_eq!(events.len(), compiler_count * 2);
        for (compiler, http) in events[..compiler_count]
            .iter()
            .zip(&events[compiler_count..])
        {
            assert_eq!(
                (
                    &compiler.code,
                    &compiler.path,
                    &compiler.expected,
                    &compiler.actual,
                    &compiler.remediation
                ),
                (
                    &http.code,
                    &http.path,
                    &http.expected,
                    &http.actual,
                    &http.remediation
                )
            );
        }
        scenarios.push(ScenarioObservation {
            scenario: scenario.to_owned(),
            input_sha256,
            events,
        });
    }
    assert_eq!(scenarios.len(), SCENARIOS.len());
    scenarios.extend(checked_record_observations());
    scenarios.extend(validation::observations());
    let observed_codes = scenarios
        .iter()
        .flat_map(|scenario| scenario.events.iter().map(|event| event.code.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    let compiler_codes = observed_codes
        .iter()
        .copied()
        .filter(|code| code.starts_with("PLUGIN_PLAN_GRAPH_V1:"))
        .collect::<std::collections::BTreeSet<_>>();
    let expected_compiler_codes = SCENARIOS
        .iter()
        .map(|(_, code)| format!("PLUGIN_PLAN_GRAPH_V1:{code}"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        compiler_codes,
        expected_compiler_codes.iter().map(String::as_str).collect()
    );
    let mut expected_codes = expected_compiler_codes;
    for (namespace, codes) in [
        (
            "PLUGIN_PLAN_INTEGRITY",
            &[
                "UNSUPPORTED_FORMAT",
                "NON_CANONICAL",
                "CONVERTERS_UNSUPPORTED",
                "REVISION_ID_MISMATCH",
                "UNKNOWN_CAPABILITY",
                "CANONICAL_ENCODING",
            ][..],
        ),
        (
            "PLUGIN_FLAVOR_INTEGRITY",
            &[
                "UNSUPPORTED_RECORD_VERSION",
                "UNSUPPORTED_CANONICAL_HASH_VERSION",
                "REVISION_ID_MISMATCH",
            ][..],
        ),
        (
            "PLUGIN_PLAN_COMPATIBILITY",
            &[
                "UNSUPPORTED_EFFECT_PROTOCOL",
                "PLUGIN_SET_MISMATCH",
                "WORKER_FLAVOR_MISMATCH",
                "CONTRACT_MISMATCH",
            ][..],
        ),
        (
            "WORKFLOW",
            &[
                "EMPTY_NAME",
                "NO_NODES",
                "DUPLICATE_NODE_KEY",
                "UNKNOWN_NODE",
                "SELF_LOOP",
                "CYCLE_DETECTED",
                "NO_ENTRY_NODES",
                "INVALID_PARAM_REF",
                "REFERENCE_WITHOUT_CONNECTION",
                "INVALID_ACTION_KEY",
                "INVALID_PLUGIN_KEY",
                "INVALID_TRIGGER",
                "UNSUPPORTED_SCHEMA",
                "INVALID_OWNER_ID",
                "DUPLICATE_CONNECTION",
                "INVALID_RETRY_CONFIG",
                "PORT_SCHEMA_INCOMPATIBLE",
                "PORT_SCHEMA_UNDECIDABLE",
                "REFERENCE_PATH_UNRESOLVED",
                "REFERENCE_TYPE_INCOMPATIBLE",
                "REFERENCE_TYPE_UNDECIDABLE",
            ][..],
        ),
    ] {
        expected_codes.extend(codes.iter().map(|code| format!("{namespace}:{code}")));
    }
    assert_eq!(
        observed_codes,
        expected_codes.iter().map(String::as_str).collect(),
        "actual diagnostic-code set must exactly cover the trusted inventory"
    );
    let fragment = Fragment {
        producer_version: 2,
        contract: "activation-diagnostics",
        scenario_inventory_version: 1,
        excluded_diagnostics: EXCLUDED_DIAGNOSTICS,
        scenarios,
    };
    if let Some(path) = std::env::var_os("NEBULA_ACTIVATION_DIAGNOSTICS_OBSERVATIONS_PATH") {
        use std::io::Write;
        let path = std::path::Path::new(&path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let bytes = serde_json::to_vec_pretty(&fragment).unwrap();
        assert!(bytes.len() <= 1024 * 1024);
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .unwrap();
        output.write_all(&bytes).unwrap();
        output.sync_all().unwrap();
    }
}

fn observe_rejection(
    scenario: &str,
    input: &Value,
    boundary: Boundary,
    rejection: &dyn nebula_error::ActivationDiagnostics,
) -> ScenarioObservation {
    let diagnostics = rejection.activation_diagnostics();
    assert!(!diagnostics.is_empty(), "{scenario}");
    let events = diagnostics
        .iter()
        .enumerate()
        .map(|(sequence, diagnostic)| DiagnosticObservation {
            sequence,
            kind: "diagnostic_raised",
            boundary,
            http_status: None,
            code: bounded_field(diagnostic.code()),
            path: bounded_field(diagnostic.path()),
            expected: bounded_field(diagnostic.expected()),
            actual: bounded_field(diagnostic.actual()),
            remediation: bounded_field(diagnostic.remediation()),
        })
        .collect();
    ScenarioObservation {
        scenario: scenario.to_owned(),
        input_sha256: hex::encode(Sha256::digest(serde_json::to_vec(input).unwrap())),
        events,
    }
}

fn checked_record_observations() -> Vec<ScenarioObservation> {
    use nebula_plugin::{
        ExecutablePlanRevision, RecordedExecutablePlanRevisionV1, RecordedWorkerFlavorRevisionV1,
        WorkerFlavorRevision,
    };
    let registry = fixtures::registry();
    let definition = common::make_valid_workflow_definition(&WorkflowId::new());
    let parsed = serde_json::from_str(&definition.to_string()).unwrap();
    let plan = registry
        .compile_graph_v1(WorkflowVersionId::new(), &parsed)
        .unwrap();
    let plan_record = serde_json::to_value(RecordedExecutablePlanRevisionV1::from(&plan)).unwrap();
    let mut observations = Vec::new();
    for scenario in [
        "legacy_compiler_1",
        "unsupported_compiler_2",
        "plugin_set_mismatch",
        "worker_flavor_mismatch",
        "contract_mismatch",
    ] {
        let mut input = plan_record.clone();
        match scenario {
            "legacy_compiler_1" | "unsupported_compiler_2" => {
                input["compiler_version"] = json!(if scenario == "legacy_compiler_1" {
                    1
                } else {
                    2
                });
                input["canonical_hash_version"] = json!(1);
                for action in input["content"]["actions"].as_array_mut().unwrap() {
                    action.as_object_mut().unwrap().remove("effect_contract");
                }
            },
            "plugin_set_mismatch" => {
                input["plugin_set_id"] =
                    serde_json::to_value(nebula_core::PluginSetId::from_bytes([0x99; 32])).unwrap();
            },
            "worker_flavor_mismatch" => {
                input["worker_flavor_revision_id"] = serde_json::to_value(
                    nebula_core::WorkerFlavorRevisionId::from_bytes([0x99; 32]),
                )
                .unwrap();
            },
            "contract_mismatch" => input["content"]["actions"][0]["max_concurrent"] = json!(17),
            _ => unreachable!(),
        }
        rehash_record(&mut input);
        let record: RecordedExecutablePlanRevisionV1 =
            serde_json::from_value(input.clone()).unwrap();
        let (observation, expected_code) = if scenario == "unsupported_compiler_2" {
            let rejection = ExecutablePlanRevision::try_from(record).unwrap_err();
            (
                observe_rejection(
                    "plan_integrity.unsupported_compiler_2",
                    &input,
                    Boundary::PlanIntegrity,
                    &rejection,
                ),
                "PLUGIN_PLAN_INTEGRITY:UNSUPPORTED_FORMAT".to_owned(),
            )
        } else {
            let checked = ExecutablePlanRevision::try_from(record).unwrap();
            let rejection = checked.validate_against(&registry).unwrap_err();
            let code = if scenario == "legacy_compiler_1" {
                "UNSUPPORTED_EFFECT_PROTOCOL".to_owned()
            } else {
                scenario.to_ascii_uppercase()
            };
            (
                observe_rejection(
                    &format!("registry_compatibility.{scenario}"),
                    &input,
                    Boundary::RegistryCompatibility,
                    &rejection,
                ),
                format!("PLUGIN_PLAN_COMPATIBILITY:{code}"),
            )
        };
        assert_eq!(observation.events[0].code, expected_code);
        observations.push(observation);
    }
    for (scenario, code) in [
        ("unsupported_format", "UNSUPPORTED_FORMAT"),
        ("noncanonical", "NON_CANONICAL"),
        ("converters_unsupported", "CONVERTERS_UNSUPPORTED"),
        ("revision_mismatch", "REVISION_ID_MISMATCH"),
        ("unknown_capability", "UNKNOWN_CAPABILITY"),
        ("canonical_encoding_depth", "CANONICAL_ENCODING"),
    ] {
        let mut input = plan_record.clone();
        match scenario {
            "unsupported_format" => input["record_version"] = json!(99),
            "canonical_encoding_depth" => {
                let mut nested = json!(0);
                for _ in 0..62 {
                    nested = json!([nested]);
                }
                input["content"]["variables"] = json!([{"name":"nested", "value":nested}]);
            },
            "unknown_capability" => {
                input["content"]["credentials"] = json!([{
                    "key":"core.credential", "plugin_key":"core", "version":input["content"]["actions"][0]["version"],
                    "pattern":"no_auth", "properties_schema":input["content"]["actions"][0]["input_schema"], "capability_bits":128
                }]);
            },
            "noncanonical" => input["content"]["actions"][0]
                .as_object_mut()
                .unwrap()
                .remove("effect_contract")
                .map(drop)
                .unwrap(),
            "converters_unsupported" => {
                input["content"]["converters"] = json!([{"key":"fixture.converter"}]);
            },
            "revision_mismatch" => {
                input["claimed_id"] = serde_json::to_value(
                    nebula_core::ExecutablePlanRevisionId::from_bytes([0x99; 32]),
                )
                .unwrap();
            },
            _ => unreachable!(),
        }
        let record: RecordedExecutablePlanRevisionV1 =
            serde_json::from_value(input.clone()).unwrap();
        let rejection = ExecutablePlanRevision::try_from(record).unwrap_err();
        let observation = observe_rejection(
            &format!("plan_integrity.{scenario}"),
            &input,
            Boundary::PlanIntegrity,
            &rejection,
        );
        assert_eq!(
            observation.events[0].code,
            format!("PLUGIN_PLAN_INTEGRITY:{code}")
        );
        observations.push(observation);
    }
    let flavor_record =
        serde_json::to_value(RecordedWorkerFlavorRevisionV1::from(registry.revision())).unwrap();
    for (scenario, code) in [
        ("unsupported_record_version", "UNSUPPORTED_RECORD_VERSION"),
        (
            "unsupported_hash_version",
            "UNSUPPORTED_CANONICAL_HASH_VERSION",
        ),
        ("revision_mismatch", "REVISION_ID_MISMATCH"),
    ] {
        let mut input = flavor_record.clone();
        match scenario {
            "unsupported_record_version" => input["record_version"] = json!(99),
            "unsupported_hash_version" => input["canonical_hash_version"] = json!(99),
            "revision_mismatch" => {
                input["claimed_id"] = serde_json::to_value(
                    nebula_core::WorkerFlavorRevisionId::from_bytes([0x99; 32]),
                )
                .unwrap();
            },
            _ => unreachable!(),
        }
        let record: RecordedWorkerFlavorRevisionV1 = serde_json::from_value(input.clone()).unwrap();
        let rejection = WorkerFlavorRevision::try_from(record).unwrap_err();
        let observation = observe_rejection(
            &format!("flavor_integrity.{scenario}"),
            &input,
            Boundary::FlavorIntegrity,
            &rejection,
        );
        assert_eq!(
            observation.events[0].code,
            format!("PLUGIN_FLAVOR_INTEGRITY:{code}")
        );
        observations.push(observation);
    }
    for (scenario, codes) in [
        ("empty_name", &["EMPTY_NAME"][..]),
        ("no_nodes", &["NO_NODES"][..]),
        ("self_loop", &["SELF_LOOP"][..]),
        ("duplicate_node", &["DUPLICATE_NODE_KEY"][..]),
        ("missing_endpoint", &["UNKNOWN_NODE"][..]),
        ("duplicate_connection", &["DUPLICATE_CONNECTION"][..]),
        ("graph_cycle", &["CYCLE_DETECTED", "NO_ENTRY_NODES"][..]),
        ("invalid_reference_contract", &["INVALID_PARAM_REF"][..]),
        ("unsupported_schema", &["UNSUPPORTED_SCHEMA"][..]),
        ("duplicate_trigger", &["INVALID_TRIGGER"][..]),
        (
            "reference_without_connection",
            &["REFERENCE_WITHOUT_CONNECTION"][..],
        ),
        ("invalid_retry", &["INVALID_RETRY_CONFIG"][..]),
    ] {
        let mut input = definition.clone();
        match scenario {
            "empty_name" => input["name"] = json!(""),
            "no_nodes" => input["nodes"] = json!([]),
            "self_loop" => {
                input["connections"] = json!([{"from_node":"step_a", "to_node":"step_a"}]);
            },
            "reference_without_connection" => {
                input["nodes"][0]["parameters"] = json!({"value":{"type":"reference", "node_key":"step_a", "output_path":"value"}});
            },
            "invalid_retry" => {
                input["config"]["retry_policy"] = json!({"max_attempts":0, "initial_delay_ms":100, "max_delay_ms":1000, "backoff_multiplier":2.0});
            },
            _ => input = fixture(scenario, parsed.id),
        }
        let parsed = serde_json::from_str(&input.to_string()).unwrap();
        let errors = nebula_workflow::validate_workflow(&parsed);
        assert!(!errors.is_empty());
        let mut observation = ScenarioObservation {
            scenario: format!("workflow_validation.{scenario}"),
            input_sha256: hex::encode(Sha256::digest(serde_json::to_vec(&input).unwrap())),
            events: Vec::new(),
        };
        for error in &errors {
            let rejected = observe_rejection(
                &observation.scenario,
                &input,
                Boundary::WorkflowValidation,
                error,
            );
            for mut event in rejected.events {
                event.sequence = observation.events.len();
                observation.events.push(event);
            }
        }
        for code in codes {
            assert!(
                observation
                    .events
                    .iter()
                    .any(|event| event.code == format!("WORKFLOW:{code}")),
                "{scenario} must observe {code}"
            );
        }
        observations.push(observation);
    }
    observations
}

fn rehash_record(record: &mut Value) {
    record.as_object_mut().unwrap().remove("claimed_id");
    let canonical = nebula_schema::FieldValue::Literal(record.clone())
        .canonical_bytes()
        .unwrap();
    let domain = if record["canonical_hash_version"] == 1 {
        b"nebula.executable-plan.graph.v1"
    } else {
        b"nebula.executable-plan.graph.v2"
    };
    let mut hash = Sha256::new();
    for (tag, bytes) in [(1_u8, domain.as_slice()), (2, canonical.as_slice())] {
        hash.update([tag]);
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    let digest: [u8; 32] = hash.finalize().into();
    record["claimed_id"] =
        serde_json::to_value(nebula_core::ExecutablePlanRevisionId::from_bytes(digest)).unwrap();
}
