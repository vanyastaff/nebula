//! Schema-time validation tests for `ValidSchema::validate`.
//!
//! Covers: empty schema, required-field checks, type-mismatch,
//! expression-forbidden, expression-deferred, predicate rules via `RuleContext`.

use nebula_schema::*;
use serde_json::json;

fn fk(s: &str) -> FieldKey {
    FieldKey::new(s).unwrap()
}

// ── Empty schema ─────────────────────────────────────────────────────────────

#[test]
fn empty_schema_empty_values_ok() {
    let schema = Schema::builder().build().unwrap();
    let values = FieldValues::from_json(json!({})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn empty_schema_with_extra_values_ok() {
    // Schema doesn't care about extra keys.
    let schema = Schema::builder().build().unwrap();
    let values = FieldValues::from_json(json!({"x": 1})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

// ── Required-field checks ────────────────────────────────────────────────────

#[test]
fn required_field_missing_emits_required() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "required"),
        "expected required error"
    );
}

#[test]
fn required_field_null_value_emits_required() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": null})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_field_present_ok() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "hello"})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn optional_field_absent_ok() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

// ── Expression handling ──────────────────────────────────────────────────────

#[test]
fn expression_in_allowed_field_deferred_not_error() {
    // ExpressionMode::Allowed (default for string) — expression skips value rules.
    let schema = Schema::builder()
        .add(Field::string(fk("x")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "{{ $ctx.value }}"})).unwrap();
    // Required field has expression value — must NOT produce "required" error.
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn expression_in_forbidden_mode_field_emits_error() {
    // BooleanField defaults to ExpressionMode::Forbidden.
    let schema = Schema::builder()
        .add(Field::boolean(fk("flag")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"flag": "{{ $x }}"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "expected expression.forbidden, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn expression_in_explicit_forbidden_string_emits_error() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")).no_expression())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "{{ $y }}"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "expression.forbidden"));
}

#[test]
fn mode_variant_empty_rejects_expression_in_placeholder() {
    let schema = Schema::builder()
        .add(
            Field::mode(fk("m"))
                .variant_empty("none", "None")
                .default_variant("none"),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({
        "m": { "mode": "none", "value": "{{ $x }}" }
    }))
    .unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "expected expression.forbidden, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn mode_field_accepts_object_wire_envelope() {
    let schema = Schema::builder()
        .add(Field::mode(fk("auth")).variant("token", "Token", Field::secret(fk("token"))))
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({
        "auth": { "mode": "token", "value": "shh" }
    }))
    .unwrap();

    assert!(schema.validate(&values).is_ok());
}

#[test]
fn mode_field_uses_default_variant_for_object_wire_envelope_without_mode() {
    let schema = Schema::builder()
        .add(
            Field::mode(fk("auth"))
                .variant("token", "Token", Field::secret(fk("token")))
                .default_variant("token"),
        )
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({
        "auth": { "value": "shh" }
    }))
    .unwrap();

    assert!(schema.validate(&values).is_ok());
}

#[test]
fn object_field_can_use_mode_and_value_keys_without_mode_coercion() {
    let schema = Schema::builder()
        .add(
            Field::object(fk("config"))
                .add(Field::string(fk("mode")).required())
                .add(Field::string(fk("value")).required()),
        )
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({
        "config": { "mode": "manual", "value": "literal" }
    }))
    .unwrap();

    assert!(schema.validate(&values).is_ok());
}

#[test]
fn computed_field_literal_emits_expression_required() {
    let schema = Schema::builder()
        .add(Field::computed(fk("derived")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"derived": "plain"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report
            .errors()
            .any(|e| e.code == "expression.required" || e.code == "expression.type_mismatch"),
        "expected expression.required/type_mismatch, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn computed_field_cannot_disable_expression_requirement() {
    let schema = Schema::builder()
        .add(Field::computed(fk("derived")).no_expression())
        .build()
        .unwrap();

    assert_eq!(
        schema.fields()[0].expression(),
        &ExpressionMode::Required,
        "computed field must remain expression-required"
    );

    let values = FieldValues::from_json(json!({"derived": "plain"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "expression.required"),
        "expected expression.required, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn notice_field_cannot_enable_expression_mode() {
    let schema = Schema::builder()
        .add(Field::notice(fk("banner")).expression_mode(ExpressionMode::Allowed))
        .build()
        .unwrap();

    assert_eq!(
        schema.fields()[0].expression(),
        &ExpressionMode::Forbidden,
        "notice field must keep expression-forbidden invariant"
    );

    let values = FieldValues::from_json(json!({"banner": "{{ $x }}"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "expected expression.forbidden, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn computed_and_notice_expression_modes_are_normalized_after_deserialize() {
    let schema: Schema = serde_json::from_value(json!({
        "fields": [
            {
                "type": "computed",
                "key": "calc",
                "expression_source": "1 + 1",
                "returns": "number",
                "expression": "forbidden"
            },
            {
                "type": "notice",
                "key": "banner",
                "severity": "info",
                "expression": "required"
            }
        ]
    }))
    .expect("schema JSON should deserialize");

    assert_eq!(schema.fields()[0].expression(), &ExpressionMode::Required);
    assert_eq!(schema.fields()[1].expression(), &ExpressionMode::Forbidden);
}

// ── Type mismatch ────────────────────────────────────────────────────────────

#[test]
fn string_field_number_value_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": 42})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "type_mismatch"));
}

#[test]
fn number_field_string_value_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(Field::number(fk("n")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"n": "not a number"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "type_mismatch"));
}

#[test]
fn boolean_field_string_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(Field::boolean(fk("ok")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"ok": "yes"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "type_mismatch"));
}

// ── Rule evaluation (via RuleContext) ─────────────────────────────────────────

#[test]
fn length_max_rule_violated() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")).max_length(5))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": "toolongvalue"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    // Rule-failure codes are surfaced verbatim from nebula-validator (no
    // schema-side remap); a `max_length` rule reports the native
    // `max_length` code. See ADR-0052 (P2 amendment).
    assert!(
        report.errors().any(|e| e.code == "max_length"),
        "expected max_length error, codes: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn length_max_rule_satisfied() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")).max_length(10))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": "alice"})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

// ── ValidValues accessors ─────────────────────────────────────────────────────

#[test]
fn valid_values_exposes_warnings_empty_by_default() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "hi"})).unwrap();
    let valid = schema.validate(&values).unwrap();
    assert!(valid.warnings().is_empty());
}

#[test]
fn valid_values_raw_values_matches_input() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "hi"})).unwrap();
    let valid = schema.validate(&values).unwrap();
    let fk_x = FieldKey::new("x").unwrap();
    assert_eq!(
        valid.raw().get(&fk_x),
        Some(&FieldValue::Literal(json!("hi")))
    );
}

// ── Nested object validation ───────────────────────────────────────────────────

#[test]
fn nested_required_field_missing_emits_required() {
    let schema = Schema::builder()
        .add(Field::object(fk("user")).add(Field::string(fk("email")).required()))
        .build()
        .unwrap();
    // Provide user object but without email.
    let values = FieldValues::from_json(json!({"user": {}})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn nested_required_field_present_ok() {
    let schema = Schema::builder()
        .add(Field::object(fk("user")).add(Field::string(fk("email")).required()))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"user": {"email": "a@b.com"}})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

// ── Select multiple/scalar mismatch (exhaustive check) ──────────────────────

#[test]
fn multi_select_with_scalar_value_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(
            Field::select(fk("tags"))
                .multiple()
                .option("a", "A")
                .option("b", "B"),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"tags": "a"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "type_mismatch"),
        "expected type_mismatch for scalar on multi select, got: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}

#[test]
fn single_select_with_array_value_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(
            Field::select(fk("choice"))
                .option("a", "A")
                .option("b", "B"),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"choice": ["a", "b"]})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "type_mismatch"),
        "expected type_mismatch for array on single select, got: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}

// ── Required + empty values ─────────────────────────────────────────────────

#[test]
fn required_string_empty_emits_required() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": ""})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_secret_empty_emits_required() {
    let schema = Schema::builder()
        .add(Field::secret(fk("token")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"token": ""})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_list_empty_emits_required() {
    let schema = Schema::builder()
        .add(
            Field::list(fk("items"))
                .item(Field::string(fk("it")))
                .required(),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"items": []})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn list_unique_duplicate_emits_items_unique() {
    let schema = Schema::builder()
        .add(
            Field::list(fk("items"))
                .item(Field::string(fk("it")))
                .unique(),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"items": ["a", "b", "a"]})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "items.unique"),
        "expected items.unique, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn list_unique_distinct_values_ok() {
    let schema = Schema::builder()
        .add(
            Field::list(fk("items"))
                .item(Field::string(fk("it")))
                .unique(),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"items": ["a", "b", "c"]})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn required_multi_file_empty_array_emits_required() {
    let schema = Schema::builder()
        .add(Field::file(fk("uploads")).multiple().required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"uploads": []})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_code_empty_emits_required() {
    let schema = Schema::builder()
        .add(Field::code(fk("script")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"script": ""})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_single_file_empty_string_emits_required() {
    let schema = Schema::builder()
        .add(Field::file(fk("upload")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"upload": ""})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_multi_select_empty_array_emits_required() {
    let schema = Schema::builder()
        .add(
            Field::select(fk("tags"))
                .multiple()
                .option("a", "A")
                .required(),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"tags": []})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn multi_select_with_expression_item_forbidden_emits_expression_forbidden() {
    // Select defaults to ExpressionMode::Forbidden. A multi-select value
    // whose list contains an expression placeholder must be rejected at
    // validate-time — otherwise `resolve` would silently evaluate it.
    let schema = Schema::builder()
        .add(
            Field::select(fk("tags"))
                .multiple()
                .option("a", "A")
                .option("b", "B"),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({
        "tags": ["a", {"$expr": "{{ $dynamic }}"}]
    }))
    .unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "expected expression.forbidden, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn required_string_single_char_ok() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": "a"})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn multi_select_with_array_of_valid_options_ok() {
    let schema = Schema::builder()
        .add(
            Field::select(fk("tags"))
                .multiple()
                .option("a", "A")
                .option("b", "B"),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"tags": ["a", "b"]})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn root_rule_error_path_snapshot() {
    let schema = Schema::builder()
        .add(Field::object(fk("config")).add(Field::string(fk("tier"))))
        .add(Field::list(fk("items")).item(Field::object(fk("row")).add(Field::string(fk("name")))))
        .root_rule(Rule::predicate(
            Predicate::eq("/config/tier", json!("pro")).unwrap(),
        ))
        .root_rule(Rule::predicate(
            Predicate::eq("/items/0/name", json!("first")).unwrap(),
        ))
        .root_rule(Rule::predicate(
            Predicate::eq("/items/0/name", json!("second")).unwrap(),
        ))
        .build()
        .unwrap();

    let values =
        FieldValues::from_json(json!({"config": {"tier": "free"}, "items": [{"name": "wrong"}]}))
            .unwrap();
    let report = schema.validate(&values).unwrap_err();
    let issues: Vec<_> = report
        .errors()
        .map(|error| {
            json!({
                "code": error.code,
                "message": error.message,
                "path": error.path.to_string(),
            })
        })
        .collect();

    insta::assert_json_snapshot!(issues, @r###"
    [
      {
        "code": "eq_failed",
        "message": "predicate failed",
        "path": "config.tier"
      },
      {
        "code": "eq_failed",
        "message": "predicate failed",
        "path": "items[0].name"
      },
      {
        "code": "eq_failed",
        "message": "predicate failed",
        "path": "items[0].name"
      }
    ]
    "###);
}

#[test]
fn nested_required_when_is_enforced_not_fail_open() {
    use nebula_validator::Predicate;
    use nebula_validator::foundation::FieldPath as ValidatorPath;

    // `secret_token` is required WHEN /auth/mode == "oauth". The old
    // Rule::evaluate flat-key path silently returned false for the nested
    // path → field was NOT enforced (fail-open). It must now be enforced.
    let schema = Schema::builder()
        .add(Field::object(fk("auth")).add(Field::string(fk("mode"))))
        .add(
            Field::string(fk("secret_token")).required_when(Rule::Predicate(Predicate::Eq(
                ValidatorPath::parse("/auth/mode").unwrap(),
                json!("oauth"),
            ))),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({
        "auth": { "mode": "oauth" }
        // secret_token absent
    }))
    .unwrap();

    let report = schema
        .validate(&values)
        .expect_err("must reject: required field absent");
    assert!(
        report.errors().any(|e| e.code == "required"),
        "nested required_when must be enforced, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn middle_skipped_field_does_not_shift_plan_to_field_mapping() {
    // f_first (min_length 5), f_mid (Never-visible), f_last (min_length 5).
    // Both f_first and f_last get too-short values. If plan<->field shifts by
    // the skipped middle, the error paths land on the wrong fields.
    let schema = Schema::builder()
        .add(Field::string(fk("f_first")).min_length(5))
        .add(Field::string(fk("f_mid")).visible(VisibilityMode::Never))
        .add(Field::string(fk("f_last")).min_length(5))
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({"f_first": "ab", "f_last": "cd"})).unwrap();
    let report = schema
        .validate(&values)
        .expect_err("both short fields must fail");

    let codes_paths: Vec<(String, String)> = report
        .errors()
        .map(|e| (e.code.to_string(), e.path.to_string()))
        .collect();
    assert!(
        codes_paths.iter().any(|(_, p)| p.contains("f_first")),
        "f_first must be the error path, not shifted: {codes_paths:?}"
    );
    assert!(
        codes_paths.iter().any(|(_, p)| p.contains("f_last")),
        "f_last must be the error path, not shifted: {codes_paths:?}"
    );
}

#[test]
fn hidden_present_required_empty_emits_single_required() {
    // Seam anchor: a field that is hidden (VisibilityMode::Never) AND
    // required, supplied with a PRESENT-but-empty value. The policy engine
    // self-reports `required` only for Presence::Active, so the schema gate
    // emits the single `required` for this Presence != Active corner. Pins
    // exactly-one `required` on the field path.
    let schema = Schema::builder()
        .add(
            Field::string(fk("secret_slot"))
                .visible(VisibilityMode::Never)
                .required(),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({"secret_slot": ""})).unwrap();
    let report = schema
        .validate(&values)
        .expect_err("hidden+present+required+empty must reject");

    let errors: Vec<_> = report.errors().collect();
    assert_eq!(
        errors.len(),
        1,
        "expected exactly one error, got: {:?}",
        errors
            .iter()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
    assert_eq!(errors[0].code, "required");
    assert_eq!(errors[0].path.to_string(), "secret_slot");
}
