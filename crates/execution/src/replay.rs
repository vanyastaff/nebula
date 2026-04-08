//! Execution replay — re-run a workflow from a specific node.
//!
//! Nodes upstream of `replay_from` use stored outputs.
//! Nodes at and downstream of `replay_from` are re-executed.

use std::collections::{HashMap, HashSet};

use nebula_core::id::{ExecutionId, NodeId};
use serde::{Deserialize, Serialize};

/// Plan for replaying a workflow execution from a specific node.
///
/// Ancestors of `replay_from` use pinned (stored) outputs.
/// The node itself and all descendants are re-executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayPlan {
    /// The source execution to replay from.
    pub source_execution_id: ExecutionId,
    /// The node to start re-executing from.
    pub replay_from: NodeId,
    /// Optional input overrides for specific nodes.
    #[serde(default)]
    pub input_overrides: HashMap<NodeId, serde_json::Value>,
    /// Stored outputs from the source execution (pinned nodes).
    #[serde(skip)]
    pub pinned_outputs: HashMap<NodeId, serde_json::Value>,
}

impl ReplayPlan {
    /// Create a new replay plan.
    pub fn new(source_execution_id: ExecutionId, replay_from: NodeId) -> Self {
        Self {
            source_execution_id,
            replay_from,
            input_overrides: HashMap::new(),
            pinned_outputs: HashMap::new(),
        }
    }

    /// Override input for a specific node.
    #[must_use]
    pub fn with_override(mut self, node_id: NodeId, input: serde_json::Value) -> Self {
        self.input_overrides.insert(node_id, input);
        self
    }

    /// Set pinned outputs from the source execution.
    #[must_use]
    pub fn with_pinned_outputs(mut self, outputs: HashMap<NodeId, serde_json::Value>) -> Self {
        self.pinned_outputs = outputs;
        self
    }

    /// Determine which nodes are pinned (use stored output) vs re-executed.
    ///
    /// Walks the graph backwards from `replay_from` — all strict ancestors
    /// are pinned, `replay_from` and its descendants are re-executed.
    pub fn partition_nodes(
        &self,
        all_nodes: &[NodeId],
        predecessors: &HashMap<NodeId, Vec<NodeId>>,
    ) -> (HashSet<NodeId>, HashSet<NodeId>) {
        // Find all ancestors of replay_from (these are pinned).
        let mut pinned = HashSet::new();
        let mut ancestor_queue: Vec<NodeId> = predecessors
            .get(&self.replay_from)
            .cloned()
            .unwrap_or_default();

        while let Some(node) = ancestor_queue.pop() {
            if pinned.insert(node)
                && let Some(preds) = predecessors.get(&node)
            {
                ancestor_queue.extend(preds.iter().copied());
            }
        }

        // Everything not pinned is re-executed.
        let rerun: HashSet<NodeId> = all_nodes
            .iter()
            .copied()
            .filter(|n| !pinned.contains(n))
            .collect();

        (pinned, rerun)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nid(n: u128) -> NodeId {
        NodeId::from(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn partition_linear_chain() {
        // A → B → C, replay from B
        let a = nid(1);
        let b = nid(2);
        let c = nid(3);

        let mut preds = HashMap::new();
        preds.insert(b, vec![a]);
        preds.insert(c, vec![b]);

        let plan = ReplayPlan::new(ExecutionId::new(), b);
        let (pinned, rerun) = plan.partition_nodes(&[a, b, c], &preds);

        assert!(pinned.contains(&a));
        assert!(rerun.contains(&b));
        assert!(rerun.contains(&c));
        assert_eq!(pinned.len(), 1);
        assert_eq!(rerun.len(), 2);
    }

    #[test]
    fn partition_diamond() {
        // A → B, A → C, B → D, C → D, replay from D
        let a = nid(1);
        let b = nid(2);
        let c = nid(3);
        let d = nid(4);

        let mut preds = HashMap::new();
        preds.insert(b, vec![a]);
        preds.insert(c, vec![a]);
        preds.insert(d, vec![b, c]);

        let plan = ReplayPlan::new(ExecutionId::new(), d);
        let (pinned, rerun) = plan.partition_nodes(&[a, b, c, d], &preds);

        assert!(pinned.contains(&a));
        assert!(pinned.contains(&b));
        assert!(pinned.contains(&c));
        assert!(rerun.contains(&d));
    }

    #[test]
    fn partition_replay_from_root() {
        // Replay from root = re-execute everything
        let a = nid(1);
        let b = nid(2);
        let preds = HashMap::new();

        let plan = ReplayPlan::new(ExecutionId::new(), a);
        let (pinned, rerun) = plan.partition_nodes(&[a, b], &preds);

        assert!(pinned.is_empty());
        assert_eq!(rerun.len(), 2);
    }
}
