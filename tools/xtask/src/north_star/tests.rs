use std::fs;

use serde_json::{Value, json};
use tempfile::TempDir;

use super::{ValidationError, schema, validate_documents};

const VALID_REGISTRY: &str = include_str!("../../gates/north-star-v1.toml");
const VALID_SCHEMA: &str = include_str!("../../schemas/gate-evidence-v1.schema.json");
const VALID_EVIDENCE: &str = include_str!("../../schemas/gate-evidence-v1.example.json");

#[test]
fn valid_manifest_and_schema_pass() {
    let fixture = validation_fixture();
    let summary = validate_documents(VALID_REGISTRY, VALID_SCHEMA, fixture.path())
        .expect("checked-in gate documents validate");

    assert_eq!(summary.registry_version, 1);
    assert_eq!(summary.schema_version, 1);
    assert_eq!(summary.gate_count, 22);
    assert_eq!(summary.status, "valid");
}

#[test]
fn draft_2020_12_schema_and_complete_multi_run_evidence_validate() {
    let schema_document: Value =
        serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    let evidence: Value =
        serde_json::from_str(VALID_EVIDENCE).expect("checked-in evidence example is JSON");

    jsonschema::draft202012::meta::validate(&schema_document)
        .expect("checked-in schema satisfies the Draft 2020-12 meta-schema");
    let validator = jsonschema::draft202012::new(&schema_document)
        .expect("checked-in Draft 2020-12 schema compiles");
    validator
        .validate(&evidence)
        .expect("complete multi-run evidence satisfies schema v1");

    assert_eq!(
        evidence["runs"].as_array().map(Vec::len),
        Some(2),
        "canonical evidence covers more than one environment and CI identity"
    );
    assert!(
        evidence
            .pointer("/threshold_evaluation/metric_results/0/observed")
            .is_some_and(Value::is_i64),
        "canonical evidence includes an integer scalar"
    );
    assert!(
        evidence
            .pointer("/threshold_evaluation/metric_results/1/observed")
            .is_some_and(Value::is_f64),
        "canonical evidence includes a floating-point scalar"
    );
}

#[test]
fn canonical_failed_example_accounts_for_exclusions_and_skips() {
    let evidence: Value =
        serde_json::from_str(VALID_EVIDENCE).expect("checked-in evidence example is JSON");
    let denominator = evidence["denominator"]
        .as_object()
        .expect("canonical denominator is an object");
    let eligible_count = denominator["eligible_count"]
        .as_u64()
        .expect("eligible count is an integer");
    let included_count = denominator["included_count"]
        .as_u64()
        .expect("included count is an integer");
    let excluded_count = denominator["excluded_count"]
        .as_u64()
        .expect("excluded count is an integer");
    let skipped_count = denominator["skipped_count"]
        .as_u64()
        .expect("skipped count is an integer");

    assert_eq!(
        eligible_count,
        included_count + excluded_count + skipped_count
    );
    assert_ne!(excluded_count + skipped_count, 0);
    assert_eq!(evidence["result_status"], json!("failed"));
    assert_eq!(evidence["threshold_evaluation"]["passed"], json!(false));
    assert!(
        evidence["threshold_evaluation"]["metric_results"]
            .as_array()
            .is_some_and(|results| results
                .iter()
                .any(|result| result["passed"] == json!(false))),
        "a failed aggregate must identify at least one failed metric"
    );
}

#[test]
fn draft_meta_schema_rejects_invalid_keyword_type() {
    let mut schema_document: Value =
        serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    schema_document["properties"]["runs"]["maxItems"] = json!("64");
    let schema_source = serde_json::to_string(&schema_document).expect("mutated schema serializes");

    assert_validation_error(
        VALID_REGISTRY,
        &schema_source,
        "Draft 2020-12 meta-schema validation failed",
    );
}

#[test]
fn sparse_evidence_instance_is_rejected() {
    let schema_document: Value =
        serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    let sparse_evidence = json!({
        "gate_id": "NS07",
        "registry_version": 1,
        "evidence_schema_version": 1,
        "result_status": "passed",
        "threshold_evaluation": {
            "passed": true
        }
    });

    let error = schema::validate_instance(&schema_document, &sparse_evidence)
        .expect_err("sparse evidence must not satisfy schema v1");
    assert!(
        matches!(error, ValidationError::InvalidEvidence { .. }),
        "unexpected error: {error}"
    );
}

#[test]
fn passed_result_with_failed_metric_is_rejected() {
    let schema_document: Value =
        serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    let mut evidence: Value =
        serde_json::from_str(VALID_EVIDENCE).expect("checked-in evidence example is JSON");
    evidence["result_status"] = json!("passed");
    evidence["threshold_evaluation"]["passed"] = json!(true);

    schema::validate_instance(&schema_document, &evidence)
        .expect_err("a passed artifact cannot contain a failed metric");
}

#[test]
fn failed_result_uses_non_passed_conditional_branch() {
    let schema_document: Value =
        serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    let mut evidence: Value =
        serde_json::from_str(VALID_EVIDENCE).expect("checked-in evidence example is JSON");
    evidence["result_status"] = json!("failed");
    evidence["threshold_evaluation"]["passed"] = json!(false);
    evidence["threshold_evaluation"]["metric_results"][0]["passed"] = json!(false);

    schema::validate_instance(&schema_document, &evidence)
        .expect("a failed result with a failed aggregate uses the disjoint else branch");
}

#[test]
fn a_run_can_record_database_non_applicability() {
    let schema_document: Value =
        serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    let mut evidence: Value =
        serde_json::from_str(VALID_EVIDENCE).expect("checked-in evidence example is JSON");
    evidence["runs"][0]["environment"]["database"] = json!({
        "applicable": false,
        "non_applicability_reason": "This run exercises a storage-independent governance gate."
    });

    schema::validate_instance(&schema_document, &evidence)
        .expect("a run may explicitly record that a database is not applicable");
}

#[test]
fn evidence_cannot_reference_secret_bearing_observations() {
    let schema_document: Value =
        serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    let mut evidence: Value =
        serde_json::from_str(VALID_EVIDENCE).expect("checked-in evidence example is JSON");
    evidence["runs"][0]["raw_observations"][0]["contains_secrets"] = json!(true);

    schema::validate_instance(&schema_document, &evidence)
        .expect_err("secret-bearing observation artifacts must be rejected");
}

#[test]
fn sampling_policies_reject_values_outside_the_closed_vocabulary() {
    let schema_document: Value =
        serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    let evidence: Value =
        serde_json::from_str(VALID_EVIDENCE).expect("checked-in evidence example is JSON");

    for (pointer, unsupported_policy) in [
        (
            "/runs/0/sampling/invalid_sample_policy",
            "discard-invalid-sample",
        ),
        ("/runs/0/sampling/retry_policy", "retry-once"),
    ] {
        let mut mutated_evidence = evidence.clone();
        *mutated_evidence
            .pointer_mut(pointer)
            .expect("canonical evidence contains every sampling policy") =
            json!(unsupported_policy);

        schema::validate_instance(&schema_document, &mutated_evidence)
            .expect_err("schema v1 must reject sampling policies outside its closed vocabulary");
    }
}

#[test]
fn sampling_policy_schema_cannot_be_widened() {
    let mut schema_document: Value =
        serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    schema_document["$defs"]["sampling"]["properties"]["retry_policy"]["enum"] =
        json!(["no-retry", "retry-once"]);
    let schema_source = serde_json::to_string(&schema_document).expect("mutated schema serializes");

    assert_validation_error(
        VALID_REGISTRY,
        &schema_source,
        "sampling `retry_policy` must be the closed policy `no-retry`",
    );
}

#[test]
fn malformed_gate_id_is_rejected() {
    let registry = VALID_REGISTRY.replacen("id = \"NS01\"", "id = \"NX01\"", 1);

    assert_validation_error(&registry, VALID_SCHEMA, "gate 1 must be `NS01`");
}

#[test]
fn out_of_order_unique_gate_ids_are_rejected() {
    let registry = VALID_REGISTRY
        .replacen("id = \"NS01\"", "id = \"SWAP\"", 1)
        .replacen("id = \"NS02\"", "id = \"NS01\"", 1)
        .replacen("id = \"SWAP\"", "id = \"NS02\"", 1);

    assert_validation_error(&registry, VALID_SCHEMA, "gate 1 must be `NS01`");
}

#[test]
fn multiple_accountable_owners_are_rejected() {
    let registry = VALID_REGISTRY.replacen(
        "owner = \"api-design-lead\"",
        "owner = [\"api-design-lead\", \"qa-lead\"]",
        1,
    );

    let error = validate_fixture(&registry, VALID_SCHEMA).expect_err("owner list must fail");
    assert!(
        matches!(error, ValidationError::RegistryParse(_)),
        "unexpected error: {error}"
    );
}

#[test]
fn backend_set_and_non_applicability_are_mutually_exclusive() {
    let registry = VALID_REGISTRY.replacen(
        "backends = [\"in-memory\", \"sqlite\", \"postgresql\"]",
        "backends = [\"in-memory\", \"sqlite\", \"postgresql\"]\nbackend_non_applicability = \"not applicable\"",
        1,
    );

    assert_validation_error(
        &registry,
        VALID_SCHEMA,
        "must define backends or backend_non_applicability, not both",
    );
}

#[test]
fn percentile_threshold_with_p50_above_p90_is_rejected() {
    let registry = VALID_REGISTRY.replacen("p50_max_minutes = 20", "p50_max_minutes = 46", 1);

    assert_validation_error(
        &registry,
        VALID_SCHEMA,
        "NS12 percentile-duration limits are invalid",
    );
}

#[test]
fn passed_state_is_rejected_by_registry_v1() {
    assert_validation_error(
        &passed_state_registry(),
        VALID_SCHEMA,
        "state `passed` is unsupported because schema v1 cannot represent trustworthy promotion",
    );
}

#[test]
fn binding_to_missing_workflow_job_is_rejected() {
    let registry = VALID_REGISTRY.replacen(
        ".github/workflows/test-matrix.yml#tests",
        ".github/workflows/test-matrix.yml#invented-job",
        1,
    );

    assert_validation_error(&registry, VALID_SCHEMA, "names a missing workflow job");
}

#[test]
fn schema_missing_required_evidence_field_is_rejected() {
    let mut schema: Value = serde_json::from_str(VALID_SCHEMA).expect("checked-in schema is JSON");
    let required = schema["required"]
        .as_array_mut()
        .expect("root required is an array");
    required.retain(|field| field != "result_status");
    let schema = serde_json::to_string(&schema).expect("mutated schema serializes");

    assert_validation_error(VALID_REGISTRY, &schema, "required field set is incomplete");
}

fn assert_validation_error(registry: &str, schema: &str, expected_detail: &str) {
    let error = validate_fixture(registry, schema).expect_err("fixture must be rejected");
    assert!(
        error.to_string().contains(expected_detail),
        "expected `{expected_detail}` in `{error}`"
    );
}

fn validate_fixture(
    registry: &str,
    schema: &str,
) -> Result<super::ValidationSummary, ValidationError> {
    let fixture = validation_fixture();
    validate_documents(registry, schema, fixture.path())
}

/// A registry whose first gate claims the unsupported `passed` state.
///
/// Targets the first `state = "red"` line rather than one gate's prose so the
/// fixture keeps testing schema-v1 promotion no matter which gates are red on
/// any given day — keying it to a specific `state_reason` made the guard
/// silently vacuous the moment that gate's reason was reworded.
fn passed_state_registry() -> String {
    let promoted = VALID_REGISTRY.replacen("state = \"red\"", "state = \"passed\"", 1);
    assert_ne!(
        promoted, VALID_REGISTRY,
        "the registry must contain a red gate for this fixture to promote"
    );
    promoted
}

fn validation_fixture() -> TempDir {
    let fixture = tempfile::tempdir().expect("temporary fixture is created");
    let workflows = fixture.path().join(".github/workflows");
    fs::create_dir_all(&workflows).expect("workflow fixture directory is created");
    fs::write(
        workflows.join("test-matrix.yml"),
        "name: Test matrix\njobs:\n  tests:\n    runs-on: fixture\n  postgres-conformance:\n    runs-on: fixture\n",
    )
    .expect("test-matrix fixture is written");
    fs::write(
        workflows.join("ci.yml"),
        "name: CI\njobs:\n  required:\n    runs-on: fixture\n",
    )
    .expect("CI fixture is written");
    fixture
}
