//! JSON Schema export for [`crate::ValidSchema`] (feature: `schemars`).
//!
//! Maps Nebula's literal-data shapes and basic constraints to Draft 2020-12.
//! Records and object payloads are open; union and mode transport envelopes are
//! closed. Expression runtime and dynamic policy semantics remain annotations,
//! not proof that admission or the full resolved validation gate will succeed.

#![cfg(feature = "schemars")]

use std::{error::Error as StdError, fmt};

use serde_json::{Map, Value};

#[path = "export_budget.rs"]
mod export_budget;

use export_budget::ExportBudget;

use crate::{
    field::{
        ComputedReturn, ListField, ModeField, ModeVariant, NumberField, ObjectField, Property,
        SelectField,
    },
    mode::{ExpressionMode, RequiredMode, VisibilityMode},
    validated::{RootShape, ScalarKind, ScalarSchema, SerdeTagging},
};

/// Canonical draft URI emitted by [`ValidSchema::json_schema`](crate::validated::ValidSchema::json_schema).
const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// Error produced while exporting [`crate::validated::ValidSchema`] to JSON Schema.
#[derive(Debug)]
#[non_exhaustive]
pub enum JsonSchemaExportError {
    /// The unelided borrowed source descriptor exceeds its serialized-byte budget.
    SourceBudgetExceeded,
    /// Cumulative serialized inputs to expansion copies exceed their byte budget.
    CopyBudgetExceeded,
    /// A source descriptor could not be measured safely, including excessive JSON depth.
    BudgetSerialization,
    /// Historical evidence cannot describe a current validation contract.
    UnsupportedPolicy,
    /// A preserved declaration has no validation or export semantics in this reader.
    UnsupportedPropertyKind {
        /// Declaration path; anonymous list items use index zero.
        path: crate::ValuePath,
    },
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
            Self::SourceBudgetExceeded => {
                f.write_str("JSON Schema source descriptor exceeds the export budget")
            },
            Self::CopyBudgetExceeded => {
                f.write_str("JSON Schema expansion copies exceed the export budget")
            },
            Self::BudgetSerialization => {
                f.write_str("JSON Schema export budget measurement failed")
            },
            Self::UnsupportedPolicy => {
                f.write_str("historical schema policy cannot export a current contract")
            },
            Self::UnsupportedPropertyKind { path } => {
                write!(f, "unsupported property kind at schema declaration {path}")
            },
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
            Self::SourceBudgetExceeded
            | Self::CopyBudgetExceeded
            | Self::BudgetSerialization
            | Self::UnsupportedPropertyKind { .. }
            | Self::UnsupportedPolicy => None,
            Self::RootRuleSerialization { source, .. } | Self::InvalidSchema(source) => {
                Some(source)
            },
        }
    }
}

impl crate::validated::ValidSchema {
    /// Export this validated schema as JSON Schema (Draft 2020-12).
    ///
    /// The export is intentionally structural:
    /// - scalar domains with exact numeric bounds, open record/object shapes,
    ///   closed envelopes, and basic constraints are mapped
    /// - dynamic runtime semantics (loaders, deferred rules, expression runtime) are not fully
    ///   representable and are omitted from strict constraints
    ///
    /// Direct basic value rules are projected; compound/contextual rules remain
    /// runtime obligations. Defaults and aliases carry metadata, but JSON Schema
    /// does not perform transformation, alias precedence, or secret handling.
    /// Export acceptance never replaces `validate` followed by full resolution.
    /// Every root identifies its definition/export writer contract with
    /// `x-nebula-schema-version`; generic validators do not enforce that marker.
    /// Export separately limits unelided source-descriptor JSON to 1 MiB and
    /// cumulative expansion-copy inputs to 8 MiB, measured before copying.
    /// These are not exact final-output size or heap limits. Defaults/options
    /// must fit the 64-level JSON value-depth limit for safe measurement.
    ///
    /// Integer domains include in-range integral JSON floats. Runtime preparation
    /// normalizes those to integers before typed decoding; JSON Schema validates
    /// the mathematical value without rewriting its representation.
    ///
    /// # Errors
    ///
    /// Returns [`JsonSchemaExportError`] for unsupported policy/kinds, exhausted
    /// export budgets, failed source measurement, root-rule
    /// serialization failure, or an invalid generated `schemars::Schema`.
    #[tracing::instrument(
        level = "debug",
        target = "nebula_schema::json_schema",
        skip(self),
        fields(
            root_kind = ?self.kind(),
            schema_wire_version = crate::SCHEMA_WIRE_VERSION,
            field_count = self.properties().len(),
            root_rule_count = self.root_rules().len(),
        )
    )]
    pub fn json_schema(&self) -> Result<schemars::Schema, JsonSchemaExportError> {
        if let Some(path) = crate::field_tree::unsupported_property_path(self.properties()) {
            tracing::debug!(code = "schema.unsupported_property_kind", %path,
                "unsupported declaration rejected before JSON Schema export");
            return Err(JsonSchemaExportError::UnsupportedPropertyKind { path });
        }
        self.ensure_current_semantics()
            .map_err(|_| JsonSchemaExportError::UnsupportedPolicy)?;
        let mut budget = ExportBudget::for_schema(self)?;
        let mut exported = match self.root_shape() {
            // A tagged union exports as a `oneOf` faithful to its serde tagging —
            // not as a record wrapping the internal `{mode,value}` envelope.
            RootShape::Union(_) => schema_for_union(self, &mut budget),
            RootShape::Record(record) => {
                schema_for_fields(record.properties(), record.root_rules(), &mut budget)
            },
            RootShape::Scalar(scalar) => schema_for_scalar(scalar),
            RootShape::Any => schemars::Schema::try_from(serde_json::json!({
                "$schema": DRAFT_2020_12,
            }))
            .map_err(JsonSchemaExportError::InvalidSchema),
        }?;
        exported.insert(
            "x-nebula-schema-version".to_owned(),
            Value::from(crate::SCHEMA_WIRE_VERSION),
        );
        Ok(exported)
    }
}

fn schema_for_scalar(scalar: &ScalarSchema) -> Result<schemars::Schema, JsonSchemaExportError> {
    let scalar_type = match scalar.kind() {
        ScalarKind::Null => "null",
        ScalarKind::Boolean => "boolean",
        ScalarKind::String => "string",
        ScalarKind::Integer => "integer",
        ScalarKind::Number => "number",
    };
    let mut root = primitive_schema(scalar_type);
    root.insert(
        "$schema".to_owned(),
        Value::String(DRAFT_2020_12.to_owned()),
    );
    if let Some(minimum) = scalar.minimum() {
        root.insert("minimum".to_owned(), Value::Number(minimum.clone()));
    }
    if let Some(maximum) = scalar.maximum() {
        root.insert("maximum".to_owned(), Value::Number(maximum.clone()));
    }
    apply_value_rules(&mut root, scalar.root_rules());
    apply_root_rule_annotations(&mut root, scalar.root_rules())?;
    schemars::Schema::try_from(Value::Object(root)).map_err(JsonSchemaExportError::InvalidSchema)
}

/// Export a [`RootShape::Union`] schema as a JSON Schema `oneOf`, faithful to the
/// recorded [`SerdeTagging`] so the document matches serde's wire form (the C1
/// invariant — schema variant key == wire key):
///
/// - **External** — a data variant is `{ "<variant>": payload }` and a unit
///   variant is the bare string `{ "const": "<variant>" }`.
/// - **Adjacent** `{ tag, content }` — a data variant is
///   `{ "<tag>": "<variant>", "<content>": payload }`, a unit variant omits the
///   content key, and a top-level `discriminator: { propertyName: "<tag>" }` is
///   emitted.
///
/// The union's variants live in its sole root [`Property::Mode`] (the marker design);
/// unit variants are recognized by the [`ModeField::EMPTY_PLACEHOLDER_KEY`] payload
/// that [`ModeField::variant_empty`] installs.
fn schema_for_union(
    schema: &crate::validated::ValidSchema,
    budget: &mut ExportBudget,
) -> Result<schemars::Schema, JsonSchemaExportError> {
    let Some(Property::Mode(mode)) = schema.properties().first() else {
        // Unreachable for schemas built by `ValidSchema::union` / its deserialize
        // (both guarantee one root `Property::Mode`); fall back to the record export
        // rather than panic on a malformed value.
        return schema_for_fields(schema.properties(), schema.root_rules(), budget);
    };
    let tagging = schema.serde_tagging().unwrap_or(&SerdeTagging::External);

    let branches: Vec<Value> = mode
        .variants
        .iter()
        .map(|variant| union_variant_branch(variant, tagging, budget))
        .collect::<Result<_, _>>()?;

    let mut root = Map::new();
    root.insert(
        "$schema".to_owned(),
        Value::String(DRAFT_2020_12.to_owned()),
    );
    root.insert("oneOf".to_owned(), Value::Array(branches));
    if let SerdeTagging::Adjacent { tag, .. } = tagging {
        let mut discriminator = Map::new();
        discriminator.insert("propertyName".to_owned(), Value::String(budget.copy(tag)?));
        root.insert("discriminator".to_owned(), Value::Object(discriminator));
    }
    schemars::Schema::try_from(Value::Object(root)).map_err(JsonSchemaExportError::InvalidSchema)
}

/// One JSON-Schema `oneOf` branch for a union variant under the given tagging.
fn union_variant_branch(
    variant: &ModeVariant,
    tagging: &SerdeTagging,
    budget: &mut ExportBudget,
) -> Result<Value, JsonSchemaExportError> {
    let is_unit = variant.field.key().as_str() == ModeField::EMPTY_PLACEHOLDER_KEY;
    Ok(match tagging {
        SerdeTagging::External if is_unit => {
            // serde external unit variant: the bare string `"Variant"`.
            let mut branch = Map::new();
            branch.insert("const".to_owned(), Value::String(variant.key.clone()));
            Value::Object(branch)
        },
        SerdeTagging::External => {
            // serde external data variant: `{ "Variant": payload }`.
            let mut props = Map::new();
            props.insert(
                variant.key.clone(),
                field_schema_value(&variant.field, budget)?,
            );
            let mut branch = primitive_schema("object");
            branch.insert("properties".to_owned(), Value::Object(props));
            branch.insert(
                "required".to_owned(),
                Value::Array(vec![Value::String(variant.key.clone())]),
            );
            branch.insert("additionalProperties".to_owned(), Value::Bool(false));
            Value::Object(branch)
        },
        SerdeTagging::Adjacent { tag, content } => {
            let mut props = Map::new();
            let mut tag_const = Map::new();
            tag_const.insert("const".to_owned(), Value::String(variant.key.clone()));
            props.insert(budget.copy(tag)?, Value::Object(tag_const));
            let mut required = vec![Value::String(budget.copy(tag)?)];
            if !is_unit {
                // serde emits the content key for every data variant.
                props.insert(
                    budget.copy(content)?,
                    field_schema_value(&variant.field, budget)?,
                );
                required.push(Value::String(budget.copy(content)?));
            }
            let mut branch = primitive_schema("object");
            branch.insert("properties".to_owned(), Value::Object(props));
            branch.insert("required".to_owned(), Value::Array(required));
            branch.insert("additionalProperties".to_owned(), Value::Bool(false));
            Value::Object(branch)
        },
    })
}

fn schema_for_fields(
    fields: &[Property],
    root_rules: &[nebula_validator::Rule],
    budget: &mut ExportBudget,
) -> Result<schemars::Schema, JsonSchemaExportError> {
    let mut root = Map::new();
    root.insert(
        "$schema".to_owned(),
        Value::String(DRAFT_2020_12.to_owned()),
    );
    root.insert("type".to_owned(), Value::String("object".to_owned()));
    root.insert(
        "properties".to_owned(),
        Value::Object(properties_for_fields(fields, budget)?),
    );

    apply_required_constraints(fields, &mut root);
    apply_root_rule_annotations(&mut root, root_rules)?;
    root.insert("additionalProperties".to_owned(), Value::Bool(true));

    schemars::Schema::try_from(Value::Object(root)).map_err(JsonSchemaExportError::InvalidSchema)
}

fn apply_root_rule_annotations(
    root: &mut Map<String, Value>,
    root_rules: &[nebula_validator::Rule],
) -> Result<(), JsonSchemaExportError> {
    if !root_rules.is_empty() {
        let mut serialized = Vec::with_capacity(root_rules.len());
        for (index, rule) in root_rules.iter().enumerate() {
            let value = serde_json::to_value(rule)
                .map_err(|source| JsonSchemaExportError::RootRuleSerialization { index, source })?;
            serialized.push(value);
        }
        root.insert("x-nebula-root-rules".to_owned(), Value::Array(serialized));
    }
    Ok(())
}

fn properties_for_fields(
    fields: &[Property],
    budget: &mut ExportBudget,
) -> Result<Map<String, Value>, JsonSchemaExportError> {
    let mut out = Map::with_capacity(fields.len());
    for field in fields {
        let value = field_schema_value(field, budget)?;
        // Read-aliases are accepted input keys (folded to the canonical key at
        // ingest), so expose each as a typed property too. Aliases are lint-guaranteed
        // disjoint from canonical keys and from each other, so this never
        // collides. The value schema is shared (the alias carries the same
        // contract as the canonical key).
        for alias in field.read_aliases() {
            out.insert(alias.as_str().to_owned(), budget.copy(&value)?);
        }
        out.insert(field.key().as_str().to_owned(), value);
    }
    Ok(out)
}

/// Emit `required` (and, for required fields with read-aliases, an `allOf` of
/// `anyOf`-required clauses) into `target`.
///
/// An unconditionally required field with no aliases is a flat `required` entry.
/// Visibility is presentation metadata and does not change property presence.
/// A required field satisfiable via a read-alias instead becomes
/// `anyOf: [{required:[canonical]}, {required:[alias]}, …]`, because a flat
/// `required: [canonical]` would reject an alias-only submission that `validate`
/// accepts (it canonicalizes the alias before the required check). The exported
/// schema must never reject input `validate` would accept.
fn apply_required_constraints(fields: &[Property], target: &mut Map<String, Value>) {
    let mut flat_required: Vec<Value> = Vec::new();
    let mut any_of_clauses: Vec<Value> = Vec::new();

    for field in fields {
        if !matches!(field.required(), RequiredMode::Always) {
            continue;
        }
        let aliases = field.read_aliases();
        if aliases.is_empty() {
            flat_required.push(Value::String(field.key().as_str().to_owned()));
            continue;
        }
        let mut clauses = Vec::with_capacity(aliases.len() + 1);
        clauses.push(Value::Object(required_clause(field.key().as_str())));
        clauses.extend(
            aliases
                .iter()
                .map(|a| Value::Object(required_clause(a.as_str()))),
        );
        let mut any_of = Map::new();
        any_of.insert("anyOf".to_owned(), Value::Array(clauses));
        any_of_clauses.push(Value::Object(any_of));
    }

    if !flat_required.is_empty() {
        target.insert("required".to_owned(), Value::Array(flat_required));
    }
    if !any_of_clauses.is_empty() {
        target.insert("allOf".to_owned(), Value::Array(any_of_clauses));
    }
}

fn required_clause(key: &str) -> Map<String, Value> {
    let mut clause = Map::new();
    clause.insert(
        "required".to_owned(),
        Value::Array(vec![Value::String(key.to_owned())]),
    );
    clause
}

fn field_schema_value(
    field: &Property,
    budget: &mut ExportBudget,
) -> Result<Value, JsonSchemaExportError> {
    let mut core_schema = match field {
        Property::String(_) | Property::Code(_) => string_like_schema(),
        Property::Secret(_) => {
            let mut s = string_like_schema();
            s.insert("writeOnly".to_owned(), Value::Bool(true));
            s
        },
        Property::Number(f) => number_schema(f),
        Property::Boolean(_) => primitive_schema("boolean"),
        Property::Select(f) => select_schema(f),
        Property::Object(f) => object_schema(f, budget)?,
        Property::List(f) => list_schema(f, budget)?,
        Property::Mode(f) => mode_schema(f, budget)?,
        Property::File(f) => file_schema(f.multiple),
        Property::Computed(f) => computed_schema(f.returns),
        // Runtime-only payload from loader; keep intentionally permissive.
        Property::Dynamic(_) => Map::new(),
        // Display-only field in UI forms; no value contract.
        Property::Notice(_) => {
            let mut s = Map::new();
            s.insert("readOnly".to_owned(), Value::Bool(true));
            s
        },
        // Forward-compat field of an unknown future kind; this version cannot
        // describe its value contract, so emit a permissive (empty) schema.
        Property::Unknown(_) => Map::new(),
    };

    apply_value_rules(&mut core_schema, field.rules());
    apply_required_value_constraints(field, &mut core_schema);
    let mut schema = apply_expression_mode(core_schema, *field.expression(), budget)?;
    apply_common_keywords(field, &mut schema);
    apply_contract_keywords(field, &mut schema);
    apply_alias_keywords(field, &mut schema);
    Ok(Value::Object(schema))
}

/// Emit alias metadata for a field.
///
/// - `x-nebula-read-aliases`: extra input keys accepted at ingest (folded onto
///   the canonical key). They are ALSO surfaced as accepted properties by
///   [`properties_for_fields`] so alias-keyed input carries the same constraints.
/// - `x-nebula-emit-as`: the key this field is emitted under by `project` /
///   `to_wire_json`. The exported document is an INPUT schema keyed on canonical
///   names, so the `emit_as` key is metadata only — an output validator reads this
///   to learn the projected key without the input contract misrepresenting it.
fn apply_alias_keywords(field: &Property, schema: &mut Map<String, Value>) {
    let read_aliases = field.read_aliases();
    if !read_aliases.is_empty() {
        schema.insert(
            "x-nebula-read-aliases".to_owned(),
            Value::Array(
                read_aliases
                    .iter()
                    .map(|alias| Value::String(alias.as_str().to_owned()))
                    .collect(),
            ),
        );
    }
    if let Some(emit_as) = field.emit_as() {
        schema.insert(
            "x-nebula-emit-as".to_owned(),
            Value::String(emit_as.as_str().to_owned()),
        );
    }
}

fn apply_common_keywords(field: &Property, schema: &mut Map<String, Value>) {
    let (label, description, default) = match field {
        Property::String(f) => (&f.label, &f.description, &f.default),
        Property::Secret(f) => (&f.label, &f.description, &f.default),
        Property::Number(f) => (&f.label, &f.description, &f.default),
        Property::Boolean(f) => (&f.label, &f.description, &f.default),
        Property::Select(f) => (&f.label, &f.description, &f.default),
        Property::Object(f) => (&f.label, &f.description, &f.default),
        Property::List(f) => (&f.label, &f.description, &f.default),
        Property::Mode(f) => (&f.label, &f.description, &f.default),
        Property::Code(f) => (&f.label, &f.description, &f.default),
        Property::File(f) => (&f.label, &f.description, &f.default),
        Property::Computed(f) => (&f.label, &f.description, &f.default),
        Property::Dynamic(f) => (&f.label, &f.description, &f.default),
        Property::Notice(f) => (&f.label, &f.description, &f.default),
        // An unknown future field has no typed label/description/default slots;
        // its decorations (if any) live opaquely in `raw`. Skip common keywords.
        Property::Unknown(_) => return,
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
    } else {
        out.extend(select_item_schema_map(field));
        // A single option may be any JSON kind except an array, even when
        // custom values make membership unconstrained.
        out.insert(
            "type".to_owned(),
            serde_json::json!(["null", "boolean", "number", "string", "object"]),
        );
    }
    out
}

fn select_item_schema(field: &SelectField) -> Value {
    Value::Object(select_item_schema_map(field))
}

fn select_item_schema_map(field: &SelectField) -> Map<String, Value> {
    let mut item = Map::new();
    if field.allow_custom || (field.dynamic && field.options.is_empty()) {
        return item;
    }
    if field.options.is_empty() {
        item.insert("not".to_owned(), Value::Object(Map::new()));
    } else {
        item.insert(
            "anyOf".to_owned(),
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

fn object_schema(
    field: &ObjectField,
    budget: &mut ExportBudget,
) -> Result<Map<String, Value>, JsonSchemaExportError> {
    let mut out = primitive_schema("object");
    out.insert(
        "properties".to_owned(),
        Value::Object(properties_for_fields(&field.fields, budget)?),
    );
    apply_required_constraints(&field.fields, &mut out);
    out.insert("additionalProperties".to_owned(), Value::Bool(true));
    Ok(out)
}

fn list_schema(
    field: &ListField,
    budget: &mut ExportBudget,
) -> Result<Map<String, Value>, JsonSchemaExportError> {
    let mut out = primitive_schema("array");
    if let Some(item) = &field.item {
        out.insert(
            "items".to_owned(),
            field_schema_value(item.as_ref(), budget)?,
        );
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
    Ok(out)
}

fn mode_schema(
    field: &ModeField,
    budget: &mut ExportBudget,
) -> Result<Map<String, Value>, JsonSchemaExportError> {
    let mut out = Map::new();
    let mut branches = Vec::with_capacity(field.variants.len());
    for variant in &field.variants {
        let mut branch = primitive_schema("object");
        let mut props = Map::new();
        let mut required = Vec::new();
        if field.default_variant.as_deref() != Some(variant.key.as_str()) {
            required.push(Value::String("mode".to_owned()));
        }

        let mut mode_const = Map::new();
        mode_const.insert("const".to_owned(), Value::String(variant.key.clone()));
        props.insert("mode".to_owned(), Value::Object(mode_const));
        props.insert(
            "value".to_owned(),
            field_schema_value(&variant.field, budget)?,
        );
        if matches!(variant.field.required(), RequiredMode::Always) {
            required.push(Value::String("value".to_owned()));
        }

        branch.insert("properties".to_owned(), Value::Object(props));
        branch.insert("required".to_owned(), Value::Array(required));
        branch.insert("additionalProperties".to_owned(), Value::Bool(false));
        branches.push(Value::Object(branch));
    }
    out.insert("oneOf".to_owned(), Value::Array(branches));
    Ok(out)
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

fn apply_required_value_constraints(field: &Property, schema: &mut Map<String, Value>) {
    if !matches!(field.required(), RequiredMode::Always) {
        return;
    }
    // Requiredness also rejects empty supplied values, including on hidden
    // fields. Presentation visibility does not alter the required contract.
    insert_constraint(schema, "not", serde_json::json!({"type": "null"}));
    let minimum = match field {
        Property::String(_) | Property::Secret(_) | Property::Code(_) => Some("minLength"),
        Property::List(_) => Some("minItems"),
        Property::File(file) => Some(if file.multiple {
            "minItems"
        } else {
            "minLength"
        }),
        Property::Select(select) if select.multiple => Some("minItems"),
        _ => None,
    };
    if let Some(keyword) = minimum {
        insert_constraint(schema, keyword, Value::from(1));
    }
}

fn apply_value_rules(schema: &mut Map<String, Value>, rules: &[nebula_validator::Rule]) {
    use nebula_validator::{RuleView, ValueRule};
    for rule in rules {
        if let RuleView::Value(v) = rule.view() {
            let (keyword, constraint) = match v {
                ValueRule::MinLength(n) => ("minLength", Value::from(*n)),
                ValueRule::MaxLength(n) => ("maxLength", Value::from(*n)),
                ValueRule::Pattern(pattern) => {
                    ("pattern", Value::String(pattern.as_str().to_owned()))
                },
                ValueRule::Email => ("format", Value::String("email".to_owned())),
                ValueRule::Url => ("format", Value::String("uri".to_owned())),
                ValueRule::Min(min) => ("minimum", Value::Number(min.clone())),
                ValueRule::Max(max) => ("maximum", Value::Number(max.clone())),
                ValueRule::GreaterThan(min) => ("exclusiveMinimum", Value::Number(min.clone())),
                ValueRule::LessThan(max) => ("exclusiveMaximum", Value::Number(max.clone())),
                ValueRule::OneOf(values) if values.is_empty() => ("not", Value::Object(Map::new())),
                ValueRule::OneOf(values) => ("enum", Value::Array(values.clone())),
                ValueRule::MinItems(n) => ("minItems", Value::from(*n)),
                ValueRule::MaxItems(n) => ("maxItems", Value::from(*n)),
                _ => continue,
            };
            insert_constraint(schema, keyword, constraint);
        }
    }
}

/// Keep repeated keywords conjunctive without comparing or coalescing their values.
fn insert_constraint(schema: &mut Map<String, Value>, keyword: &str, constraint: Value) {
    if let serde_json::map::Entry::Vacant(entry) = schema.entry(keyword) {
        entry.insert(constraint);
        return;
    }
    let mut conjunction = match schema.remove("allOf") {
        Some(Value::Array(clauses)) => clauses,
        Some(existing) => vec![serde_json::json!({"allOf": existing})],
        None => Vec::new(),
    };
    conjunction.push(serde_json::json!({keyword: constraint}));
    schema.insert("allOf".to_owned(), Value::Array(conjunction));
}

fn apply_expression_mode(
    core: Map<String, Value>,
    mode: ExpressionMode,
    budget: &mut ExportBudget,
) -> Result<Map<String, Value>, JsonSchemaExportError> {
    // The `x-nebula-resolved-value-schema` extension is always emitted (regardless
    // of expression mode) so that UI / downstream consumers have a stable shape:
    // they can read the post-resolution value schema without branching on mode.
    Ok(match mode {
        ExpressionMode::Forbidden => {
            // No expression wrapper — the JSON-Schema-Draft 2020-12 part IS the
            // resolved-value schema; expose both as the same map so consumers
            // can rely on the extension key being present.
            let mut out = budget.copy(&core)?;
            out.insert(
                "x-nebula-resolved-value-schema".to_owned(),
                Value::Object(core),
            );
            out
        },
        ExpressionMode::Allowed => {
            let mut out = Map::new();
            out.insert(
                "anyOf".to_owned(),
                Value::Array(vec![
                    Value::Object(budget.copy(&core)?),
                    Value::Object(expression_wrapper_schema()),
                ]),
            );
            out.insert(
                "x-nebula-resolved-value-schema".to_owned(),
                Value::Object(core),
            );
            out
        },
        ExpressionMode::Required => {
            let mut wrapper = expression_wrapper_schema();
            wrapper.insert(
                "x-nebula-resolved-value-schema".to_owned(),
                Value::Object(core),
            );
            wrapper
        },
    })
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

fn apply_contract_keywords(field: &Property, schema: &mut Map<String, Value>) {
    // For an `Unknown` field, `type_name()` collapses to the literal "unknown";
    // emit the real future discriminator so the exported contract stays accurate.
    let field_kind = field.unknown_type().unwrap_or_else(|| field.type_name());
    schema.insert(
        "x-nebula-field-kind".to_owned(),
        Value::String(field_kind.to_owned()),
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

    if let Property::File(f) = field {
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
    if let Property::Select(f) = field {
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
    if let Property::Mode(f) = field
        && let Some(default_variant) = &f.default_variant
    {
        schema.insert(
            "x-nebula-mode-default-variant".to_owned(),
            Value::String(default_variant.clone()),
        );
    }
}

#[cfg(test)]
#[path = "json_schema_tests.rs"]
mod tests;
