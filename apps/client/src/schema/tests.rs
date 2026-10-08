use super::*;
use serde_json::json;

/// Every field type and every widget the schema crate declares, in its wire format.
pub(crate) fn every_field() -> Value {
    serde_json::from_str(include_str!("every_field.json")).unwrap()
}

fn field<'a>(form: &'a Form, key: &str) -> &'a Field {
    form.fields.iter().find(|field| field.key == key).unwrap()
}

#[test]
fn every_field_type_and_widget_is_read() {
    let form = Form::parse(&every_field());
    assert_eq!(form.fields.len(), 44);
    assert!(matches!(
        field(&form, "intro").kind,
        Kind::Notice {
            severity: Severity::Info
        }
    ));
    assert!(matches!(
        field(&form, "notes").kind,
        Kind::Text {
            hint: Hint::Markdown,
            multiline: true
        }
    ));
    assert!(matches!(
        field(&form, "certificate").kind,
        Kind::Secret { multiline: true }
    ));
    assert!(matches!(
        field(&form, "ratio").kind,
        Kind::Number {
            widget: NumberWidget::Slider,
            ..
        }
    ));
    assert!(matches!(
        field(&form, "limit").kind,
        Kind::Number {
            widget: NumberWidget::Bytes,
            ..
        }
    ));
    assert!(matches!(
        field(&form, "strict").kind,
        Kind::Boolean {
            widget: BooleanWidget::Radio
        }
    ));
    assert!(matches!(
        field(&form, "labels").kind,
        Kind::Select {
            widget: SelectWidget::Tags,
            multiple: true,
            ..
        }
    ));
    assert!(matches!(
        field(&form, "sheet").kind,
        Kind::Select {
            loader: Some(_),
            ..
        }
    ));
    assert!(matches!(
        field(&form, "pages").kind,
        Kind::Object {
            widget: ObjectWidget::Tabs,
            ..
        }
    ));
    assert!(matches!(
        field(&form, "headers").kind,
        Kind::List {
            widget: ListWidget::KeyValue,
            ..
        }
    ));
    assert!(matches!(
        field(&form, "query").kind,
        Kind::Code { simple: true, .. }
    ));
    assert!(matches!(
        field(&form, "attachment").kind,
        Kind::File {
            multiple: false,
            ..
        }
    ));
    assert_eq!(field(&form, "total").expression, ExpressionMode::Required);
    assert!(matches!(field(&form, "columns").kind, Kind::Dynamic { .. }));
    match &field(&form, "auth").kind {
        Kind::Mode {
            variants,
            default_variant,
        } => {
            assert_eq!(variants.len(), 2);
            assert_eq!(default_variant.as_deref(), Some("none"));
        },
        other => panic!("auth is a mode field, not {other:?}"),
    }
}

#[test]
fn an_unknown_field_type_stays_editable_instead_of_failing_the_form() {
    let form = Form::parse(&every_field());
    assert_eq!(
        field(&form, "future").kind,
        Kind::Unknown {
            type_name: "hologram".into()
        }
    );
}

#[test]
fn rules_become_bounds_for_controls_and_checks() {
    let form = Form::parse(&every_field());
    let count = field(&form, "count");
    assert_eq!(
        (count.bounds.min, count.bounds.max),
        (Some(1.0), Some(10.0))
    );
    assert_eq!(
        count.problem(&json!(11), false).as_deref(),
        Some("At most 10.")
    );
    assert_eq!(
        count.problem(&json!(2.5), false).as_deref(),
        Some("Use a whole number.")
    );
    assert_eq!(count.problem(&json!(4), false), None);

    let name = field(&form, "name");
    assert_eq!(name.problem(&json!(""), true).as_deref(), Some("Required."));
    assert_eq!(
        name.problem(&json!("a"), true).as_deref(),
        Some("At least 2 characters.")
    );
}

/// What `nebula-schema` puts on the wire, read by the client.
fn served(schema: &nebula_schema::ValidSchema) -> Form {
    Form::parse(&serde_json::to_value(schema).unwrap())
}

#[test]
fn the_reader_matches_what_nebula_schema_serializes() {
    use nebula_schema::prelude::*;

    let schema = Schema::builder()
        .property(Property::boolean(field_key!("enabled")))
        .property(
            Property::boolean(field_key!("computed_flag")).expression_mode(ExpressionMode::Allowed),
        )
        .property(
            Property::select(field_key!("mode"))
                .option("a", "A")
                .option("b", "B"),
        )
        .property(Property::string(field_key!("contact")).with_rule(Rule::email()))
        .property(
            Property::object(field_key!("extra")).property(
                Property::string(field_key!("detail")).visible_when(
                    Rule::predicate(Predicate::eq("mode", json!("b")).unwrap()).unwrap(),
                ),
            ),
        )
        .build()
        .unwrap();
    let form = served(&schema);
    // A boolean forbids expressions unless its author allows them. `allowed` is the wire default and
    // is left off, so an absent mode must read as allowed.
    assert_eq!(
        field(&form, "enabled").expression,
        super::ExpressionMode::Forbidden
    );
    assert_eq!(
        field(&form, "computed_flag").expression,
        super::ExpressionMode::Allowed
    );
    // A rule without an argument travels as a bare name.
    assert!(field(&form, "contact").bounds.email);
    assert!(!form.free_form);
    // A nested field's condition names a path from the root of the node's values.
    let detail = match &field(&form, "extra").kind {
        Kind::Object { fields, .. } => fields[0].clone(),
        other => panic!("extra is an object, not {other:?}"),
    };
    let entries = |mode: &str| {
        Values::of(
            &form,
            json!({"mode": {"type": "literal", "value": mode}})
                .as_object()
                .unwrap(),
        )
    };
    assert!(detail.is_visible(&entries("b")));
    assert!(!detail.is_visible(&entries("a")));
}

#[test]
fn a_root_without_fields_is_free_form() {
    assert!(served(&nebula_schema::ValidSchema::any()).free_form);
    assert!(!Form::parse(&json!({"fields": []})).free_form);
}

#[test]
fn conditions_treat_missing_and_run_time_values_as_the_server_does() {
    let form = Form::parse(&json!({"fields": [
        {"type": "string", "key": "kind"},
        {"type": "number", "key": "size"},
        {"type": "string", "key": "plan", "default": "pro"}
    ]}));
    let values = |entries: Value| Values::of(&form, entries.as_object().unwrap());

    let none = values(json!({}));
    // A missing value: `ne` and `empty` hold, every other predicate fails, `null` included.
    assert_eq!(holds(&json!({"ne": ["/kind", "a"]}), &none), Some(true));
    assert_eq!(holds(&json!({"empty": "/kind"}), &none), Some(true));
    assert_eq!(holds(&json!({"eq": ["/kind", null]}), &none), Some(false));
    assert_eq!(holds(&json!({"gt": ["/size", 1]}), &none), Some(false));
    assert_eq!(holds(&json!({"in": ["/kind", [null]]}), &none), Some(false));
    // An absent parameter reads as its declared default.
    assert_eq!(holds(&json!({"eq": ["/plan", "pro"]}), &none), Some(true));

    // An expression is only known when the node runs, so the client cannot judge the condition.
    let pending = values(json!({"kind": {"type": "expression", "expr": "{{ $input.kind }}"}}));
    assert_eq!(holds(&json!({"eq": ["/kind", "a"]}), &pending), None);
    assert_eq!(holds(&json!({"empty": "/kind"}), &pending), None);
}

#[test]
fn hints_advise_on_the_shape_of_text_without_calling_it_an_error() {
    let form = Form::parse(&every_field());
    let contact = field(&form, "contact");
    // A hint only guides rendering on the server, so a mismatch is advice, not a problem.
    assert!(contact.problem(&json!("nobody"), false).is_none());
    assert!(contact.advice(&json!("nobody")).is_some());
    assert!(contact.advice(&json!("a@b.io")).is_none());
    assert!(field(&form, "site").advice(&json!("example.com")).is_some());
    assert!(
        field(&form, "starts")
            .advice(&json!("2026-10-08"))
            .is_none()
    );
    assert!(field(&form, "starts").advice(&json!("8 Oct")).is_some());
    // An empty field gets no advice; there is nothing typed to look at.
    assert!(contact.advice(&json!("")).is_none());
}

#[test]
fn an_email_rule_is_a_problem() {
    let form = Form::parse(&json!({"fields": [
        {"type": "string", "key": "to", "rules": ["email", {"described": [{"min_length": 3}, "Too short"]}]}
    ]}));
    let to = &form.fields[0];
    assert_eq!(to.bounds.min_length, Some(3));
    assert!(to.problem(&json!("nobody@nowhere"), false).is_some());
    assert!(to.problem(&json!("a@b.io"), false).is_none());
}

#[test]
fn list_bounds_count_items() {
    let form = Form::parse(&every_field());
    let recipients = field(&form, "recipients");
    assert_eq!(
        recipients
            .problem(&json!(["a@b.io", "c@d.io", "e@f.io", "g@h.io"]), false)
            .as_deref(),
        Some("Keep at most 3 items.")
    );
}

/// Fixed values for conditions, as literal parameter entries.
fn literals(form: &Form, values: Value) -> Values {
    let entries: Map<String, Value> = values
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.clone(), json!({"type": "literal", "value": value})))
        .collect();
    Values::of(form, &entries)
}

#[test]
fn a_conditional_field_follows_its_sibling() {
    let form = Form::parse(&every_field());
    let api_key = field(&form, "api_key");
    let get = literals(&form, json!({"method": "GET"}));
    assert!(!api_key.is_visible(&get));
    assert!(!api_key.is_required(&get));

    let post = literals(&form, json!({"method": "POST"}));
    assert!(api_key.is_visible(&post));
    assert!(api_key.is_required(&post));
}

#[test]
fn rule_logic_combines_and_unknown_rules_are_undecided() {
    let values = literals(
        &Form::default(),
        json!({"kind": "a", "on": true, "auth": {"mode": "basic"}}),
    );
    let all = json!({"all": [{"eq": ["/kind", "a"]}, {"is_true": "/on"}]});
    assert_eq!(holds(&all, &values), Some(true));
    let not = json!({"not": {"in": ["/kind", ["b", "c"]]}});
    assert_eq!(holds(&not, &values), Some(true));
    assert_eq!(
        holds(&json!({"eq": ["/auth/mode", "basic"]}), &values),
        Some(true)
    );
    assert_eq!(holds(&json!({"matches": ["/kind", "^a$"]}), &values), None);
}

#[test]
fn an_undecided_condition_shows_the_field() {
    let form = Form::parse(&json!({"fields": [
        {"type": "string", "key": "x", "visible": {"kind": "when", "matches": ["/y", "^z"]}}
    ]}));
    assert!(form.fields[0].is_visible(&Values::default()));
}

#[test]
fn initial_values_follow_defaults_and_shapes() {
    let form = Form::parse(&every_field());
    // Without a declared default a choice and a number start empty instead of guessing.
    assert_eq!(field(&form, "method").initial(), Value::Null);
    assert_eq!(field(&form, "count").initial(), Value::Null);
    assert_eq!(field(&form, "extras").initial(), json!({}));
    assert_eq!(field(&form, "days").initial(), json!([]));
    // An object starts with its declared defaults only; other keys are written when filled in.
    assert_eq!(field(&form, "address").initial(), json!({}));
    let with_defaults = Form::parse(
        &json!({"fields": [{"type": "object", "key": "o", "fields": [
            {"type": "string", "key": "sep", "default": "."},
            {"type": "string", "key": "from"}
        ]}]}),
    );
    assert_eq!(with_defaults.fields[0].initial(), json!({"sep": "."}));
    assert_eq!(
        field(&form, "auth").initial(),
        json!({"mode": "none", "value": ""})
    );
    let with_default =
        Form::parse(&json!({"fields": [{"type": "number", "key": "n", "default": 5}]}));
    assert_eq!(with_default.fields[0].initial(), json!(5));
}

#[test]
fn a_field_without_a_label_reads_its_key_as_words() {
    let form = Form::parse(&every_field());
    assert_eq!(field(&form, "api_key").title(), "Api key");
    assert_eq!(field(&form, "name").title(), "Name");
}

#[test]
fn entries_without_a_key_are_skipped() {
    let form =
        Form::parse(&json!({"fields": [{"type": "string"}, 7, {"type": "string", "key": "ok"}]}));
    assert_eq!(form.fields.len(), 1);
}
