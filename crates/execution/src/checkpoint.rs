//! Execution-owned node replay evidence, committed with the execution state.

use std::{collections::BTreeMap, fmt};

use nebula_core::NodeKey;
use serde::{Deserialize, Serialize};

/// Versioned per-node evidence from owner-processed outcomes.
///
/// Presence distinguishes a supported empty checkpoint from a legacy state
/// that never persisted replay evidence. Action payload integrity is checked by
/// the runtime; this lower-layer vocabulary does not depend on the action crate.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCheckpoint {
    format_version: u16,
    nodes: BTreeMap<NodeKey, NodeCheckpoint>,
}

impl ExecutionCheckpoint {
    /// Start a supported checkpoint with no committed node outcomes.
    #[must_use]
    pub fn empty_v1() -> Self {
        Self {
            format_version: 1,
            nodes: BTreeMap::new(),
        }
    }

    /// Recorded wire format; readers reject unsupported versions.
    #[must_use]
    pub const fn format_version(&self) -> u16 {
        self.format_version
    }

    /// Previously committed per-node outcomes.
    #[must_use]
    pub fn nodes(&self) -> &BTreeMap<NodeKey, NodeCheckpoint> {
        &self.nodes
    }

    /// Replace one owner-processed outcome, returning the previous value.
    pub fn insert(&mut self, node: NodeKey, checkpoint: NodeCheckpoint) -> Option<NodeCheckpoint> {
        self.nodes.insert(node, checkpoint)
    }

    /// Remove one recorded outcome, returning its previous value.
    pub fn remove(&mut self, node: &NodeKey) -> Option<NodeCheckpoint> {
        self.nodes.remove(node)
    }
}

impl fmt::Debug for ExecutionCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutionCheckpoint")
            .field("format_version", &self.format_version)
            .field("node_count", &self.nodes.len())
            .finish()
    }
}

/// Exact routing fact and payload for one processed node.
///
/// A real action result contains its output once. The runtime derives the raw
/// output projection from the checked result, avoiding duplicate durable payloads.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum NodeCheckpoint {
    /// Full action result, including its routing variant and all output ports.
    ActionResult {
        /// Version of the runtime-owned action-result wire format.
        format_version: u16,
        /// Opaque JSON checked by the action-owning runtime before replay.
        value: serde_json::Value,
    },
    /// A timer completed an existing wait and releases its main edge.
    TimerCompleted {
        /// Previously committed partial output retained when the timer completes.
        partial_output: Option<serde_json::Value>,
    },
    /// The node failed; only error edges may activate.
    Failed {
        /// Canonical error-port input when the failure has a handler.
        error_port_output: Option<serde_json::Value>,
    },
    /// The configured recovery policy substitutes a null main-port output.
    Recovered {},
    /// An explicitly disabled node was bypassed through its main edge.
    Bypassed {},
    /// A node was skipped without running an action and activates no edges.
    Skipped {},
}

impl fmt::Debug for NodeCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ActionResult { .. } => "NodeCheckpoint::ActionResult(<redacted>)",
            Self::TimerCompleted { .. } => "NodeCheckpoint::TimerCompleted(<redacted>)",
            Self::Failed { .. } => "NodeCheckpoint::Failed(<redacted>)",
            Self::Recovered {} => "NodeCheckpoint::Recovered",
            Self::Bypassed {} => "NodeCheckpoint::Bypassed",
            Self::Skipped {} => "NodeCheckpoint::Skipped",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_action_routing_facts_reject_unknown_fields() {
        for kind in ["recovered", "bypassed", "skipped"] {
            let encoded =
                serde_json::to_vec(&serde_json::json!({"kind":kind,"untrusted":"secret-canary"}))
                    .unwrap();
            assert!(
                serde_json::from_slice::<NodeCheckpoint>(&encoded).is_err(),
                "{kind} must reject unknown fields"
            );
        }
    }

    #[test]
    fn checkpoint_roundtrip_retains_format_and_redacts_payload() {
        let mut checkpoint = ExecutionCheckpoint::empty_v1();
        checkpoint.insert(
            nebula_core::node_key!("run"),
            NodeCheckpoint::ActionResult {
                format_version: 1,
                value: serde_json::json!({"private":"secret-canary"}),
            },
        );
        let encoded = serde_json::to_vec(&checkpoint).unwrap();
        let restored: ExecutionCheckpoint = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.format_version(), 1);
        assert_eq!(serde_json::to_vec(&restored).unwrap(), encoded);
        assert!(!format!("{restored:?} {:?}", restored.nodes()).contains("secret-canary"));
    }
}
