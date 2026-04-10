//! Action package validation (`metadata` + `ports`).

use std::collections::HashSet;

use crate::metadata::ActionMetadata;
use crate::port::{InputPort, OutputPort};

/// Validation error for action package integrity checks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ActionPackageValidationError {
    /// Required metadata field is empty.
    #[error("metadata field `{field}` must not be empty")]
    EmptyMetadataField {
        /// Metadata field name.
        field: &'static str,
    },
    /// Input port list is empty.
    #[error("action must declare at least one input port")]
    MissingInputPorts,
    /// Output port list is empty.
    #[error("action must declare at least one output port")]
    MissingOutputPorts,
    /// Duplicate input port key found.
    #[error("duplicate input port key `{key}`")]
    DuplicateInputPortKey {
        /// Duplicate key.
        key: String,
    },
    /// Duplicate output port key found.
    #[error("duplicate output port key `{key}`")]
    DuplicateOutputPortKey {
        /// Duplicate key.
        key: String,
    },
    /// Invalid support port declaration.
    #[error("support port `{key}` must have non-empty name and description")]
    InvalidSupportPort {
        /// Support port key.
        key: String,
    },
    /// Invalid dynamic output declaration.
    #[error("dynamic output port `{key}` must define non-empty source_field")]
    InvalidDynamicPort {
        /// Dynamic port key.
        key: String,
    },
}

/// Collection of package validation failures.
///
/// Construct via [`validate_action_package`]; inspect via
/// [`ActionPackageValidationErrors::errors`]. The error list is stored
/// privately so new fields (severity, suggestions, source spans) can
/// be added without breaking downstream pattern-matching.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("action package validation failed with {errors:?}")]
#[non_exhaustive]
pub struct ActionPackageValidationErrors {
    errors: Vec<ActionPackageValidationError>,
}

impl ActionPackageValidationErrors {
    /// Read-only access to the collected validation errors.
    #[must_use]
    pub fn errors(&self) -> &[ActionPackageValidationError] {
        &self.errors
    }
}

/// Validate action package structure and declarations.
pub fn validate_action_package(
    metadata: &ActionMetadata,
) -> Result<(), ActionPackageValidationErrors> {
    let mut errors = Vec::new();

    if metadata.key.as_str().is_empty() {
        errors.push(ActionPackageValidationError::EmptyMetadataField { field: "key" });
    }
    if metadata.name.trim().is_empty() {
        errors.push(ActionPackageValidationError::EmptyMetadataField { field: "name" });
    }
    if metadata.description.trim().is_empty() {
        errors.push(ActionPackageValidationError::EmptyMetadataField {
            field: "description",
        });
    }
    if metadata.inputs.is_empty() {
        errors.push(ActionPackageValidationError::MissingInputPorts);
    }
    if metadata.outputs.is_empty() {
        errors.push(ActionPackageValidationError::MissingOutputPorts);
    }

    let mut input_keys = HashSet::new();
    for input in &metadata.inputs {
        let key = input.key().to_string();
        if !input_keys.insert(key.clone()) {
            errors.push(ActionPackageValidationError::DuplicateInputPortKey { key });
        }
        if let InputPort::Support(port) = input
            && (port.name.trim().is_empty() || port.description.trim().is_empty())
        {
            errors.push(ActionPackageValidationError::InvalidSupportPort {
                key: port.key.clone(),
            });
        }
    }

    let mut output_keys = HashSet::new();
    for output in &metadata.outputs {
        let key = output.key().to_string();
        if !output_keys.insert(key.clone()) {
            errors.push(ActionPackageValidationError::DuplicateOutputPortKey { key });
        }
        if let OutputPort::Dynamic(port) = output
            && port.source_field.trim().is_empty()
        {
            errors.push(ActionPackageValidationError::InvalidDynamicPort {
                key: port.key.clone(),
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ActionPackageValidationErrors { errors })
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::action_key;

    use super::*;
    use crate::port::{DynamicPort, SupportPort};

    fn valid_metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("test.action"), "Test", "desc")
    }

    #[test]
    fn valid_package_passes() {
        let meta = valid_metadata();
        assert!(validate_action_package(&meta).is_ok());
    }

    #[test]
    fn duplicate_ports_fail_validation() {
        let meta = ActionMetadata::new(action_key!("test.action"), "Test", "desc")
            .with_inputs(vec![InputPort::flow("in"), InputPort::flow("in")])
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("out")]);

        let err = validate_action_package(&meta).unwrap_err();
        assert!(err.errors().iter().any(|e| matches!(
            e,
            ActionPackageValidationError::DuplicateInputPortKey { .. }
        )));
        assert!(err.errors().iter().any(|e| matches!(
            e,
            ActionPackageValidationError::DuplicateOutputPortKey { .. }
        )));
    }

    #[test]
    fn invalid_support_and_dynamic_ports_fail_validation() {
        let meta = ActionMetadata::new(action_key!("test.action"), "Test", "desc")
            .with_inputs(vec![InputPort::Support(SupportPort {
                key: "tools".into(),
                name: "".into(),
                description: "".into(),
                required: false,
                multi: true,
                filter: Default::default(),
            })])
            .with_outputs(vec![OutputPort::Dynamic(DynamicPort {
                key: "rule".into(),
                source_field: "".into(),
                label_field: None,
                include_fallback: false,
            })]);

        let err = validate_action_package(&meta).unwrap_err();
        assert!(
            err.errors()
                .iter()
                .any(|e| matches!(e, ActionPackageValidationError::InvalidSupportPort { .. }))
        );
        assert!(
            err.errors()
                .iter()
                .any(|e| matches!(e, ActionPackageValidationError::InvalidDynamicPort { .. }))
        );
    }
}
