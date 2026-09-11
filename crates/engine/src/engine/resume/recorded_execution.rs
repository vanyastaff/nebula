//! Validation of the immutable execution contract used by a durable turn.

use super::*;

pub(super) struct ValidatedRecordedExecution {
    pub(super) loaded: crate::revision_catalog::LoadedPlanFlavorRevision,
    pub(super) workflow: nebula_plugin::ExecutableGraph,
    pub(super) persisted_outputs: Vec<(NodeKey, serde_json::Value)>,
    pub(super) factories: HashMap<NodeKey, Arc<dyn nebula_action::ActionFactory>>,
}

impl WorkflowEngine {
    pub(super) async fn validate_recorded_execution(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        state: &ExecutionState,
    ) -> Result<ValidatedRecordedExecution, EngineError> {
        let (Some(plan_id), Some(flavor_id)) = (
            state.executable_plan_revision_id,
            state.worker_flavor_revision_id,
        ) else {
            return Err(EngineError::MissingRevisionPins);
        };
        let exact_runtime = self
            .plan_flavor_runtime
            .as_ref()
            .ok_or(EngineError::MissingExactRuntime)?;
        let execution_key = execution_id.to_string();
        let stored = exact_runtime
            .bundles
            .read_contract_bundle(scope, &execution_key)
            .await
            .map_err(|source| EngineError::ContractBundleRead { source })?
            .ok_or(EngineError::MissingContractBundle)?;
        let bundle = crate::recorded_contract::checked_bundle(scope, &execution_key, &stored)
            .map_err(|rejection| match rejection {
                crate::recorded_contract::RecordedContractRejection::Integrity(source) => {
                    EngineError::ContractBundleIntegrity { source }
                },
                crate::recorded_contract::RecordedContractRejection::Identity
                | crate::recorded_contract::RecordedContractRejection::Malformed => {
                    EngineError::InvalidRecordedContract
                },
            })?;
        if plan_id != bundle.executable_plan_revision_id()
            || flavor_id != bundle.revisions().worker_flavor()
            || state
                .workflow_version_number
                .is_none_or(|number| number == 0)
        {
            return Err(EngineError::InvalidRecordedContract);
        }

        let loaded = exact_runtime
            .loader
            .load_exact(
                nebula_storage_port::PlanFlavorRevisionIds::new(
                    bundle.executable_plan_revision_id(),
                    bundle.revisions().worker_flavor(),
                ),
                Arc::clone(&exact_runtime.registry),
            )
            .await
            .map_err(|source| EngineError::ExactRevision {
                source: Box::new(source),
            })?;
        if loaded.plan().workflow_id() != workflow_id
            || loaded.plan().workflow_version_id() != bundle.revisions().workflow()
            || loaded.plan().plugin_set_id() != bundle.plugin_set_id()
        {
            return Err(EngineError::InvalidRecordedContract);
        }
        if !loaded.plan().bindings().is_empty() || !bundle.authorized_credential_ids().is_empty() {
            return Err(EngineError::UnresolvedPlanBindings);
        }

        let workflow = loaded
            .plan()
            .execution_graph()
            .map_err(|source| EngineError::ExactGraphProjection { source })?;
        crate::recorded_graph::validate_recorded_graph(&workflow).map_err(|rejection| {
            match rejection {
                crate::recorded_graph::RecordedGraphRejection::UnresolvedBindings => {
                    EngineError::UnresolvedPlanBindings
                },
                crate::recorded_graph::RecordedGraphRejection::UnsupportedSemantics => {
                    EngineError::UnsupportedRecordedSemantics
                },
            }
        })?;

        let node_keys = workflow
            .nodes()
            .iter()
            .map(|node| node.id.clone())
            .collect();
        let disabled_nodes = workflow
            .nodes()
            .iter()
            .filter(|node| !node.enabled)
            .map(|node| node.id.clone())
            .collect();
        let persisted_outputs =
            checkpoint::validated_checkpoint_outputs(state, &node_keys, &disabled_nodes)?;
        let factories = workflow
            .nodes()
            .iter()
            .map(|node| {
                let factory = loaded
                    .registry()
                    .resolve_action(&node.action_key)
                    .ok_or(EngineError::ExactFactoryUnavailable)?;
                let metadata = factory.metadata();
                if node.interface_version.as_ref() != Some(metadata.base().version()) {
                    return Err(EngineError::ExactFactoryUnavailable);
                }
                let effect_contract = loaded
                    .plan()
                    .action_effect_contract(&node.action_key)
                    .map_err(|_| EngineError::ExactFactoryUnavailable)?;
                let effect_contract = match effect_contract {
                    nebula_plugin::PlanActionEffectContract::Declared(effect_contract) => {
                        effect_contract
                    },
                    nebula_plugin::PlanActionEffectContract::LegacyUndeclared => {
                        return Err(EngineError::UnsupportedRecordedSemantics);
                    },
                    nebula_plugin::PlanActionEffectContract::UnknownAction => {
                        return Err(EngineError::InvalidRecordedExecution);
                    },
                };
                if &effect_contract != metadata.effect_contract() {
                    return Err(EngineError::ExactFactoryUnavailable);
                }
                Ok((node.id.clone(), factory))
            })
            .collect::<Result<HashMap<_, _>, EngineError>>()?;

        if state.execution_id != execution_id
            || state.workflow_id != loaded.plan().workflow_id()
            || state
                .node_states
                .keys()
                .any(|key| !factories.contains_key(key))
            || persisted_outputs
                .iter()
                .any(|(key, _)| !factories.contains_key(key))
            || (!state.node_states.is_empty() && state.node_states.len() != workflow.nodes().len())
        {
            return Err(EngineError::InvalidRecordedExecution);
        }

        Ok(ValidatedRecordedExecution {
            loaded,
            workflow,
            persisted_outputs,
            factories,
        })
    }
}
