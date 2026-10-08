//! Reads the JSON Schema projection the server publishes for credential types
//! (`GET /credentials/types`) into the same [`Form`] model as an action's parameters, so one
//! renderer draws both. The projection carries `nebula-schema`'s field kinds as
//! `x-nebula-field-kind` and the value's own schema as `x-nebula-resolved-value-schema`. A root
//! `oneOf` of single-property objects is a union tagged by that property, such as OAuth2's
//! `{"authorization_code": {...}}`; it becomes one mode field whose variants are the tags.

use super::{
    Bounds, Choice, Condition, ExpressionMode, Field, Form, Hint, Kind, ListWidget, NumberWidget,
    ObjectWidget, SelectWidget, Variant, humanize, text, word,
};
use serde_json::Value;

/// Key of the mode field that stands for a tagged union root.
pub(crate) const UNION_FIELD: &str = "type";

impl Form {
    pub(crate) fn from_json_schema(schema: &Value) -> Self {
        if let Some(variants) = schema["oneOf"].as_array() {
            let variants: Vec<Variant> = variants
                .iter()
                .filter_map(|variant| {
                    let properties = variant["properties"].as_object()?;
                    let (tag, property) =
                        properties.iter().next().filter(|_| properties.len() == 1)?;
                    Some(Variant {
                        key: tag.clone(),
                        label: humanize(tag),
                        field: property_field(tag, property, true),
                    })
                })
                .collect();
            // `oneOf` names no default, so the person picks the type.
            return Self {
                fields: vec![Field {
                    key: UNION_FIELD.to_owned(),
                    label: Some("Type".to_owned()),
                    description: None,
                    placeholder: None,
                    default: None,
                    visible: Condition::Always,
                    required: Condition::Always,
                    expression: ExpressionMode::Forbidden,
                    group: None,
                    bounds: Bounds::default(),
                    kind: Kind::Mode {
                        variants,
                        default_variant: None,
                    },
                }],
                any_value: false,
                tagged_union: true,
            };
        }
        Self {
            fields: object_fields(schema),
            any_value: false,
            tagged_union: false,
        }
    }
}

/// The properties of an object schema as fields: required ones first, and within each group plain
/// fields before secrets, so a username comes before its password.
fn object_fields(schema: &Value) -> Vec<Field> {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut fields: Vec<Field> = schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, property)| property_field(key, property, required.contains(&key.as_str())))
        .collect();
    fields.sort_by_key(|field| {
        (
            field.required != Condition::Always,
            matches!(field.kind, Kind::Secret { .. }),
        )
    });
    fields
}

/// The value's own schema: the projection's resolved schema, or the property itself.
fn value_schema(property: &Value) -> &Value {
    property
        .get("x-nebula-resolved-value-schema")
        .unwrap_or(property)
}

fn property_field(key: &str, property: &Value, required: bool) -> Field {
    let value = value_schema(property);
    let kind = match word(property, "x-nebula-field-kind") {
        "secret" => Kind::Secret { multiline: false },
        "number" => Kind::Number {
            integer: value["type"] == "integer",
            widget: NumberWidget::Plain,
            step: None,
        },
        "boolean" => Kind::Boolean {
            widget: super::BooleanWidget::Toggle,
        },
        "select" => Kind::Select {
            options: choices(value),
            multiple: property["x-nebula-select-multiple"]
                .as_bool()
                .unwrap_or(false),
            allow_custom: property["x-nebula-select-allow-custom"]
                .as_bool()
                .unwrap_or(false),
            searchable: false,
            widget: SelectWidget::Dropdown,
            loader: None,
        },
        "object" => Kind::Object {
            fields: object_fields(value),
            widget: ObjectWidget::Inline,
        },
        "list" => Kind::List {
            item: value
                .get("items")
                .map(|items| Box::new(property_field("item", items, false))),
            min_items: value["minItems"].as_u64(),
            max_items: value["maxItems"].as_u64(),
            unique: value["uniqueItems"].as_bool().unwrap_or(false),
            widget: ListWidget::Plain,
        },
        "string" | "" => Kind::Text {
            hint: match word(value, "format") {
                "uri" => Hint::Url,
                "email" => Hint::Email,
                "date-time" => Hint::DateTime,
                _ => Hint::Text,
            },
            multiline: false,
        },
        other => Kind::Unknown {
            type_name: other.to_owned(),
        },
    };
    Field {
        key: key.to_owned(),
        label: text(&property["title"]),
        description: text(&property["description"]),
        placeholder: None,
        default: property
            .get("default")
            .filter(|default| !default.is_null())
            .cloned(),
        visible: Condition::Always,
        required: if required {
            Condition::Always
        } else {
            Condition::Never
        },
        expression: match word(property, "x-nebula-expression-mode") {
            "allowed" => ExpressionMode::Allowed,
            "required" => ExpressionMode::Required,
            _ => ExpressionMode::Forbidden,
        },
        group: None,
        bounds: Bounds {
            min: value["minimum"].as_f64(),
            max: value["maximum"].as_f64(),
            min_length: value["minLength"].as_u64(),
            max_length: value["maxLength"].as_u64(),
            email: value["format"] == "email",
            url: value["format"] == "uri",
        },
        kind,
    }
}

/// `anyOf` / `oneOf` of `{"const": ..., "title": ...}`, or a plain `enum`.
fn choices(value: &Value) -> Vec<Choice> {
    let constants = value["anyOf"]
        .as_array()
        .or_else(|| value["oneOf"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|option| {
            let constant = option.get("const")?.clone();
            Some(Choice {
                label: text(&option["title"]).unwrap_or_else(|| super::display(&constant)),
                value: constant,
                description: text(&option["description"]),
                disabled: false,
            })
        });
    let enumerated = value["enum"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|constant| Choice {
            label: super::display(constant),
            value: constant.clone(),
            description: None,
            disabled: false,
        });
    constants.chain(enumerated).collect()
}

/// Turns the form's parameter entries into a credential's `data`: literals as written, expressions
/// as `{"$expr": ...}`, and a tagged union's mode back into `{"tag": payload}`.
pub(crate) fn credential_data(
    form: &Form,
    entries: &serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    let mut data = serde_json::Map::new();
    for (key, entry) in entries {
        let value = match entry["type"].as_str() {
            Some("literal") => entry["value"].clone(),
            Some("expression") => serde_json::json!({"$expr": entry["expr"]}),
            _ => continue,
        };
        if form.tagged_union && key == UNION_FIELD {
            if let Some(tag) = value["mode"].as_str() {
                data.insert(tag.to_owned(), value["value"].clone());
            }
            continue;
        }
        data.insert(key.clone(), value);
    }
    data
}
