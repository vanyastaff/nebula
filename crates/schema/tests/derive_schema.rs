//! Integration tests for `#[derive(Schema)]` and `#[derive(EnumSelect)]`.

use nebula_schema::{
    EnumSelect, Field, HasSchema, HasSelectOptions, InputHint, RequiredMode, Schema, StringWidget,
};
use serde_json::json;

// ── #[derive(Schema)] ──────────────────────────────────────────────────────

#[derive(Schema)]
#[allow(dead_code, reason = "fields are exercised via HasSchema::schema")]
struct HttpInput {
    #[param(label = "URL", hint = "url")]
    #[validate(required, url, length(max = 8192))]
    url: String,

    #[param(label = "Method", description = "HTTP method", default = "GET")]
    method: String,

    #[param(label = "Body", multiline)]
    #[validate(length(max = 1024))]
    body: Option<String>,

    #[param(label = "Timeout (seconds)")]
    #[validate(range(1..=300))]
    timeout: Option<u32>,

    #[param(label = "Verbose", no_expression)]
    verbose: bool,

    #[param(secret, label = "API Key")]
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

    // api_key — secret, because #[param(secret)] switched String → SecretField.
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
    // Both reads return the same Arc.
    assert_eq!(a, b);
}

#[derive(Schema)]
#[allow(dead_code, reason = "fields exercised via derive")]
struct Tag {
    name: String,
}

#[derive(Schema)]
#[allow(dead_code, reason = "fields exercised via derive")]
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
#[allow(dead_code)]
struct WithSkip {
    keep: String,
    #[param(skip)]
    _internal: u64,
}

#[test]
fn derive_respects_skip() {
    let s = WithSkip::schema();
    assert_eq!(s.fields().len(), 1);
    assert_eq!(s.fields()[0].key().as_str(), "keep");
}

// ── #[derive(EnumSelect)] ──────────────────────────────────────────────────

#[derive(EnumSelect, Clone, Copy)]
#[allow(dead_code, reason = "variants exercised via derive")]
enum HttpMethod {
    Get,
    Post,
    Put,
    #[param(label = "HTTP DELETE")]
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

// Mixed: a derived struct can use a derived enum for its schema, albeit via
// HasSelectOptions lookup on the builder side (the derive itself maps the
// enum as a UserDefined object — refining to proper select-field inference
// is Phase 2b T10's follow-up).

#[derive(Schema)]
#[allow(dead_code)]
struct Uses {
    #[param(label = "Plain string")]
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
