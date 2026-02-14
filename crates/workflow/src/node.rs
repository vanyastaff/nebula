//! Node (action step) definition within a workflow.

use std::collections::HashMap;
use std::time::Duration;

use nebula_core::{ActionId, InterfaceVersion, NodeId};
use serde::{Deserialize, Serialize};

use crate::definition::RetryConfig;

/// A single action step inside a workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDefinition {
    /// Unique node identifier within this workflow.
    pub id: NodeId,
    /// Human-readable label.
    pub name: String,
    /// Which action this node executes.
    pub action_id: ActionId,
    /// Optional pinned interface version for the action.
    #[serde(default)]
    pub interface_version: Option<InterfaceVersion>,
    /// Parameters passed to the action at runtime.
    #[serde(default)]
    pub parameters: HashMap<String, ParamValue>,
    /// Node-level retry policy (overrides the workflow default).
    #[serde(default)]
    pub retry_policy: Option<RetryConfig>,
    /// Node-level timeout (overrides the workflow default).
    #[serde(default, with = "crate::serde_duration_opt")]
    pub timeout: Option<Duration>,
    /// Optional description of what this node does.
    #[serde(default)]
    pub description: Option<String>,
}

impl NodeDefinition {
    /// Create a minimal node definition.
    #[must_use]
    pub fn new(id: NodeId, name: impl Into<String>, action_id: ActionId) -> Self {
        Self {
            id,
            name: name.into(),
            action_id,
            interface_version: None,
            parameters: HashMap::new(),
            retry_policy: None,
            timeout: None,
            description: None,
        }
    }

    /// Pin an interface version.
    #[must_use]
    pub fn with_interface_version(mut self, version: InterfaceVersion) -> Self {
        self.interface_version = Some(version);
        self
    }

    /// Add a parameter.
    #[must_use]
    pub fn with_parameter(mut self, key: impl Into<String>, value: ParamValue) -> Self {
        self.parameters.insert(key.into(), value);
        self
    }

    /// Set a node-level retry policy.
    #[must_use]
    pub fn with_retry(mut self, retry: RetryConfig) -> Self {
        self.retry_policy = Some(retry);
        self
    }

    /// Set a node-level timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set a description.
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

/// A parameter value that can be a literal, expression, template, or reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamValue {
    /// A static JSON value.
    Literal {
        /// The literal value.
        value: serde_json::Value,
    },
    /// An expression to be evaluated at runtime.
    Expression {
        /// The expression string.
        expr: String,
    },
    /// A template string with interpolation placeholders.
    Template {
        /// The template string.
        template: String,
    },
    /// A reference to the output of another node.
    Reference {
        /// The source node producing the output.
        node_id: NodeId,
        /// JSONPath-like path into the source node's output.
        output_path: String,
    },
}

impl ParamValue {
    /// Construct a literal parameter.
    #[must_use]
    pub fn literal(value: serde_json::Value) -> Self {
        Self::Literal { value }
    }

    /// Construct an expression parameter.
    #[must_use]
    pub fn expression(expr: impl Into<String>) -> Self {
        Self::Expression { expr: expr.into() }
    }

    /// Construct a template parameter.
    #[must_use]
    pub fn template(template: impl Into<String>) -> Self {
        Self::Template {
            template: template.into(),
        }
    }

    /// Construct a reference parameter.
    #[must_use]
    pub fn reference(node_id: NodeId, output_path: impl Into<String>) -> Self {
        Self::Reference {
            node_id,
            output_path: output_path.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::ActionId;

    #[test]
    fn node_definition_new() {
        let id = NodeId::v4();
        let action_id = ActionId::v4();
        let node = NodeDefinition::new(id, "fetch", action_id);

        assert_eq!(node.id, id);
        assert_eq!(node.name, "fetch");
        assert_eq!(node.action_id, action_id);
        assert!(node.interface_version.is_none());
        assert!(node.parameters.is_empty());
        assert!(node.retry_policy.is_none());
        assert!(node.timeout.is_none());
        assert!(node.description.is_none());
    }

    #[test]
    fn node_definition_builder_methods() {
        let id = NodeId::v4();
        let action_id = ActionId::v4();
        let node = NodeDefinition::new(id, "fetch", action_id)
            .with_interface_version(InterfaceVersion::new(1, 0))
            .with_parameter(
                "url",
                ParamValue::literal(serde_json::json!("https://example.com")),
            )
            .with_retry(RetryConfig::fixed(3, 500))
            .with_timeout(Duration::from_secs(10))
            .with_description("Fetches data from the API");

        assert_eq!(node.interface_version, Some(InterfaceVersion::new(1, 0)));
        assert_eq!(node.parameters.len(), 1);
        assert!(node.retry_policy.is_some());
        assert_eq!(node.timeout, Some(Duration::from_secs(10)));
        assert_eq!(
            node.description.as_deref(),
            Some("Fetches data from the API")
        );
    }

    #[test]
    fn param_value_literal() {
        let pv = ParamValue::literal(serde_json::json!(42));
        match pv {
            ParamValue::Literal { value } => assert_eq!(value, serde_json::json!(42)),
            _ => panic!("expected Literal"),
        }
    }

    #[test]
    fn param_value_expression() {
        let pv = ParamValue::expression("{{ nodes.a.output.count + 1 }}");
        match pv {
            ParamValue::Expression { expr } => {
                assert_eq!(expr, "{{ nodes.a.output.count + 1 }}")
            }
            _ => panic!("expected Expression"),
        }
    }

    #[test]
    fn param_value_template() {
        let pv = ParamValue::template("Hello, {{ name }}!");
        match pv {
            ParamValue::Template { template } => assert_eq!(template, "Hello, {{ name }}!"),
            _ => panic!("expected Template"),
        }
    }

    #[test]
    fn param_value_reference() {
        let source = NodeId::v4();
        let pv = ParamValue::reference(source, "$.data.items");
        match pv {
            ParamValue::Reference {
                node_id,
                output_path,
            } => {
                assert_eq!(node_id, source);
                assert_eq!(output_path, "$.data.items");
            }
            _ => panic!("expected Reference"),
        }
    }

    #[test]
    fn param_value_serde_roundtrip_all_variants() {
        let source = NodeId::v4();
        let values = [
            ParamValue::literal(serde_json::json!({"key": "value"})),
            ParamValue::expression("1 + 2"),
            ParamValue::template("Hello {{ world }}"),
            ParamValue::reference(source, "$.out"),
        ];

        for original in &values {
            let json = serde_json::to_string(original).unwrap();
            let back: ParamValue = serde_json::from_str(&json).unwrap();
            // Compare via re-serialization
            let json_back = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json_back);
        }
    }

    #[test]
    fn param_value_serde_tagged_format() {
        let pv = ParamValue::literal(serde_json::json!(true));
        let json = serde_json::to_value(&pv).unwrap();
        assert_eq!(json["type"], "literal");
        assert_eq!(json["value"], true);
    }

    #[test]
    fn node_definition_serde_roundtrip() {
        let id = NodeId::v4();
        let action_id = ActionId::v4();
        let node = NodeDefinition::new(id, "transform", action_id)
            .with_parameter("input", ParamValue::literal(serde_json::json!("data")))
            .with_timeout(Duration::from_secs(30));

        let json = serde_json::to_string(&node).unwrap();
        let back: NodeDefinition = serde_json::from_str(&json).unwrap();

        assert_eq!(back.id, id);
        assert_eq!(back.name, "transform");
        assert_eq!(back.action_id, action_id);
        assert_eq!(back.timeout, Some(Duration::from_secs(30)));
        assert_eq!(back.parameters.len(), 1);
    }
}
