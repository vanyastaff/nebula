//! Conservative migration of current legacy declarations into graph admission.

use serde_json::{Value, json};

use crate::{
    Property, RequiredMode, RootShape, ScalarKind, ValidSchema, ValidationError, ValidationReport,
};

use super::{AdmittedSchemaGraph, MAX_GRAPH_DEFINITIONS, SchemaGraphDocument};

impl ValidSchema {
    /// Convert an exactly representable current legacy schema into an admitted graph.
    ///
    /// This bridge preserves declared domains and occurrence policies. It refuses
    /// semantics the graph cannot express exactly, including legacy conditional
    /// requiredness and integer properties accepting floating-point literals.
    /// Presentation and legacy default hints remain descriptive extensions; they
    /// never become graph input defaults or semantic commitments.
    ///
    /// # Errors
    /// Returns a payload-free `schema.graph.bridge.*` diagnostic for unsupported
    /// declarations, or the graph admission diagnostics for an invalid result.
    #[tracing::instrument(name = "schema.graph.bridge", skip_all, err)]
    pub fn to_admitted_graph(&self) -> Result<AdmittedSchemaGraph, ValidationReport> {
        self.ensure_current_semantics()?;
        let mut bridge = LegacyGraph {
            definitions: Vec::new(),
        };
        let (body, nullable) = match self.root_shape() {
            RootShape::Any => (json!({"kind": "any"}), true),
            RootShape::Scalar(scalar) => {
                let kind = match scalar.kind() {
                    ScalarKind::Null => "null",
                    ScalarKind::Boolean => "boolean",
                    ScalarKind::String => "string",
                    ScalarKind::Integer => "integer",
                    ScalarKind::Number => "number",
                };
                if kind == "null" && !scalar.root_rules().is_empty() {
                    return Err(unsupported("null_rules"));
                }
                let mut body = json!({"kind": kind});
                if kind != "null" {
                    body["intrinsic_rules"] = encode(scalar.root_rules())?;
                }
                if let Some(minimum) = scalar.minimum() {
                    body["minimum"] = json!(minimum);
                }
                if let Some(maximum) = scalar.maximum() {
                    body["maximum"] = json!(maximum);
                }
                (body, kind == "null")
            },
            RootShape::Record(record) => (
                bridge.record(record.properties(), record.root_rules())?,
                false,
            ),
            RootShape::Union(_) => return Err(unsupported("union")),
        };
        let target = bridge.define(body)?;
        let document: SchemaGraphDocument = serde_json::from_value(json!({
            "version": 3,
            "root": {"target": target, "null": if nullable {"allow"} else {"reject"}},
            "definitions": bridge.definitions,
            "x-legacy-policy-version": self.policy_version()
        }))
        .map_err(|_| unsupported("document"))?;
        document.admit().map_err(|error| error.report().clone())
    }
}

struct LegacyGraph {
    definitions: Vec<Value>,
}

impl LegacyGraph {
    fn define(&mut self, body: Value) -> Result<String, ValidationReport> {
        if self.definitions.len() >= MAX_GRAPH_DEFINITIONS {
            return Err(unsupported("definition_limit"));
        }
        let key = format!("legacy_{}", self.definitions.len());
        self.definitions.push(json!({"key": key, "body": body}));
        Ok(key)
    }

    fn record(
        &mut self,
        fields: &[Property],
        rules: &[nebula_validator::Rule],
    ) -> Result<Value, ValidationReport> {
        let properties = fields
            .iter()
            .map(|field| self.property(field))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(
            json!({"kind": "record", "properties": properties, "additional_properties": "open", "intrinsic_rules": encode(rules)?}),
        )
    }

    fn property(&mut self, field: &Property) -> Result<Value, ValidationReport> {
        let mut occurrence = self.occurrence(field)?;
        occurrence["key"] = json!(field.key().as_str());
        occurrence["presence"] = json!(match field.required() {
            RequiredMode::Never => "optional",
            RequiredMode::Always => "required",
            RequiredMode::When(_) => return Err(unsupported("conditional_presence")),
        });
        occurrence["aliases"] = json!({
            "read": field.read_aliases().iter().map(crate::FieldKey::as_str).collect::<Vec<_>>(),
            "write": field.emit_as().map(crate::FieldKey::as_str)
        });
        Ok(occurrence)
    }

    fn occurrence(&mut self, field: &Property) -> Result<Value, ValidationReport> {
        let required = match field.required() {
            RequiredMode::Never => false,
            RequiredMode::Always => true,
            RequiredMode::When(_) => return Err(unsupported("conditional_presence")),
        };
        let body = match field {
            Property::String(_) | Property::Secret(_) | Property::Code(_) => {
                json!({"kind": "string"})
            },
            Property::Boolean(_) => json!({"kind": "boolean"}),
            Property::Number(number) if !number.integer => json!({"kind": "number"}),
            Property::Number(_) => return Err(unsupported("integer_property")),
            Property::Object(object) => self.record(&object.fields, &[])?,
            Property::List(list) => {
                let element = if let Some(item) = &list.item {
                    self.occurrence(item)?
                } else {
                    let target = self.define(json!({"kind": "any"}))?;
                    json!({"target": target, "null": "allow"})
                };
                json!({"kind": "array", "element": element, "min_items": list.min_items.unwrap_or(0), "max_items": list.max_items, "unique": list.unique})
            },
            Property::Select(_)
            | Property::Mode(_)
            | Property::File(_)
            | Property::Computed(_)
            | Property::Dynamic(_)
            | Property::Notice(_)
            | Property::Unknown(_) => return Err(unsupported("property_kind")),
        };
        let target = self.define(body)?;
        let strings = matches!(
            field,
            Property::String(_) | Property::Secret(_) | Property::Code(_)
        );
        let lists = matches!(field, Property::List(_));
        Ok(json!({
            "target": target,
            "null": "reject",
            "empty_string": if required && strings {"reject"} else {"allow"},
            "empty_collection": if required && lists {"reject"} else {"allow"},
            "expression": field.expression(),
            "rules": encode(field.rules())?,
            "transformers": encode(field.transformers())?,
            "protection": if matches!(field, Property::Secret(_)) {"secret_utf8"} else {"public"},
            "x-legacy-declaration": encode(field)?
        }))
    }
}

fn encode<T: serde::Serialize + ?Sized>(value: &T) -> Result<Value, ValidationReport> {
    serde_json::to_value(value).map_err(|_| unsupported("serialization"))
}

fn unsupported(feature: &'static str) -> ValidationReport {
    tracing::debug!(
        feature,
        "legacy schema cannot be represented exactly by a graph"
    );
    ValidationError::builder(format!("schema.graph.bridge.{feature}"))
        .message("legacy schema semantics are not exactly representable by this graph bridge")
        .build()
        .into()
}

#[cfg(test)]
#[path = "legacy_graph_tests.rs"]
mod tests;
