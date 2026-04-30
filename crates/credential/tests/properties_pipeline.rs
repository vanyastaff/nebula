//! Phase 9 / Task 9.3 — Credential properties validation pipeline.
//!
//! Exercises the validation half of the action pipeline against the `<Name>Properties` companion
//! struct that Phase 5 attached to every `Credential` impl. The test pins:
//!
//!   1. `<C as Credential>::properties_schema()` returns the schema of `<C::Properties as
//!      HasSchema>::schema()`.
//!   2. JSON properties → `FieldValues::from_json` → `schema.validate` →
//!      `serde_json::from_value::<C::Properties>`. The two passes (schema and serde) are
//!      independent.
//!   3. **Credential properties never run through `ValidValues::resolve`.** The engine deliberately
//!      omits the expression-resolution step from the credential pipeline (canon §12.5: secrets
//!      must not depend on runtime workflow state). This test asserts the policy by validating the
//!      schema directly without `.resolve(...)` and by shape-checking the post-validate value tree
//!      (no template gets replaced).
//!
//! See `crates/credential/README.md` "Expressions in credential properties" for the architectural
//! rationale.

use nebula_credential::{Credential, credentials::ApiKeyCredential};
use nebula_schema::{FieldValue, FieldValues, HasSchema};
use serde_json::json;

// ── Pipeline happy path on built-in ApiKeyCredential ───────────────────────

#[test]
fn properties_schema_matches_companion_struct() {
    let from_credential = ApiKeyCredential::properties_schema();
    let from_companion = <<ApiKeyCredential as Credential>::Properties as HasSchema>::schema();
    assert_eq!(
        format!("{from_credential:?}"),
        format!("{from_companion:?}"),
        "Credential::properties_schema() must equal <Properties as HasSchema>::schema()"
    );
}

#[test]
fn properties_pipeline_accepts_well_formed_json() {
    let schema = ApiKeyCredential::properties_schema();
    let raw = json!({
        "server": "https://api.example.com",
        "api_key": "sk-test-12345",
    });

    // 1. Ingest into FieldValues.
    let values = FieldValues::from_json(raw.clone()).expect("ingest");

    // 2. Schema validation (no `.resolve(...)` step — see expression policy below).
    schema.validate(&values).expect("validate must pass");

    // 3. Typed deserialize into the companion struct.
    let typed: <ApiKeyCredential as Credential>::Properties =
        serde_json::from_value(raw).expect("typed deserialize");
    assert_eq!(typed.server.as_deref(), Some("https://api.example.com"));
    assert_eq!(typed.api_key, "sk-test-12345");
}

#[test]
fn properties_pipeline_rejects_missing_required_api_key() {
    let schema = ApiKeyCredential::properties_schema();
    let raw = json!({
        "server": "https://api.example.com",
        // `api_key` omitted — it carries `#[validate(required)]`.
    });
    let values = FieldValues::from_json(raw).expect("ingest");
    let report = schema.validate(&values).expect_err("must reject");
    assert!(
        report
            .errors()
            .any(|e| e.code.as_ref() == "required" && e.path.to_string() == "api_key"),
        "expected `required` on api_key, got: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}

// ── Expression policy (canon §12.5) ────────────────────────────────────────

/// Policy: the credential pipeline does NOT run `valid.resolve(...)`.
///
/// Even though every individual `Field` defaults to
/// `ExpressionMode::Allowed` at the schema layer, credentials skip the
/// resolution step entirely. A `{{ ... }}` template in a credential
/// property survives validation as `FieldValue::Expression` and is then
/// rejected by `serde::Deserialize` (which cannot deserialize a tagged
/// expression object into the target type).
///
/// Rationale: secrets must not depend on runtime workflow state. A property
/// value resolved via expression would couple credential storage to
/// per-execution variables, breaking encapsulation and making secret
/// rotation reason about workflow context.
///
/// Enforcement points (defense in depth):
///
/// 1. **Engine pipeline shape** — `nebula-engine` passes credential properties through
///    `ValidSchema::validate` only, never through `.resolve(...)`. This is the authoritative seam;
///    documented in `crates/credential/README.md`.
/// 2. **Serde refusal** — even if a caller sneaks a `{{ ... }}` template past validation,
///    `serde_json::from_value::<C::Properties>(...)` fails to deserialize the `{"$expr": "..."}`
///    envelope into the typed `String` / `i64` / etc. property field.
///
/// This test asserts (2) by exercising the `from_value` step directly.
#[test]
fn expressions_in_properties_fail_serde_deserialize() {
    // Template inside the secret field. Validation passes (expressions are
    // syntactically allowed at the schema layer), but the resolved value
    // tree still contains `FieldValue::Expression` because the credential
    // pipeline does not call `.resolve(...)`.
    let raw = json!({
        "server": "https://api.example.com",
        "api_key": { "$expr": "{{ $execution.id }}" },
    });

    // Schema validate — passes because ExpressionMode is Allowed by default.
    let schema = ApiKeyCredential::properties_schema();
    let values = FieldValues::from_json(raw.clone()).expect("ingest");
    let validated = schema.validate(&values).expect("validate must pass");

    // Inspect: the raw value tree retains the expression literal — the
    // credential pipeline never resolves it.
    let api_key_value = validated
        .raw()
        .get(&nebula_schema::FieldKey::new("api_key").unwrap())
        .expect("api_key value present");
    assert!(
        matches!(api_key_value, FieldValue::Expression(_)),
        "credential pipeline must leave FieldValue::Expression unresolved; \
         got: {api_key_value:?}",
    );

    // serde::Deserialize attempt — fails because `api_key` is `String` but
    // the JSON tree carries a `{"$expr": "..."}` object.
    let result = serde_json::from_value::<<ApiKeyCredential as Credential>::Properties>(raw);
    assert!(
        result.is_err(),
        "serde::Deserialize must refuse expression-bearing credential properties; \
         got Ok variant (Properties does not impl Debug, cannot print)",
    );
}

#[test]
fn expressions_in_optional_property_field_also_fail_serde() {
    let raw = json!({
        "server": { "$expr": "{{ $workflow.base_url }}" },
        "api_key": "sk-real-secret",
    });

    let schema = ApiKeyCredential::properties_schema();
    let values = FieldValues::from_json(raw.clone()).expect("ingest");
    schema.validate(&values).expect("validate passes");

    // The optional `server: Option<String>` is also targeted by canon §12.5;
    // serde refuses the `{"$expr": ...}` envelope as a `String`.
    let result = serde_json::from_value::<<ApiKeyCredential as Credential>::Properties>(raw);
    assert!(
        result.is_err(),
        "expressions in optional credential properties must also fail serde; \
         got Ok variant (Properties does not impl Debug)",
    );
}
