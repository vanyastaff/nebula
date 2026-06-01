//! seam — no-payload mode-variant expression smuggle, runtime
//! defence-in-depth.
//!
//! `mode.no_payload_variant_must_forbid_expression` (a build-fatal lint) is the
//! primary boundary: a no-payload variant whose placeholder does not pin
//! `ExpressionMode::Forbidden` cannot be built into a `ValidSchema` at all. This
//! file pins the *runtime* half of the same property: for the schema that the
//! lint blesses — a no-payload variant built via `ModeField::variant_empty`,
//! which pins `Forbidden` — a submitted `{"mode":"<k>","value":{"$expr":…}}`
//! (or the `{{ … }}` string form) is still rejected with `expression.forbidden`
//! at `validate()`, before it can reach `resolve()` and be evaluated. The two
//! controls are independent: the lint refuses the dangerous schema shape; this
//! proves the realistic smuggle against the *safe* shape never escapes either.

use nebula_schema::mode::VisibilityMode;
use nebula_schema::{Field, FieldValues, Schema, field_key};
use serde_json::json;

// Test helper: render a canonical RFC-6901 field pointer (`/a/b/0`) back into
// the schema's dotted/bracketed display (`a.b[0]`) so historical path
// assertions keep their original form after the nebula-error migration.
fn field_dotted(e: &nebula_schema::ValidationError) -> String {
    let Some(pointer) = e.field.as_deref() else {
        return String::new();
    };
    let mut out = String::new();
    for seg in pointer.trim_start_matches('/').split('/') {
        if seg.is_empty() {
            continue;
        }
        let unescaped = seg.replace("~1", "/").replace("~0", "~");
        if unescaped.chars().all(|c| c.is_ascii_digit()) {
            out.push('[');
            out.push_str(&unescaped);
            out.push(']');
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(&unescaped);
        }
    }
    out
}

fn codes(report: &nebula_schema::ValidationReport) -> Vec<(String, String)> {
    report
        .errors()
        .map(|e| (e.code.to_string(), field_dotted(e)))
        .collect()
}

#[test]
fn visible_no_payload_variant_object_expr_payload_is_rejected_at_validate() {
    // The realistic smuggle against the lint-blessed shape: a *visible*
    // (default) no-payload variant built via `variant_empty` (placeholder pins
    // ExpressionMode::Forbidden). An attacker submits an `{"$expr":…}` object
    // under the hidden placeholder. `validate()` must reject it with
    // `expression.forbidden` — the expression must never reach `resolve()`.
    // This is independent of the build-time lint (the schema is valid; the
    // runtime path is what fails closed here).
    let schema = Schema::builder()
        .add(Field::mode(field_key!("auth")).variant_empty("none", "None"))
        .build()
        .expect("variant_empty schema builds (placeholder forbids expressions)");

    let values = FieldValues::from_json(json!({
        "auth": { "mode": "none", "value": { "$expr": "{{ $secrets.leak }}" } }
    }))
    .expect("from_json");

    let report = schema
        .validate(&values)
        .expect_err("smuggled expression payload must be rejected at validate");
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "no-payload variant placeholder must fail closed on a smuggled \
         expression, got: {:?}",
        codes(&report)
    );
}

#[test]
fn no_payload_variant_string_marker_payload_is_rejected_at_validate() {
    // The other expression encoding: a `{{ … }}` string (not the `{"$expr":…}`
    // object) submitted as the variant payload. `FieldValue::from_json` turns
    // an expression-marker string into `FieldValue::Expression` too, so the
    // no-payload placeholder must reject this form as well.
    let schema = Schema::builder()
        .add(Field::mode(field_key!("auth")).variant_empty("none", "None"))
        .build()
        .expect("variant_empty schema builds");

    let values = FieldValues::from_json(json!({
        "auth": { "mode": "none", "value": "{{ $secrets.leak }}" }
    }))
    .expect("from_json");

    let report = schema
        .validate(&values)
        .expect_err("smuggled expression-marker string must be rejected");
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "no-payload variant placeholder must fail closed on a `{{ }}` string \
         payload too, got: {:?}",
        codes(&report)
    );
}

#[test]
fn nested_no_payload_variant_expr_payload_is_rejected_at_validate() {
    // Depth coverage that mirrors the lint's whole-tree traversal: a no-payload
    // mode variant nested inside an Object. The runtime gate must recurse into
    // the nested mode and still reject the smuggled expression at the hidden
    // placeholder.
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("outer"))
                .add(Field::mode(field_key!("auth")).variant_empty("none", "None")),
        )
        .build()
        .expect("nested variant_empty schema builds");

    let values = FieldValues::from_json(json!({
        "outer": { "auth": { "mode": "none", "value": { "$expr": "{{ $secrets.leak }}" } } }
    }))
    .expect("from_json");

    let report = schema
        .validate(&values)
        .expect_err("nested smuggled expression payload must be rejected");
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "nested no-payload variant must also fail closed on a smuggled \
         expression, got: {:?}",
        codes(&report)
    );
}

#[test]
fn hidden_no_payload_variant_literal_payload_still_builds_and_resolves() {
    // No false-positive at runtime: a no-payload variant supplied with NO
    // `value` (the legitimate shape) validates and resolves cleanly. The guard
    // must reject only smuggled expressions, not the normal no-payload form.
    let schema = Schema::builder()
        .add(
            Field::mode(field_key!("auth"))
                .visible(VisibilityMode::Never)
                .variant_empty("none", "None"),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({ "auth": { "mode": "none" } })).expect("from_json");

    schema
        .validate(&values)
        .expect("a no-payload variant with no value is the legitimate shape");
}
