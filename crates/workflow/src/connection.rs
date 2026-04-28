//! Edge (connection) types linking workflow nodes.
//!
//! Connections are pure wires: they carry an output from one node's port to
//! another node's port and nothing else. All routing logic (conditionals,
//! error handling, branch selection) lives in explicit `ControlAction`
//! nodes — the trait is defined in `nebula_action::control`; the 7 canonical
//! implementations (`If`, `Switch`, `Router`, `Filter`, `NoOp`, `Stop`,
//! `Fail`) are shipped downstream in a reference / plugin crate, not in
//! this workspace. The shape of a workflow is therefore always visible on
//! the graph, never hiding inside edge metadata. Spec 28 §2.2 replaced the
//! previous `EdgeCondition` / `ResultMatcher` / `ErrorMatcher` trio with
//! this port-driven routing. Error routing follows the same model: failed
//! nodes activate only edges whose `from_port == "error"` (see
//! `engine::evaluate_edge`); authors wire that port into whichever
//! `ControlAction` (typically a `Switch` keyed on error class, or a
//! recovery node) fits their workflow.
//!
//! ## Edge activation contract
//!
//! An edge `A → B` with ports `(from_port, to_port)` activates when node
//! `A` produces an output on the **matching port**:
//!
//! | `A`'s `ActionResult` variant | Port chosen by engine             |
//! |------------------------------|------------------------------------|
//! | `Success`                    | `"main"` (or `from_port == None`)  |
//! | `Route { port }`             | `port`                             |
//! | `Branch { selected }`        | `selected` (legacy alias for Route)|
//! | `MultiOutput { outputs }`    | every port present in `outputs`    |
//! | Failed (error)               | `"error"`                          |
//! | `Skip` / `Drop` / `Terminate`| no edges activate                  |
//!
//! An edge with `from_port: None` is treated as `from_port: Some("main")`.

use nebula_core::NodeKey;
use serde::{Deserialize, Serialize};

/// A directed edge from one node's output port to another node's input port.
///
/// Edges are pure wires — they do not carry conditions, matchers, or
/// expressions. Authors wire conditional flow through explicit
/// `ControlAction` nodes (e.g. `If`, `Switch`, `Router`); the engine picks
/// which outgoing edge to activate based solely on the source node's output
/// port (see module docs for the canonical 7 and the error-routing model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// Source node.
    pub from_node: NodeKey,
    /// Target node.
    pub to_node: NodeKey,
    /// Source output port. `None` is interpreted as `Some("main")`.
    #[serde(default)]
    pub from_port: Option<String>,
    /// Target input port. `None` means the node's default flow input.
    #[serde(default)]
    pub to_port: Option<String>,
}

impl Connection {
    /// Create a connection on the default (main) output and input ports.
    #[must_use]
    pub fn new(from_node: NodeKey, to_node: NodeKey) -> Self {
        Self {
            from_node,
            to_node,
            from_port: None,
            to_port: None,
        }
    }

    /// Set the source output port.
    ///
    /// Use this to wire a specific branch of an `If` / `Switch` / `Router`
    /// node, or to pull from the `"error"` port of any action that routes
    /// failures explicitly.
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

    /// The effective source port — `from_port` if set, otherwise `"main"`.
    #[must_use]
    pub fn effective_from_port(&self) -> &str {
        self.from_port.as_deref().unwrap_or("main")
    }

    /// Returns `true` if this connection forms a self-loop.
    #[must_use]
    pub fn is_self_loop(&self) -> bool {
        self.from_node == self.to_node
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

    use super::*;

    #[test]
    fn connection_new_defaults_to_main_port() {
        let a = node_key!("a");
        let b = node_key!("b");
        let conn = Connection::new(a.clone(), b.clone());
        assert_eq!(conn.from_node, a);
        assert_eq!(conn.to_node, b);
        assert!(conn.from_port.is_none());
        assert!(conn.to_port.is_none());
        assert_eq!(conn.effective_from_port(), "main");
    }

    #[test]
    fn connection_is_self_loop() {
        let a = node_key!("a");
        let b = node_key!("b");
        assert!(Connection::new(a.clone(), a.clone()).is_self_loop());
        assert!(!Connection::new(a, b).is_self_loop());
    }

    #[test]
    fn connection_with_from_port_sets_effective_port() {
        let a = node_key!("a");
        let b = node_key!("b");
        let conn = Connection::new(a, b).with_from_port("error");
        assert_eq!(conn.effective_from_port(), "error");
    }

    #[test]
    fn connection_builder_methods() {
        let a = node_key!("a");
        let b = node_key!("b");
        let conn = Connection::new(a, b).with_ports("output_0", "model");
        assert_eq!(conn.from_port.as_deref(), Some("output_0"));
        assert_eq!(conn.to_port.as_deref(), Some("model"));
    }

    #[test]
    fn connection_serde_roundtrip() {
        let a = node_key!("a");
        let b = node_key!("b");
        let conn = Connection::new(a.clone(), b.clone())
            .with_from_port("true")
            .with_to_port("in");

        let json = serde_json::to_string(&conn).unwrap();
        let back: Connection = serde_json::from_str(&json).unwrap();
        assert_eq!(back.from_node, a);
        assert_eq!(back.to_node, b);
        assert_eq!(back.from_port.as_deref(), Some("true"));
        assert_eq!(back.to_port.as_deref(), Some("in"));
    }
}
