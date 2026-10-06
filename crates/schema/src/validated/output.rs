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
        values::validate_output(self.clone(), data)
    }
}
