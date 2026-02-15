//! Port definitions describing action input/output connection points.
//!
//! Every action declares its ports via [`InputPort`] and [`OutputPort`] enums
//! stored in [`ActionMetadata`](crate::ActionMetadata). Ports describe the
//! connection topology — how nodes wire together in a workflow graph.
//!
//! Three port semantics exist:
//!
//! - **Flow** — main data pipe; every action has at least one input and output.
//! - **Support** — sub-node / supply inputs (e.g. AI tool, memory, model slots).
//! - **Dynamic** — config-driven outputs generated from an array in node config
//!   (e.g. Switch node producing one output per rule).

use serde::{Deserialize, Serialize};

/// Type alias for port keys (e.g. `"in"`, `"out"`, `"error"`, `"tools"`).
pub type PortKey = String;

// ── FlowKind ────────────────────────────────────────────────────────────────

/// Discriminant for flow output ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    /// Primary data output.
    Main,
    /// Error output (appears when on-error handling is enabled).
    Error,
}

// ── ConnectionFilter ────────────────────────────────────────────────────────

/// Restricts which node types may connect to a [`SupportPort`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConnectionFilter {
    /// When set, only nodes whose type key is in this list may connect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_node_types: Option<Vec<String>>,
    /// When set, only nodes carrying at least one of these tags may connect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
}

impl ConnectionFilter {
    /// Create an empty (unrestricted) filter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict to specific node types.
    #[must_use]
    pub fn with_allowed_node_types(mut self, types: Vec<String>) -> Self {
        self.allowed_node_types = Some(types);
        self
    }

    /// Restrict to specific tags.
    #[must_use]
    pub fn with_allowed_tags(mut self, tags: Vec<String>) -> Self {
        self.allowed_tags = Some(tags);
        self
    }

    /// Returns `true` if no restrictions are set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allowed_node_types.is_none() && self.allowed_tags.is_none()
    }
}

// ── SupportPort ─────────────────────────────────────────────────────────────

/// A sub-node / supply input port (e.g. AI tool, memory, model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportPort {
    /// Unique key within this action's inputs (e.g. `"model"`, `"tools"`).
    pub key: PortKey,
    /// Human-readable display name (e.g. `"AI Model"`).
    pub name: String,
    /// Short description of what this port accepts.
    pub description: String,
    /// Whether a connection to this port is required for the action to run.
    #[serde(default)]
    pub required: bool,
    /// Whether multiple sub-nodes may connect to this port simultaneously.
    #[serde(default)]
    pub multi: bool,
    /// Restricts which node types may connect.
    #[serde(default)]
    pub filter: ConnectionFilter,
}

// ── DynamicPort ─────────────────────────────────────────────────────────────

/// A config-driven output port template (e.g. Switch node outputs).
///
/// At resolve time, the engine reads an array from the node's config at
/// `source_field` and generates one concrete output port per element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicPort {
    /// Base key prefix for generated ports (e.g. `"rule"` → `"rule_0"`, `"rule_1"`).
    pub key: PortKey,
    /// Config path to the array that drives port generation (e.g. `"rules"`).
    pub source_field: String,
    /// Optional field name within each array element to use as the port label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_field: Option<String>,
    /// Whether to append a fallback port after the generated ports.
    #[serde(default)]
    pub include_fallback: bool,
}

// ── InputPort ───────────────────────────────────────────────────────────────

/// An input port declaration on an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputPort {
    /// Main data flow input.
    Flow {
        /// Port key (e.g. `"in"`).
        key: PortKey,
    },
    /// Sub-node / supply input (e.g. AI tool, memory, model).
    Support(SupportPort),
}

impl InputPort {
    /// Create a flow input port.
    #[must_use]
    pub fn flow(key: impl Into<PortKey>) -> Self {
        Self::Flow { key: key.into() }
    }

    /// Create a support input port with sensible defaults.
    ///
    /// Defaults: `required = false`, `multi = false`, empty filter.
    #[must_use]
    pub fn support(
        key: impl Into<PortKey>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self::Support(SupportPort {
            key: key.into(),
            name: name.into(),
            description: description.into(),
            required: false,
            multi: false,
            filter: ConnectionFilter::default(),
        })
    }

    /// Returns the port key regardless of variant.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Flow { key } => key,
            Self::Support(p) => &p.key,
        }
    }

    /// Returns `true` if this is a flow port.
    #[must_use]
    pub fn is_flow(&self) -> bool {
        matches!(self, Self::Flow { .. })
    }

    /// Returns `true` if this is a support port.
    #[must_use]
    pub fn is_support(&self) -> bool {
        matches!(self, Self::Support(_))
    }
}

// ── OutputPort ──────────────────────────────────────────────────────────────

/// An output port declaration on an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputPort {
    /// Data or error flow output.
    Flow {
        /// Port key (e.g. `"out"`, `"error"`).
        key: PortKey,
        /// Whether this carries main data or error data.
        kind: FlowKind,
    },
    /// Config-driven dynamic outputs (e.g. Switch, Router).
    Dynamic(DynamicPort),
}

impl OutputPort {
    /// Create a main flow output port.
    #[must_use]
    pub fn flow(key: impl Into<PortKey>) -> Self {
        Self::Flow {
            key: key.into(),
            kind: FlowKind::Main,
        }
    }

    /// Create an error flow output port.
    #[must_use]
    pub fn error(key: impl Into<PortKey>) -> Self {
        Self::Flow {
            key: key.into(),
            kind: FlowKind::Error,
        }
    }

    /// Create a dynamic output port template.
    ///
    /// Defaults: no `label_field`, `include_fallback = false`.
    #[must_use]
    pub fn dynamic(key: impl Into<PortKey>, source_field: impl Into<String>) -> Self {
        Self::Dynamic(DynamicPort {
            key: key.into(),
            source_field: source_field.into(),
            label_field: None,
            include_fallback: false,
        })
    }

    /// Returns the port key regardless of variant.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Flow { key, .. } => key,
            Self::Dynamic(p) => &p.key,
        }
    }

    /// Returns `true` if this is a flow port.
    #[must_use]
    pub fn is_flow(&self) -> bool {
        matches!(self, Self::Flow { .. })
    }

    /// Returns `true` if this is a dynamic port.
    #[must_use]
    pub fn is_dynamic(&self) -> bool {
        matches!(self, Self::Dynamic(_))
    }
}

// ── Default factories ───────────────────────────────────────────────────────

/// Returns the default input ports: a single flow input `"in"`.
#[must_use]
pub fn default_input_ports() -> Vec<InputPort> {
    vec![InputPort::flow("in")]
}

/// Returns the default output ports: a single main flow output `"out"`.
#[must_use]
pub fn default_output_ports() -> Vec<OutputPort> {
    vec![OutputPort::flow("out")]
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── FlowKind ────────────────────────────────────────────────────

    #[test]
    fn flow_kind_serde_roundtrip() {
        for kind in [FlowKind::Main, FlowKind::Error] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: FlowKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn flow_kind_serde_values() {
        assert_eq!(serde_json::to_string(&FlowKind::Main).unwrap(), r#""main""#);
        assert_eq!(
            serde_json::to_string(&FlowKind::Error).unwrap(),
            r#""error""#
        );
    }

    // ── ConnectionFilter ────────────────────────────────────────────

    #[test]
    fn connection_filter_default_is_empty() {
        let f = ConnectionFilter::new();
        assert!(f.is_empty());
        assert!(f.allowed_node_types.is_none());
        assert!(f.allowed_tags.is_none());
    }

    #[test]
    fn connection_filter_builder() {
        let f = ConnectionFilter::new()
            .with_allowed_node_types(vec!["openai".into(), "anthropic".into()])
            .with_allowed_tags(vec!["llm".into()]);
        assert!(!f.is_empty());
        assert_eq!(f.allowed_node_types.as_ref().unwrap().len(), 2);
        assert_eq!(f.allowed_tags.as_ref().unwrap(), &["llm"]);
    }

    #[test]
    fn connection_filter_serde_roundtrip() {
        let f = ConnectionFilter::new()
            .with_allowed_node_types(vec!["slack".into()])
            .with_allowed_tags(vec!["messaging".into()]);
        let json = serde_json::to_string(&f).unwrap();
        let back: ConnectionFilter = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn connection_filter_empty_skips_serialization() {
        let f = ConnectionFilter::new();
        let json = serde_json::to_value(&f).unwrap();
        assert!(json.as_object().unwrap().is_empty());
    }

    // ── InputPort ───────────────────────────────────────────────────

    #[test]
    fn input_port_flow_constructor() {
        let port = InputPort::flow("in");
        assert_eq!(port.key(), "in");
        assert!(port.is_flow());
        assert!(!port.is_support());
    }

    #[test]
    fn input_port_support_constructor() {
        let port = InputPort::support("tools", "Tools", "Available tools");
        assert_eq!(port.key(), "tools");
        assert!(!port.is_flow());
        assert!(port.is_support());
        if let InputPort::Support(s) = &port {
            assert_eq!(s.name, "Tools");
            assert_eq!(s.description, "Available tools");
            assert!(!s.required);
            assert!(!s.multi);
            assert!(s.filter.is_empty());
        }
    }

    #[test]
    fn input_port_serde_roundtrip() {
        let ports = [
            InputPort::flow("in"),
            InputPort::support("model", "Model", "LLM model"),
        ];
        for port in &ports {
            let json = serde_json::to_string(port).unwrap();
            let back: InputPort = serde_json::from_str(&json).unwrap();
            assert_eq!(port, &back);
        }
    }

    #[test]
    fn input_port_flow_serde_tagged() {
        let port = InputPort::flow("in");
        let json = serde_json::to_value(&port).unwrap();
        assert_eq!(json["type"], "flow");
        assert_eq!(json["key"], "in");
    }

    #[test]
    fn input_port_support_serde_tagged() {
        let port = InputPort::support("tools", "Tools", "desc");
        let json = serde_json::to_value(&port).unwrap();
        assert_eq!(json["type"], "support");
        assert_eq!(json["key"], "tools");
        assert_eq!(json["name"], "Tools");
    }

    // ── OutputPort ──────────────────────────────────────────────────

    #[test]
    fn output_port_flow_constructor() {
        let port = OutputPort::flow("out");
        assert_eq!(port.key(), "out");
        assert!(port.is_flow());
        assert!(!port.is_dynamic());
        if let OutputPort::Flow { kind, .. } = &port {
            assert_eq!(*kind, FlowKind::Main);
        }
    }

    #[test]
    fn output_port_error_constructor() {
        let port = OutputPort::error("error");
        assert_eq!(port.key(), "error");
        assert!(port.is_flow());
        if let OutputPort::Flow { kind, .. } = &port {
            assert_eq!(*kind, FlowKind::Error);
        }
    }

    #[test]
    fn output_port_dynamic_constructor() {
        let port = OutputPort::dynamic("rule", "rules");
        assert_eq!(port.key(), "rule");
        assert!(port.is_dynamic());
        assert!(!port.is_flow());
        if let OutputPort::Dynamic(d) = &port {
            assert_eq!(d.source_field, "rules");
            assert!(d.label_field.is_none());
            assert!(!d.include_fallback);
        }
    }

    #[test]
    fn output_port_serde_roundtrip() {
        let ports = [
            OutputPort::flow("out"),
            OutputPort::error("error"),
            OutputPort::dynamic("rule", "rules"),
        ];
        for port in &ports {
            let json = serde_json::to_string(port).unwrap();
            let back: OutputPort = serde_json::from_str(&json).unwrap();
            assert_eq!(port, &back);
        }
    }

    #[test]
    fn output_port_flow_serde_tagged() {
        let port = OutputPort::flow("out");
        let json = serde_json::to_value(&port).unwrap();
        assert_eq!(json["type"], "flow");
        assert_eq!(json["key"], "out");
        assert_eq!(json["kind"], "main");
    }

    #[test]
    fn output_port_dynamic_serde_tagged() {
        let port = OutputPort::dynamic("rule", "rules");
        let json = serde_json::to_value(&port).unwrap();
        assert_eq!(json["type"], "dynamic");
        assert_eq!(json["key"], "rule");
        assert_eq!(json["source_field"], "rules");
    }

    // ── DynamicPort with options ────────────────────────────────────

    #[test]
    fn dynamic_port_with_label_and_fallback() {
        let port = OutputPort::Dynamic(DynamicPort {
            key: "rule".into(),
            source_field: "rules".into(),
            label_field: Some("label".into()),
            include_fallback: true,
        });
        if let OutputPort::Dynamic(d) = &port {
            assert_eq!(d.label_field.as_deref(), Some("label"));
            assert!(d.include_fallback);
        }
        let json = serde_json::to_string(&port).unwrap();
        let back: OutputPort = serde_json::from_str(&json).unwrap();
        assert_eq!(port, back);
    }

    // ── SupportPort with full config ────────────────────────────────

    #[test]
    fn support_port_full_config() {
        let port = InputPort::Support(SupportPort {
            key: "model".into(),
            name: "AI Model".into(),
            description: "Language model to use".into(),
            required: true,
            multi: false,
            filter: ConnectionFilter::new()
                .with_allowed_node_types(vec!["openai".into(), "anthropic".into()])
                .with_allowed_tags(vec!["llm".into()]),
        });
        assert_eq!(port.key(), "model");
        assert!(port.is_support());
        let json = serde_json::to_string(&port).unwrap();
        let back: InputPort = serde_json::from_str(&json).unwrap();
        assert_eq!(port, back);
    }

    // ── Defaults ────────────────────────────────────────────────────

    #[test]
    fn default_input_ports_single_flow() {
        let ports = default_input_ports();
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].key(), "in");
        assert!(ports[0].is_flow());
    }

    #[test]
    fn default_output_ports_single_flow() {
        let ports = default_output_ports();
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].key(), "out");
        assert!(ports[0].is_flow());
        if let OutputPort::Flow { kind, .. } = &ports[0] {
            assert_eq!(*kind, FlowKind::Main);
        }
    }
}
