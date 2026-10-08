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

#[test]
fn hints_check_the_shape_of_text() {
    let form = Form::parse(&every_field());
    assert!(
        field(&form, "contact")
            .problem(&json!("nobody"), false)
            .is_some()
    );
    assert!(
        field(&form, "contact")
            .problem(&json!("a@b.io"), false)
            .is_none()
    );
    assert!(
        field(&form, "site")
            .problem(&json!("example.com"), false)
            .is_some()
    );
    assert!(
        field(&form, "starts")
            .problem(&json!("2026-10-08"), false)
            .is_none()
    );
    assert!(
        field(&form, "starts")
            .problem(&json!("8 Oct"), false)
            .is_some()
    );
    // An empty optional field is not a problem; the hint only checks what was typed.
    assert!(field(&form, "contact").problem(&json!(""), false).is_none());
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

#[test]
fn a_conditional_field_follows_its_sibling() {
    let form = Form::parse(&every_field());
    let api_key = field(&form, "api_key");
    let mut siblings = Map::new();
    siblings.insert("method".into(), json!("GET"));
    assert!(!api_key.is_visible(&siblings));
    assert!(!api_key.is_required(&siblings));

    siblings.insert("method".into(), json!("POST"));
    assert!(api_key.is_visible(&siblings));
    assert!(api_key.is_required(&siblings));
}

#[test]
fn rule_logic_combines_and_unknown_rules_are_undecided() {
    let mut siblings = Map::new();
    siblings.insert("kind".into(), json!("a"));
    siblings.insert("on".into(), json!(true));
    siblings.insert("auth".into(), json!({"mode": "basic"}));
    let all = json!({"all": [{"eq": ["/kind", "a"]}, {"is_true": "/on"}]});
    assert_eq!(holds(&all, &siblings), Some(true));
    let not = json!({"not": {"in": ["/kind", ["b", "c"]]}});
    assert_eq!(holds(&not, &siblings), Some(true));
    assert_eq!(
        holds(&json!({"eq": ["/auth/mode", "basic"]}), &siblings),
        Some(true)
    );
    assert_eq!(
        holds(&json!({"matches": ["/kind", "^a$"]}), &siblings),
        None
    );
}

#[test]
fn an_undecided_condition_shows_the_field() {
    let form = Form::parse(&json!({"fields": [
        {"type": "string", "key": "x", "visible": {"kind": "when", "matches": ["/y", "^z"]}}
    ]}));
    assert!(form.fields[0].is_visible(&Map::new()));
}

#[test]
fn initial_values_follow_defaults_and_shapes() {
    let form = Form::parse(&every_field());
    assert_eq!(field(&form, "method").initial(), json!("GET"));
    assert_eq!(field(&form, "days").initial(), json!([]));
    assert_eq!(
        field(&form, "address").initial(),
        json!({"city": "", "zip": ""})
    );
    assert_eq!(
        field(&form, "auth").initial(),
        json!({"mode": "none", "value": ""})
    );
    let with_default =
        Form::parse(&json!({"fields": [{"type": "number", "key": "n", "default": 5}]}));
    assert_eq!(with_default.fields[0].initial(), json!(5));
}

#[test]
fn entries_without_a_key_are_skipped() {
    let form =
        Form::parse(&json!({"fields": [{"type": "string"}, 7, {"type": "string", "key": "ok"}]}));
    assert_eq!(form.fields.len(), 1);
}
