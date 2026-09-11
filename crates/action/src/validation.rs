//! Action package validation (`metadata` + `ports`).

use std::collections::HashSet;

use crate::{
    metadata::ActionMetadata,
    port::{InputPort, OutputPort},
};

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
/// Construct during action metadata admission; inspect via
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

/// Validate action package structure and declarations during admission.
pub(crate) fn validate_action_package(
    metadata: &ActionMetadata,
) -> Result<(), ActionPackageValidationErrors> {
    let mut errors = Vec::new();

    if metadata.base().key().as_str().is_empty() {
        errors.push(ActionPackageValidationError::EmptyMetadataField { field: "key" });
    }
    if metadata.base().name().trim().is_empty() {
        errors.push(ActionPackageValidationError::EmptyMetadataField { field: "name" });
    }
    if metadata.base().description().trim().is_empty() {
        errors.push(ActionPackageValidationError::EmptyMetadataField {
            field: "description",
        });
    }
    if metadata.inputs().is_empty() {
        errors.push(ActionPackageValidationError::MissingInputPorts);
    }
    if metadata.outputs().is_empty() && metadata.kind() != crate::ActionKind::Control {
        errors.push(ActionPackageValidationError::MissingOutputPorts);
    }

    let mut input_keys = HashSet::new();
    for input in metadata.inputs() {
        let key = input.key().to_string();
        if !input_keys.insert(key.clone()) {
            errors.push(ActionPackageValidationError::DuplicateInputPortKey { key });
        }
        if let InputPort::Support(port) = input
            && (port.name.trim().is_empty() || port.description.trim().is_empty())
        {
            errors.push(ActionPackageValidationError::InvalidSupportPort {
                key: port.key.to_string(),
            });
        }
    }

    let mut output_keys = HashSet::new();
    for output in metadata.outputs() {
        let key = output.key().to_string();
        if !output_keys.insert(key.clone()) {
            errors.push(ActionPackageValidationError::DuplicateOutputPortKey { key });
        }
        if let OutputPort::Dynamic(port) = output
            && port.source_field.trim().is_empty()
        {
            errors.push(ActionPackageValidationError::InvalidDynamicPort {
                key: port.key.to_string(),
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
    use std::sync::OnceLock;

    use nebula_core::{Dependencies, action_key};

    use super::*;
    use crate::{
        port::{DynamicPort, SupportPort},
        port_key,
    };

    struct ValidationAction;

    impl crate::Action for ValidationAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> crate::ActionMetadataDraft {
            crate::ActionMetadataDraft::new(
                action_key!("test.action"),
                crate::metadata_name!("Test"),
                "desc",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
            DEPENDENCIES.get_or_init(Dependencies::new)
        }
    }

    fn admit(draft: crate::ActionMetadataDraft) -> ActionMetadata {
        draft
            .admit_for::<ValidationAction>(crate::ActionKind::Stateless)
            .expect("test metadata must admit")
    }

    fn valid_metadata() -> ActionMetadata {
        admit(crate::ActionMetadataDraft::new(
            action_key!("test.action"),
            crate::metadata_name!("Test"),
            "desc",
        ))
    }

    #[test]
    fn valid_package_passes() {
        let meta = valid_metadata();
        assert!(validate_action_package(&meta).is_ok());
    }

    #[test]
    fn only_control_actions_may_terminate_without_output_ports() {
        let terminal = crate::ActionMetadataDraft::new(
            action_key!("test.terminal"),
            crate::metadata_name!("Terminal"),
            "terminal control",
        )
        .with_outputs(Vec::new());

        let admitted = terminal
            .clone()
            .admit_for::<ValidationAction>(crate::ActionKind::Control)
            .expect("terminal control metadata must admit");
        assert!(admitted.outputs().is_empty());

        let crate::ActionMetadataAdmissionError::Package(error) = terminal
            .admit_for::<ValidationAction>(crate::ActionKind::Stateless)
            .expect_err("stateless actions must retain an output port")
        else {
            panic!("missing stateless outputs must be a package admission failure");
        };
        assert!(
            error
                .errors()
                .contains(&ActionPackageValidationError::MissingOutputPorts)
        );
    }

    #[test]
    fn duplicate_ports_fail_validation() {
        let draft = crate::ActionMetadataDraft::new(
            action_key!("test.action"),
            crate::metadata_name!("Test"),
            "desc",
        )
        .with_inputs(vec![
            InputPort::flow(port_key!("in")),
            InputPort::flow(port_key!("in")),
        ])
        .with_outputs(vec![
            OutputPort::flow(port_key!("out")),
            OutputPort::error(port_key!("out")),
        ]);

        let crate::ActionMetadataAdmissionError::Package(err) = draft
            .admit_for::<ValidationAction>(crate::ActionKind::Stateless)
            .unwrap_err()
        else {
            panic!("duplicate ports must be a package admission failure");
        };
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
        let draft = crate::ActionMetadataDraft::new(
            action_key!("test.action"),
            crate::metadata_name!("Test"),
            "desc",
        )
        .with_inputs(vec![InputPort::Support(SupportPort {
            key: port_key!("tools"),
            name: String::new(),
            description: String::new(),
            required: false,
            multi: true,
            filter: Default::default(),
        })])
        .with_outputs(vec![OutputPort::Dynamic(DynamicPort {
            key: port_key!("rule"),
            source_field: String::new(),
            label_field: None,
            include_fallback: false,
        })]);

        let crate::ActionMetadataAdmissionError::Package(err) = draft
            .admit_for::<ValidationAction>(crate::ActionKind::Stateless)
            .unwrap_err()
        else {
            panic!("invalid ports must be a package admission failure");
        };
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
