//! Integration tests for the typed-closure builder DSL.

use nebula_schema::{
    Field, FieldCollector, FieldValues, GroupBuilder, InputHint, RequiredMode, Schema,
    StringWidget, VisibilityMode, field_key,
};
use nebula_validator::{Predicate, Rule};

fn eq_rule(path: &str, value: impl Into<serde_json::Value>) -> Rule {
    Rule::predicate(Predicate::eq(path, value).expect("valid path"))
}
use serde_json::json;

#[test]
fn closure_string_produces_equivalent_schema() {
    let via_closure = Schema::builder()
        .string(field_key!("name"), |s| {
            s.label("Name")
                .required()
                .min_length(1)
                .max_length(64)
                .hint(InputHint::Email)
        })
        .build()
        .unwrap();

    let via_direct = Schema::builder()
        .add(
            Field::string(field_key!("name"))
                .label("Name")
                .required()
                .min_length(1)
                .max_length(64)
                .hint(InputHint::Email),
        )
        .build()
        .unwrap();

    assert_eq!(via_closure, via_direct);
}

#[test]
fn closure_number_integer_flag_applied() {
    let schema = Schema::builder()
        .integer(field_key!("count"), |n| n.min(0_i64).max(100_i64))
        .build()
        .unwrap();
    match &schema.fields()[0] {
        Field::Number(n) => {
            assert!(n.integer);
            assert!(n.rules.len() >= 2);
        },
        other => panic!("expected NumberField, got {other:?}"),
    }
}

#[test]
fn closure_boolean_chainable() {
    let schema = Schema::builder()
        .boolean(field_key!("flag"), |b| b.label("Flag").no_expression())
        .build()
        .unwrap();
    match &schema.fields()[0] {
        Field::Boolean(b) => {
            assert_eq!(b.label.as_deref(), Some("Flag"));
            assert!(matches!(
                b.expression,
                nebula_schema::ExpressionMode::Forbidden
            ));
        },
        other => panic!("expected BooleanField, got {other:?}"),
    }
}

#[test]
fn closure_nested_object_holds_children() {
    let schema = Schema::builder()
        .object(field_key!("user"), |o| {
            o.label("User")
                .string(field_key!("name"), nebula_schema::StringBuilder::required)
                .number(field_key!("age"), |n| n.integer().min(0_i64))
        })
        .build()
        .unwrap();

    match &schema.fields()[0] {
        Field::Object(obj) => {
            assert_eq!(obj.fields.len(), 2);
            assert_eq!(obj.fields[0].key().as_str(), "name");
            assert_eq!(obj.fields[1].key().as_str(), "age");
        },
        other => panic!("expected ObjectField, got {other:?}"),
    }
}

#[test]
fn closure_list_item_via_closure() {
    let schema = Schema::builder()
        .list(field_key!("tags"), |l| {
            l.min_items(1)
                .max_items(10)
                .item_string(field_key!("entry"), |s| s.max_length(32))
        })
        .build()
        .unwrap();

    match &schema.fields()[0] {
        Field::List(list) => {
            assert_eq!(list.min_items, Some(1));
            assert_eq!(list.max_items, Some(10));
            assert!(list.item.is_some());
            match list.item.as_deref().unwrap() {
                Field::String(s) => assert_eq!(s.key.as_str(), "entry"),
                other => panic!("expected String item, got {other:?}"),
            }
        },
        other => panic!("expected ListField, got {other:?}"),
    }
}

#[test]
fn builder_full_example_from_spec() {
    let schema = Schema::builder()
        .string(field_key!("url"), |s| {
            s.label("URL")
                .hint(InputHint::Url)
                .required()
                .max_length(8192)
        })
        .integer(field_key!("timeout"), |n| {
            n.label("Timeout (s)").min(1_i64).max(300_i64)
        })
        .boolean(
            field_key!("verbose"),
            nebula_schema::BooleanBuilder::no_expression,
        )
        .build()
        .unwrap();

    assert_eq!(schema.fields().len(), 3);
    let values =
        FieldValues::from_json(json!({ "url": "https://x.test/", "timeout": 5, "verbose": false }))
            .unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn group_propagates_visible_when_to_children() {
    let rule = eq_rule("method", "POST");
    let schema = Schema::builder()
        .string(field_key!("method"), nebula_schema::StringBuilder::required)
        .group("body_section", |g| {
            g.visible_when(rule.clone())
                .string(field_key!("body"), |s| s.widget(StringWidget::Multiline))
                .integer(field_key!("content_length"), |n| n)
        })
        .build()
        .unwrap();

    // Group children are flattened into the top-level schema fields.
    assert_eq!(schema.fields().len(), 3);
    // First is the ungrouped `method`; the next two are grouped.
    let body = &schema.fields()[1];
    let content_length = &schema.fields()[2];

    match body {
        Field::String(s) => {
            assert_eq!(s.group.as_deref(), Some("body_section"));
            assert!(matches!(s.visible, VisibilityMode::When(_)));
        },
        other => panic!("expected grouped StringField, got {other:?}"),
    }
    match content_length {
        Field::Number(n) => {
            assert_eq!(n.group.as_deref(), Some("body_section"));
            assert!(matches!(n.visible, VisibilityMode::When(_)));
        },
        other => panic!("expected grouped NumberField, got {other:?}"),
    }
}

#[test]
fn group_propagates_required_when_to_children() {
    let rule = eq_rule("enabled", true);
    let schema = Schema::builder()
        .boolean(field_key!("enabled"), |b| b)
        .group("details", |g| {
            g.required_when(rule.clone())
                .string(field_key!("detail_a"), |s| s)
                .string(field_key!("detail_b"), |s| s)
        })
        .build()
        .unwrap();

    for f in schema.fields().iter().skip(1) {
        match f {
            Field::String(s) => {
                assert_eq!(s.group.as_deref(), Some("details"));
                assert!(matches!(s.required, RequiredMode::When(_)));
            },
            other => panic!("unexpected field: {other:?}"),
        }
    }
}

#[test]
fn group_composes_existing_child_visible_when() {
    let group_rule = eq_rule("section", "A");
    let child_rule = eq_rule("mode", "advanced");
    let schema = Schema::builder()
        .string(field_key!("section"), |s| s)
        .string(field_key!("mode"), |s| s)
        .group("g", |g| {
            g.visible_when(group_rule.clone())
                .string(field_key!("x"), |s| s.visible_when(child_rule.clone()))
        })
        .build()
        .unwrap();

    // The grouped `x` is the third field (after `section` and `mode`).
    match &schema.fields()[2] {
        Field::String(s) => match &s.visible {
            VisibilityMode::When(rule) => {
                // The composed rule must mention both field paths.
                let debug = format!("{rule:?}");
                assert!(debug.contains("section"));
                assert!(debug.contains("mode"));
            },
            other => panic!("expected composed visible_when, got {other:?}"),
        },
        other => panic!("expected StringField, got {other:?}"),
    }
}

#[test]
fn group_builder_name_accessor() {
    let g = GroupBuilder::new("g-label").visible_when(eq_rule("x", "y"));
    assert_eq!(g.name(), "g-label");
}

#[test]
fn group_required_when_composes_with_always_and_never_children() {
    let rule = eq_rule("enabled", true);
    let schema = Schema::builder()
        .boolean(field_key!("enabled"), |b| b)
        // A field declared `.required()` before the group applies its
        // `required_when` must stay `Always` (compose_required's
        // `Always` branch).
        .group("details", |g| {
            g.required_when(rule.clone())
                .string(field_key!("always_required"), nebula_schema::StringBuilder::required)
                .string(field_key!("optional_by_default"), |s| s)
        })
        .build()
        .unwrap();

    match &schema.fields()[1] {
        Field::String(s) => {
            assert!(
                matches!(s.required, RequiredMode::Always),
                "an explicitly-required child must stay Always after group compose"
            );
        },
        other => panic!("expected StringField, got {other:?}"),
    }
    match &schema.fields()[2] {
        Field::String(s) => {
            assert!(
                matches!(s.required, RequiredMode::When(_)),
                "an optional child must flip to When(..) after group compose"
            );
        },
        other => panic!("expected StringField, got {other:?}"),
    }
}

#[test]
fn group_preserves_explicit_child_group_label() {
    let schema = Schema::builder()
        .group("outer", |g| {
            g.string(field_key!("inherits"), |s| s)
                .string(field_key!("overrides"), |s| s.group("inner"))
        })
        .build()
        .unwrap();

    // Child without an explicit `.group(..)` inherits the group label.
    match &schema.fields()[0] {
        Field::String(s) => assert_eq!(s.group.as_deref(), Some("outer")),
        other => panic!("expected StringField, got {other:?}"),
    }
    // Child with its own `.group("inner")` is preserved — `set_group`'s
    // None-guard must not overwrite it.
    match &schema.fields()[1] {
        Field::String(s) => assert_eq!(s.group.as_deref(), Some("inner")),
        other => panic!("expected StringField, got {other:?}"),
    }
}
