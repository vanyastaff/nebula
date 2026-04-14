//! Execution replay — re-run a workflow from a specific node.
//!
//! The replay contract partitions the workflow's node set into two
//! disjoint classes:
//!
//! - **pinned** — reuse the stored output from the source execution. Includes *every* node that is
//!   NOT forward-reachable from `replay_from`: strict ancestors, unrelated siblings, and every
//!   other branch of the DAG that happens to share no descendant path with the replay target.
//! - **rerun** — re-execute. Exactly `{replay_from} ∪ strict_descendants`.
//!
//! The earlier implementation computed `rerun = all_nodes \ ancestors`,
//! which silently re-executed unrelated sibling branches and duplicated
//! their side effects (sent emails, charged cards, posted webhooks).
//! Closes GitHub issue #254.
//!
//! `pinned_outputs` carries the stored output values for every pinned
//! node and MUST round-trip through serde. The previous
//! `#[serde(skip)]` dropped the whole map on persist, so a plan
//! reloaded from storage had nothing to feed downstream nodes and
//! replay silently re-executed the whole graph. Closes GitHub issue #253.

use std::collections::{HashMap, HashSet};

use nebula_core::id::{ExecutionId, NodeId};
use serde::{Deserialize, Serialize};

/// Plan for replaying a workflow execution from a specific node.
///
/// `replay_from` and its strict descendants are re-executed; every
/// other node reuses its stored output from the source execution. The
/// `pinned_outputs` map must be populated with outputs for every node
/// that the partition will classify as pinned — the executor pre-seeds
/// the workflow output map from this field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayPlan {
    /// The source execution to replay from.
    pub source_execution_id: ExecutionId,
    /// The node to start re-executing from.
    pub replay_from: NodeId,
    /// Optional input overrides for specific nodes.
    #[serde(default)]
    pub input_overrides: HashMap<NodeId, serde_json::Value>,
    /// Stored outputs from the source execution, keyed by node id.
    ///
    /// Populated at construction from the source execution's node-output
    /// table. Every entry for a node that the partition classifies as
    /// pinned is copied verbatim into the new execution's output map,
    /// so downstream rerun nodes see identical upstream data.
    ///
    /// This field serializes normally — persisting a `ReplayPlan`
    /// through storage must preserve it, otherwise a reloaded plan
    /// would have nothing to feed downstream nodes and the whole
    /// workflow would silently be re-executed (#253).
    #[serde(default)]
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

    /// Partition `all_nodes` into `(pinned, rerun)`.
    ///
    /// The `successors` map must contain every outgoing edge for every
    /// node in the graph (keyed by the source node). It can be built
    /// from the workflow definition by walking `connections` once; the
    /// engine's `replay_execution` does exactly that before calling in.
    ///
    /// # Partition semantics
    ///
    /// - `rerun` = `{replay_from}` plus every node strictly reachable forward from `replay_from`
    ///   via `successors`.
    /// - `pinned` = `all_nodes \ rerun` — i.e., ancestors, unrelated siblings, and any branch that
    ///   never touches the replay target.
    ///
    /// Nodes in `rerun` are re-executed from scratch. Nodes in `pinned`
    /// reuse their stored output from `pinned_outputs`. A node that is
    /// in `pinned` but missing from `pinned_outputs` is a bug in the
    /// caller — the partition contract is that every pinned node has a
    /// pre-recorded output.
    ///
    /// # Replay from root
    ///
    /// If `replay_from` is not in `all_nodes` (unknown node), the
    /// method treats it as "rerun everything reachable from it" and
    /// returns an empty `pinned` set. Callers should validate
    /// membership before relying on this.
    pub fn partition_nodes(
        &self,
        all_nodes: &[NodeId],
        successors: &HashMap<NodeId, Vec<NodeId>>,
    ) -> (HashSet<NodeId>, HashSet<NodeId>) {
        // Forward traversal: every node reachable from replay_from is
        // in the rerun set, including replay_from itself.
        let mut rerun = HashSet::new();
        let mut frontier = vec![self.replay_from];
        while let Some(node) = frontier.pop() {
            if rerun.insert(node)
                && let Some(succs) = successors.get(&node)
            {
                frontier.extend(succs.iter().copied());
            }
        }

        // Everything NOT in rerun is pinned — ancestors, unrelated
        // siblings, and any disconnected branch.
        let pinned: HashSet<NodeId> = all_nodes
            .iter()
            .copied()
            .filter(|n| !rerun.contains(n))
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

    /// Build a successors map from `(from, to)` connection pairs.
    fn successors_from(edges: &[(NodeId, NodeId)]) -> HashMap<NodeId, Vec<NodeId>> {
        let mut out: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for (from, to) in edges {
            out.entry(*from).or_default().push(*to);
        }
        out
    }

    #[test]
    fn partition_linear_chain() {
        // A → B → C, replay from B.
        // B and C are re-executed, A is pinned.
        let a = nid(1);
        let b = nid(2);
        let c = nid(3);
        let succ = successors_from(&[(a, b), (b, c)]);

        let plan = ReplayPlan::new(ExecutionId::new(), b);
        let (pinned, rerun) = plan.partition_nodes(&[a, b, c], &succ);

        assert!(pinned.contains(&a));
        assert!(rerun.contains(&b));
        assert!(rerun.contains(&c));
        assert_eq!(pinned.len(), 1);
        assert_eq!(rerun.len(), 2);
    }

    #[test]
    fn partition_diamond_replay_from_tail() {
        // A → B, A → C, B → D, C → D, replay from D.
        // Only D is re-executed; A, B, C all reuse their stored outputs.
        let a = nid(1);
        let b = nid(2);
        let c = nid(3);
        let d = nid(4);
        let succ = successors_from(&[(a, b), (a, c), (b, d), (c, d)]);

        let plan = ReplayPlan::new(ExecutionId::new(), d);
        let (pinned, rerun) = plan.partition_nodes(&[a, b, c, d], &succ);

        assert!(pinned.contains(&a));
        assert!(pinned.contains(&b));
        assert!(pinned.contains(&c));
        assert!(rerun.contains(&d));
        assert_eq!(pinned.len(), 3);
        assert_eq!(rerun.len(), 1);
    }

    /// Regression for issue #254: replaying from a diamond midpoint
    /// must NOT re-execute the sibling branch that never touches the
    /// replay target.
    #[test]
    fn partition_diamond_preserves_unrelated_sibling() {
        // A → B, A → C, B → D, C → D, replay from B.
        // Before #254 fix: rerun = {B, C, D} — C re-executed even
        // though it is completely unrelated to the replay path. After
        // the fix: rerun = {B, D}, pinned = {A, C}.
        let a = nid(1);
        let b = nid(2);
        let c = nid(3);
        let d = nid(4);
        let succ = successors_from(&[(a, b), (a, c), (b, d), (c, d)]);

        let plan = ReplayPlan::new(ExecutionId::new(), b);
        let (pinned, rerun) = plan.partition_nodes(&[a, b, c, d], &succ);

        assert!(pinned.contains(&a), "ancestor A must be pinned");
        assert!(
            pinned.contains(&c),
            "unrelated sibling C must be pinned — it is not a descendant of B"
        );
        assert!(rerun.contains(&b));
        assert!(rerun.contains(&d));
        assert_eq!(pinned.len(), 2);
        assert_eq!(rerun.len(), 2);
    }

    /// Regression for issue #254: completely disjoint DAG branches
    /// must not be re-executed when replay targets a different branch.
    #[test]
    fn partition_disjoint_branches() {
        // A → B,  X → Y — two unrelated subgraphs in one workflow.
        // Replay from A: rerun = {A, B}, pinned = {X, Y}.
        let a = nid(1);
        let b = nid(2);
        let x = nid(10);
        let y = nid(11);
        let succ = successors_from(&[(a, b), (x, y)]);

        let plan = ReplayPlan::new(ExecutionId::new(), a);
        let (pinned, rerun) = plan.partition_nodes(&[a, b, x, y], &succ);

        assert!(pinned.contains(&x));
        assert!(pinned.contains(&y));
        assert!(rerun.contains(&a));
        assert!(rerun.contains(&b));
    }

    #[test]
    fn partition_replay_from_root() {
        // No outgoing edges from A means rerun = {A} only.
        let a = nid(1);
        let b = nid(2);
        let succ = HashMap::new();

        let plan = ReplayPlan::new(ExecutionId::new(), a);
        let (pinned, rerun) = plan.partition_nodes(&[a, b], &succ);

        // A is re-executed; B is unrelated (no edge A→B in this fixture).
        assert!(rerun.contains(&a));
        assert!(pinned.contains(&b));
    }

    #[test]
    fn partition_replay_from_root_with_descendants() {
        // A → B → C, replay from A. Every node is re-executed.
        let a = nid(1);
        let b = nid(2);
        let c = nid(3);
        let succ = successors_from(&[(a, b), (b, c)]);

        let plan = ReplayPlan::new(ExecutionId::new(), a);
        let (pinned, rerun) = plan.partition_nodes(&[a, b, c], &succ);

        assert!(pinned.is_empty());
        assert_eq!(rerun.len(), 3);
    }

    /// Regression for issue #253: a `ReplayPlan` round-tripped through
    /// serde must preserve `pinned_outputs`. The previous
    /// `#[serde(skip)]` silently dropped the whole map.
    #[test]
    fn serde_round_trip_preserves_pinned_outputs() {
        let a = nid(1);
        let b = nid(2);
        let mut outputs = HashMap::new();
        outputs.insert(a, serde_json::json!({"result": "from_a"}));
        outputs.insert(b, serde_json::json!(42));

        let plan = ReplayPlan::new(ExecutionId::new(), b).with_pinned_outputs(outputs.clone());

        let json = serde_json::to_string(&plan).expect("serialize");
        let restored: ReplayPlan = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.pinned_outputs.len(), 2);
        assert_eq!(
            restored.pinned_outputs.get(&a),
            Some(&serde_json::json!({"result": "from_a"}))
        );
        assert_eq!(
            restored.pinned_outputs.get(&b),
            Some(&serde_json::json!(42))
        );
    }

    /// Guard against a future "optimization" that replaces the
    /// `HashSet::insert` short-circuit with an unconditional
    /// `frontier.extend` — that would loop forever on a cyclic
    /// successors map. DAGs are supposed to be acyclic, but the
    /// partition routine is defensive and must terminate on hostile
    /// input. Three nodes, one cycle: `A → B → C → A`, replay from A.
    #[test]
    fn partition_terminates_on_cyclic_successors() {
        let a = nid(1);
        let b = nid(2);
        let c = nid(3);
        let succ = successors_from(&[(a, b), (b, c), (c, a)]);

        let plan = ReplayPlan::new(ExecutionId::new(), a);
        let (pinned, rerun) = plan.partition_nodes(&[a, b, c], &succ);

        // Whole cycle is forward-reachable from A.
        assert_eq!(rerun.len(), 3);
        assert!(pinned.is_empty());
    }

    /// A replay plan referencing a node that has since been removed
    /// from the workflow is a stale plan. `partition_nodes` must not
    /// panic — it treats the unknown node as "rerun this node",
    /// leaves every real node in `pinned`, and lets the executor
    /// surface the missing pinned output via the new
    /// "missing pinned output" planning error at the engine layer.
    #[test]
    fn partition_replay_from_not_in_all_nodes() {
        let a = nid(1);
        let b = nid(2);
        let ghost = nid(99);
        let succ = successors_from(&[(a, b)]);

        let plan = ReplayPlan::new(ExecutionId::new(), ghost);
        let (pinned, rerun) = plan.partition_nodes(&[a, b], &succ);

        // The unknown node is the only rerun entry; real nodes are pinned.
        assert!(rerun.contains(&ghost));
        assert!(pinned.contains(&a));
        assert!(pinned.contains(&b));
    }

    /// A pre-fix plan (serialized without the `pinned_outputs` field)
    /// must still deserialize thanks to `#[serde(default)]` — the
    /// resulting plan has an empty map and the executor will surface
    /// the missing outputs loudly.
    #[test]
    fn serde_defaults_pinned_outputs_when_missing() {
        let a = nid(1);
        let json = format!(
            r#"{{"source_execution_id":"{}","replay_from":"{}"}}"#,
            ExecutionId::new(),
            a
        );
        let restored: ReplayPlan = serde_json::from_str(&json).expect("deserialize");
        assert!(restored.pinned_outputs.is_empty());
        assert!(restored.input_overrides.is_empty());
    }
}
