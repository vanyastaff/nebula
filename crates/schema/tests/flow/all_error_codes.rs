//! Integration tests that emit every testable entry from `STANDARD_CODES`.
//!
//! # Coverage notes
//!
//! The codes in `STANDARD_CODES` map to codes produced by two layers:
//! - `ValidSchema::validate` (proof-token pipeline)
//! - `SchemaBuilder::build` (lint/build time via `lint_tree`)
//!
//! As of Phase 1 (Task 26), validator codes are translated in `run_rules` via
//! `translate_validator_code` before being stored in `ValidationReport`.
//! So `"min_length"` → `"length.min"`, `"max_length"` → `"length.max"`, etc.
//!
//! # Loader-family codes
//!
//! The following `STANDARD_CODES` entries are loader-registry-scoped. They
//! require an async runtime (`#[tokio::test]`) and a `LoaderRegistry`, so
//! they live in `crates/schema/tests/lint_and_loader.rs` rather than here:
//!
//! - `"loader.not_registered"` — `loader_registry_reports_missing_loader_registration`.
//! - `"loader.failed"` — covered via `loader.rs` unit tests
//!   (`loader_failure_wraps_as_loader_failed`).
//! - `"field.not_found"` — `load_select_options_unknown_key_emits_field_not_found`.
//! - `"field.type_mismatch"` — `load_select_options_wrong_field_type_emits_type_mismatch`,
//!   `load_dynamic_records_wrong_field_type_emits_type_mismatch`.
//! - `"loader.missing_config"` — `load_select_options_without_loader_emits_missing_config`.

use nebula_schema::{
    EvalFuture, ExpressionAst, ExpressionContext, Field, FieldKey, FieldValue, FieldValues, Schema,
    ValidationError, ValidationReport, field_key,
};
use serde_json::json;

fn fk(s: &str) -> FieldKey {
    FieldKey::new(s).unwrap()
}

fn has_code(r: &ValidationReport, code: &str) -> bool {
    r.errors().any(|e| e.code == code)
}

fn raw_schema(fields: impl IntoIterator<Item = Field>) -> Schema {
    let fields: Vec<Field> = fields.into_iter().collect();
    serde_json::from_value(json!({ "fields": fields })).expect("raw schema from field list")
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
fn emits_items_unique() {
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("xs"))
                .item(Field::string(fk("_item")))
                .unique(),
        )
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"xs": ["a", "b", "a"]})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "items.unique"),
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
fn emits_mode_required() {
    let schema = Schema::builder()
        .add(Field::mode(field_key!("m")).variant("a", "A", Field::string(fk("val"))))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({
        "m": {
            "value": "missing mode selector"
        }
    }))
    .unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "mode.required"),
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

#[test]
fn emits_expression_required() {
    let schema = Schema::builder()
        .add(Field::computed(field_key!("derived")))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"derived": "literal"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    assert!(
        has_code(&err, "expression.required"),
        "expected expression.required, got: {:?}",
        err.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_expression_parse() {
    let schema = Schema::builder()
        .add(Field::number(field_key!("n")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"n": {"$expr": "{{ 1 + }}"}})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        has_code(&report, "expression.parse"),
        "expected expression.parse, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

struct RuntimeFailCtx;

impl ExpressionContext for RuntimeFailCtx {
    fn evaluate<'a>(&'a self, _ast: &'a ExpressionAst) -> EvalFuture<'a> {
        Box::pin(async move {
            Err(ValidationError::builder("expression.runtime")
                .message("forced runtime failure")
                .build())
        })
    }
}

struct ConstCtx(serde_json::Value);

impl ExpressionContext for ConstCtx {
    fn evaluate<'a>(&'a self, _ast: &'a ExpressionAst) -> EvalFuture<'a> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

#[tokio::test]
async fn emits_expression_runtime() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": {"$expr": "{{ $bad }}"}})).unwrap();
    let validated = schema.validate(&values).unwrap();
    let report = validated.resolve(&RuntimeFailCtx).await.unwrap_err();
    assert!(
        has_code(&report, "expression.runtime"),
        "expected expression.runtime, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn emits_expression_type_mismatch() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": {"$expr": "{{ $n }}"}})).unwrap();
    let validated = schema.validate(&values).unwrap();
    let report = validated.resolve(&ConstCtx(json!(123))).await.unwrap_err();
    assert!(
        has_code(&report, "expression.type_mismatch"),
        "expected expression.type_mismatch, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
    assert!(
        !report.errors().any(|e| e.code == "type_mismatch"),
        "raw type_mismatch should have been remapped, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

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
fn emits_invalid_key_for_mode_variant_path_segment() {
    let report = Schema::builder()
        .add(Field::mode(field_key!("auth")).variant(
            "oauth-token",
            "OAuth",
            Field::string(fk("token")),
        ))
        .build()
        .unwrap_err();
    assert!(has_code(&report, "invalid_key"));
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
fn emits_schema_index_overflow() {
    let mut builder = Schema::builder();
    for i in 0..(usize::from(u16::MAX) + 2) {
        builder = builder.add(Field::string(fk(&format!("f{i}"))));
    }
    let report = builder.build().unwrap_err();
    assert!(has_code(&report, "schema.index_overflow"));
}

#[test]
fn emits_schema_depth_limit() {
    let mut leaf = Field::string(field_key!("leaf")).into_field();
    for i in 0..usize::from(u8::MAX) {
        leaf = Field::object(fk(&format!("n{i}"))).add(leaf).into_field();
    }

    let report = Schema::builder().add(leaf).build().unwrap_err();
    assert!(has_code(&report, "schema.depth_limit"));
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
    let field = DynamicField::new(field_key!("deps"))
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
    use nebula_validator::{Predicate, Rule, foundation::FieldPath};

    // IsTrue predicate on field `b` causes field `a` to reference `b`.
    let rule_a_references_b = Rule::predicate(Predicate::IsTrue(FieldPath::parse("b").unwrap()));
    let rule_b_references_a = Rule::predicate(Predicate::IsTrue(FieldPath::parse("a").unwrap()));

    let schema = raw_schema(vec![
        Field::string(fk("a"))
            .visible_when(rule_a_references_b)
            .into(),
        Field::string(fk("b"))
            .visible_when(rule_b_references_a)
            .into(),
    ]);

    let lint = schema.lint();
    assert!(
        lint.errors().any(|d| d.code == "visibility_cycle"),
        "expected visibility_cycle, got: {:?}",
        lint.errors().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_required_cycle() {
    // A's required predicate references B, B's required predicate references A.
    use nebula_validator::{Predicate, Rule, foundation::FieldPath};

    let rule_a_references_b = Rule::predicate(Predicate::IsTrue(FieldPath::parse("b").unwrap()));
    let rule_b_references_a = Rule::predicate(Predicate::IsTrue(FieldPath::parse("a").unwrap()));

    let schema = raw_schema(vec![
        Field::string(fk("a"))
            .required_when(rule_a_references_b)
            .into(),
        Field::string(fk("b"))
            .required_when(rule_b_references_a)
            .into(),
    ]);

    let lint = schema.lint();
    assert!(
        lint.errors().any(|d| d.code == "required_cycle"),
        "expected required_cycle, got: {:?}",
        lint.errors().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_dangling_reference() {
    // A rule referencing an unknown field key → dangling_reference.
    use nebula_validator::{Predicate, Rule, foundation::FieldPath};

    let rule_unknown = Rule::predicate(Predicate::IsTrue(
        FieldPath::parse("nonexistent_field").unwrap(),
    ));
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
    let schema = raw_schema(vec![Field::select(fk("s")).dynamic().into()]);
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
    let mut sf = SelectField::new(field_key!("s2"));
    sf.dynamic = false;
    sf.loader = Some("my_loader".into());
    let schema = raw_schema(vec![Field::Select(sf)]);
    let lint = schema.lint();
    assert!(
        lint.warnings().any(|d| d.code == "loader_without_dynamic"),
        "expected loader_without_dynamic, got: {:?}",
        lint.warnings().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn emits_duplicate_dependency_warning() {
    let dep = nebula_schema::FieldPath::parse("team_id").unwrap();
    let schema = raw_schema(vec![
        Field::select(fk("workspace"))
            .dynamic()
            .loader("workspace_loader")
            .depends_on(dep.clone())
            .depends_on(dep)
            .into(),
    ]);
    let lint = schema.lint();
    assert!(
        lint.warnings().any(|d| d.code == "duplicate_dependency"),
        "expected duplicate_dependency, got: {:?}",
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
            let lint = raw_schema(vec![
                Field::mode(fk("m"))
                    .variant("v", "", Field::string(fk("x")))
                    .into(),
            ])
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

    let mut nf = NoticeField::new(field_key!("n"));
    nf.required = RequiredMode::Always;
    let schema = raw_schema(vec![Field::Notice(nf)]);
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

    let nf = NoticeField::new(field_key!("info"));
    let schema = raw_schema(vec![Field::Notice(nf)]);
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

    let schema = raw_schema(vec![
        Field::number(fk("n"))
            .with_rule(Rule::pattern("^[0-9]+$"))
            .into(),
    ]);
    let lint = schema.lint();
    assert!(
        lint.warnings().any(|d| d.code == "rule.incompatible"),
        "expected rule.incompatible, got: {:?}",
        lint.warnings().map(|d| &d.code).collect::<Vec<_>>()
    );
}
