//! Integration tests for `#[derive(Schema)]` and `#[derive(EnumSelect)]`.

use nebula_schema::{
    EnumSelect, Field, FieldKey, FieldValues, HasSchema, HasSelectOptions, InputHint, RequiredMode,
    Schema, StringWidget,
};
use serde_json::json;

fn fk(s: &str) -> FieldKey {
    FieldKey::new(s).expect("valid field key")
}

// ── #[derive(Schema)] ──────────────────────────────────────────────────────

#[derive(Schema)]
#[expect(dead_code, reason = "fields are exercised via HasSchema::schema")]
struct HttpInput {
    #[field(label = "URL", hint = "url")]
    #[validate(required, url, length(max = 8192))]
    url: String,

    #[field(label = "Method", description = "HTTP method", default = "GET")]
    method: String,

    #[field(label = "Body", multiline)]
    #[validate(length(max = 1024))]
    body: Option<String>,

    #[field(label = "Timeout (seconds)")]
    #[validate(range(1..=300))]
    timeout: Option<u32>,

    #[field(label = "Verbose", no_expression)]
    verbose: bool,

    #[field(secret, label = "API Key")]
    #[validate(required)]
    api_key: String,
}

#[test]
fn derive_schema_matches_hand_written_schema() {
    let derived = HttpInput::schema();
    // 6 fields declared (skip would exclude).
    assert_eq!(derived.fields().len(), 6);

    // url — required String with url + max_length.
    match &derived.fields()[0] {
        Field::String(s) => {
            assert_eq!(s.key.as_str(), "url");
            assert_eq!(s.label.as_deref(), Some("URL"));
            assert!(matches!(s.required, RequiredMode::Always));
            assert!(matches!(s.hint, InputHint::Url));
            assert!(s.rules.len() >= 2);
        },
        other => panic!("expected StringField, got {other:?}"),
    }

    // method — plain string with default "GET".
    match &derived.fields()[1] {
        Field::String(s) => {
            assert_eq!(s.key.as_str(), "method");
            assert_eq!(s.default.as_ref(), Some(&json!("GET")));
        },
        other => panic!("expected StringField, got {other:?}"),
    }

    // body — Option<String> + multiline widget.
    match &derived.fields()[2] {
        Field::String(s) => {
            assert_eq!(s.key.as_str(), "body");
            assert!(matches!(s.required, RequiredMode::Never));
            assert!(matches!(s.widget, StringWidget::Multiline));
        },
        other => panic!("expected StringField, got {other:?}"),
    }

    // timeout — Option<u32> with range.
    match &derived.fields()[3] {
        Field::Number(n) => {
            assert_eq!(n.key.as_str(), "timeout");
            assert!(n.integer);
            assert!(matches!(n.required, RequiredMode::Never));
            assert!(n.rules.len() >= 2);
        },
        other => panic!("expected NumberField, got {other:?}"),
    }

    // verbose — bool with no_expression.
    match &derived.fields()[4] {
        Field::Boolean(b) => {
            assert_eq!(b.key.as_str(), "verbose");
            assert!(matches!(
                b.expression,
                nebula_schema::ExpressionMode::Forbidden
            ));
        },
        other => panic!("expected BooleanField, got {other:?}"),
    }

    // api_key — secret, because #[field(secret)] switched String → SecretField.
    match &derived.fields()[5] {
        Field::Secret(s) => {
            assert_eq!(s.key.as_str(), "api_key");
            assert!(matches!(s.required, RequiredMode::Always));
        },
        other => panic!("expected SecretField, got {other:?}"),
    }
}

#[test]
fn derive_schema_is_cached() {
    let a = HttpInput::schema();
    let b = HttpInput::schema();
    // `PartialEq` would succeed on structural equality even if the cache
    // is broken — `ptr_eq` is the actual invariant we care about.
    assert!(
        a.ptr_eq(&b),
        "derive(Schema) must cache the built schema behind a shared Arc"
    );
}

#[derive(Schema)]
#[expect(dead_code, reason = "fields exercised via derive")]
struct Tag {
    name: String,
}

#[derive(Schema)]
#[expect(dead_code, reason = "fields exercised via derive")]
struct TagList {
    tags: Vec<String>,
    owner: Option<Tag>,
}

#[test]
fn derive_handles_vec_and_nested_user_type() {
    let schema = TagList::schema();
    assert_eq!(schema.fields().len(), 2);

    match &schema.fields()[0] {
        Field::List(l) => {
            assert_eq!(l.key.as_str(), "tags");
            assert!(l.item.is_some());
            match l.item.as_deref().unwrap() {
                Field::String(_) => {},
                other => panic!("expected String list item, got {other:?}"),
            }
        },
        other => panic!("expected ListField, got {other:?}"),
    }

    match &schema.fields()[1] {
        Field::Object(o) => {
            assert_eq!(o.key.as_str(), "owner");
            // Tag has one field (name) — inlined via user-defined object.
            assert_eq!(o.fields.len(), 1);
            assert_eq!(o.fields[0].key().as_str(), "name");
        },
        other => panic!("expected ObjectField, got {other:?}"),
    }
}

#[derive(Schema)]
#[expect(dead_code)]
struct WithSkip {
    keep: String,
    #[field(skip)]
    _internal: u64,
}

#[test]
fn derive_respects_skip() {
    let s = WithSkip::schema();
    assert_eq!(s.fields().len(), 1);
    assert_eq!(s.fields()[0].key().as_str(), "keep");
}

#[derive(Schema)]
#[schema(reserved("legacy_token", "v1_secret"))]
#[expect(dead_code)]
struct WithReservedKeys {
    name: String,
    enabled: bool,
}

#[test]
fn derive_reserved_keys_do_not_materialize_or_block_other_fields() {
    let s = WithReservedKeys::schema();
    let keys: Vec<&str> = s.fields().iter().map(|f| f.key().as_str()).collect();
    // The real fields build normally — reserving unrelated keys is a no-op for them.
    assert_eq!(keys, ["name", "enabled"]);
    // The reserved keys are guard rails only: they are not materialized as fields.
    assert!(!keys.contains(&"legacy_token"));
    assert!(!keys.contains(&"v1_secret"));
}

#[derive(Schema)]
#[schema(reserved("dropped"))]
#[expect(dead_code)]
struct ReservedMatchesSkippedField {
    keep: String,
    // A skipped field has no wire key, so reserving its name is not a collision —
    // this pins the skip-before-key-collection ordering in the derive.
    #[field(skip)]
    dropped: u64,
}

#[test]
fn derive_reserved_key_matching_a_skipped_field_is_allowed() {
    let s = ReservedMatchesSkippedField::schema();
    let keys: Vec<&str> = s.fields().iter().map(|f| f.key().as_str()).collect();
    assert_eq!(
        keys,
        ["keep"],
        "the skipped `dropped` field never reaches the schema"
    );
}

#[derive(Schema, serde::Deserialize)]
#[schema(reserved("removed"))]
#[expect(dead_code)]
struct AliasDoesNotCollide {
    // An alias that does NOT match a reserved key is allowed — only a collision
    // is rejected, aliases are not blanket-forbidden on reserved-key structs.
    #[serde(alias = "name_old")]
    name: String,
}

#[test]
fn derive_reserved_allows_a_non_colliding_serde_alias() {
    let s = AliasDoesNotCollide::schema();
    let keys: Vec<&str> = s.fields().iter().map(|f| f.key().as_str()).collect();
    assert_eq!(keys, ["name"]);
}

// ── #[derive(EnumSelect)] ──────────────────────────────────────────────────

#[derive(EnumSelect, Clone, Copy)]
#[expect(dead_code, reason = "variants exercised via derive")]
enum HttpMethod {
    Get,
    Post,
    Put,
    #[field(label = "HTTP DELETE")]
    Delete,
}

#[test]
fn derive_enum_select_generates_options() {
    let options = HttpMethod::select_options();
    assert_eq!(options.len(), 4);
    assert_eq!(options[0].value, json!("get"));
    assert_eq!(options[0].label, "Get");
    assert_eq!(options[1].value, json!("post"));
    assert_eq!(options[3].value, json!("delete"));
    assert_eq!(options[3].label, "HTTP DELETE");
}

#[derive(Schema)]
#[expect(dead_code, reason = "fields exercised via HasSchema::schema")]
struct RequestLine {
    #[field(label = "HTTP method", enum_select, default = "get")]
    method: HttpMethod,
    #[field(label = "Optional override", enum_select)]
    alt: Option<HttpMethod>,
}

#[test]
fn derive_enum_select_field_becomes_select() {
    let schema = RequestLine::schema();
    assert_eq!(schema.fields().len(), 2);

    match &schema.fields()[0] {
        Field::Select(s) => {
            assert_eq!(s.key.as_str(), "method");
            assert_eq!(s.options.len(), 4);
            assert_eq!(s.default.as_ref(), Some(&json!("get")));
            assert!(matches!(s.required, RequiredMode::Always));
        },
        other => panic!("expected SelectField for enum_select, got {other:?}"),
    }

    match &schema.fields()[1] {
        Field::Select(s) => {
            assert_eq!(s.key.as_str(), "alt");
            assert_eq!(s.options.len(), 4);
            assert!(matches!(s.required, RequiredMode::Never));
        },
        other => panic!("expected SelectField for optional enum, got {other:?}"),
    }
}

#[derive(Schema)]
#[expect(dead_code)]
struct Uses {
    #[field(label = "Plain string")]
    value: String,
}

#[test]
fn sanity_build_many_fields_via_derive() {
    // Confirm that `.add(Uses::schema().into())` also works via builder.
    let s = Schema::builder()
        .add_many(Uses::schema().fields().iter().cloned())
        .build()
        .expect("derived fields build into a new Schema");
    assert_eq!(s.fields().len(), 1);
}

// ── raw identifiers (keywords as field names) ──────────────────────────────

#[derive(Schema)]
#[expect(dead_code, reason = "fields exercised via HasSchema::schema")]
struct RawIdentFields {
    // `r#type` / `r#async` use Rust keywords as field names. The derive must
    // strip the raw-identifier prefix to the schema keys `type` / `async`
    // (matching serde) instead of panicking at runtime on the `#` in `r#type`.
    r#type: String,
    r#async: bool,
}

#[test]
fn derive_schema_strips_raw_identifier_prefix() {
    let schema = RawIdentFields::schema();
    let keys: Vec<&str> = schema.fields().iter().map(|f| f.key().as_str()).collect();
    assert_eq!(keys, ["type", "async"]);
}

// ── acronym-aware snake_case for enum option values ────────────────────────

#[derive(EnumSelect, Clone, Copy)]
#[expect(dead_code, reason = "variants exercised via derive")]
enum AcronymMethod {
    HTTPProxy,
    PostBody,
}

#[test]
fn derive_enum_select_uses_heck_for_acronyms() {
    let options = AcronymMethod::select_options();
    // `heck` splits the leading acronym run: `HTTPProxy` → `http_proxy`
    // (the previous hand-rolled pass produced `httpproxy`).
    assert_eq!(options[0].value, json!("http_proxy"));
    assert_eq!(options[1].value, json!("post_body"));
}

// ── serde key alignment (C1 keystone) ──────────────────────────────────────

#[derive(Schema, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct CamelConfig {
    user_name: String,
    api_key_id: String,
}

#[test]
fn derive_schema_honors_serde_rename_all_matching_wire() {
    // The schema key MUST equal serde's wire key, otherwise the validator checks a
    // field the deserializer never produces. Before this fix the keys stayed
    // `user_name` / `api_key_id` while serde emitted `userName` / `apiKeyId`.
    let schema = CamelConfig::schema();
    let schema_keys: Vec<&str> = schema.fields().iter().map(|f| f.key().as_str()).collect();
    assert_eq!(schema_keys, ["userName", "apiKeyId"]);

    // Parity guard: every schema key is an actual serde wire key (order-independent
    // because serde_json sorts object keys without `preserve_order`).
    let wire = serde_json::to_value(CamelConfig::default()).expect("serializes");
    let wire_obj = wire.as_object().expect("struct serializes to an object");
    assert_eq!(schema_keys.len(), wire_obj.len());
    for key in &schema_keys {
        assert!(
            wire_obj.contains_key(*key),
            "schema key `{key}` is not a serde wire key; wire = {wire_obj:?}"
        );
    }
}

#[derive(Schema, serde::Deserialize)]
#[expect(dead_code, reason = "exercised via HasSchema::schema")]
struct RenamedField {
    #[serde(rename = "apiKey")]
    api_key: String,
}

#[test]
fn derive_schema_honors_serde_field_rename() {
    let schema = RenamedField::schema();
    assert_eq!(schema.fields()[0].key().as_str(), "apiKey");
}

#[derive(Schema, serde::Deserialize)]
#[expect(dead_code, reason = "exercised via HasSchema::schema")]
struct WithSkipped {
    kept: String,
    #[serde(skip)]
    internal: String,
}

#[test]
fn derive_schema_drops_serde_skipped_field() {
    let schema = WithSkipped::schema();
    let keys: Vec<&str> = schema.fields().iter().map(|f| f.key().as_str()).collect();
    assert_eq!(keys, ["kept"]);
}

#[derive(EnumSelect, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[expect(dead_code, reason = "variants exercised via derive")]
enum ScreamingMethod {
    GetThing,
    PutThing,
}

#[test]
fn derive_enum_select_honors_serde_rename_all() {
    let options = ScreamingMethod::select_options();
    assert_eq!(options[0].value, json!("GET_THING"));
    assert_eq!(options[1].value, json!("PUT_THING"));
}

#[derive(EnumSelect, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum AcronymRenamed {
    HTTPProxy,
}

#[test]
fn derive_enum_select_value_matches_serde_for_acronym_rename_all() {
    // serde's variant `snake_case` is naive — `_` before every capital — so
    // `HTTPProxy` becomes `h_t_t_p_proxy`, NOT heck's `http_proxy`. The catalog
    // value must equal serde's wire name exactly or the option cannot round-trip.
    let catalog = AcronymRenamed::select_options()[0].value.clone();
    let wire = serde_json::to_value(AcronymRenamed::HTTPProxy).expect("serializes");
    assert_eq!(catalog, wire);
    assert_eq!(catalog, json!("h_t_t_p_proxy"));
}

// ── field aliases (Step 12b) ────────────────────────────────────────────────

#[derive(Schema, serde::Deserialize)]
#[expect(dead_code, reason = "exercised via HasSchema::schema")]
struct AliasedInput {
    #[serde(alias = "display_name", alias = "displayName")]
    name: String,
}

#[test]
fn derive_serde_alias_becomes_read_alias() {
    // `#[serde(alias)]` keys become read-aliases — serde deserializes them AND the
    // schema accepts them, keeping wire and schema in sync.
    let schema = AliasedInput::schema();
    let aliases: Vec<&str> = schema.fields()[0]
        .read_aliases()
        .iter()
        .map(FieldKey::as_str)
        .collect();
    assert_eq!(aliases, ["display_name", "displayName"]);

    // Input under an alias is accepted and folded onto the canonical key.
    let valid = schema
        .validate(&FieldValues::from_json(json!({"displayName": "Alice"})).unwrap())
        .expect("alias-keyed input must be accepted");
    assert_eq!(valid.raw().get_string(&fk("name")), Some("Alice"));
}

#[derive(Schema)]
#[expect(dead_code, reason = "exercised via HasSchema::schema")]
struct RemappedOutput {
    #[field(emit_as = "externalId")]
    internal_id: String,
}

#[test]
fn derive_field_emit_as_emits_on_projection() {
    let schema = RemappedOutput::schema();
    assert_eq!(
        schema.fields()[0].emit_as().map(FieldKey::as_str),
        Some("externalId")
    );

    // Projection emits the field under the emit_as key, not the canonical key.
    let projected = schema.project(&FieldValues::from_json(json!({"internal_id": "x1"})).unwrap());
    assert_eq!(projected["externalId"], json!("x1"));
    assert!(projected.get("internal_id").is_none());
}

#[derive(Schema, serde::Deserialize)]
#[expect(dead_code, reason = "exercised via HasSchema::schema")]
struct RoundTripField {
    #[serde(alias = "wire")]
    #[field(emit_as = "wire")]
    internal: String,
}

#[test]
fn derive_same_field_read_and_emit_as_reuse_builds() {
    // Reading from and emitting to the SAME wire key on one field is round-trip
    // stable, so it must build (cross-field reuse would be rejected at compile time).
    let schema = RoundTripField::schema();
    let field = &schema.fields()[0];
    assert_eq!(field.read_aliases()[0].as_str(), "wire");
    assert_eq!(field.emit_as().map(FieldKey::as_str), Some("wire"));
}

#[derive(Schema, serde::Deserialize)]
#[expect(dead_code, reason = "exercised via HasSchema::schema")]
struct DuplicateAlias {
    #[serde(alias = "alt", alias = "alt")]
    name: String,
}

#[test]
fn derive_duplicate_serde_alias_is_deduped_not_rejected() {
    // serde tolerates a repeated alias on one field; the derive dedups so the
    // generated schema has exactly ONE read-alias and builds — without the dedup
    // the runtime scope_duplicate lint would reject it and panic `schema()`.
    let schema = DuplicateAlias::schema();
    let aliases: Vec<&str> = schema.fields()[0]
        .read_aliases()
        .iter()
        .map(FieldKey::as_str)
        .collect();
    assert_eq!(aliases, ["alt"]);
}

// ── EnumSelect / serde divergence contract ─────────────────────────────────
//
// Without `#[serde(rename_all = ...)]` the catalog value produced by
// `#[derive(EnumSelect)]` defaults to `heck`-based snake_case. This is an
// explicit UI convention — it diverges from serde's verbatim default (the
// variant name as written). The tests below pin that contract and prove a
// caller must opt in to exact serde alignment by adding
// `#[serde(rename_all = "snake_case")]`.

/// An enum with no `rename_all` annotation: serde keeps variant names verbatim
/// but EnumSelect converts them to heck snake_case.
#[derive(EnumSelect, serde::Serialize, Clone, Copy)]
#[expect(dead_code)] // AnotherOne is exercised only via select_options(), not direct construction
enum NoPolicyEnum {
    SimpleVariant,
    AnotherOne,
}

#[test]
fn enum_select_without_rename_all_defaults_to_heck_snake_case() {
    // Contract: catalog values are heck snake_case, NOT serde verbatim.
    // This is intentional — callers must set `#[serde(rename_all = "snake_case")]`
    // to align catalog values with serde's wire names.
    let options = NoPolicyEnum::select_options();
    assert_eq!(
        options[0].value,
        json!("simple_variant"),
        "EnumSelect default is heck snake_case, not verbatim"
    );
    assert_eq!(options[1].value, json!("another_one"));
}

#[test]
fn enum_select_without_rename_all_diverges_from_serde_verbatim() {
    // Deliberately assert the divergence so any future change to the default
    // (e.g. switching to verbatim) is caught by a red test, not a silent data-desync.
    let serde_wire_0 = serde_json::to_value(NoPolicyEnum::SimpleVariant).expect("serializes");
    let catalog_value_0 = NoPolicyEnum::select_options()[0].value.clone();
    // serde verbatim = "SimpleVariant"; catalog (heck) = "simple_variant" — they differ.
    assert_ne!(
        catalog_value_0, serde_wire_0,
        "without rename_all, catalog value must diverge from serde verbatim; \
         if they match the default changed — update the derive_enum.rs doc"
    );
}

/// Same enum but with `#[serde(rename_all = "snake_case")]` explicitly set:
/// catalog values and serde wire names align.
#[derive(EnumSelect, serde::Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum AlignedWithSerdeEnum {
    SimpleVariant,
    AnotherOne,
}

#[test]
fn enum_select_with_snake_case_rename_all_matches_serde_wire() {
    // With explicit `rename_all = "snake_case"` the catalog value equals serde's
    // wire representation — the round-trip is safe.
    let options = AlignedWithSerdeEnum::select_options();
    for (i, variant) in [
        AlignedWithSerdeEnum::SimpleVariant,
        AlignedWithSerdeEnum::AnotherOne,
    ]
    .iter()
    .enumerate()
    {
        let wire = serde_json::to_value(variant).expect("serializes");
        assert_eq!(
            options[i].value, wire,
            "catalog value must equal serde wire name when rename_all = snake_case is set"
        );
    }
}
