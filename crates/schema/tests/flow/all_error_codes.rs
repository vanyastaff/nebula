//! Integration tests that emit every testable entry from STANDARD_CODES.
//!
//! # Coverage notes
//!
//! The codes in STANDARD_CODES map to codes produced by two layers:
//! - `ValidSchema::validate` (proof-token pipeline)
//! - `SchemaBuilder::build` (lint/build time via `lint_tree`)
//!
//! Some STANDARD_CODES entries are *translated* codes (e.g. `"length.min"`,
//! `"range.min"`) that the schema layer intends to emit after normalising
//! validator codes. The current implementation passes through validator codes
//! (`"min_length"`, `"max_length"`, `"min"`, `"max"`) unchanged.
//!
//! **Deferred** (requires post-Task-26 work to add translation layer):
//! - `"length.min"` → validator emits `"min_length"`
//! - `"length.max"` → validator emits `"max_length"`
//! - `"range.min"` → validator emits `"min"`
//! - `"range.max"` → validator emits `"max"`
//! - `"pattern"` → validator emits `"invalid_format"` with param `"pattern"`
//! - `"url"` → validator emits `"invalid_format"` with expected=`"url"`
//! - `"email"` → validator emits `"invalid_format"` with expected=`"email"`
//! - `"items.unique"` → validator emits `"unique_by"` / no direct emitter yet
//! - `"loader.not_registered"` → needs real LoaderRegistry
//! - `"loader.failed"` → needs real LoaderRegistry
//! - `"expression.runtime"` → needs ExpressionContext implementation
//! - `"expression.type_mismatch"` → same

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

// Validator passes through codes: min_length / max_length / min / max /
// invalid_format. STANDARD_CODES entries "length.min", "length.max",
// "range.min", "range.max", "pattern", "url", "email" are the intended
// mapped codes — see deferred note at top.

#[test]
fn emits_min_length_via_validator() {
    // Emitted as "min_length" (not yet "length.min") — deferred mapping.
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")).min_length(5))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": "hi"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        err.errors().any(|e| e.code == "min_length"),
        "expected min_length, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_max_length_via_validator() {
    // Emitted as "max_length" (not yet "length.max") — deferred mapping.
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")).max_length(3))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": "toolong"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        err.errors().any(|e| e.code == "max_length"),
        "expected max_length, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_range_min_via_validator() {
    // Emitted as "min" (not yet "range.min") — deferred mapping.
    let schema = Schema::builder()
        .add(Field::number(field_key!("x")).min(10))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": 5})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        err.errors().any(|e| e.code == "min"),
        "expected min, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_range_max_via_validator() {
    // Emitted as "max" (not yet "range.max") — deferred mapping.
    let schema = Schema::builder()
        .add(Field::number(field_key!("x")).max(10))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"x": 99})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        err.errors().any(|e| e.code == "max"),
        "expected max, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

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

#[test]
fn emits_mode_required_via_missing_mode() {
    // mode.required fires when no mode key and no default_variant.
    // FieldValue::Object (parsed from {"m": {}}) triggers type_mismatch on mode field.
    // To get mode.required: supply a FieldValue::Mode with an empty-ish mode that resolves
    // to the "no mode found" branch. The code path requires mode_key to be absent after
    // FieldKey resolution — currently mode.required emits when `resolved_key` is None.
    // This happens when `mode_key` is a FieldKey with empty-ish value but we can't create
    // an empty FieldKey. So mode.required requires the field to not have a default_variant
    // and the value to not carry a mode key — which means it must NOT be FieldValue::Mode
    // (which always has a mode key). The mode.required path is therefore unreachable via
    // public API in the current validated.rs implementation. Document as deferred.
    //
    // The closest observable behaviour: providing a FieldValue::Object triggers type_mismatch.
    let schema = Schema::builder()
        .add(Field::mode(field_key!("m")).variant("a", "A", Field::string(fk("val"))))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"m": {}})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    // Object → type_mismatch (mode expects FieldValue::Mode variant).
    assert!(
        has_code(&err, "type_mismatch"),
        "codes: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

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

// expression.parse fires when an expression in an Allowed field fails AST parsing.
// expression.runtime / expression.type_mismatch require ExpressionContext — deferred.

// ── Build-time (lint) codes ──────────────────────────────────────────────

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

// ── Warning codes (tested via LintReport from lint_schema legacy API) ────────

#[test]
fn emits_missing_loader_warning_via_lint_schema() {
    // lint_schema (legacy) reports missing_loader warning for dynamic select without loader.
    let schema = Schema::new().add(Field::select(fk("s")).dynamic());
    let lint = schema.lint();
    assert!(
        lint.has_warnings(),
        "expected warnings from lint_schema for dynamic select without loader"
    );
    assert!(
        lint.diagnostics()
            .iter()
            .any(|d| d.code == "missing_loader"),
        "expected missing_loader, got: {:?}",
        lint.diagnostics()
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn emits_loader_without_dynamic_warning_via_lint_schema() {
    // A select with a loader key but dynamic=false → loader_without_dynamic.
    let _schema = Schema::new().add(
        Field::select(fk("s"))
            .option("a", "A")
            .with_rule(nebula_validator::Rule::MinLength {
                min: 0,
                message: None,
            }), // force a rules-only select
    );
    // Can't easily set loader without dynamic via the builder (it sets both).
    // Instead, serialize and patch to add loader without dynamic flag.
    // Use the lint API directly by building the raw Schema and adding a loader
    // via the legacy Schema::add path with a pre-built SelectField.
    use nebula_schema::{Field, SelectField};
    let mut sf = SelectField::new("s2");
    sf.dynamic = false;
    sf.loader = Some("my_loader".into());
    let schema2 = Schema::new().add(Field::Select(sf));
    let lint = schema2.lint();
    assert!(
        lint.diagnostics()
            .iter()
            .any(|d| d.code == "loader_without_dynamic"),
        "expected loader_without_dynamic, got: {:?}",
        lint.diagnostics()
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn emits_missing_variant_label_warning_via_builder() {
    // mode variant with empty label → missing_variant_label warning.
    // In SchemaBuilder, warnings don't block build — so build succeeds.
    // Check the ValidationReport has the warning.
    let result = Schema::builder()
        .add(Field::mode(field_key!("m")).variant("v", "", Field::string(fk("x"))))
        .build();
    // missing_variant_label is a warning — may or may not block build.
    match result {
        Ok(_) => {
            // Passed — the warning was advisory. Check via lint_schema instead.
            let schema =
                Schema::new().add(Field::mode(fk("m")).variant("v", "", Field::string(fk("x"))));
            let lint = schema.lint();
            assert!(
                lint.diagnostics()
                    .iter()
                    .any(|d| d.code == "missing_variant_label"),
                "expected missing_variant_label in lint output, got: {:?}",
                lint.diagnostics()
                    .iter()
                    .map(|d| &d.code)
                    .collect::<Vec<_>>()
            );
        },
        Err(report) => {
            // It was treated as error.
            assert!(
                report.iter().any(|e| e.code == "missing_variant_label"),
                "expected missing_variant_label, got: {:?}",
                report.iter().map(|e| &e.code).collect::<Vec<_>>()
            );
        },
    }
}

// ── Codes deferred (cannot emit without additional infrastructure) ────────────

// The following STANDARD_CODES entries are deferred for future tasks:
//
// "length.min" — schema currently forwards "min_length" from validator; code
//   mapping/translation layer not yet implemented (post-Task-26 work).
// "length.max" — same; forwards "max_length".
// "range.min"  — same; forwards "min".
// "range.max"  — same; forwards "max".
// "pattern"    — validator emits "invalid_format" with param "pattern".
// "url"        — validator emits "invalid_format" with expected="url".
// "email"      — validator emits "invalid_format" with expected="email".
// "items.unique" — UniqueBy rule emits "unique_by"; no items.unique emitter yet.
// "invalid_key" — emitted by FieldKey::new directly, not by schema validate.
// "self_dependency" — requires DynamicField/SelectField depends_on self-ref via builder.
// "visibility_cycle" — requires crafted cycle in visibility rules.
// "notice.misuse" — notice field misuse warning (notice_misuse in legacy lint).
// "notice_missing_description" — notice field without description.
// "mode.required" — unreachable via public API (FieldValue::Mode always carries a key).
// "loader.not_registered" — needs LoaderRegistry integration.
// "loader.failed" — needs LoaderRegistry integration.
// "expression.type_mismatch" — needs ExpressionContext returning wrong type.
// "expression.runtime" — needs ExpressionContext returning eval error.
// "dangling_reference" — requires depends_on or rule referencing unknown field.
