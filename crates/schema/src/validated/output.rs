//! Literal outbound proof admission, distinct from authored input preparation.

use serde_json::Value;

use super::{ResolvedValues, SchemaKind, ValidSchema, values};
use crate::{Property, ValidationError, ValidationReport, ValuePath};

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
    pub fn validate_output_data(&self, data: Value) -> Result<ResolvedValues, ValidationReport> {
        self.ensure_public_output_domain()?;
        let data = if self.kind() == SchemaKind::Union {
            self.rewrite_union_wire(data)?
        } else {
            data
        };
        values::validate_output(self.clone(), data)
    }
}
