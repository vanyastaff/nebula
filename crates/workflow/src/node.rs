//! Node (action step) definition within a workflow.

use std::{collections::HashMap, time::Duration};

use nebula_core::{ActionKey, NodeKey, prelude::KeyParseError};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::definition::RetryConfig;

/// A single action step inside a workflow graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDefinition {
    /// Unique node identifier within this workflow.
    pub id: NodeKey,
    /// Human-readable label.
    pub name: String,
    /// Which action/plugin this node runs (e.g. `"http_request"`, `"echo"`).
    pub action_key: ActionKey,
    /// Optional pinned interface version for the action.
    ///
    /// Uses `semver::Version` for exact-match dispatch. Flexible pinning
    /// (`VersionReq`, e.g. `^1.0` / `~1.2`) is a future-work item — see
    /// the `2026-04-17-replace-interfaceversion-with-semver` spec.
    #[serde(default)]
    pub interface_version: Option<Version>,
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
    /// Whether this node is active. Disabled nodes are skipped during execution.
    /// Useful for debugging workflows without removing nodes from the graph.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional rate limit for this node's action.
    /// Engine throttles execution to stay within the limit.
    #[serde(default)]
    pub rate_limit: Option<RateLimit>,
}

/// Rate limit configuration for an action.
///
/// Engine enforces this before dispatching the node — queuing requests
/// that exceed the limit.
///
/// # Examples
///
/// ```yaml
/// rate_limit:
///   max_requests: 60
///   window_secs: 60
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateLimit {
    /// Maximum requests allowed within the window.
    pub max_requests: u32,
    /// Window duration in seconds.
    pub window_secs: u64,
}

fn default_true() -> bool {
    true
}

impl NodeDefinition {
    /// Create a minimal node definition.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidActionKey`](crate::WorkflowError::InvalidActionKey) if `action_key` is not
    /// a valid [`ActionKey`] (lowercase alphanumeric, underscores, dots, hyphens).
    pub fn new(
        id: NodeKey,
        name: impl Into<String>,
        action_key: impl AsRef<str>,
    ) -> Result<Self, crate::WorkflowError> {
        let key_str = action_key.as_ref();
        let parsed_key =
            key_str
                .parse()
                .map_err(|e: KeyParseError| crate::WorkflowError::InvalidActionKey {
                    key: key_str.to_string(),
                    reason: e.to_string(),
                })?;
        Ok(Self {
            id,
            name: name.into(),
            action_key: parsed_key,
            interface_version: None,
            parameters: HashMap::new(),
            retry_policy: None,
            timeout: None,
            description: None,
            enabled: true,
            rate_limit: None,
        })
    }

    /// Pin an interface version.
    #[must_use]
    pub fn with_interface_version(mut self, version: Version) -> Self {
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

    /// Disable this node. Disabled nodes are skipped during execution.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// A parameter value that can be a literal, expression, template, or reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
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
        node_key: NodeKey,
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
    pub fn reference(node_key: NodeKey, output_path: impl Into<String>) -> Self {
        Self::Reference {
            node_key,
            output_path: output_path.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

    use super::*;

    #[test]
    fn new_rejects_invalid_action_key() {
        let result = NodeDefinition::new(node_key!("test"), "test", "INVALID KEY!!!");
        assert!(result.is_err());
    }

    #[test]
    fn new_accepts_valid_action_key() {
        let result = NodeDefinition::new(node_key!("test"), "test", "http_request");
        assert!(result.is_ok());
    }

    #[test]
    fn node_definition_new() {
        let id = node_key!("test");
        let node = NodeDefinition::new(id.clone(), "fetch", "http_request").unwrap();

        assert_eq!(node.id, id);
        assert_eq!(node.name, "fetch");
        assert_eq!(node.action_key.as_str(), "http_request");
        assert!(node.interface_version.is_none());
        assert!(node.parameters.is_empty());
        assert!(node.retry_policy.is_none());
        assert!(node.timeout.is_none());
        assert!(node.description.is_none());
    }

    #[test]
    fn node_definition_builder_methods() {
        let id = node_key!("test");
        let node = NodeDefinition::new(id, "fetch", "http_request")
            .unwrap()
            .with_interface_version(Version::new(1, 0, 0))
            .with_parameter(
                "url",
                ParamValue::literal(serde_json::json!("https://example.com")),
            )
            .with_retry(RetryConfig::fixed(3, 500))
            .with_timeout(Duration::from_secs(10))
            .with_description("Fetches data from the API");

        assert_eq!(node.interface_version, Some(Version::new(1, 0, 0)));
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
            },
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
        let source = node_key!("source");
        let pv = ParamValue::reference(source.clone(), "$.data.items");
        match pv {
            ParamValue::Reference {
                node_key,
                output_path,
            } => {
                assert_eq!(node_key, source);
                assert_eq!(output_path, "$.data.items");
            },
            _ => panic!("expected Reference"),
        }
    }

    #[test]
    fn param_value_serde_roundtrip_all_variants() {
        let source = node_key!("source");
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
        let id = node_key!("test");
        let node = NodeDefinition::new(id.clone(), "transform", "echo")
            .unwrap()
            .with_parameter("input", ParamValue::literal(serde_json::json!("data")))
            .with_timeout(Duration::from_secs(30));

        let json = serde_json::to_string(&node).unwrap();
        let back: NodeDefinition = serde_json::from_str(&json).unwrap();

        assert_eq!(back.id, id);
        assert_eq!(back.name, "transform");
        assert_eq!(back.action_key.as_str(), "echo");
        assert_eq!(back.timeout, Some(Duration::from_secs(30)));
        assert_eq!(back.parameters.len(), 1);
    }

    #[test]
    fn interface_version_serde_roundtrip_in_node() {
        let id = node_key!("test");
        let iv = Version::new(2, 3, 0);
        let node = NodeDefinition::new(id, "versioned", "echo")
            .unwrap()
            .with_interface_version(iv);

        let json = serde_json::to_string(&node).unwrap();
        let back: NodeDefinition = serde_json::from_str(&json).unwrap();

        assert_eq!(
            back.interface_version,
            Some(Version::new(2, 3, 0)),
            "semver::Version must survive serde roundtrip on NodeDefinition"
        );
    }
}
