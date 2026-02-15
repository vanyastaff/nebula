//! Edge (connection) types linking workflow nodes.

use nebula_core::NodeId;
use serde::{Deserialize, Serialize};

/// A directed edge from one node to another, optionally conditional.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    /// Source node.
    pub from_node: NodeId,
    /// Target node.
    pub to_node: NodeId,
    /// When the edge should be traversed.
    #[serde(default)]
    pub condition: EdgeCondition,
    /// Optional branch key (e.g., "true" / "false" for if-nodes).
    #[serde(default)]
    pub branch_key: Option<String>,
    /// Source output port (e.g., "error", "output_0"). `None` means default output.
    #[serde(default)]
    pub from_port: Option<String>,
    /// Target input port (e.g., "model", "tools"). `None` means default flow input.
    #[serde(default)]
    pub to_port: Option<String>,
}

impl Connection {
    /// Create an unconditional connection.
    #[must_use]
    pub fn new(from_node: NodeId, to_node: NodeId) -> Self {
        Self {
            from_node,
            to_node,
            condition: EdgeCondition::Always,
            branch_key: None,
            from_port: None,
            to_port: None,
        }
    }

    /// Set the edge condition.
    #[must_use]
    pub fn with_condition(mut self, condition: EdgeCondition) -> Self {
        self.condition = condition;
        self
    }

    /// Set the branch key.
    #[must_use]
    pub fn with_branch_key(mut self, key: impl Into<String>) -> Self {
        self.branch_key = Some(key.into());
        self
    }

    /// Set the source output port.
    #[must_use]
    pub fn with_from_port(mut self, port: impl Into<String>) -> Self {
        self.from_port = Some(port.into());
        self
    }

    /// Set the target input port.
    #[must_use]
    pub fn with_to_port(mut self, port: impl Into<String>) -> Self {
        self.to_port = Some(port.into());
        self
    }

    /// Set both source and target ports.
    #[must_use]
    pub fn with_ports(mut self, from_port: impl Into<String>, to_port: impl Into<String>) -> Self {
        self.from_port = Some(from_port.into());
        self.to_port = Some(to_port.into());
        self
    }

    /// Returns `true` if this connection forms a self-loop.
    #[must_use]
    pub fn is_self_loop(&self) -> bool {
        self.from_node == self.to_node
    }
}

/// Condition that determines whether an edge is traversed.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EdgeCondition {
    /// Always traverse this edge.
    #[default]
    Always,
    /// Evaluate an expression at runtime.
    Expression {
        /// The expression to evaluate.
        expr: String,
    },
    /// Traverse when the source node's result matches.
    OnResult {
        /// The result matcher.
        matcher: ResultMatcher,
    },
    /// Traverse when the source node produces an error that matches.
    OnError {
        /// The error matcher.
        matcher: ErrorMatcher,
    },
}

/// Matches against a node's successful output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResultMatcher {
    /// Match any successful result.
    Success,
    /// Match when a specific output field equals a value.
    FieldEquals {
        /// The field name.
        field: String,
        /// The expected value.
        value: serde_json::Value,
    },
    /// Match via an expression evaluated against the output.
    Expression {
        /// The expression to evaluate.
        expr: String,
    },
}

/// Matches against a node's error output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ErrorMatcher {
    /// Match any error.
    Any,
    /// Match a specific error code.
    Code {
        /// The error code to match.
        code: String,
    },
    /// Match via an expression evaluated against the error.
    Expression {
        /// The expression to evaluate.
        expr: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_new() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let conn = Connection::new(a, b);
        assert_eq!(conn.from_node, a);
        assert_eq!(conn.to_node, b);
        assert!(matches!(conn.condition, EdgeCondition::Always));
        assert!(conn.branch_key.is_none());
        assert!(conn.from_port.is_none());
        assert!(conn.to_port.is_none());
    }

    #[test]
    fn connection_is_self_loop() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        assert!(Connection::new(a, a).is_self_loop());
        assert!(!Connection::new(a, b).is_self_loop());
    }

    #[test]
    fn connection_builder_methods() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let conn = Connection::new(a, b)
            .with_condition(EdgeCondition::Expression {
                expr: "x > 0".into(),
            })
            .with_branch_key("true")
            .with_from_port("output_0")
            .with_to_port("model");

        assert!(matches!(conn.condition, EdgeCondition::Expression { .. }));
        assert_eq!(conn.branch_key.as_deref(), Some("true"));
        assert_eq!(conn.from_port.as_deref(), Some("output_0"));
        assert_eq!(conn.to_port.as_deref(), Some("model"));
    }

    #[test]
    fn edge_condition_default_is_always() {
        let cond = EdgeCondition::default();
        assert!(matches!(cond, EdgeCondition::Always));
    }

    #[test]
    fn edge_condition_variants() {
        let _always = EdgeCondition::Always;
        let _expr = EdgeCondition::Expression {
            expr: "true".into(),
        };
        let _on_result = EdgeCondition::OnResult {
            matcher: ResultMatcher::Success,
        };
        let _on_error = EdgeCondition::OnError {
            matcher: ErrorMatcher::Any,
        };
    }

    #[test]
    fn connection_serde_roundtrip() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let conn = Connection::new(a, b)
            .with_condition(EdgeCondition::OnResult {
                matcher: ResultMatcher::FieldEquals {
                    field: "status".into(),
                    value: serde_json::json!(200),
                },
            })
            .with_branch_key("success");

        let json = serde_json::to_string(&conn).unwrap();
        let back: Connection = serde_json::from_str(&json).unwrap();
        assert_eq!(back.from_node, a);
        assert_eq!(back.to_node, b);
        assert_eq!(back.branch_key.as_deref(), Some("success"));
    }

    #[test]
    fn edge_condition_serde_roundtrip_all_variants() {
        let conditions = [
            EdgeCondition::Always,
            EdgeCondition::Expression {
                expr: "a > b".into(),
            },
            EdgeCondition::OnResult {
                matcher: ResultMatcher::Success,
            },
            EdgeCondition::OnResult {
                matcher: ResultMatcher::FieldEquals {
                    field: "ok".into(),
                    value: serde_json::json!(true),
                },
            },
            EdgeCondition::OnResult {
                matcher: ResultMatcher::Expression {
                    expr: "len > 0".into(),
                },
            },
            EdgeCondition::OnError {
                matcher: ErrorMatcher::Any,
            },
            EdgeCondition::OnError {
                matcher: ErrorMatcher::Code {
                    code: "TIMEOUT".into(),
                },
            },
            EdgeCondition::OnError {
                matcher: ErrorMatcher::Expression {
                    expr: "retryable".into(),
                },
            },
        ];

        for cond in &conditions {
            let json = serde_json::to_string(cond).unwrap();
            let back: EdgeCondition = serde_json::from_str(&json).unwrap();
            let json_back = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json_back);
        }
    }

    #[test]
    fn result_matcher_serde_tagged() {
        let matcher = ResultMatcher::FieldEquals {
            field: "code".into(),
            value: serde_json::json!(200),
        };
        let json = serde_json::to_value(&matcher).unwrap();
        assert_eq!(json["type"], "field_equals");
        assert_eq!(json["field"], "code");
        assert_eq!(json["value"], 200);
    }

    #[test]
    fn error_matcher_serde_tagged() {
        let matcher = ErrorMatcher::Code {
            code: "NOT_FOUND".into(),
        };
        let json = serde_json::to_value(&matcher).unwrap();
        assert_eq!(json["type"], "code");
        assert_eq!(json["code"], "NOT_FOUND");
    }
}
