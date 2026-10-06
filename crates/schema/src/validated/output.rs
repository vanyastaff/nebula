//! Literal outbound proof admission, distinct from authored input preparation.

use std::sync::Arc;

use serde_json::Value;

use super::{SchemaKind, ValidSchema, values};
use crate::{FieldKey, Property, ResolvedValue, ValidationError, ValidationReport, ValuePath};

/// Literal outbound data checked against one admitted output schema.
///
/// This is an output proof only. Output admission skips input preparation
/// (aliases, transforms, defaults and expression-required enforcement), so it
/// deliberately has no typed decode and cannot stand in for
/// [`ResolvedValues`](crate::ResolvedValues), the resolved-input proof:
///
/// ```compile_fail
/// use nebula_schema::{ResolvedValues, Schema};
///
/// fn consume_input(_: ResolvedValues) {}
///
/// let schema = Schema::builder().build().unwrap();
/// let output = schema.validate_output_data(serde_json::json!({})).unwrap();
/// consume_input(output);
/// ```
///
/// ```compile_fail
/// use nebula_schema::Schema;
///
/// let schema = Schema::builder().build().unwrap();
/// let output = schema.validate_output_data(serde_json::json!({})).unwrap();
/// let _: serde_json::Value = output.into_typed().unwrap();
/// ```
///
/// The same output is otherwise inspectable:
///
/// ```
/// use nebula_schema::Schema;
///
/// let schema = Schema::builder().build().unwrap();
/// let output = schema.validate_output_data(serde_json::json!({})).unwrap();
/// assert!(output.schema().ptr_eq(&schema));
/// assert_eq!(output.into_json(), serde_json::json!({}));
/// ```
#[derive(Debug, Clone)]
#[must_use]
pub struct ValidatedOutput {
    schema: ValidSchema,
    values: ResolvedValue,
    warnings: Arc<[ValidationError]>,
}

impl ValidatedOutput {
    pub(super) fn new(
        schema: ValidSchema,
        values: ResolvedValue,
        warnings: Arc<[ValidationError]>,
    ) -> Self {
        Self {
            schema,
            values,
            warnings,
        }
    }

    /// Output schema snapshot this proof is bound to.
    #[must_use]
    pub const fn schema(&self) -> &ValidSchema {
        &self.schema
    }

    /// Checked literal output data.
    #[must_use]
    pub const fn values(&self) -> &ResolvedValue {
        &self.values
    }

    /// Non-fatal output validation diagnostics.
    #[must_use]
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Borrow a scalar output field.
    #[must_use]
    pub fn get(&self, key: &FieldKey) -> Option<&Value> {
        match self.values.get(key.as_str()) {
            Some(crate::ValueTree::Literal(value)) => Some(value.as_json()),
            _ => None,
        }
    }

    /// Borrow any output node using its RFC6901 location.
    #[must_use]
    pub fn get_path(&self, path: &ValuePath) -> Option<&ResolvedValue> {
        self.values.get_path(path)
    }

    /// Return the checked output as JSON data.
    #[must_use]
    pub fn into_json(self) -> Value {
        self.values.to_json()
    }
}

impl ValidSchema {
    /// Reject protected output declarations before executing an action.
    ///
    /// This examines every declaration, including anonymous list items and
    /// inactive mode variants. A redacting serializer does not waive the gate.
    ///
    /// # Errors
    /// Returns a payload-free error for unsupported or protected declarations.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn ensure_public_output_domain(&self) -> Result<(), ValidationReport> {
        self.ensure_current_semantics()?;
        let mut pending: Vec<_> = self
            .properties()
            .iter()
            .map(|property| (property, ValuePath::root().push(property.key().as_str())))
            .collect();
        while let Some((property, path)) = pending.pop() {
            match property {
                Property::Secret(_) => {
                    return Err(ValidationError::builder("schema.output.protected_domain")
                        .at(path)
                        .message("protected declarations cannot be action output domains")
                        .build()
                        .into());
                },
                Property::Object(object) => pending.extend(
                    object
                        .fields
                        .iter()
                        .map(|child| (child, path.push(child.key().as_str()))),
                ),
                Property::List(list) => {
                    pending.extend(list.item.as_deref().map(|item| (item, path.push("0"))));
                },
                Property::Mode(mode) => pending.extend(
                    mode.variants
                        .iter()
                        .map(|variant| (variant.field.as_ref(), path.push(&variant.key))),
                ),
                _ => {},
            }
        }
        Ok(())
    }

    /// Validate serialized literal output against this admitted outbound schema.
    ///
    /// No input preparation runs: aliases, transforms and defaults never repair
    /// output, and expression-looking strings and objects remain literal data.
    /// The returned proof retains this exact immutable schema.
    ///
    /// # Errors
    /// Returns declaration, shape, value-budget, rule or conditional-policy errors.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn validate_output_data(&self, data: Value) -> Result<ValidatedOutput, ValidationReport> {
        self.ensure_public_output_domain()?;
        let data = if self.kind() == SchemaKind::Union {
            self.rewrite_union_wire(data)?
        } else {
            data
        };
        let data = level_to_canonical(self.properties(), data, &ValuePath::root())?;
        values::validate_output(self.clone(), data)
    }
}

/// Invert the output projection's `emit_as` renames so validation reads the
/// candidate by canonical declaration keys, at every declared nesting level.
///
/// A canonical key that the projection reserves for a renamed field is never
/// emitted, so its presence on the wire is refused rather than validated.
/// Recursion follows declared schema nesting only, never data depth.
fn level_to_canonical(
    fields: &[Property],
    value: Value,
    path: &ValuePath,
) -> Result<Value, ValidationError> {
    let Value::Object(entries) = value else {
        return Ok(value);
    };
    let mut out = serde_json::Map::with_capacity(entries.len());
    for (key, value) in entries {
        if let Some(field) = fields.iter().find(|field| emitted(field) == key) {
            let child = property_to_canonical(field, value, &path.push(field.key().as_str()))?;
            out.insert(field.key().as_str().to_owned(), child);
        } else if fields
            .iter()
            .any(|field| field.emit_as().is_some() && field.key().as_str() == key)
        {
            return Err(ValidationError::builder("schema.output.unprojected_key")
                .at(path.push(&key))
                .message("output carries a declaration key in place of its emitted name")
                .build());
        } else {
            out.insert(key, value);
        }
    }
    Ok(Value::Object(out))
}

fn property_to_canonical(
    field: &Property,
    value: Value,
    path: &ValuePath,
) -> Result<Value, ValidationError> {
    match (field, value) {
        (Property::Object(object), value) => level_to_canonical(&object.fields, value, path),
        (Property::List(list), Value::Array(items)) => match list.item.as_deref() {
            Some(item) => items
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    property_to_canonical(item, value, &path.push(index.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            None => Ok(Value::Array(items)),
        },
        (Property::Mode(mode), Value::Object(mut envelope)) => {
            let selector = match envelope.get(super::MODE_SELECTOR_KEY) {
                Some(Value::String(selector)) => Some(selector.clone()),
                Some(_) => None,
                None => mode.default_variant.clone(),
            };
            let variant = selector
                .as_deref()
                .and_then(|key| mode.variants.iter().find(|variant| variant.key == key));
            if let Some(variant) = variant
                && let Some(payload) = envelope.remove(super::MODE_PAYLOAD_KEY)
            {
                let payload = property_to_canonical(
                    &variant.field,
                    payload,
                    &path.push(super::MODE_PAYLOAD_KEY),
                )?;
                envelope.insert(super::MODE_PAYLOAD_KEY.to_owned(), payload);
            }
            Ok(Value::Object(envelope))
        },
        (_, value) => Ok(value),
    }
}

fn emitted(field: &Property) -> &str {
    field.emit_as().unwrap_or_else(|| field.key()).as_str()
}
