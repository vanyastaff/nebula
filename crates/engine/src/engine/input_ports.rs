//! Named connections are support bindings, never implicit root-flow bindings.

use std::{collections::HashMap, sync::Arc};

use nebula_action::{ActionFactory, ActionMetadata, InputPort, SupportPort};
use nebula_core::{NodeKey, PortKey};
use nebula_workflow::{DependencyGraph, NodeDefinition};

use super::WorkflowEngine;
use crate::EngineError;

impl WorkflowEngine {
    #[tracing::instrument(name = "engine.input_ports.validate", skip_all, fields(
        node_count = node_map.len(),
    ))]
    pub(super) fn validate_declared_input_ports(
        &self,
        graph: &DependencyGraph,
        node_map: &HashMap<NodeKey, &NodeDefinition>,
        factories: Option<&HashMap<NodeKey, Arc<dyn ActionFactory>>>,
    ) -> Result<(), EngineError> {
        let mut violations = Vec::new();
        for (target_id, node) in node_map {
            if !node.enabled {
                continue;
            }
            let incoming = graph.incoming_connections(target_id.clone());
            let Some(metadata) = self.action_metadata(target_id, node, factories)? else {
                continue;
            };

            for connection in &incoming {
                let Some(port) = &connection.to_port else {
                    continue;
                };
                if !metadata
                    .inputs()
                    .iter()
                    .any(|input| input.is_support() && input.key() == port.as_str())
                {
                    violations.push(InputPortViolation::Unsupported {
                        from_node: connection.from_node.clone(),
                        to_node: target_id.clone(),
                        port: port.clone(),
                    });
                }
            }

            for input in metadata.inputs() {
                let InputPort::Support(port) = input else {
                    continue;
                };
                let enabled_connections = incoming
                    .iter()
                    .filter(|connection| connection.to_port.as_ref() == Some(&port.key))
                    .filter(|connection| {
                        node_map
                            .get(&connection.from_node)
                            .is_some_and(|source| source.enabled)
                    })
                    .copied()
                    .collect::<Vec<_>>();

                if port.required && enabled_connections.is_empty() {
                    violations.push(InputPortViolation::MissingRequired {
                        to_node: target_id.clone(),
                        port: port.key.clone(),
                    });
                }
                if !port.multi && enabled_connections.len() > 1 {
                    violations.push(InputPortViolation::Multiplicity {
                        to_node: target_id.clone(),
                        port: port.key.clone(),
                        actual: enabled_connections.len(),
                    });
                }
                for connection in enabled_connections {
                    let Some(source) = node_map.get(&connection.from_node) else {
                        continue;
                    };
                    let source_metadata =
                        self.action_metadata(&connection.from_node, source, factories)?;
                    if !support_filter_accepts(port, source, source_metadata.as_deref()) {
                        violations.push(InputPortViolation::Filtered {
                            from_node: connection.from_node.clone(),
                            to_node: target_id.clone(),
                            port: port.key.clone(),
                        });
                    }
                }
            }
        }
        violations.sort_unstable_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        if let Some(violation) = violations.into_iter().next() {
            return Err(violation.into_error());
        }
        Ok(())
    }

    fn action_metadata(
        &self,
        node_id: &NodeKey,
        node: &NodeDefinition,
        factories: Option<&HashMap<NodeKey, Arc<dyn ActionFactory>>>,
    ) -> Result<Option<Arc<ActionMetadata>>, EngineError> {
        if let Some(factories) = factories {
            return factories
                .get(node_id)
                .map(|factory| Some(Arc::clone(factory.metadata())))
                .ok_or(EngineError::ExactFactoryUnavailable);
        }
        let registered = match node.interface_version.as_ref() {
            Some(version) => self
                .runtime
                .registry()
                .get_factory_versioned(&node.action_key, version),
            None => self.runtime.registry().get_factory(&node.action_key),
        };
        Ok(registered.map(|(metadata, _)| metadata))
    }
}

fn support_filter_accepts(
    port: &SupportPort,
    source: &NodeDefinition,
    source_metadata: Option<&ActionMetadata>,
) -> bool {
    let type_allowed = port
        .filter
        .allowed_node_types
        .as_ref()
        .is_none_or(|allowed| {
            allowed
                .iter()
                .any(|candidate| candidate == source.action_key.as_str())
        });
    let tags_allowed = port.filter.allowed_tags.as_ref().is_none_or(|allowed| {
        source_metadata.is_some_and(|metadata| {
            metadata
                .base()
                .tags()
                .iter()
                .any(|tag| allowed.contains(tag))
        })
    });
    type_allowed && tags_allowed
}

enum InputPortViolation {
    Unsupported {
        from_node: NodeKey,
        to_node: NodeKey,
        port: PortKey,
    },
    MissingRequired {
        to_node: NodeKey,
        port: PortKey,
    },
    Multiplicity {
        to_node: NodeKey,
        port: PortKey,
        actual: usize,
    },
    Filtered {
        from_node: NodeKey,
        to_node: NodeKey,
        port: PortKey,
    },
}

impl InputPortViolation {
    fn sort_key(&self) -> (u8, &str, &str, &str) {
        match self {
            Self::Unsupported {
                from_node,
                to_node,
                port,
            } => (0, to_node.as_str(), port.as_str(), from_node.as_str()),
            Self::MissingRequired { to_node, port } => (1, to_node.as_str(), port.as_str(), ""),
            Self::Multiplicity { to_node, port, .. } => (2, to_node.as_str(), port.as_str(), ""),
            Self::Filtered {
                from_node,
                to_node,
                port,
            } => (3, to_node.as_str(), port.as_str(), from_node.as_str()),
        }
    }

    fn into_error(self) -> EngineError {
        match self {
            Self::Unsupported {
                from_node,
                to_node,
                port,
            } => {
                tracing::warn!(%from_node, %to_node, %port, error_code = "ENGINE:UNSUPPORTED_INPUT_PORT", "named input connection does not bind a declared support port");
                EngineError::UnsupportedInputPort {
                    from_node,
                    to_node,
                    port,
                }
            },
            Self::MissingRequired { to_node, port } => {
                tracing::warn!(%to_node, %port, error_code = "ENGINE:MISSING_SUPPORT_INPUT", "required support input is not connected");
                EngineError::MissingRequiredSupportInput { to_node, port }
            },
            Self::Multiplicity {
                to_node,
                port,
                actual,
            } => {
                tracing::warn!(%to_node, %port, actual, error_code = "ENGINE:SUPPORT_INPUT_MULTIPLICITY", "single-valued support input has multiple connections");
                EngineError::SupportInputMultiplicity {
                    to_node,
                    port,
                    actual,
                }
            },
            Self::Filtered {
                from_node,
                to_node,
                port,
            } => {
                tracing::warn!(%from_node, %to_node, %port, error_code = "ENGINE:SUPPORT_INPUT_FILTERED", "support input source rejected by declaration filter");
                EngineError::SupportInputFiltered {
                    from_node,
                    to_node,
                    port,
                }
            },
        }
    }
}
