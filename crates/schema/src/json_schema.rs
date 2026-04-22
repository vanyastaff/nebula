//! JSON Schema export for [`crate::ValidSchema`] (feature: `schemars`).
//!
//! This module provides a pragmatic Phase-4 baseline mapper from Nebula's
//! `Field` model to JSON Schema Draft 2020-12.

#![cfg(feature = "schemars")]

use std::{error::Error as StdError, fmt};

use serde_json::{Map, Value};

use crate::{
    field::{ComputedReturn, Field, ListField, ModeField, NumberField, ObjectField, SelectField},
    mode::{ExpressionMode, RequiredMode, VisibilityMode},
};

/// Canonical draft URI emitted by [`ValidSchema::json_schema`].
const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// Error produced while exporting [`crate::validated::ValidSchema`] to JSON Schema.
#[derive(Debug)]
pub enum JsonSchemaExportError {
    /// Failed to serialize a root-level rule into JSON.
    RootRuleSerialization {
        /// Index of the root rule in `ValidSchema::root_rules()`.
        index: usize,
        /// Serialization error emitted by `serde_json`.
        source: serde_json::Error,
    },
    /// Constructed JSON payload is rejected by `schemars::Schema`.
    InvalidSchema(serde_json::Error),
}

impl fmt::Display for JsonSchemaExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootRuleSerialization { index, source } => {
                write!(
                    f,
                    "failed to serialize root rule at index {index}: {source}"
                )
            },
            Self::InvalidSchema(source) => {
                write!(f, "failed to construct JSON Schema document: {source}")
            },
        }
    }
}

impl StdError for JsonSchemaExportError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::RootRuleSerialization { source, .. } => Some(source),
            Self::InvalidSchema(source) => Some(source),
        }
    }
}

impl crate::validated::ValidSchema {
    /// Export this validated schema as JSON Schema (Draft 2020-12).
    ///
    /// The export is intentionally structural:
    /// - field shape and basic constraints are mapped
    /// - dynamic runtime semantics (loaders, deferred rules, expression runtime) are not fully
    ///   representable and are omitted from strict constraints
    pub fn json_schema(&self) -> Result<schemars::Schema, JsonSchemaExportError> {
        schema_for_fields(self.fields(), self.root_rules())
    }
}

fn schema_for_fields(
    fields: &[Field],
    root_rules: &[nebula_validator::Rule],
) -> Result<schemars::Schema, JsonSchemaExportError> {
    let mut root = Map::new();
    root.insert(
        "$schema".to_owned(),
        Value::String(DRAFT_2020_12.to_owned()),
    );
    root.insert("type".to_owned(), Value::String("object".to_owned()));
    root.insert(
        "properties".to_owned(),
        Value::Object(properties_for_fields(fields)),
    );

    let required = required_for_fields(fields);
    if !required.is_empty() {
        root.insert(
            "required".to_owned(),
            Value::Array(required.into_iter().map(Value::String).collect()),
        );
    }
    if !root_rules.is_empty() {
        let mut serialized = Vec::with_capacity(root_rules.len());
        for (index, rule) in root_rules.iter().enumerate() {
            let value = serde_json::to_value(rule)
                .map_err(|source| JsonSchemaExportError::RootRuleSerialization { index, source })?;
            serialized.push(value);
        }
        root.insert("x-nebula-root-rules".to_owned(), Value::Array(serialized));
    }
    root.insert("additionalProperties".to_owned(), Value::Bool(false));

    schemars::Schema::try_from(Value::Object(root)).map_err(JsonSchemaExportError::InvalidSchema)
}

fn properties_for_fields(fields: &[Field]) -> Map<String, Value> {
    let mut out = Map::with_capacity(fields.len());
    for field in fields {
        out.insert(field.key().as_str().to_owned(), field_schema_value(field));
    }
    out
}

fn required_for_fields(fields: &[Field]) -> Vec<String> {
    fields
        .iter()
        .filter(|field| matches!(field.required(), RequiredMode::Always))
        .map(|field| field.key().as_str().to_owned())
        .collect()
}

fn field_schema_value(field: &Field) -> Value {
    let core_schema = match field {
        Field::String(f) => {
            let mut s = string_like_schema();
            apply_value_rules(&mut s, &f.rules);
            s
        },
        Field::Secret(f) => {
            let mut s = string_like_schema();
            s.insert("writeOnly".to_owned(), Value::Bool(true));
            apply_value_rules(&mut s, &f.rules);
            s
        },
        Field::Code(f) => {
            let mut s = string_like_schema();
            apply_value_rules(&mut s, &f.rules);
            s
        },
        Field::Number(f) => {
            let mut s = number_schema(f);
            apply_value_rules(&mut s, &f.rules);
            s
        },
        Field::Boolean(_) => primitive_schema("boolean"),
        Field::Select(f) => select_schema(f),
        Field::Object(f) => {
            let mut s = object_schema(f);
            apply_value_rules(&mut s, &f.rules);
            s
        },
        Field::List(f) => list_schema(f),
        Field::Mode(f) => mode_schema(f),
        Field::File(f) => file_schema(f.multiple),
        Field::Computed(f) => computed_schema(f.returns),
        // Runtime-only payload from loader; keep intentionally permissive.
        Field::Dynamic(_) => Map::new(),
        // Display-only field in UI forms; no value contract.
        Field::Notice(_) => {
            let mut s = Map::new();
            s.insert("readOnly".to_owned(), Value::Bool(true));
            s
        },
    };

    let mut schema = apply_expression_mode(core_schema, field.expression());
    apply_common_keywords(field, &mut schema);
    apply_contract_keywords(field, &mut schema);
    Value::Object(schema)
}

fn apply_common_keywords(field: &Field, schema: &mut Map<String, Value>) {
    let (label, description, default) = match field {
        Field::String(f) => (&f.label, &f.description, &f.default),
        Field::Secret(f) => (&f.label, &f.description, &f.default),
        Field::Number(f) => (&f.label, &f.description, &f.default),
        Field::Boolean(f) => (&f.label, &f.description, &f.default),
        Field::Select(f) => (&f.label, &f.description, &f.default),
        Field::Object(f) => (&f.label, &f.description, &f.default),
        Field::List(f) => (&f.label, &f.description, &f.default),
        Field::Mode(f) => (&f.label, &f.description, &f.default),
        Field::Code(f) => (&f.label, &f.description, &f.default),
        Field::File(f) => (&f.label, &f.description, &f.default),
        Field::Computed(f) => (&f.label, &f.description, &f.default),
        Field::Dynamic(f) => (&f.label, &f.description, &f.default),
        Field::Notice(f) => (&f.label, &f.description, &f.default),
    };

    if let Some(title) = label {
        schema.insert("title".to_owned(), Value::String(title.clone()));
    }
    if let Some(desc) = description {
        schema.insert("description".to_owned(), Value::String(desc.clone()));
    }
    if let Some(default) = default {
        schema.insert("default".to_owned(), default.clone());
    }
}

fn primitive_schema(kind: &str) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("type".to_owned(), Value::String(kind.to_owned()));
    out
}

fn string_like_schema() -> Map<String, Value> {
    primitive_schema("string")
}

fn number_schema(field: &NumberField) -> Map<String, Value> {
    primitive_schema(if field.integer { "integer" } else { "number" })
}

fn select_schema(field: &SelectField) -> Map<String, Value> {
    let mut out = Map::new();
    if field.multiple {
        out.insert("type".to_owned(), Value::String("array".to_owned()));
        out.insert("items".to_owned(), select_item_schema(field));
        apply_value_rules(&mut out, &field.rules);
    } else {
        out.extend(select_item_schema_map(field));
        apply_value_rules(&mut out, &field.rules);
    }
    out
}

fn select_item_schema(field: &SelectField) -> Value {
    Value::Object(select_item_schema_map(field))
}

fn select_item_schema_map(field: &SelectField) -> Map<String, Value> {
    let mut item = Map::new();
    if !field.options.is_empty() && !field.allow_custom {
        item.insert(
            "oneOf".to_owned(),
            Value::Array(
                field
                    .options
                    .iter()
                    .map(|option| {
                        let mut o = Map::new();
                        o.insert("const".to_owned(), option.value.clone());
                        o.insert("title".to_owned(), Value::String(option.label.clone()));
                        if let Some(desc) = &option.description {
                            o.insert("description".to_owned(), Value::String(desc.clone()));
                        }
                        if option.disabled {
                            o.insert("x-nebula-disabled".to_owned(), Value::Bool(true));
                        }
                        Value::Object(o)
                    })
                    .collect(),
            ),
        );
    }
    item
}

fn object_schema(field: &ObjectField) -> Map<String, Value> {
    let mut out = primitive_schema("object");
    out.insert(
        "properties".to_owned(),
        Value::Object(properties_for_fields(&field.fields)),
    );
    let required = required_for_fields(&field.fields);
    if !required.is_empty() {
        out.insert(
            "required".to_owned(),
            Value::Array(required.into_iter().map(Value::String).collect()),
        );
    }
    out.insert("additionalProperties".to_owned(), Value::Bool(false));
    out
}

fn list_schema(field: &ListField) -> Map<String, Value> {
    let mut out = primitive_schema("array");
    if let Some(item) = &field.item {
        out.insert("items".to_owned(), field_schema_value(item.as_ref()));
    }
    if let Some(min) = field.min_items {
        out.insert("minItems".to_owned(), Value::from(min));
    }
    if let Some(max) = field.max_items {
        out.insert("maxItems".to_owned(), Value::from(max));
    }
    if field.unique {
        out.insert("uniqueItems".to_owned(), Value::Bool(true));
    }
    apply_value_rules(&mut out, &field.rules);
    out
}

fn mode_schema(field: &ModeField) -> Map<String, Value> {
    let mut out = Map::new();
    let mut branches = Vec::with_capacity(field.variants.len());
    for variant in &field.variants {
        let mut branch = primitive_schema("object");
        let mut props = Map::new();
        let mut required = vec![Value::String("mode".to_owned())];

        let mut mode_const = Map::new();
        mode_const.insert("const".to_owned(), Value::String(variant.key.clone()));
        props.insert("mode".to_owned(), Value::Object(mode_const));
        props.insert("value".to_owned(), field_schema_value(&variant.field));
        if matches!(variant.field.required(), RequiredMode::Always) {
            required.push(Value::String("value".to_owned()));
        }

        branch.insert("properties".to_owned(), Value::Object(props));
        branch.insert("required".to_owned(), Value::Array(required));
        branch.insert("additionalProperties".to_owned(), Value::Bool(false));
        branches.push(Value::Object(branch));
    }
    out.insert("oneOf".to_owned(), Value::Array(branches));
    out
}

fn file_schema(multiple: bool) -> Map<String, Value> {
    if multiple {
        let mut out = primitive_schema("array");
        out.insert("items".to_owned(), Value::Object(string_like_schema()));
        out
    } else {
        string_like_schema()
    }
}

fn computed_schema(returns: ComputedReturn) -> Map<String, Value> {
    match returns {
        ComputedReturn::String => primitive_schema("string"),
        ComputedReturn::Number => primitive_schema("number"),
        ComputedReturn::Boolean => primitive_schema("boolean"),
    }
}

fn apply_value_rules(schema: &mut Map<String, Value>, rules: &[nebula_validator::Rule]) {
    use nebula_validator::{Rule, ValueRule};
    for rule in rules {
        if let Rule::Value(v) = rule {
            match v {
                ValueRule::MinLength(n) => {
                    schema.insert("minLength".to_owned(), Value::from(*n));
                },
                ValueRule::MaxLength(n) => {
                    schema.insert("maxLength".to_owned(), Value::from(*n));
                },
                ValueRule::Pattern(pattern) => {
                    schema.insert("pattern".to_owned(), Value::String(pattern.clone()));
                },
                ValueRule::Email => {
                    schema.insert("format".to_owned(), Value::String("email".to_owned()));
                },
                ValueRule::Url => {
                    schema.insert("format".to_owned(), Value::String("uri".to_owned()));
                },
                ValueRule::Min(min) => {
                    schema.insert("minimum".to_owned(), Value::Number(min.clone()));
                },
                ValueRule::Max(max) => {
                    schema.insert("maximum".to_owned(), Value::Number(max.clone()));
                },
                ValueRule::GreaterThan(min) => {
                    schema.insert("exclusiveMinimum".to_owned(), Value::Number(min.clone()));
                },
                ValueRule::LessThan(max) => {
                    schema.insert("exclusiveMaximum".to_owned(), Value::Number(max.clone()));
                },
                ValueRule::OneOf(values) => {
                    schema.insert("enum".to_owned(), Value::Array(values.clone()));
                },
                ValueRule::MinItems(n) => {
                    schema.insert("minItems".to_owned(), Value::from(*n));
                },
                ValueRule::MaxItems(n) => {
                    schema.insert("maxItems".to_owned(), Value::from(*n));
                },
                _ => {},
            }
        }
    }
}

fn apply_expression_mode(
    mut core: Map<String, Value>,
    mode: &ExpressionMode,
) -> Map<String, Value> {
    let mut out = Map::new();
    match mode {
        ExpressionMode::Forbidden => core,
        ExpressionMode::Allowed => {
            out.insert(
                "anyOf".to_owned(),
                Value::Array(vec![
                    Value::Object(core.clone()),
                    Value::Object(expression_wrapper_schema()),
                ]),
            );
            out.insert(
                "x-nebula-resolved-value-schema".to_owned(),
                Value::Object(core.clone()),
            );
            out
        },
        ExpressionMode::Required => {
            let mut wrapper = expression_wrapper_schema();
            wrapper.insert(
                "x-nebula-resolved-value-schema".to_owned(),
                Value::Object(std::mem::take(&mut core)),
            );
            wrapper
        },
    }
}

fn expression_wrapper_schema() -> Map<String, Value> {
    let mut wrapper = primitive_schema("object");
    let mut properties = Map::new();
    properties.insert("$expr".to_owned(), Value::Object(string_like_schema()));
    wrapper.insert("properties".to_owned(), Value::Object(properties));
    wrapper.insert(
        "required".to_owned(),
        Value::Array(vec![Value::String("$expr".to_owned())]),
    );
    wrapper.insert("additionalProperties".to_owned(), Value::Bool(false));
    wrapper
}

fn apply_contract_keywords(field: &Field, schema: &mut Map<String, Value>) {
    schema.insert(
        "x-nebula-field-kind".to_owned(),
        Value::String(field.type_name().to_owned()),
    );

    schema.insert(
        "x-nebula-expression-mode".to_owned(),
        Value::String(
            match field.expression() {
                ExpressionMode::Forbidden => "forbidden",
                ExpressionMode::Allowed => "allowed",
                ExpressionMode::Required => "required",
            }
            .to_owned(),
        ),
    );
    schema.insert(
        "x-nebula-required-mode".to_owned(),
        Value::String(
            match field.required() {
                RequiredMode::Never => "never",
                RequiredMode::Always => "always",
                RequiredMode::When(_) => "when",
            }
            .to_owned(),
        ),
    );
    schema.insert(
        "x-nebula-visibility-mode".to_owned(),
        Value::String(
            match field.visible() {
                VisibilityMode::Always => "always",
                VisibilityMode::Never => "never",
                VisibilityMode::When(_) => "when",
            }
            .to_owned(),
        ),
    );

    if let Field::File(f) = field {
        if let Some(accept) = &f.accept {
            schema.insert(
                "x-nebula-file-accept".to_owned(),
                Value::String(accept.clone()),
            );
        }
        if let Some(max_size) = f.max_size {
            schema.insert("x-nebula-file-max-size".to_owned(), Value::from(max_size));
        }
    }
    if let Field::Select(f) = field {
        schema.insert("x-nebula-select-dynamic".to_owned(), Value::Bool(f.dynamic));
        schema.insert(
            "x-nebula-select-multiple".to_owned(),
            Value::Bool(f.multiple),
        );
        schema.insert(
            "x-nebula-select-allow-custom".to_owned(),
            Value::Bool(f.allow_custom),
        );
    }
    if let Field::Mode(f) = field {
        schema.insert(
            "x-nebula-mode-allow-dynamic".to_owned(),
            Value::Bool(f.allow_dynamic_mode),
        );
        if let Some(default_variant) = &f.default_variant {
            schema.insert(
                "x-nebula-mode-default-variant".to_owned(),
                Value::String(default_variant.clone()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::{Field, FieldKey, Schema};

    #[test]
    fn exports_basic_object_shape_and_required() {
        let schema = Schema::builder()
            .add(
                Field::string(FieldKey::new("name").expect("static key"))
                    .required()
                    .min_length(2),
            )
            .add(Field::secret(
                FieldKey::new("password").expect("static key"),
            ))
            .build()
            .expect("valid schema");

        let json = schema.json_schema().expect("json schema export").to_value();
        assert_eq!(
            json["$schema"],
            json!("https://json-schema.org/draft/2020-12/schema")
        );
        assert_eq!(json["type"], json!("object"));
        assert_eq!(json["required"], json!(["name"]));
        assert_eq!(
            json["properties"]["name"]["x-nebula-resolved-value-schema"]["type"],
            json!("string")
        );
        assert_eq!(
            json["properties"]["name"]["x-nebula-resolved-value-schema"]["minLength"],
            json!(2)
        );
        assert_eq!(
            json["properties"]["password"]["x-nebula-resolved-value-schema"]["type"],
            json!("string")
        );
        assert_eq!(
            json["properties"]["password"]["x-nebula-resolved-value-schema"]["writeOnly"],
            json!(true)
        );
        assert_eq!(
            json["properties"]["name"]["x-nebula-expression-mode"],
            json!("allowed")
        );
        assert_eq!(
            json["properties"]["name"]["x-nebula-required-mode"],
            json!("always")
        );
    }

    #[test]
    fn exports_mode_as_one_of_branches() {
        let schema = Schema::builder()
            .add(
                Field::mode(FieldKey::new("auth").expect("static key"))
                    .variant(
                        "none",
                        "None",
                        Field::notice(FieldKey::new("n").expect("static key")),
                    )
                    .variant(
                        "token",
                        "Token",
                        Field::secret(FieldKey::new("token").expect("static key")).required(),
                    ),
            )
            .build()
            .expect("valid schema");

        let json = schema.json_schema().expect("json schema export").to_value();
        let one_of = json["properties"]["auth"]["x-nebula-resolved-value-schema"]["oneOf"]
            .as_array()
            .expect("oneOf array");
        assert_eq!(one_of.len(), 2);
        assert!(
            one_of
                .iter()
                .any(|v| v["properties"]["mode"]["const"] == Value::String("none".to_owned()))
        );
        assert!(
            one_of
                .iter()
                .any(|v| v["properties"]["mode"]["const"] == Value::String("token".to_owned()))
        );
        let token = one_of
            .iter()
            .find(|v| v["properties"]["mode"]["const"] == Value::String("token".to_owned()))
            .expect("token branch exists");
        assert_eq!(token["required"], json!(["mode", "value"]));
    }

    #[test]
    fn exports_allowed_expression_mode_with_any_of() {
        let schema = Schema::builder()
            .add(Field::dynamic(
                FieldKey::new("runtime").expect("static key"),
            ))
            .build()
            .expect("valid schema");

        let json = schema.json_schema().expect("json schema export").to_value();
        assert!(json["properties"]["runtime"]["anyOf"].is_array());
        assert!(json["properties"]["runtime"]["oneOf"].is_null());
    }

    #[test]
    fn exports_number_rules_and_expression_wrapper_contract() {
        let schema = Schema::builder()
            .add(
                Field::number(FieldKey::new("count").expect("static key"))
                    .min(1)
                    .max(10)
                    .with_rule(nebula_validator::Rule::greater_than(2)),
            )
            .add(
                Field::computed(FieldKey::new("total").expect("static key"))
                    .returns(crate::field::ComputedReturn::Number),
            )
            .build()
            .expect("valid schema");

        let json = schema.json_schema().expect("json schema export").to_value();
        assert_eq!(
            json["properties"]["count"]["x-nebula-resolved-value-schema"]["type"],
            json!("number")
        );
        assert_eq!(
            json["properties"]["count"]["x-nebula-resolved-value-schema"]["minimum"],
            json!(1)
        );
        assert_eq!(
            json["properties"]["count"]["x-nebula-resolved-value-schema"]["maximum"],
            json!(10)
        );
        assert_eq!(
            json["properties"]["count"]["x-nebula-resolved-value-schema"]["exclusiveMinimum"],
            json!(2)
        );

        // Computed fields are ExpressionMode::Required -> wrapper schema.
        assert_eq!(json["properties"]["total"]["type"], json!("object"));
        assert_eq!(json["properties"]["total"]["required"], json!(["$expr"]));
        assert_eq!(
            json["properties"]["total"]["x-nebula-expression-mode"],
            json!("required")
        );
        assert_eq!(
            json["properties"]["total"]["x-nebula-resolved-value-schema"]["type"],
            json!("number")
        );
    }
}
