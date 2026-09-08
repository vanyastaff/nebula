use serde_json::{Value, json};

use super::{ActivationDiagnosticError, SCENARIOS, verify};

// Captured from the real API producer after all 67 scenarios passed. No
// synthetic passing events or authored result booleans seed these mutations.
const OBSERVED: &[u8] = include_bytes!("observed.json");

fn observed() -> Value {
    super::super::super::json::decode(OBSERVED).unwrap()
}

#[test]
fn actual_producer_fragment_satisfies_the_semantic_inventory() {
    let fragment = observed();
    assert_eq!(SCENARIOS.len(), 67);
    assert_eq!(fragment["scenarios"].as_array().unwrap().len(), 67);
    assert_eq!(verify(&fragment), Ok(()));
    let expected = SCENARIOS
        .iter()
        .map(|rule| rule.name)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(expected.len(), SCENARIOS.len());
}

#[test]
fn every_required_scenario_is_individually_required_and_unique() {
    let actual = observed();
    for index in 0..SCENARIOS.len() {
        let mut missing = actual.clone();
        missing["scenarios"].as_array_mut().unwrap().remove(index);
        assert_eq!(verify(&missing), Err(ActivationDiagnosticError::Inventory));

        let mut duplicate = actual.clone();
        duplicate["scenarios"][index] = actual["scenarios"][(index + 1) % SCENARIOS.len()].clone();
        assert_eq!(
            verify(&duplicate),
            Err(ActivationDiagnosticError::Inventory)
        );

        let mut renamed = actual.clone();
        renamed["scenarios"][index]["scenario"] = json!("untrusted-replacement");
        assert_eq!(verify(&renamed), Err(ActivationDiagnosticError::Inventory));
    }
}

#[test]
fn every_event_is_required_with_its_actual_sequence_boundary_and_code() {
    let actual = observed();
    for (scenario_index, scenario) in actual["scenarios"].as_array().unwrap().iter().enumerate() {
        for event_index in 0..scenario["events"].as_array().unwrap().len() {
            let mut missing = actual.clone();
            missing["scenarios"][scenario_index]["events"]
                .as_array_mut()
                .unwrap()
                .remove(event_index);
            assert_eq!(verify(&missing), Err(ActivationDiagnosticError::Event));
            for (field, replacement) in [
                ("sequence", json!(999)),
                ("boundary", json!("invented_boundary")),
                ("kind", json!("test_passed")),
                ("code", json!("WORKFLOW:GRAPH_ERROR")),
                ("http_status", json!(200)),
            ] {
                let mut mutation = actual.clone();
                mutation["scenarios"][scenario_index]["events"][event_index][field] = replacement;
                assert_eq!(
                    verify(&mutation),
                    Err(ActivationDiagnosticError::Event),
                    "scenario {scenario_index}, event {event_index}, {field}"
                );
            }
        }
    }
}

#[test]
fn closed_objects_reject_missing_fields_and_authored_success_claims() {
    let actual = observed();
    for pointer in [
        "",
        "/scenarios/0",
        "/scenarios/0/events/0",
        "/excluded_diagnostics/0",
    ] {
        let original = actual.pointer(pointer).unwrap().as_object().unwrap();
        for field in original.keys() {
            let mut missing = actual.clone();
            missing
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(verify(&missing).is_err(), "{pointer}: {field}");
        }
        let mut extra = actual.clone();
        extra
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("passed".to_owned(), json!(true));
        assert!(verify(&extra).is_err());
    }
}

#[test]
fn versions_hashes_and_exclusions_are_not_self_declared_policy() {
    let actual = observed();
    for (pointer, replacement) in [
        ("/producer_version", json!(1)),
        ("/scenario_inventory_version", json!(2)),
        ("/contract", json!("untrusted-contract")),
        ("/scenarios/0/input_sha256", json!("A".repeat(64))),
        ("/scenarios/0/input_sha256", json!("0".repeat(63))),
        ("/scenarios/0/input_sha256", json!(null)),
        ("/excluded_diagnostics", json!([])),
        (
            "/excluded_diagnostics/0/code",
            json!("PLUGIN_PLAN_INTEGRITY:CANONICAL_ENCODING"),
        ),
        ("/excluded_diagnostics/0/reason", json!("not exercised")),
    ] {
        let mut mutation = actual.clone();
        *mutation.pointer_mut(pointer).unwrap() = replacement;
        assert!(verify(&mutation).is_err(), "{pointer}");
    }
}

#[test]
fn diagnostic_fields_are_bounded_nonempty_and_secret_free() {
    let actual = observed();
    for field in ["code", "path", "expected", "actual", "remediation"] {
        for replacement in [
            json!(null),
            json!(""),
            json!(" \t "),
            json!("x".repeat(4097)),
            json!("prefix activation-private-description-canary suffix"),
        ] {
            let mut mutation = actual.clone();
            mutation["scenarios"][0]["events"][0][field] = replacement;
            assert_eq!(
                verify(&mutation),
                Err(ActivationDiagnosticError::Diagnostic),
                "{field}"
            );
        }
    }
    let mut invalid_path = actual;
    invalid_path["scenarios"][0]["events"][0]["path"] = json!("not-a-pointer");
    assert_eq!(
        verify(&invalid_path),
        Err(ActivationDiagnosticError::Diagnostic)
    );
}

#[test]
fn http_must_preserve_all_five_compiler_fields() {
    let actual = observed();
    for field in ["path", "expected", "actual", "remediation"] {
        let mut mutation = actual.clone();
        mutation["scenarios"][0]["events"][1][field] = json!("/changed-but-nonempty");
        assert_eq!(
            verify(&mutation),
            Err(ActivationDiagnosticError::HttpParity),
            "{field}"
        );
    }
    let mut wrong_code = actual;
    wrong_code["scenarios"][0]["events"][1]["code"] = json!("PLUGIN_PLAN_GRAPH_V1:MISSING_PLUGIN");
    assert_eq!(verify(&wrong_code), Err(ActivationDiagnosticError::Event));
}

#[test]
fn actual_json_cannot_gain_duplicate_keys_before_semantic_admission() {
    let actual = std::str::from_utf8(OBSERVED).unwrap();
    let duplicate = actual.replacen(
        "\"producer_version\": 2",
        "\"producer_version\": 2, \"producer_version\": 2",
        1,
    );
    assert!(super::super::super::json::decode::<Value>(duplicate.as_bytes()).is_err());
    let unknown = actual.replacen(
        "\"producer_version\": 2",
        "\"producer_version\": 2, \"passed\": true",
        1,
    );
    let decoded = super::super::super::json::decode::<Value>(unknown.as_bytes()).unwrap();
    assert_eq!(verify(&decoded), Err(ActivationDiagnosticError::Shape));
}
