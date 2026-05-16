//! ADR-0052 seam: validator is the sole `required` emitter, and a hidden
//! field that nonetheless carries a present value is STILL structurally
//! validated (a smuggled expression in a no-payload mode-variant placeholder
//! must not escape to resolve). The carve-out is moved, not deleted.
use nebula_schema::mode::VisibilityMode;
use nebula_schema::{Field, FieldValues, Schema, field_key};
use serde_json::json;

#[test]
fn hidden_mode_present_expr_payload_is_rejected_not_skipped() {
    // A Mode field, hidden (Never). Variant `flag` is no-payload. An attacker
    // submits an expression payload to the hidden mode. Pre-fold the runner
    // reached validate_field via the raw.is_some() branch and rejected the
    // expression. Post-fold (validator sole emitter) the hidden+present field
    // MUST still be structurally validated → expression.forbidden.
    let schema = Schema::builder()
        .add(
            Field::mode(field_key!("auth"))
                .visible(VisibilityMode::Never)
                .variant_empty("flag", "Flag"),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({
        "auth": { "mode": "flag", "value": { "$expr": "{{ $secrets.leak }}" } }
    }))
    .unwrap();

    let report = schema.validate(&values).expect_err("must reject");
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "hidden+present mode payload must still be structurally validated, got: {:?}",
        report
            .errors()
            .map(|e| (e.code.to_string(), e.path.to_string()))
            .collect::<Vec<_>>()
    );
}
