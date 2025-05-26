use derive_builder::Builder;
use serde::{Deserialize, Serialize};
use crate::{Parameter, ParameterError, ParameterMetadata, ParameterType, ParameterValue};

/// Parameter wrapper that adds routing capabilities to any parameter type.
/// This allows connecting a parameter to other nodes in the workflow.
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(pattern = "owned", setter(strip_option))]
pub struct RoutingParameter {
    /// The wrapped parameter that will have routing capabilities
    #[builder(setter)]
    pub parameter: Box<ParameterType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ParameterValue>,

    /// Configuration options for routing behavior
    #[builder(default, setter(strip_option))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_options: Option<RoutingParameterOptions>,
}

/// Configuration options for routing parameters
#[derive(Debug, Clone, Builder, Serialize, Deserialize, Default)]
#[builder(pattern = "owned", setter(strip_option), default)]
pub struct RoutingParameterOptions {
    /// Maximum number of connections allowed
    /// None means unlimited connections
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<usize>,

    /// Minimum number of connections required
    /// Default is 0 (no connections required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_connections: Option<usize>,
}

impl Parameter for RoutingParameter {
    fn metadata(&self) -> &ParameterMetadata {
        todo!()
    }

    fn get_value(&self) -> Option<&ParameterValue> {
        todo!()
    }

    fn set_value(&mut self, _value: ParameterValue) -> Result<(), ParameterError> {
        todo!()
    }
}