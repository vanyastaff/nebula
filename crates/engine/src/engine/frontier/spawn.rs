//! Frontier spawn — the per-node task builder behind Phase 1 dispatch.
//!
//! [`WorkflowEngine::spawn_node`] resolves the action factory, prepares the
//! node input, drives the node's state machine to `Running`, and spawns the
//! `NodeTask` future into the caller's `JoinSet`. It returns `false` on a
//! setup failure (factory / param-resolution / state-transition refusal),
//! which the Phase 1 drain in `super::dispatch` handles. An `impl
//! WorkflowEngine` method in a child module, so it keeps full access to the
//! engine's private fields, sibling methods, helper free functions, and types
//! through this module's explicit imports.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dashmap::DashMap;
use nebula_action::ActionResult;
use nebula_action::capability::default_resource_accessor;
use nebula_core::NodeKey;
use nebula_core::accessor::{CredentialAccessor, ResourceAccessor};
use nebula_core::id::{ExecutionId, WorkflowId};
use nebula_credential::default_credential_accessor;
use nebula_error::ErrorCode;
use nebula_execution::state::ExecutionState;
use nebula_storage_port::Scope;
use nebula_workflow::DependencyGraph;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::credential_accessor::EngineCredentialAccessor;
use crate::engine::{
    FactoryDispatch, NodeFactoryDispatch, NodeTask, WorkflowEngine, durable_error_envelope,
    resolve_node_input_with_support, setup_refusal,
};
use crate::error::EngineError;
use crate::resolver::NodeInputRequest;
use crate::resource_accessor::EngineResourceAccessor;
use crate::scoped_resources::LayeredResourceAccessor;

impl WorkflowEngine {
    /// Spawn a single node into the JoinSet.
    ///
    /// Returns `true` if the node was spawned, `false` if it failed during setup
    /// (e.g., param resolution error).
    #[expect(clippy::too_many_arguments)]
    pub(super) fn spawn_node(
        &self,
        scope: &Scope,
        fencing: Option<nebula_storage_port::FencingToken>,
        node_key: NodeKey,
        node_map: &HashMap<NodeKey, &nebula_workflow::NodeDefinition>,
        factory_dispatch: &FactoryDispatch<'_>,
        graph: &DependencyGraph,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        shared_expression_outputs: &Arc<DashMap<NodeKey, Arc<serde_json::Value>>>,
        semaphore: &Arc<Semaphore>,
        cancel_token: &CancellationToken,
        exec_state: &mut ExecutionState,
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        input: &serde_json::Value,
        activated_edges: &HashMap<NodeKey, HashSet<NodeKey>>,
        join_set: &mut JoinSet<(
            NodeKey,
            Result<ActionResult<serde_json::Value>, EngineError>,
        )>,
        task_nodes: &mut HashMap<tokio::task::Id, NodeKey>,
    ) -> bool {
        let Some(node_def) = node_map.get(&node_key) else {
            // Unknown node — route through the setup-failure path so
            // the frontier loop records the error and checkpoints the
            // state (issues #300, #321).
            let _ = exec_state.mark_setup_failed(
                node_key.clone(),
                setup_refusal(
                    ErrorCode::new(crate::error::codes::NODE_NOT_FOUND),
                    format!("node {node_key} is not in the workflow's node map"),
                ),
            );
            return false;
        };
        let action_key = node_def.action_key.as_str().to_owned();
        let factory_dispatch = match factory_dispatch {
            FactoryDispatch::DirectRegistry => {
                let selected = match node_def.interface_version.as_ref() {
                    Some(version) => self
                        .runtime
                        .registry()
                        .get_factory_versioned(&node_def.action_key, version),
                    None => self.runtime.registry().get_factory(&node_def.action_key),
                };
                let Some((_, factory)) = selected else {
                    let error =
                        EngineError::Runtime(crate::runtime::RuntimeError::ActionNotFound {
                            key: action_key,
                        });
                    let _ = exec_state
                        .mark_setup_failed(node_key.clone(), durable_error_envelope(&error));
                    return false;
                };
                NodeFactoryDispatch::DirectRegistry { factory }
            },
            FactoryDispatch::Frozen { factories, plan } => {
                let Some(factory) = factories.get(&node_key) else {
                    let _ = exec_state.mark_setup_failed(
                        node_key.clone(),
                        setup_refusal(
                            ErrorCode::new(crate::error::codes::EXACT_FACTORY_UNAVAILABLE),
                            "exact factory witness is missing a graph node",
                        ),
                    );
                    return false;
                };
                let effect_contract = match plan.action_effect_contract(&node_def.action_key) {
                    Ok(nebula_plugin::PlanActionEffectContract::Declared(effect_contract)) => {
                        effect_contract
                    },
                    Ok(nebula_plugin::PlanActionEffectContract::LegacyUndeclared) => {
                        let _ = exec_state.mark_setup_failed(
                            node_key.clone(),
                            setup_refusal(
                                ErrorCode::new(crate::error::codes::UNSUPPORTED_RECORDED_SEMANTICS),
                                "legacy executable plan has no action effect declaration",
                            ),
                        );
                        return false;
                    },
                    Ok(nebula_plugin::PlanActionEffectContract::UnknownAction) => {
                        let _ = exec_state.mark_setup_failed(
                            node_key.clone(),
                            setup_refusal(
                                ErrorCode::new(crate::error::codes::EXACT_GRAPH_PROJECTION),
                                "executable plan does not contain the requested action",
                            ),
                        );
                        return false;
                    },
                    Err(_) => {
                        let _ = exec_state.mark_setup_failed(
                            node_key.clone(),
                            setup_refusal(
                                ErrorCode::new(crate::error::codes::CONTRACT_BUNDLE_INTEGRITY),
                                "recorded action effect declaration failed integrity validation",
                            ),
                        );
                        return false;
                    },
                };
                let Some(action_version) = node_def.interface_version.clone() else {
                    let _ = exec_state.mark_setup_failed(
                        node_key.clone(),
                        setup_refusal(
                            ErrorCode::new(crate::error::codes::MISSING_EXACT_RUNTIME),
                            "recorded action version is unavailable",
                        ),
                    );
                    return false;
                };
                NodeFactoryDispatch::Frozen {
                    factory: Arc::clone(factory),
                    effect_contract,
                    action_version,
                }
            },
        };
        let factory = match &factory_dispatch {
            NodeFactoryDispatch::DirectRegistry { factory }
            | NodeFactoryDispatch::Frozen { factory, .. } => factory,
        };
        let metadata = factory.metadata();

        // Partition incoming connections into flow (to_port=None) and support (to_port=Some)
        let (node_input, support_inputs) = resolve_node_input_with_support(
            node_key.clone(),
            graph,
            outputs,
            input,
            activated_edges,
        );

        // Admit authored parameters before the cancellable node task evaluates programs.
        let action_input = match self.resolver.prepare(NodeInputRequest {
            node_key: &node_key,
            parameters: &node_def.parameters,
            predecessor_input: node_input,
            outputs,
            shared_outputs: shared_expression_outputs,
            schema: metadata.base().schema(),
            cancellation: cancel_token.clone(),
        }) {
            Ok(prepared) => prepared,
            Err(e) => {
                // Parameter resolution failed. `mark_setup_failed`
                // overrides the node state to Failed via
                // `override_node_state` (Pending → Failed is not a
                // valid forward transition) and bumps the parent
                // version for CAS readers (issues #255, #300).
                let _ = exec_state.mark_setup_failed(node_key.clone(), durable_error_envelope(&e));
                return false;
            },
        };

        // Drive the node to Running via the typed state-machine
        // helper. `start_node_attempt` models the only legal
        // transition path (Pending → Ready → Running) and returns an
        // error for anything else — the engine does not retry, so
        // Failed is terminal at the node level. On error we do NOT
        // silently spawn the task on stale state — route through the
        // setup-failure path instead (issue #300).
        if let Err(err) = exec_state.start_node_attempt(node_key.clone()) {
            let _ = exec_state.mark_setup_failed(
                node_key.clone(),
                setup_refusal(
                    ErrorCode::new(crate::error::codes::FRONTIER_INTEGRITY),
                    format!("cannot start node attempt: {err}"),
                ),
            );
            return false;
        }

        let Some(attempt_generation) = exec_state
            .node_states
            .get(&node_key)
            .and_then(|state| u64::try_from(state.attempt_count()).ok())
            .and_then(|count| count.checked_add(1))
        else {
            let _ = exec_state.mark_setup_failed(
                node_key.clone(),
                setup_refusal(
                    ErrorCode::new(crate::error::codes::FRONTIER_INTEGRITY),
                    "node attempt generation is invalid",
                ),
            );
            return false;
        };
        let runtime = self.runtime.clone();
        let cancel = cancel_token.clone();
        let sem = semaphore.clone();
        let outputs_ref = outputs.clone();

        // The exact durable manifest is the sole action credential allowlist.
        // Direct/storeless execution and a missing site entry both deny every key.
        let credential_bindings = self
            .credential_bindings_by_execution
            .get(&execution_id)
            .map(|manifest| {
                manifest
                    .entries()
                    .iter()
                    .filter_map(|entry| match (entry.site(), entry.target()) {
                        (
                            nebula_execution::ExecutionBindingSiteV2::Node(site),
                            nebula_execution::ExecutionBindingTargetV2::Credential {
                                credential_id,
                                contract,
                            },
                        ) if site == &node_key => Some((
                            entry.slot_key().to_owned(),
                            (*credential_id, contract.clone()),
                        )),
                        _ => None,
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let allowed_keys = credential_bindings.keys().cloned().collect();
        let credentials: Arc<dyn CredentialAccessor> = if let (Some(resolver), Some(scope)) = (
            &self.credential_resolver,
            self.credential_scopes_by_execution
                .get(&execution_id)
                .map(|scope| scope.value().clone()),
        ) {
            let resolver = Arc::clone(resolver);
            let cancel = cancel_token.clone();
            let credential_bindings = Arc::new(credential_bindings);
            Arc::new(EngineCredentialAccessor::new(
                allowed_keys,
                move |slot_key: &str| {
                    let resolver = Arc::clone(&resolver);
                    let scope = scope.clone();
                    let cancel = cancel.clone();
                    let binding = credential_bindings.get(slot_key).cloned();
                    async move {
                        let (credential_id, contract) = binding.ok_or_else(|| {
                            nebula_core::CoreError::RegistryInvariant(
                                "credential slot allowlist and manifest diverged",
                            )
                        })?;
                        let required_capabilities = contract.required_capabilities().iter().fold(
                            nebula_credential::Capabilities::empty(),
                            |set, capability| {
                                set | match capability {
                                    nebula_execution::CredentialCapability::Interactive => {
                                        nebula_credential::Capabilities::INTERACTIVE
                                    },
                                    nebula_execution::CredentialCapability::Refreshable => {
                                        nebula_credential::Capabilities::REFRESHABLE
                                    },
                                    nebula_execution::CredentialCapability::Revocable => {
                                        nebula_credential::Capabilities::REVOCABLE
                                    },
                                    nebula_execution::CredentialCapability::Testable => {
                                        nebula_credential::Capabilities::TESTABLE
                                    },
                                    nebula_execution::CredentialCapability::Dynamic => {
                                        nebula_credential::Capabilities::DYNAMIC
                                    },
                                }
                            },
                        );
                        let key = contract.key().clone();
                        let guard = resolver
                            .resolve_slot(
                                &scope,
                                credential_id,
                                key.clone(),
                                required_capabilities,
                                cancel,
                            )
                            .await
                            .map_err(|error| {
                                tracing::debug!(
                                    credential.key = key.as_str(),
                                    error = %error,
                                    "credential slot resolution failed"
                                );
                                nebula_core::CoreError::credential_not_found(key)
                            })?;
                        Ok(Box::new(guard) as Box<dyn std::any::Any + Send + Sync>)
                    }
                },
                node_def.action_key.as_str().to_owned(),
            ))
        } else {
            default_credential_accessor()
        };

        // Build resource accessor: wrap the manager-backed global accessor in
        // a LayeredResourceAccessor (M6.1 — Phase 6). Phase 6 plugs in the
        // empty scoped map; Phase 7 (M6.2) swaps the inner scoped layer for
        // the per-branch DashMap implementation. Action call sites
        // (`ctx.acquire_resource_by_id`, `ctx.resource::<R>()`) consult the
        // layered accessor transparently — `scoped → global`, closest
        // ancestor wins.
        let resources: Arc<dyn ResourceAccessor> = if let Some(manager) = &self.resource_manager {
            let extra = self
                .execution_acquire_scopes
                .get(&execution_id)
                .map(|entry| entry.value().clone())
                .unwrap_or_default();
            let scope = nebula_core::scope::Scope {
                execution_id: Some(execution_id),
                workflow_id: Some(workflow_id),
                org_id: extra.org_id,
                workspace_id: extra.workspace_id,
                ..Default::default()
            };
            let slot_identities = self
                .resource_slot_identities_by_execution
                .get(&execution_id)
                .map(|entry| Arc::clone(entry.value()))
                .unwrap_or_else(|| Arc::new(HashMap::new()));
            let global: Arc<dyn ResourceAccessor> = Arc::new(
                EngineResourceAccessor::new(Arc::clone(manager), scope, cancel_token.clone())
                    .with_slot_identities_arc(slot_identities),
            );
            Arc::new(LayeredResourceAccessor::global_only(global))
        } else {
            default_resource_accessor()
        };

        // Only forward the refresh hook when a credential resolver is configured.
        // Without a resolver there are no credentials to refresh, so the hook
        // would fire unconditionally on every node — even actions that do not use
        // credentials at all.
        let credential_refresh = if self.credential_resolver.is_some() {
            self.credential_refresh.clone()
        } else {
            None
        };

        // Rate limiter from the node's `rate_limit` policy. The bucket is
        // shared per action key across this engine's lifetime (see
        // `WorkflowEngine::rate_limiters`): rebuilding it per dispatch would
        // reset the quota on every retry. An invalid policy is a setup
        // refusal, not a silent `None` — a configured limit that the engine
        // fails to honor is worse than no limit, because the operator
        // believes it is enforced.
        let rate_limiter = match node_def.rate_limit.as_ref() {
            None => None,
            Some(rl) => match self.rate_limiter_for_action(&action_key, rl) {
                Ok(limiter) => Some(limiter),
                Err(error) => {
                    let _ = exec_state.mark_setup_failed(node_key.clone(), error);
                    return false;
                },
            },
        };

        let handle = join_set.spawn(
            NodeTask {
                clock: Arc::clone(&self.clock),
                attempt_generation,
                scope: scope.clone(),
                fencing,
                runtime,
                factory_dispatch,
                cancel,
                sem,
                outputs: outputs_ref,
                execution_id,
                node_key: node_key.clone(),
                workflow_id,
                action_key,
                node: Arc::new((*node_def).clone()),
                input: action_input,
                support_inputs,
                credentials,
                resources,
                credential_refresh,
                rate_limiter,
                operation_ledger: self
                    .stores
                    .as_ref()
                    .map(|stores| Arc::clone(&stores.operation_ledger)),
            }
            .run(),
        );
        task_nodes.insert(handle.id(), node_key);

        true
    }
}
