//! Integration tests for the field-alias subsystem.
//!
//! Covers:
//! - Builder API (read_alias / write_alias / read_aliases methods)
//! - Ingest canonicalization (alias-keyed → canonical-keyed before validation)
//! - Security: alias-keyed secret is canonicalized, not leaked under the alias key
//! - Projection via `ValidSchema::project` and `ValidValues::to_wire_json`
//! - Lint: all alias.* error codes emitted at `SchemaBuilder::build` time
//! - No-alias path: fields without aliases produce no extra wire keys

use nebula_schema::{Field, FieldAliases, FieldKey, FieldValue, FieldValues, Schema};
use serde_json::json;

fn fk(s: &str) -> FieldKey {
    FieldKey::new(s).unwrap()
}

fn has_error_code(r: &nebula_schema::ValidationReport, code: &str) -> bool {
    r.errors().any(|e| e.code == code)
}

// ── Builder API ───────────────────────────────────────────────────────────────

#[test]
fn builder_read_alias_registers_on_field_enum() {
    let field = Field::string(fk("name"))
        .read_alias("display_name")
        .unwrap()
        .into_field();
    // Accessor on the Field enum returns the alias slice.
    assert_eq!(field.read_aliases().len(), 1);
    assert_eq!(field.read_aliases()[0].as_str(), "display_name");
}

#[test]
fn builder_read_alias_rejects_invalid_key() {
    let err = Field::string(fk("name"))
        .read_alias("has-dash")
        .unwrap_err();
    assert_eq!(err.code, "alias.invalid_key");
}

#[test]
fn builder_read_aliases_bulk_replaces_set() {
    let aliases = FieldAliases::new(["a", "b"]).unwrap();
    let field = Field::string(fk("name")).read_aliases(aliases).into_field();
    let keys: Vec<&str> = field.read_aliases().iter().map(FieldKey::as_str).collect();
    assert_eq!(keys, ["a", "b"]);
}

#[test]
fn builder_write_alias_registers_on_field_enum() {
    let field = Field::string(fk("internal_name"))
        .write_alias("displayName")
        .unwrap()
        .into_field();
    assert_eq!(
        field.write_alias().map(FieldKey::as_str),
        Some("displayName")
    );
}

#[test]
fn builder_write_alias_rejects_invalid_key() {
    let err = Field::string(fk("name"))
        .write_alias("has-dash")
        .unwrap_err();
    assert_eq!(err.code, "alias.invalid_key");
}

#[test]
fn field_enum_without_aliases_returns_empty_slice_and_none() {
    let field = Field::string(fk("x")).into_field();
    assert!(field.read_aliases().is_empty());
    assert!(field.write_alias().is_none());
}

// ── Ingest canonicalization ───────────────────────────────────────────────────

#[test]
fn alias_key_accepted_and_stored_under_canonical_key() {
    let schema = Schema::builder()
        .add(
            Field::string(fk("canonical_name"))
                .read_alias("alias_name")
                .unwrap(),
        )
        .build()
        .unwrap();

    let submitted = FieldValues::from_json(json!({"alias_name": "hello"})).unwrap();
    let valid = schema
        .validate(&submitted)
        .expect("alias_name must be accepted");

    // After ingest the value lives under the canonical key only.
    assert!(
        valid.raw().get(&fk("canonical_name")).is_some(),
        "value must be stored under the canonical key"
    );
    assert!(
        valid.raw().get(&fk("alias_name")).is_none(),
        "alias key must not remain in stored values"
    );
}

#[test]
fn canonical_key_wins_over_alias_when_both_submitted() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")).read_alias("alt_name").unwrap())
        .build()
        .unwrap();

    // Submit both canonical and alias — canonical must win (matches serde alias semantics).
    let submitted =
        FieldValues::from_json(json!({"name": "canonical_value", "alt_name": "alias_value"}))
            .unwrap();
    let valid = schema.validate(&submitted).expect("should accept");

    let stored_string = valid.raw().get_string(&fk("name"));
    assert_eq!(
        stored_string,
        Some("canonical_value"),
        "canonical value must win when both submitted"
    );
    assert!(valid.raw().get(&fk("alt_name")).is_none());
}

#[test]
fn multiple_aliases_first_present_wins() {
    let schema = Schema::builder()
        .add(
            Field::string(fk("target"))
                .read_alias("first_alias")
                .unwrap()
                .read_alias("second_alias")
                .unwrap(),
        )
        .build()
        .unwrap();

    // Submit second alias only.
    let submitted = FieldValues::from_json(json!({"second_alias": "from_second"})).unwrap();
    let valid = schema
        .validate(&submitted)
        .expect("second_alias must be accepted");
    assert!(
        valid.raw().get(&fk("target")).is_some(),
        "value must be stored under canonical key"
    );
    assert!(valid.raw().get(&fk("second_alias")).is_none());
}

#[test]
fn required_field_satisfied_via_alias_key() {
    let schema = Schema::builder()
        .add(
            Field::string(fk("email"))
                .required()
                .read_alias("email_address")
                .unwrap(),
        )
        .build()
        .unwrap();

    let submitted = FieldValues::from_json(json!({"email_address": "user@example.com"})).unwrap();
    schema
        .validate(&submitted)
        .expect("required satisfied via alias must not emit 'required'");
}

#[test]
fn field_validation_runs_on_alias_submitted_value() {
    // min_length constraint must be enforced even when value is submitted via alias.
    let schema = Schema::builder()
        .add(
            Field::string(fk("name"))
                .min_length(5)
                .read_alias("user_name")
                .unwrap(),
        )
        .build()
        .unwrap();

    let too_short = FieldValues::from_json(json!({"user_name": "ab"})).unwrap();
    let report = schema.validate(&too_short).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "min_length"),
        "min_length validation must run on alias-submitted value; got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn nested_object_alias_is_canonicalized() {
    let schema = Schema::builder()
        .add(
            Field::object(fk("user"))
                .add(Field::string(fk("username")).read_alias("login").unwrap()),
        )
        .build()
        .unwrap();

    let submitted = FieldValues::from_json(json!({"user": {"login": "admin"}})).unwrap();
    let valid = schema
        .validate(&submitted)
        .expect("nested alias must be accepted");

    // Inner value must be stored under canonical key.
    let user_value = valid.raw().get(&fk("user"));
    if let Some(FieldValue::Object(inner_map)) = user_value {
        assert!(
            inner_map.get(&fk("username")).is_some(),
            "nested value must be under canonical key"
        );
        assert!(
            inner_map.get(&fk("login")).is_none(),
            "alias key must be removed"
        );
    } else {
        panic!("expected Object value for 'user', got: {user_value:?}");
    }
}

// ── Security: alias must not bypass secret-strip ──────────────────────────────

#[test]
fn secret_via_alias_is_canonicalized_not_stored_under_alias_key() {
    // SECURITY: if alias canonicalization fails, a secret stored under the alias key
    // would bypass the secret-strip in context.rs (which indexes by field.key()).
    let schema = Schema::builder()
        .add(Field::secret(fk("api_key")).read_alias("token").unwrap())
        .build()
        .unwrap();

    let submitted = FieldValues::from_json(json!({"token": "s3cr3t"})).unwrap();
    let valid = schema
        .validate(&submitted)
        .expect("secret via alias must be accepted");

    // Alias key must be absent; canonical key present so secret-strip can find it.
    assert!(
        valid.raw().get(&fk("token")).is_none(),
        "alias key must not remain after ingest — secret would bypass secret-strip"
    );
    assert!(
        valid.raw().get(&fk("api_key")).is_some(),
        "secret must be stored under canonical key so context.rs redaction runs"
    );
}

// ── Projection: ValidSchema::project ─────────────────────────────────────────

#[test]
fn project_emits_write_alias_key_instead_of_canonical_key() {
    let schema = Schema::builder()
        .add(
            Field::string(fk("internal_id"))
                .write_alias("externalId")
                .unwrap(),
        )
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({"internal_id": "abc123"})).unwrap();
    let projected = schema.project(&values);

    assert_eq!(projected["externalId"], json!("abc123"));
    assert!(
        projected.get("internal_id").is_none(),
        "canonical key must not appear in projected output when write_alias is set"
    );
}

#[test]
fn project_emits_canonical_key_when_no_write_alias() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")))
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({"name": "alice"})).unwrap();
    let projected = schema.project(&values);

    assert_eq!(projected["name"], json!("alice"));
}

#[test]
fn project_excludes_secret_fields() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")))
        .add(Field::secret(fk("password")))
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({"name": "alice", "password": "s3cr3t"})).unwrap();
    let projected = schema.project(&values);

    assert_eq!(projected["name"], json!("alice"));
    assert!(
        projected.get("password").is_none(),
        "secrets must never appear in projected output"
    );
}

#[test]
fn project_passes_extra_non_schema_keys_through_unchanged() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")))
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({"x": "hello", "extra_key": 99})).unwrap();
    let projected = schema.project(&values);

    assert_eq!(projected["x"], json!("hello"));
    assert_eq!(projected["extra_key"], json!(99));
}

#[test]
fn project_recurses_into_nested_object_write_aliases() {
    let schema = Schema::builder()
        .add(
            Field::object(fk("contact")).add(
                Field::string(fk("phone_number"))
                    .write_alias("phoneNumber")
                    .unwrap(),
            ),
        )
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({"contact": {"phone_number": "555-1234"}})).unwrap();
    let projected = schema.project(&values);

    assert_eq!(projected["contact"]["phoneNumber"], json!("555-1234"));
    assert!(
        projected["contact"].get("phone_number").is_none(),
        "canonical nested key must not appear in projected output"
    );
}

// ── ValidValues::to_wire_json ─────────────────────────────────────────────────

#[test]
fn to_wire_json_applies_write_alias_to_validated_output() {
    let schema = Schema::builder()
        .add(
            Field::string(fk("internal_field"))
                .write_alias("wireField")
                .unwrap(),
        )
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({"internal_field": "payload"})).unwrap();
    let valid = schema.validate(&values).expect("validates");
    let wire = valid.to_wire_json();

    assert_eq!(wire["wireField"], json!("payload"));
    assert!(wire.get("internal_field").is_none());
}

// ── Lint: alias.* error codes ─────────────────────────────────────────────────

#[test]
fn lint_write_on_secret_emits_error() {
    // Field::Secret with a write_alias is forbidden (exposes secret key on wire).
    let report = Schema::builder()
        .add(Field::secret(fk("api_key")).write_alias("apiKey").unwrap())
        .build()
        .unwrap_err();
    assert!(
        has_error_code(&report, "alias.write_on_secret"),
        "expected alias.write_on_secret, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn lint_read_alias_equal_to_own_key_emits_self_collision() {
    let report = Schema::builder()
        .add(Field::string(fk("name")).read_alias("name").unwrap())
        .build()
        .unwrap_err();
    assert!(
        has_error_code(&report, "alias.self_collision"),
        "expected alias.self_collision, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn lint_read_alias_equal_to_sibling_canonical_key_emits_scope_collision() {
    let report = Schema::builder()
        .add(Field::string(fk("name")).read_alias("email").unwrap())
        .add(Field::string(fk("email")))
        .build()
        .unwrap_err();
    assert!(
        has_error_code(&report, "alias.scope_collision"),
        "expected alias.scope_collision, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn lint_shared_read_alias_across_sibling_fields_emits_scope_duplicate() {
    let report = Schema::builder()
        .add(Field::string(fk("a")).read_alias("alt").unwrap())
        .add(Field::string(fk("b")).read_alias("alt").unwrap())
        .build()
        .unwrap_err();
    assert!(
        has_error_code(&report, "alias.scope_duplicate"),
        "expected alias.scope_duplicate, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn lint_write_alias_equal_to_sibling_canonical_key_emits_write_collision() {
    // write_alias "target" collides with the canonical key of the "target" field.
    let report = Schema::builder()
        .add(Field::string(fk("source")).write_alias("target").unwrap())
        .add(Field::string(fk("target")))
        .build()
        .unwrap_err();
    assert!(
        has_error_code(&report, "alias.write_collision"),
        "expected alias.write_collision, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn lint_shared_write_alias_across_sibling_fields_emits_write_scope_duplicate() {
    let report = Schema::builder()
        .add(Field::string(fk("a")).write_alias("shared_out").unwrap())
        .add(Field::string(fk("b")).write_alias("shared_out").unwrap())
        .build()
        .unwrap_err();
    assert!(
        has_error_code(&report, "alias.write_scope_duplicate"),
        "expected alias.write_scope_duplicate, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── No-alias path stays wire-identical ───────────────────────────────────────

#[test]
fn field_without_aliases_emits_no_extra_wire_keys() {
    // `read_aliases` must be skipped (`skip_serializing_if = "is_empty"`);
    // `write_alias` must be skipped (`skip_serializing_if = "Option::is_none"`).
    let field = Field::string(fk("name"))
        .label("Name")
        .required()
        .into_field();
    let wire = serde_json::to_value(&field).unwrap();

    assert!(
        wire.get("read_aliases").is_none(),
        "read_aliases must not appear in wire format when empty"
    );
    assert!(
        wire.get("write_alias").is_none(),
        "write_alias must not appear in wire format when None"
    );
}

#[test]
fn schema_without_aliases_round_trips_byte_identical() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")).required())
        .add(Field::number(fk("y")))
        .build()
        .unwrap();

    let wire_first = serde_json::to_string(&schema).unwrap();
    let schema_back: Schema = serde_json::from_str(&wire_first).unwrap();
    let wire_second = serde_json::to_string(&schema_back).unwrap();

    assert_eq!(
        wire_first, wire_second,
        "no-alias schema must round-trip byte-identical"
    );
}

// ── Security: aliases & secrets nested in Mode / List payloads ─────────────────
//
// Regression guards for the canonicalize/project/lint recursion gaps found in
// the Step-12a adversarial review: a wire `{"mode","value"}` envelope parses to
// a `FieldValue::Object` (never `FieldValue::Mode`), so canonicalization and
// projection must handle the Object shape at every container depth, and the
// build-time collision lint must reach the same depth the canonicalizer folds.

/// Serialize a validated value tree to its wire string, for leak assertions.
fn wire_string(valid: &nebula_schema::ValidValues) -> String {
    serde_json::to_string(&valid.to_wire_json()).expect("wire json serializes")
}

#[test]
fn mode_payload_secret_alias_canonicalized_not_leaked() {
    let schema = Schema::builder()
        .add(
            Field::mode(fk("auth"))
                .variant(
                    "apikey",
                    "API Key",
                    Field::object(fk("payload")).add(
                        Field::secret(fk("api_key"))
                            .read_alias("token_alias")
                            .unwrap(),
                    ),
                )
                .default_variant("apikey"),
        )
        .build()
        .unwrap();

    let submitted = FieldValues::from_json(json!({
        "auth": {"mode": "apikey", "value": {"token_alias": "PLAINTEXT_SECRET"}}
    }))
    .unwrap();
    let valid = schema
        .validate(&submitted)
        .expect("mode-payload alias must be accepted");

    // The alias key must have folded onto the canonical secret key at depth.
    let raw = valid.raw().to_json();
    assert_eq!(
        raw["auth"]["value"]["api_key"],
        json!("PLAINTEXT_SECRET"),
        "mode-payload alias must fold onto the canonical secret key"
    );
    assert!(
        raw["auth"]["value"].get("token_alias").is_none(),
        "alias key must not survive inside the mode payload"
    );
    // And the secret must never reach the wire — neither key nor plaintext.
    assert!(
        !wire_string(&valid).contains("PLAINTEXT_SECRET"),
        "secret plaintext leaked into wire projection"
    );
}

#[test]
fn mode_payload_alias_canonicalized_via_default_variant() {
    // `mode` omitted → the active variant comes from `default_variant`; a nested
    // alias must still fold (regression for the default-variant ingest path).
    let schema = Schema::builder()
        .add(
            Field::mode(fk("auth"))
                .variant(
                    "token",
                    "Token",
                    Field::object(fk("payload")).add(
                        Field::string(fk("client_id"))
                            .read_alias("clientId")
                            .unwrap(),
                    ),
                )
                .default_variant("token"),
        )
        .build()
        .unwrap();

    let submitted =
        FieldValues::from_json(json!({"auth": {"value": {"clientId": "abc"}}})).unwrap();
    let valid = schema
        .validate(&submitted)
        .expect("default-variant nested alias must be accepted");

    let raw = valid.raw().to_json();
    assert_eq!(
        raw["auth"]["value"]["client_id"],
        json!("abc"),
        "default-variant nested alias must fold onto the canonical key"
    );
    assert!(raw["auth"]["value"].get("clientId").is_none());
}

#[test]
fn list_item_mode_payload_secret_alias_not_leaked() {
    // The leak must also be closed through array-of-record bulk-ingest shapes.
    let schema = Schema::builder()
        .add(
            Field::list(fk("rows")).item(
                Field::object(fk("row")).add(
                    Field::mode(fk("auth"))
                        .variant(
                            "apikey",
                            "API Key",
                            Field::object(fk("cfg"))
                                .add(Field::secret(fk("api_key")).read_alias("token").unwrap()),
                        )
                        .default_variant("apikey"),
                ),
            ),
        )
        .build()
        .unwrap();

    // A mode variant's object payload maps its child fields directly under
    // `value` — the variant field's own key is not a wire wrapper.
    let submitted = FieldValues::from_json(json!({
        "rows": [{"auth": {"mode": "apikey", "value": {"token": "PLAINTEXT_SECRET"}}}]
    }))
    .unwrap();
    let valid = schema
        .validate(&submitted)
        .expect("list-item mode alias accepted");

    let raw = valid.raw().to_json();
    assert_eq!(
        raw["rows"][0]["auth"]["value"]["api_key"],
        json!("PLAINTEXT_SECRET"),
        "alias inside a list-item mode payload must fold onto the canonical key"
    );
    assert!(raw["rows"][0]["auth"]["value"].get("token").is_none());
    assert!(
        !wire_string(&valid).contains("PLAINTEXT_SECRET"),
        "secret plaintext leaked into wire projection from a list-item mode payload"
    );
}

#[test]
fn project_drops_secret_nested_in_list() {
    // A secret nested in a list item must never appear in projection; its
    // non-secret sibling must.
    let schema = Schema::builder()
        .add(
            Field::list(fk("creds")).item(
                Field::object(fk("cred"))
                    .add(Field::string(fk("user")))
                    .add(Field::secret(fk("password"))),
            ),
        )
        .build()
        .unwrap();

    let values =
        FieldValues::from_json(json!({"creds": [{"user": "alice", "password": "PLAINTEXT_LEAK"}]}))
            .unwrap();
    let projected = schema.project(&values);

    assert_eq!(projected["creds"][0]["user"], json!("alice"));
    assert!(
        projected["creds"][0].get("password").is_none(),
        "secret in a list item must be dropped from projection"
    );
    assert!(
        !serde_json::to_string(&projected)
            .unwrap()
            .contains("PLAINTEXT_LEAK")
    );
}

#[test]
fn project_drops_secret_nested_in_mode_payload() {
    let schema = Schema::builder()
        .add(
            Field::mode(fk("auth"))
                .variant(
                    "basic",
                    "Basic",
                    Field::object(fk("payload"))
                        .add(Field::string(fk("user")))
                        .add(Field::secret(fk("password"))),
                )
                .default_variant("basic"),
        )
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({
        "auth": {"mode": "basic", "value": {"user": "bob", "password": "MODE_SECRET"}}
    }))
    .unwrap();
    let projected = schema.project(&values);

    assert_eq!(projected["auth"]["value"]["user"], json!("bob"));
    assert!(
        projected["auth"]["value"].get("password").is_none(),
        "secret in a mode payload must be dropped from projection"
    );
    assert!(
        !serde_json::to_string(&projected)
            .unwrap()
            .contains("MODE_SECRET")
    );
}

#[test]
fn project_applies_write_alias_inside_list_items() {
    let schema = Schema::builder()
        .add(
            Field::list(fk("rows")).item(
                Field::object(fk("row")).add(
                    Field::string(fk("internal_id"))
                        .write_alias("externalId")
                        .unwrap(),
                ),
            ),
        )
        .build()
        .unwrap();

    let values =
        FieldValues::from_json(json!({"rows": [{"internal_id": "x1"}, {"internal_id": "x2"}]}))
            .unwrap();
    let projected = schema.project(&values);

    assert_eq!(projected["rows"][0]["externalId"], json!("x1"));
    assert_eq!(projected["rows"][1]["externalId"], json!("x2"));
    assert!(
        projected["rows"][0].get("internal_id").is_none(),
        "canonical key must be remapped to the write_alias inside list items"
    );
}

#[test]
fn project_applies_write_alias_inside_mode_payload() {
    let schema = Schema::builder()
        .add(
            Field::mode(fk("auth"))
                .variant(
                    "token",
                    "Token",
                    Field::object(fk("payload")).add(
                        Field::string(fk("client_id"))
                            .write_alias("clientId")
                            .unwrap(),
                    ),
                )
                .default_variant("token"),
        )
        .build()
        .unwrap();

    let values =
        FieldValues::from_json(json!({"auth": {"mode": "token", "value": {"client_id": "abc"}}}))
            .unwrap();
    let projected = schema.project(&values);

    assert_eq!(projected["auth"]["value"]["clientId"], json!("abc"));
    assert!(projected["auth"]["value"].get("client_id").is_none());
}

#[test]
fn project_extra_key_cannot_clobber_write_alias_output() {
    // INTEGRITY: an attacker-supplied extra key equal to a field's write_alias
    // output name must not overwrite the field's real projected value.
    let schema = Schema::builder()
        .add(
            Field::string(fk("internal_id"))
                .write_alias("externalId")
                .unwrap(),
        )
        .build()
        .unwrap();

    let values =
        FieldValues::from_json(json!({"internal_id": "REAL", "externalId": "ATTACKER"})).unwrap();

    let projected = schema.project(&values);
    assert_eq!(
        projected["externalId"],
        json!("REAL"),
        "declared field must win over a colliding extra pass-through key"
    );
    // The validated path stores the extra key, so the guard must hold there too.
    let valid = schema.validate(&values).expect("validates");
    assert_eq!(valid.to_wire_json()["externalId"], json!("REAL"));
}

#[test]
fn project_extra_key_cannot_occupy_absent_write_alias_output_slot() {
    // INTEGRITY: a write-aliased field reserves its output slot even when the
    // field is ABSENT this submission — an attacker-supplied extra key equal to
    // that output name must not occupy the schema-owned slot.
    let schema = Schema::builder()
        .add(
            Field::string(fk("internal_id"))
                .write_alias("externalId")
                .unwrap(),
        )
        .build()
        .unwrap();

    // `internal_id` is optional and omitted; only the spoof key is present.
    let values = FieldValues::from_json(json!({"externalId": "ATTACKER"})).unwrap();

    let projected = schema.project(&values);
    assert!(
        projected.get("externalId").is_none(),
        "extra key must not occupy an absent write-aliased field's output slot, got: {projected}"
    );
    // Same on the validated wire path (validate stores the extra key).
    let valid = schema.validate(&values).expect("validates");
    assert!(valid.to_wire_json().get("externalId").is_none());
}

#[test]
fn project_extra_key_cannot_occupy_dropped_secret_field_output_slot() {
    // INTEGRITY: a secret-bearing structured field with a write_alias whose value
    // is dropped (wrong-shape blob → over-redacted to nothing) still reserves its
    // output slot; an extra key must not slip into it.
    let schema = Schema::builder()
        .add(
            Field::object(fk("creds"))
                .add(Field::secret(fk("password")))
                .write_alias("credsOut")
                .unwrap(),
        )
        .build()
        .unwrap();

    // `creds` is a wrong-shape literal blob (dropped by the over-redact guard),
    // plus an attacker payload under the reserved `credsOut` output name.
    let values = FieldValues::from_json(json!({
        "creds": "wrong-shape-blob",
        "credsOut": {"injected": "ATTACKER"}
    }))
    .unwrap();

    let projected = schema.project(&values);
    assert!(
        projected.get("credsOut").is_none(),
        "extra key must not occupy a dropped write-aliased field's output slot, got: {projected}"
    );
    assert!(
        !serde_json::to_string(&projected)
            .unwrap()
            .contains("ATTACKER"),
        "attacker payload must not appear in projection"
    );
}

#[test]
fn project_on_raw_read_aliased_secret_does_not_leak() {
    // `project` canonicalizes first, so a secret submitted under its read-alias
    // is folded onto the canonical secret key and dropped — even when called
    // directly on raw, never-validated values.
    let schema = Schema::builder()
        .add(Field::secret(fk("api_key")).read_alias("token").unwrap())
        .build()
        .unwrap();

    let raw = FieldValues::from_json(json!({"token": "s3cr3t"})).unwrap();
    let projected = schema.project(&raw);

    assert!(
        projected.get("token").is_none(),
        "a secret submitted under its read-alias must not pass through projection"
    );
    assert!(
        projected.get("api_key").is_none(),
        "secret must be dropped, never emitted"
    );
    assert!(
        !serde_json::to_string(&projected)
            .unwrap()
            .contains("s3cr3t")
    );
}

#[test]
fn lint_catches_alias_scope_collision_at_list_of_list_depth() {
    // The collision lint must reach the same depth `canonicalize_aliases` folds.
    // A read-alias stealing a sibling secret's canonical key two containers deep
    // must still be rejected at build time.
    let report = Schema::builder()
        .add(
            Field::list(fk("outer")).item(
                Field::list(fk("inner")).item(
                    Field::object(fk("row"))
                        .add(Field::string(fk("public")).read_alias("api_key").unwrap())
                        .add(Field::secret(fk("api_key"))),
                ),
            ),
        )
        .build()
        .unwrap_err();

    assert!(
        has_error_code(&report, "alias.scope_collision"),
        "expected alias.scope_collision at list-of-list depth, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn lint_catches_alias_self_collision_at_mode_of_list_depth() {
    let report = Schema::builder()
        .add(
            Field::mode(fk("auth"))
                .variant(
                    "v",
                    "V",
                    Field::list(fk("items")).item(
                        Field::object(fk("row"))
                            .add(Field::string(fk("name")).read_alias("name").unwrap()),
                    ),
                )
                .default_variant("v"),
        )
        .build()
        .unwrap_err();

    assert!(
        has_error_code(&report, "alias.self_collision"),
        "expected alias.self_collision at mode-of-list depth, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn lint_catches_write_on_secret_for_bare_list_item() {
    // A bare secret list item is not a scope member, so write_on_secret must be
    // checked on it directly.
    let report = Schema::builder()
        .add(
            Field::list(fk("tokens")).item(
                Field::secret(fk("tok"))
                    .write_alias("exposedToken")
                    .unwrap(),
            ),
        )
        .build()
        .unwrap_err();

    assert!(
        has_error_code(&report, "alias.write_on_secret"),
        "expected alias.write_on_secret for a bare secret list item, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn lint_nested_self_collision_reported_exactly_once() {
    // A single recursion owner: a collision inside a nested object is reported
    // exactly once, not duplicated by a second recursion pass.
    let report = Schema::builder()
        .add(Field::object(fk("user")).add(Field::string(fk("name")).read_alias("name").unwrap()))
        .build()
        .unwrap_err();

    let collision_count = report
        .errors()
        .filter(|e| e.code == "alias.self_collision")
        .count();
    assert_eq!(
        collision_count, 1,
        "nested self_collision must be reported exactly once, got {collision_count}"
    );
}
