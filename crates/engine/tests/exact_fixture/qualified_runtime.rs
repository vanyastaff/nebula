//! Exact runtime assembly for fixtures whose action keys already contain a namespace.

use std::sync::Arc;

use nebula_engine::{ActionRegistry, PlanFlavorRevisionLoader, WorkflowEngine};
use nebula_execution::ExecutionState;
use nebula_plugin::FrozenPluginRegistry;
use nebula_storage::InMemoryExecutionStore;
use nebula_storage_port::{Scope, store::WorkflowVersionStore};

pub(crate) struct QualifiedRuntime {
    frozen: Arc<FrozenPluginRegistry>,
}

impl QualifiedRuntime {
    pub(crate) fn new(registry: &ActionRegistry) -> Self {
        let keys = registry.keys();
        let actions: Vec<_> = keys
            .iter()
            .map(|key| {
                let key = key.as_str();
                let (plugin, _) = key.split_once('.').expect("qualified fixture action");
                (plugin, key)
            })
            .collect();
        Self {
            frozen: super::exact_fixture::freeze_registry(registry, &actions),
        }
    }

    pub(crate) fn attach(
        &self,
        engine: WorkflowEngine,
        execution: &InMemoryExecutionStore,
    ) -> WorkflowEngine {
        engine.with_plan_flavor_runtime(
            Arc::new(PlanFlavorRevisionLoader::new(Arc::new(
                execution.plan_flavor_catalog(),
            ))),
            Arc::clone(&self.frozen),
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                execution,
            )),
        )
    }

    pub(crate) async fn pin(
        &self,
        execution: &InMemoryExecutionStore,
        versions: &dyn WorkflowVersionStore,
        scope: &Scope,
        state: &mut ExecutionState,
    ) {
        let record = versions
            .get(scope, &state.workflow_id.to_string(), 0)
            .await
            .unwrap()
            .expect("fixture version zero");
        let encoded = serde_json::to_string(&record.definition).unwrap();
        let mut workflow: nebula_workflow::WorkflowDefinition =
            serde_json::from_str(&encoded).unwrap();
        // Legacy fixtures declared `core` for their already-qualified `test.*` keys.
        // Correct that fixture namespace before compiling the exact execution graph.
        for node in &mut workflow.nodes {
            if node.plugin_key.as_str() != "core" {
                continue;
            }
            let (plugin, _) = node
                .action_key
                .as_str()
                .split_once('.')
                .expect("qualified fixture action");
            node.plugin_key = plugin.parse().expect("fixture plugin key");
        }
        super::exact_fixture::materialize_state(execution, scope, &self.frozen, &workflow, state)
            .await;
    }
}
