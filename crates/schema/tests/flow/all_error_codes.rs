//! Integration tests that emit every testable entry from STANDARD_CODES.
//!
//! # Coverage notes
//!
//! The codes in STANDARD_CODES map to codes produced by two layers:
//! - `ValidSchema::validate` (proof-token pipeline)
//! - `SchemaBuilder::build` (lint/build time via `lint_tree`)
//!
//! As of Phase 1 (Task 26), validator codes are translated in `run_rules` via
//! `translate_validator_code` before being stored in `ValidationReport`.
//! So `"min_length"` → `"length.min"`, `"max_length"` → `"length.max"`, etc.
//!
//! # Deferred codes (Phase 4)
//!
//! The following STANDARD_CODES entries cannot be emitted without additional
//! infrastructure that is out of scope for Phase 1:
//!
//! - `"expression.parse"` — Phase 1's `Expression::parse()` is a no-op stub that always succeeds; a
//!   real parse failure requires nebula-expression AST (Phase 4).
//!
//! - `"expression.runtime"` — requires a real `ExpressionContext` returning an eval error; Phase 4
//!   scope.
//!
//! - `"expression.type_mismatch"` — requires `ExpressionContext` resolving to a wrong type; Phase 4
//!   scope.
//!
//! - `"mode.required"` — unreachable via the public API: `FieldValue::Mode` always carries a
//!   non-empty `FieldKey` (validated at construction time), so the `mode_key.is_empty()` branch in
//!   `validated.rs` can never fire.
//!
//! - `"items.unique"` — `Rule::UniqueBy` is classified as a deferred rule in nebula-validator and
//!   is silently skipped at `ExecutionMode::StaticOnly` (Phase 1). A full runtime evaluation path
//!   is needed (Phase 4).
//!
//! - `"loader.not_registered"` — requires real `LoaderRegistry` wiring; Phase 2/4 scope.
//!
//! - `"loader.failed"` — same as above.

use nebula_schema::{
    Field, FieldKey, FieldValue, FieldValues, Schema, ValidationReport, field_key,
};
use serde_json::json;

fn fk(s: &str) -> FieldKey {
    FieldKey::new(s).unwrap()
}

fn has_code(r: &ValidationReport, code: &str) -> bool {
    r.errors().any(|e| e.code == code)
}

// ── Value-validation codes ──────────────────────────────────────────────────

#[test]
fn emits_required() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")).required())
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "required"),
        "codes: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_type_mismatch_string() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": 42})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "type_mismatch"),
        "codes: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_type_mismatch_number() {
    let schema = Schema::builder()
        .add(Field::number(field_key!("x")))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": "not_a_number"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(has_code(&err, "type_mismatch"));
}

#[test]
fn emits_type_mismatch_boolean() {
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("x")))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": "yes"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(has_code(&err, "type_mismatch"));
}

// ── String length codes (translated: min_length → length.min, max_length → length.max) ──

#[test]
fn emits_length_min() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")).min_length(5))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": "hi"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "length.min"),
        "expected length.min, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_length_max() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")).max_length(3))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": "abcdef"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "length.max"),
        "expected length.max, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── Numeric range codes (translated: min → range.min, max → range.max) ──────

#[test]
fn emits_range_min() {
    let schema = Schema::builder()
        .add(Field::number(field_key!("x")).min(10))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": 3})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "range.min"),
        "expected range.min, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_range_max() {
    let schema = Schema::builder()
        .add(Field::number(field_key!("x")).max(10))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": 99})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "range.max"),
        "expected range.max, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── Pattern / URL / email codes (translated from invalid_format) ─────────────

#[test]
fn emits_pattern() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")).pattern("^[a-z]+$"))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": "HI"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "pattern"),
        "expected pattern, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_url() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")).url())
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": "not-a-url"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "url"),
        "expected url, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_email() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")).email())
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": "not-an-email"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "email"),
        "expected email, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── List item count codes ────────────────────────────────────────────────────

#[test]
fn emits_items_min() {
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("xs"))
                .item(Field::string(fk("_item")))
                .min_items(3),
        )
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"xs": ["a"]})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "items.min"),
        "codes: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_items_max() {
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("xs"))
                .item(Field::string(fk("_item")))
                .max_items(2),
        )
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"xs": ["a", "b", "c"]})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "items.max"),
        "codes: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_option_invalid() {
    let schema = Schema::builder()
        .add(
            Field::select(field_key!("color"))
                .option("red", "Red")
                .option("blue", "Blue"),
        )
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"color": "green"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "option.invalid"),
        "codes: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── Mode codes ──────────────────────────────────────────────────────────────

#[test]
fn emits_mode_invalid() {
    let schema = Schema::builder()
        .add(Field::mode(field_key!("m")).variant("a", "A", Field::string(fk("val"))))
        .build()
        .unwrap();
    let mut vs = FieldValues::new();
    // Supply an unknown variant key.
    vs.set(
        fk("m"),
        FieldValue::Mode {
            mode: fk("nonexistent"),
            value: None,
        },
    );
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "mode.invalid"),
        "codes: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// mode.required is unreachable via the public API — see deferred note at top of file.

// ── Expression codes ──────────────────────────────────────────────────────

#[test]
fn emits_expression_forbidden() {
    // BooleanField has ExpressionMode::Forbidden by default.
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("flag")))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"flag": "{{ $x }}"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(has_code(&err, "expression.forbidden"));
}

// expression.parse / expression.runtime / expression.type_mismatch require Phase 4.

// ── Build-time (lint) codes ──────────────────────────────────────────────

#[test]
fn emits_invalid_key() {
    // invalid_key is emitted by FieldKey::new when the key is malformed.
    let err = FieldKey::new("has-dash").unwrap_err();
    assert_eq!(err.code, "invalid_key");

    let err2 = FieldKey::new("").unwrap_err();
    assert_eq!(err2.code, "invalid_key");
}

#[test]
fn emits_duplicate_key() {
    let report = Schema::builder()
        .add(Field::string(fk("x")))
        .add(Field::number(fk("x")))
        .build()
        .unwrap_err();
    assert!(
        has_code(&report, "duplicate_key"),
        "codes: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_missing_item_schema() {
    // A List field with no item schema.
    let report = Schema::builder()
        .add(Field::list(field_key!("xs")))
        .build()
        .unwrap_err();
    assert!(
        has_code(&report, "missing_item_schema"),
        "codes: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_invalid_default_variant() {
    let report = Schema::builder()
        .add(Field::mode(field_key!("m")).default_variant("nonexistent"))
        .build()
        .unwrap_err();
    assert!(has_code(&report, "invalid_default_variant"));
}

#[test]
fn emits_duplicate_variant() {
    let report = Schema::builder()
        .add(
            Field::mode(field_key!("m"))
                .variant("v", "V one", Field::string(fk("x")))
                .variant("v", "V two", Field::string(fk("y"))),
        )
        .build()
        .unwrap_err();
    assert!(has_code(&report, "duplicate_variant"));
}

#[test]
fn emits_rule_contradictory() {
    // min_length > max_length → rule.contradictory error.
    let report = Schema::builder()
        .add(Field::string(field_key!("x")).min_length(10).max_length(5))
        .build()
        .unwrap_err();
    assert!(
        has_code(&report, "rule.contradictory"),
        "all codes: {:?}",
        report.iter().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_self_dependency() {
    // A DynamicField whose depends_on references its own key → self_dependency.
    use nebula_schema::{DynamicField, FieldPath};

    let path = FieldPath::parse("deps").unwrap();
    let field = DynamicField::new("deps")
        .loader("my_loader")
        .depends_on(path)
        .into_field();

    let report = Schema::builder().add(field).build().unwrap_err();
    assert!(
        has_code(&report, "self_dependency"),
        "codes: {:?}",
        report.iter().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_visibility_cycle() {
    // A's visibility references B, B's visibility references A → cycle.
    use nebula_validator::Rule;

    // Rule::IsTrue { field: "b" } causes field "a" to reference "b".
    // Rule::IsTrue { field: "a" } causes field "b" to reference "a".
    let rule_a_references_b = Rule::IsTrue {
        field: "b".to_owned(),
    };
    let rule_b_references_a = Rule::IsTrue {
        field: "a".to_owned(),
    };

    let schema = Schema::new()
        .add(Field::string(fk("a")).visible_when(rule_a_references_b))
        .add(Field::string(fk("b")).visible_when(rule_b_references_a));

    let lint = schema.lint();
    assert!(
        lint.errors().any(|d| d.code == "visibility_cycle"),
        "expected visibility_cycle, got: {:?}",
        lint.errors().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_dangling_reference() {
    // A rule referencing an unknown field key → dangling_reference.
    use nebula_validator::Rule;

    let rule_unknown = Rule::IsTrue {
        field: "nonexistent_field".to_owned(),
    };
    let report = Schema::builder()
        .add(Field::string(field_key!("x")).visible_when(rule_unknown))
        .build()
        .unwrap_err();
    assert!(
        has_code(&report, "dangling_reference"),
        "codes: {:?}",
        report.iter().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── Warning codes (tested via ValidationReport from Schema::lint()) ────────

#[test]
fn emits_missing_loader_warning() {
    let schema = Schema::new().add(Field::select(fk("s")).dynamic());
    let lint = schema.lint();
    assert!(
        lint.has_warnings(),
        "expected warnings for dynamic select without loader"
    );
    assert!(
        lint.warnings().any(|d| d.code == "missing_loader"),
        "expected missing_loader, got: {:?}",
        lint.warnings().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_loader_without_dynamic_warning() {
    // A select with a loader key but dynamic=false → loader_without_dynamic.
    use nebula_schema::{Field, SelectField};
    let mut sf = SelectField::new("s2");
    sf.dynamic = false;
    sf.loader = Some("my_loader".into());
    let schema = Schema::new().add(Field::Select(sf));
    let lint = schema.lint();
    assert!(
        lint.warnings().any(|d| d.code == "loader_without_dynamic"),
        "expected loader_without_dynamic, got: {:?}",
        lint.warnings().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_missing_variant_label_warning() {
    // mode variant with empty label → missing_variant_label warning.
    let result = Schema::builder()
        .add(Field::mode(field_key!("m")).variant("v", "", Field::string(fk("x"))))
        .build();
    match result {
        Ok(schema) => {
            // Warning is advisory — build succeeded. Verify via lint().
            let lint = Schema::new()
                .add(Field::mode(fk("m")).variant("v", "", Field::string(fk("x"))))
                .lint();
            assert!(
                lint.warnings().any(|d| d.code == "missing_variant_label"),
                "expected missing_variant_label in lint output, got: {:?}",
                lint.warnings().map(|d| &d.code).collect::<Vec<_>>()
            );
            let _ = schema;
        },
        Err(report) => {
            assert!(
                report.iter().any(|e| e.code == "missing_variant_label"),
                "expected missing_variant_label, got: {:?}",
                report.iter().map(|e| &e.code).collect::<Vec<_>>()
            );
        },
    }
}

#[test]
fn emits_notice_misuse() {
    // NoticeField with required=Always → notice.misuse warning via lint_tree/SchemaBuilder.
    use nebula_schema::{Field, NoticeField, RequiredMode};

    let mut nf = NoticeField::new("n");
    nf.required = RequiredMode::Always;
    let schema = Schema::new().add(Field::Notice(nf));
    let lint = schema.lint();
    assert!(
        lint.warnings().any(|d| d.code == "notice.misuse"),
        "expected notice.misuse, got: {:?}",
        lint.warnings().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_notice_missing_description() {
    // NoticeField without description → notice_missing_description warning.
    use nebula_schema::{Field, NoticeField};

    let nf = NoticeField::new("info");
    let schema = Schema::new().add(Field::Notice(nf));
    let lint = schema.lint();
    assert!(
        lint.warnings()
            .any(|d| d.code == "notice_missing_description"),
        "expected notice_missing_description, got: {:?}",
        lint.warnings().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_rule_incompatible_warning() {
    use nebula_validator::Rule;

    let schema = Schema::new().add(Field::number(fk("n")).with_rule(Rule::Pattern {
        pattern: "^[0-9]+$".to_owned(),
        message: None,
    }));
    let lint = schema.lint();
    assert!(
        lint.warnings().any(|d| d.code == "rule.incompatible"),
        "expected rule.incompatible, got: {:?}",
        lint.warnings().map(|d| &d.code).collect::<Vec<_>>()
    );
}
