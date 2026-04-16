//! Execution planning — builds a parallel execution schedule from a workflow.

use chrono::{DateTime, Utc};
use nebula_core::{ExecutionId, NodeKey, WorkflowId};
use nebula_workflow::{DependencyGraph, WorkflowDefinition};
use serde::{Deserialize, Serialize};

use crate::{context::ExecutionBudget, error::ExecutionError};

/// A pre-computed execution plan derived from a workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    /// Execution this plan belongs to.
    pub execution_id: ExecutionId,
    /// Workflow this plan was derived from.
    pub workflow_id: WorkflowId,
    /// Parallel execution groups (each group can run concurrently).
    pub parallel_groups: Vec<Vec<NodeKey>>,
    /// Nodes with no predecessors (start points).
    pub entry_nodes: Vec<NodeKey>,
    /// Nodes with no successors (end points).
    pub exit_nodes: Vec<NodeKey>,
    /// Total number of nodes in the plan.
    pub total_nodes: usize,
    /// Resource budget for this execution.
    pub budget: ExecutionBudget,
    /// When this plan was created.
    pub created_at: DateTime<Utc>,
}

impl ExecutionPlan {
    /// Build an execution plan from a workflow definition.
    pub fn from_workflow(
        execution_id: ExecutionId,
        workflow: &WorkflowDefinition,
        budget: ExecutionBudget,
    ) -> Result<Self, ExecutionError> {
        if workflow.nodes.is_empty() {
            return Err(ExecutionError::PlanValidation(
                "workflow has no nodes".into(),
            ));
        }

        let graph = DependencyGraph::from_definition(workflow).map_err(|e| {
            ExecutionError::PlanValidation(format!("graph construction failed: {e}"))
        })?;

        let parallel_groups = graph.compute_levels().map_err(|e| {
            ExecutionError::PlanValidation(format!("level computation failed: {e}"))
        })?;

        let entry_nodes = graph.entry_nodes();
        let exit_nodes = graph.exit_nodes();
        let total_nodes = graph.node_count();

        Ok(Self {
            execution_id,
            workflow_id: workflow.id,
            parallel_groups,
            entry_nodes,
            exit_nodes,
            total_nodes,
            budget,
            created_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nebula_core::{WorkflowId, node_key};
    use nebula_workflow::{
        Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
    };

    use super::*;

    fn make_workflow(
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
    ) -> WorkflowDefinition {
        let now = Utc::now();
        WorkflowDefinition {
            id: WorkflowId::new(),
            name: "test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes,
            connections,
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            trigger: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: 1,
        }
    }

    fn node(id: NodeKey) -> NodeDefinition {
        NodeDefinition::new(id, "n", "n").unwrap()
    }

    #[test]
    fn plan_from_linear_workflow() {
        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let wf = make_workflow(
            vec![node(a.clone()), node(b.clone()), node(c.clone())],
            vec![
                Connection::new(a.clone(), b.clone()),
                Connection::new(b, c.clone()),
            ],
        );
        let plan =
            ExecutionPlan::from_workflow(ExecutionId::new(), &wf, ExecutionBudget::default())
                .unwrap();

        assert_eq!(plan.total_nodes, 3);
        assert_eq!(plan.parallel_groups.len(), 3);
        assert_eq!(plan.entry_nodes, vec![a]);
        assert_eq!(plan.exit_nodes, vec![c]);
    }

    #[test]
    fn plan_from_diamond_workflow() {
        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let d = node_key!("d");
        let wf = make_workflow(
            vec![
                node(a.clone()),
                node(b.clone()),
                node(c.clone()),
                node(d.clone()),
            ],
            vec![
                Connection::new(a.clone(), b.clone()),
                Connection::new(a, c.clone()),
                Connection::new(b, d.clone()),
                Connection::new(c, d),
            ],
        );
        let plan =
            ExecutionPlan::from_workflow(ExecutionId::new(), &wf, ExecutionBudget::default())
                .unwrap();

        assert_eq!(plan.total_nodes, 4);
        assert_eq!(plan.parallel_groups.len(), 3);
        // Middle level has 2 parallel nodes
        assert_eq!(plan.parallel_groups[1].len(), 2);
    }

    #[test]
    fn plan_rejects_empty_workflow() {
        let wf = make_workflow(vec![], vec![]);
        let err = ExecutionPlan::from_workflow(ExecutionId::new(), &wf, ExecutionBudget::default())
            .unwrap_err();
        assert!(err.to_string().contains("no nodes"));
    }

    #[test]
    fn plan_preserves_ids() {
        let exec_id = ExecutionId::new();
        let a = node_key!("a");
        let wf = make_workflow(vec![node(a)], vec![]);
        let plan = ExecutionPlan::from_workflow(exec_id, &wf, ExecutionBudget::default()).unwrap();

        assert_eq!(plan.execution_id, exec_id);
        assert_eq!(plan.workflow_id, wf.id);
    }

    #[test]
    fn plan_single_node() {
        let a = node_key!("a");
        let wf = make_workflow(vec![node(a.clone())], vec![]);
        let plan =
            ExecutionPlan::from_workflow(ExecutionId::new(), &wf, ExecutionBudget::default())
                .unwrap();

        assert_eq!(plan.total_nodes, 1);
        assert_eq!(plan.parallel_groups.len(), 1);
        assert_eq!(plan.entry_nodes, vec![a.clone()]);
        assert_eq!(plan.exit_nodes, vec![a]);
    }

    #[test]
    fn plan_serde_roundtrip() {
        let a = node_key!("a");
        let b = node_key!("b");
        let wf = make_workflow(
            vec![node(a.clone()), node(b.clone())],
            vec![Connection::new(a, b)],
        );
        let plan =
            ExecutionPlan::from_workflow(ExecutionId::new(), &wf, ExecutionBudget::default())
                .unwrap();

        let json = serde_json::to_string(&plan).unwrap();
        let back: ExecutionPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back.execution_id, plan.execution_id);
        assert_eq!(back.total_nodes, 2);
        assert_eq!(back.parallel_groups.len(), 2);
    }
}
