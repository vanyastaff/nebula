//! Action package validation (`metadata` + `ports`).

use std::collections::HashSet;

use crate::{
    metadata::ActionDefinition,
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
    /// An agent declared capability-gated isolation, which the runtime does
    /// not support for agents.
    #[error(
        "agent actions must run with IsolationLevel::None: capability-gated agent execution \
         is not supported"
    )]
    CapabilityGatedAgent,
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
    metadata: &ActionDefinition,
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
    if metadata.kind() == crate::ActionKind::Agent
        && metadata.isolation_level() != crate::IsolationLevel::None
    {
        errors.push(ActionPackageValidationError::CapabilityGatedAgent);
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

    errors
        .is_empty()
        .ok_or(ActionPackageValidationErrors { errors })
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{
        metadata::ActionInterface,
        port::{DynamicPort, SupportPort},
        port_key,
    };

    struct ValidationAction;

    impl crate::Component for ValidationAction {
        type Kind = crate::kind::Action;
        type Config = serde_json::Value;

        const KEY: &'static str = "test.action";

        fn metadata() -> crate::Metadata {
            crate::Metadata::new(crate::metadata_name!("Test"), "desc")
        }
    }

    impl crate::Action for ValidationAction {
        type Output = serde_json::Value;
    }

    fn admit_as(
        interface: ActionInterface,
        kind: crate::ActionKind,
    ) -> Result<ActionDefinition, crate::ActionAdmissionError> {
        use crate::Component as _;
        ActionDefinition::admit_with::<ValidationAction>(
            ValidationAction::metadata(),
            interface,
            kind,
        )
    }

    fn valid_metadata() -> ActionDefinition {
        admit_as(ActionInterface::default(), crate::ActionKind::Stateless)
            .expect("test metadata must admit")
    }

    #[test]
    fn valid_package_passes() {
        let meta = valid_metadata();
        assert!(validate_action_package(&meta).is_ok());
    }

    #[test]
    fn only_control_actions_may_terminate_without_output_ports() {
        let terminal = || ActionInterface {
            outputs: Vec::new(),
            ..ActionInterface::default()
        };

        let admitted = admit_as(terminal(), crate::ActionKind::Control)
            .expect("terminal control metadata must admit");
        assert!(admitted.outputs().is_empty());

        let crate::ActionAdmissionError::Package(error) =
            admit_as(terminal(), crate::ActionKind::Stateless)
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
    fn a_capability_gated_agent_is_refused_at_admission() {
        let gated = |isolation_level| ActionInterface {
            isolation_level,
            ..ActionInterface::default()
        };

        let crate::ActionAdmissionError::Package(error) = admit_as(
            gated(crate::IsolationLevel::CapabilityGated),
            crate::ActionKind::Agent,
        )
        .expect_err("the runtime never runs a capability-gated agent") else {
            panic!("a capability-gated agent must be a package admission failure");
        };
        assert_eq!(
            error.errors(),
            [ActionPackageValidationError::CapabilityGatedAgent]
        );

        admit_as(gated(crate::IsolationLevel::None), crate::ActionKind::Agent)
            .expect("an ungated agent admits");
        admit_as(
            gated(crate::IsolationLevel::CapabilityGated),
            crate::ActionKind::Stateless,
        )
        .expect("a capability-gated stateless action admits");
    }

    #[test]
    fn duplicate_ports_fail_validation() {
        let interface = ActionInterface {
            inputs: vec![
                InputPort::flow(port_key!("in")),
                InputPort::flow(port_key!("in")),
            ],
            outputs: vec![
                OutputPort::flow(port_key!("out")),
                OutputPort::error(port_key!("out")),
            ],
            ..ActionInterface::default()
        };

        let crate::ActionAdmissionError::Package(err) =
            admit_as(interface, crate::ActionKind::Stateless).unwrap_err()
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
        let interface = ActionInterface {
            inputs: vec![InputPort::Support(SupportPort {
                key: port_key!("tools"),
                name: String::new(),
                description: String::new(),
                required: false,
                multi: true,
                filter: Default::default(),
            })],
            outputs: vec![OutputPort::Dynamic(DynamicPort {
                key: port_key!("rule"),
                source_field: String::new(),
                label_field: None,
                include_fallback: false,
            })],
            ..ActionInterface::default()
        };

        let crate::ActionAdmissionError::Package(err) =
            admit_as(interface, crate::ActionKind::Stateless).unwrap_err()
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
