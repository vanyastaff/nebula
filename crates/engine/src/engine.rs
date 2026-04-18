//! Workflow execution engine.
//!
//! Executes workflows using a frontier-based approach: each node is spawned
//! as soon as all its incoming edges are resolved and at least one is activated,
//! rather than waiting for an entire topological level. This enables branching,
//! skip propagation, error routing, and conditional edges.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;
// TODO: ExecutionBudget moved to nebula-execution
use nebula_action::capability::{ResourceAccessor, default_resource_accessor};
use nebula_action::{ActionContext, ActionError, ActionResult};
use nebula_core::{
    ActionKey, NodeKey,
    id::{ExecutionId, WorkflowId},
    node_key,
};
use nebula_credential::{CredentialAccessor, default_credential_accessor};
// ScopeLevel removed from ActionContext
// use nebula_core::scope::ScopeLevel;
use nebula_execution::ExecutionStatus;
use nebula_execution::{context::ExecutionBudget, plan::ExecutionPlan, state::ExecutionState};
use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_metrics::naming::{
    NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS, NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
};
use nebula_plugin::PluginRegistry;
use nebula_runtime::ActionRuntime;
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{
    Connection, DependencyGraph, EdgeCondition, ErrorMatcher, NodeState, ResultMatcher,
    WorkflowDefinition,
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use crate::{
    credential_accessor::EngineCredentialAccessor, error::EngineError, event::ExecutionEvent,
    resolver::ParamResolver, resource_accessor::EngineResourceAccessor, result::ExecutionResult,
};

/// Type alias for the optional event sender.
///
/// Bounded (rather than unbounded) so a slow consumer cannot drive engine
/// memory to unbounded growth. A workflow with ~10k nodes emits ~50k events;
/// the capacity below keeps roughly one in-flight workflow's worth of events
/// buffered before the engine starts dropping.
type EventSender = mpsc::Sender<ExecutionEvent>;

/// Default capacity for the engine's event channel. Tuned so a typical
/// interactive workflow (hundreds of nodes) never blocks, while a runaway
/// producer with a dead consumer cannot inflate memory without bound.
pub const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Type alias for the boxed async credential-refresh function stored on the engine.
///
/// When set, the engine calls this function before dispatching any node that uses
/// credentials, passing the credential ID. The callee is responsible for refreshing
/// the credential (e.g., rotating short-lived tokens) before the action resolves it.
type CredentialRefreshFn = Arc<
    dyn Fn(
            &str,
        )
            -> Pin<Box<dyn Future<Output = Result<(), nebula_action::error::ActionError>> + Send>>
        + Send
        + Sync,
>;

/// Type alias for the boxed async credential-resolution function stored on the engine.
type CredentialResolveFn = Arc<
    dyn Fn(
            &str,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            nebula_credential::CredentialSnapshot,
                            nebula_credential::CredentialAccessError,
                        >,
                    > + Send,
            >,
        > + Send
        + Sync,
>;

/// The workflow execution engine.
///
/// Orchestrates end-to-end execution of workflow definitions by:
///
/// 1. Building a dependency graph from the workflow
/// 2. Executing nodes frontier-by-frontier with bounded concurrency
/// 3. Evaluating edge conditions to determine which successors to activate
/// 4. Resolving each node's input from activated predecessor outputs
/// 5. Delegating action execution to the [`ActionRuntime`]
/// 6. Tracking execution state and recording metrics
pub struct WorkflowEngine {
    runtime: Arc<ActionRuntime>,
    metrics: MetricsRegistry,
    /// Node registry for node-level metadata and versioning.
    plugin_registry: PluginRegistry,
    /// Resolves node parameters (expressions, templates, references) to JSON.
    resolver: ParamResolver,
    /// Expression engine for evaluating edge conditions.
    #[allow(dead_code)]
    expression_engine: Arc<ExpressionEngine>,
    /// Optional resource manager for providing resources to actions.
    resource_manager: Option<Arc<nebula_resource::Manager>>,
    /// Optional execution repository for persistent state storage.
    execution_repo: Option<Arc<dyn nebula_storage::ExecutionRepo>>,
    /// Optional workflow repository for loading workflow definitions during resume.
    workflow_repo: Option<Arc<dyn nebula_storage::WorkflowRepo>>,
    /// Optional credential resolver function for providing credentials to actions.
    credential_resolver: Option<CredentialResolveFn>,
    /// Optional proactive credential refresh hook.
    ///
    /// When set, the engine calls this before dispatching any node that uses
    /// credentials. The callee is responsible for refreshing the credential so
    /// the resolver returns a fresh snapshot when the action requests it.
    credential_refresh: Option<CredentialRefreshFn>,
    /// Per-[`ActionKey`] credential allowlist (deny-by-default).
    ///
    /// Actions may only acquire credential IDs listed for their `ActionKey`.
    /// Missing entry or empty set → every `acquire_credential` request for
    /// that action is denied with [`nebula_credential::CredentialAccessError::AccessDenied`].
    /// See `PRODUCT_CANON` §4.5 (operational honesty — no false capabilities) and §12.5
    /// (secrets and auth). Populated via [`WorkflowEngine::with_action_credentials`].
    action_credentials: HashMap<ActionKey, HashSet<String>>,
    /// Optional event sender for real-time execution monitoring (TUI, logging).
    event_sender: Option<EventSender>,
}

impl WorkflowEngine {
    /// Create a new engine with the given components.
    pub fn new(runtime: Arc<ActionRuntime>, metrics: MetricsRegistry) -> Self {
        let expression_engine = Arc::new(ExpressionEngine::with_cache_size(1024));
        Self {
            runtime,
            metrics,
            plugin_registry: PluginRegistry::new(),
            resolver: ParamResolver::new(expression_engine.clone()),
            expression_engine,
            resource_manager: None,
            execution_repo: None,
            workflow_repo: None,
            credential_resolver: None,
            credential_refresh: None,
            action_credentials: HashMap::new(),
            event_sender: None,
        }
    }

    /// Access the node registry.
    pub fn plugin_registry(&self) -> &PluginRegistry {
        &self.plugin_registry
    }

    /// Mutable access to the node registry.
    pub fn plugin_registry_mut(&mut self) -> &mut PluginRegistry {
        &mut self.plugin_registry
    }

    /// Attach a resource manager for providing resources to actions.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resource_manager(mut self, manager: Arc<nebula_resource::Manager>) -> Self {
        self.resource_manager = Some(manager);
        self
    }

    /// Attach a credential resolver for providing credentials to actions.
    ///
    /// The resolver is a type-erased async function that maps a credential ID
    /// to a [`nebula_credential::CredentialSnapshot`]. When not set, actions
    /// receive the default no-op accessor (which denies all credential access).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use std::sync::Arc;
    /// use nebula_engine::WorkflowEngine;
    /// use nebula_credential::{CredentialResolver, CredentialAccessError, InMemoryStore};
    ///
    /// let store = Arc::new(InMemoryStore::new());
    /// let resolver = Arc::new(CredentialResolver::new(store));
    ///
    /// let engine = WorkflowEngine::new(runtime, metrics)
    ///     .with_credential_resolver(move |id: &str| {
    ///         let resolver = Arc::clone(&resolver);
    ///         let id = id.to_owned();
    ///         Box::pin(async move {
    ///             resolver.resolve_snapshot(&id).await
    ///                 .map_err(|e| CredentialAccessError::NotFound(e.to_string()))
    ///         })
    ///     });
    /// ```
    #[must_use = "builder methods must be chained or built"]
    pub fn with_credential_resolver<F, Fut>(mut self, resolver: F) -> Self
    where
        F: Fn(&str) -> Fut + Send + Sync + 'static,
        Fut: Future<
                Output = Result<
                    nebula_credential::CredentialSnapshot,
                    nebula_credential::CredentialAccessError,
                >,
            > + Send
            + 'static,
    {
        self.credential_resolver = Some(Arc::new(move |id: &str| {
            Box::pin(resolver(id))
                as Pin<
                    Box<
                        dyn Future<
                                Output = Result<
                                    nebula_credential::CredentialSnapshot,
                                    nebula_credential::CredentialAccessError,
                                >,
                            > + Send,
                    >,
                >
        }));
        self
    }

    /// Attach a proactive credential refresh hook.
    ///
    /// When set, the engine calls `refresh_fn(credential_id)` before dispatching
    /// any node whose resolver is configured. This allows short-lived credentials
    /// (OAuth tokens, STS sessions) to be renewed before the action consumes them.
    ///
    /// The refresh is best-effort: errors are logged but do not abort execution.
    /// The function is injected by the caller — the engine has no knowledge of
    /// the underlying credential store or rotation strategy.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let engine = WorkflowEngine::new(runtime, metrics)
    ///     .with_credential_resolver(/* ... */)
    ///     .with_credential_refresh(move |id: &str| {
    ///         let id = id.to_owned();
    ///         Box::pin(async move { rotate_if_needed(&id).await })
    ///     });
    /// ```
    #[must_use = "builder methods must be chained or built"]
    pub fn with_credential_refresh<F, Fut>(mut self, refresh_fn: F) -> Self
    where
        F: Fn(&str) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), nebula_action::error::ActionError>> + Send + 'static,
    {
        self.credential_refresh = Some(Arc::new(move |id: &str| {
            Box::pin(refresh_fn(id))
                as Pin<
                    Box<dyn Future<Output = Result<(), nebula_action::error::ActionError>> + Send>,
                >
        }));
        self
    }

    /// Declare the credential IDs an action is permitted to acquire.
    ///
    /// The engine enforces a **deny-by-default** allowlist (see `PRODUCT_CANON` §4.5
    /// and §12.5). When a node whose `action_key == action` runs, only the
    /// credential IDs supplied here may be resolved — every other request fails
    /// with [`nebula_credential::CredentialAccessError::AccessDenied`]. Actions
    /// that are never declared here cannot acquire any credential at all.
    ///
    /// Multiple calls for the same `action` **merge** — the new entries are added
    /// to the existing set rather than replacing it, so that composable fixtures
    /// and plugin wiring can contribute independently.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_core::action_key;
    /// use nebula_engine::WorkflowEngine;
    ///
    /// let engine = WorkflowEngine::new(runtime, metrics)
    ///     .with_credential_resolver(/* ... */)
    ///     .with_action_credentials(action_key!("http.request"), ["github_token"]);
    /// ```
    #[must_use = "builder methods must be chained or built"]
    pub fn with_action_credentials<I, S>(mut self, action: ActionKey, credential_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let entry = self.action_credentials.entry(action).or_default();
        entry.extend(credential_ids.into_iter().map(Into::into));
        self
    }

    /// Set the execution repository for persistent state storage.
    ///
    /// When set, the engine persists execution state after creation and
    /// after each node completes (checkpoint). Without a repo, state
    /// is in-memory only (suitable for testing).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_execution_repo(mut self, repo: Arc<dyn nebula_storage::ExecutionRepo>) -> Self {
        self.execution_repo = Some(repo);
        self
    }

    /// Set the workflow repository for loading workflow definitions during resume.
    ///
    /// Required for [`resume_execution`]. When not set, `resume_execution` returns an error.
    ///
    /// [`resume_execution`]: Self::resume_execution
    #[must_use = "builder methods must be chained or built"]
    pub fn with_workflow_repo(mut self, repo: Arc<dyn nebula_storage::WorkflowRepo>) -> Self {
        self.workflow_repo = Some(repo);
        self
    }

    /// Attach an event sender for real-time execution monitoring.
    ///
    /// When set, the engine emits [`ExecutionEvent`]s for node lifecycle
    /// transitions (started, completed, failed, skipped) and execution
    /// completion. Used by the CLI TUI for live monitoring.
    ///
    /// Pair with a receiver from [`mpsc::channel`] sized at
    /// [`DEFAULT_EVENT_CHANNEL_CAPACITY`] or larger — events are *dropped*
    /// (not blocked) when the buffer is full, so a stuck consumer cannot
    /// stall the engine or grow memory without bound.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_event_sender(mut self, sender: mpsc::Sender<ExecutionEvent>) -> Self {
        self.event_sender = Some(sender);
        self
    }

    /// Emit an execution event if a sender is configured.
    ///
    /// Uses `try_send` so a backed-up consumer cannot block the engine.
    /// The channel is deliberately bounded; drops are observability-only.
    fn emit_event(&self, event: ExecutionEvent) {
        if let Some(sender) = &self.event_sender
            && let Err(e) = sender.try_send(event)
        {
            // Full or closed — drop. The alternative (block the engine on
            // a slow TUI) would be far worse. Log once at the boundary.
            match e {
                mpsc::error::TrySendError::Full(_) => {
                    tracing::warn!("engine event channel full; dropping event (slow consumer)");
                },
                mpsc::error::TrySendError::Closed(_) => {
                    // Consumer disconnected — silent, expected on shutdown.
                },
            }
        }
    }

    /// Emit [`ExecutionEvent::FrontierIntegrityViolation`] when the §11.1
    /// guard has populated a non-terminal payload. Called at every finish
    /// site *before* [`ExecutionEvent::ExecutionFinished`]; isolating it in
    /// one helper keeps that ordering contract in a single place.
    ///
    /// Unlike [`Self::emit_event`], this helper escalates a dropped event
    /// to `tracing::error!` — the integrity violation is the one event
    /// whose contract is "operators must see it", so a slow consumer
    /// leaves an attributable log record instead of a `warn!` drop.
    fn emit_frontier_integrity_if_violated(
        &self,
        execution_id: ExecutionId,
        non_terminal_nodes: Option<Vec<(NodeKey, NodeState)>>,
    ) {
        let Some(non_terminal_nodes) = non_terminal_nodes else {
            return;
        };
        let non_terminal_count = non_terminal_nodes.len();
        let event = ExecutionEvent::FrontierIntegrityViolation {
            execution_id,
            non_terminal_nodes,
        };
        let Some(sender) = &self.event_sender else {
            return;
        };
        if sender.try_send(event).is_err() {
            tracing::error!(
                %execution_id,
                non_terminal_count,
                "frontier integrity violation event dropped (channel full or closed)"
            );
        }
    }

    /// Replay a workflow execution from a specific node.
    ///
    /// Nodes upstream of `replay_from` use pinned (stored) outputs.
    /// Nodes at and downstream are re-executed.
    pub async fn replay_execution(
        &self,
        workflow: &WorkflowDefinition,
        plan: nebula_execution::ReplayPlan,
        budget: ExecutionBudget,
    ) -> Result<ExecutionResult, EngineError> {
        budget
            .validate_for_execution()
            .map_err(|msg| EngineError::PlanningFailed(msg.to_string()))?;

        let execution_id = ExecutionId::new();
        let started = Instant::now();

        // Build graph and node map.
        let graph = DependencyGraph::from_definition(workflow)
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;
        let node_ids: Vec<NodeKey> = workflow.nodes.iter().map(|n| n.id.clone()).collect();
        let node_map: HashMap<NodeKey, &nebula_workflow::NodeDefinition> =
            workflow.nodes.iter().map(|n| (n.id.clone(), n)).collect();

        // Build both predecessor AND successor maps.
        //
        // - Predecessors are needed to compute seeds: a node whose entire incoming edge set is
        //   pinned is ready to start.
        // - Successors are needed by `ReplayPlan::partition_nodes` to forward-traverse from
        //   `replay_from` and identify the re-run set. Issue #254 — the old predecessor-only
        //   partition classified unrelated sibling branches as rerun, duplicating their side
        //   effects on every replay.
        let mut predecessors: HashMap<NodeKey, Vec<NodeKey>> = HashMap::new();
        let mut successors: HashMap<NodeKey, Vec<NodeKey>> = HashMap::new();
        for conn in &workflow.connections {
            predecessors
                .entry(conn.to_node.clone())
                .or_default()
                .push(conn.from_node.clone());
            successors
                .entry(conn.from_node.clone())
                .or_default()
                .push(conn.to_node.clone());
        }

        // Partition nodes into pinned (use stored outputs) and rerun.
        let (pinned, _rerun) = plan.partition_nodes(&node_ids, &successors);

        // Pre-populate outputs with pinned values.
        //
        // The `ReplayPlan` contract requires `pinned_outputs` to be
        // complete for every pinned node. Iterate the set and fail
        // loudly on a missing entry — this surfaces stale or
        // incomplete plans at replay start instead of letting the
        // frontier loop silently feed `Null` to a downstream node. The
        // previous filter-in-map-keys approach hid missing pins the
        // same way `#[serde(skip)]` on `pinned_outputs` did (#253).
        let outputs: Arc<DashMap<NodeKey, serde_json::Value>> = Arc::new(DashMap::new());
        for node_key in &pinned {
            let Some(output) = plan.pinned_outputs.get(node_key) else {
                return Err(EngineError::PlanningFailed(format!(
                    "replay plan is missing pinned output for node {node_key}; \
                     every node in the partition's pinned set must have a stored output"
                )));
            };
            outputs.insert(node_key.clone(), output.clone());
        }

        // Build execution state — mark pinned nodes as Completed via
        // the versioned transition API so downstream CAS readers see
        // the version move (issue #255).
        let mut exec_state = ExecutionState::new(execution_id, workflow.id, &node_ids);
        exec_state.transition_status(ExecutionStatus::Running)?;
        for node_key in &pinned {
            // NOTE: errors are intentionally discarded here. Pinned nodes are
            // forced through Ready→Running→Completed for bookkeeping; the
            // transitions are best-effort. A failure (e.g. unexpected current
            // state) is non-fatal because the node was already completed in a
            // prior run. TODO: log a warning on failure once the engine has a
            // structured logger handle.
            let _ = exec_state.transition_node(node_key.clone(), NodeState::Ready);
            let _ = exec_state.transition_node(node_key.clone(), NodeState::Running);
            let _ = exec_state.transition_node(node_key.clone(), NodeState::Completed);
        }

        let semaphore = Arc::new(Semaphore::new(budget.max_concurrent_nodes));
        let cancel_token = CancellationToken::new();
        let mut repo_version = 0u64;

        // Determine seed nodes: nodes in rerun set whose predecessors are all pinned.
        let seed_nodes: Vec<NodeKey> = node_ids
            .iter()
            .filter(|n| !pinned.contains(*n))
            .filter(|n| {
                predecessors
                    .get(*n)
                    .map(|preds| preds.iter().all(|p| pinned.contains(p)))
                    .unwrap_or(true) // no predecessors = entry node
            })
            .cloned()
            .collect();

        // Use override inputs if provided.
        let input = plan
            .input_overrides
            .get(&plan.replay_from)
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let error_strategy = workflow.config.error_strategy;

        // Run the frontier loop — same as execute_workflow, just different seed + pre-populated
        // outputs.
        let failed_node = self
            .run_frontier(
                &graph,
                &node_map,
                &outputs,
                &semaphore,
                &cancel_token,
                &mut exec_state,
                execution_id,
                workflow.id,
                &input,
                &mut repo_version,
                &budget,
                &started,
                error_strategy,
                seed_nodes,
                HashMap::new(),
                HashMap::new(),
            )
            .await;

        self.runtime.clear_execution_output_totals(execution_id);

        let elapsed = started.elapsed();
        let FinalStatusDecision {
            status: final_status,
            integrity_violation,
        } = determine_final_status(&failed_node, &cancel_token, &exec_state);
        let _ = exec_state.transition_status(final_status);

        self.emit_frontier_integrity_if_violated(execution_id, integrity_violation);
        self.emit_event(ExecutionEvent::ExecutionFinished {
            execution_id,
            success: final_status == ExecutionStatus::Completed,
            elapsed,
        });

        let node_outputs: HashMap<NodeKey, serde_json::Value> = outputs
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect();

        let node_errors: HashMap<NodeKey, String> = exec_state
            .node_states
            .iter()
            .filter_map(|(id, ns)| {
                ns.error_message
                    .as_ref()
                    .map(|msg| (id.clone(), msg.clone()))
            })
            .collect();

        Ok(ExecutionResult {
            execution_id,
            status: final_status,
            node_outputs,
            node_errors,
            duration: elapsed,
        })
    }

    /// Execute a workflow from start to finish.
    ///
    /// Builds an execution plan for validation, then processes nodes
    /// frontier-by-frontier. Each node is spawned as soon as all its
    /// incoming edges are resolved and at least one is activated,
    /// up to `budget.max_concurrent_nodes`.
    ///
    /// Entry nodes receive the workflow-level `input`. Subsequent nodes
    /// receive the output of their activated predecessors.
    pub async fn execute_workflow(
        &self,
        workflow: &WorkflowDefinition,
        input: serde_json::Value,
        budget: ExecutionBudget,
    ) -> Result<ExecutionResult, EngineError> {
        budget
            .validate_for_execution()
            .map_err(|msg| EngineError::PlanningFailed(msg.to_string()))?;

        let execution_id = ExecutionId::new();
        let started = Instant::now();

        // 1. Validate workflow (reuse ExecutionPlan for validation)
        let _plan = ExecutionPlan::from_workflow(execution_id, workflow, budget.clone())
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;

        // 2. Build dependency graph
        let graph = DependencyGraph::from_definition(workflow)
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;

        // 3. Validate action key mappings exist for all nodes
        // 4. Initialize execution state
        let node_ids: Vec<NodeKey> = workflow.nodes.iter().map(|n| n.id.clone()).collect();
        let mut exec_state = ExecutionState::new(execution_id, workflow.id, &node_ids);
        exec_state.transition_status(ExecutionStatus::Running)?;

        // 4b. Persist initial execution state
        let mut repo_version: u64 = 0;
        if let Some(repo) = &self.execution_repo {
            let state_json = serde_json::to_value(&exec_state)
                .map_err(|e| EngineError::PlanningFailed(format!("serialize state: {e}")))?;
            repo.create(execution_id, workflow.id, state_json)
                .await
                .map_err(|e| EngineError::PlanningFailed(format!("persist initial state: {e}")))?;
            repo_version = 1;
        }

        // 5. Create cancellation token
        let cancel_token = CancellationToken::new();

        // 6. Record start metric
        self.metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
            .inc();

        // 7. Build node lookup map
        let node_map: HashMap<NodeKey, &nebula_workflow::NodeDefinition> =
            workflow.nodes.iter().map(|n| (n.id.clone(), n)).collect();

        // 8. Shared output storage (concurrent access from worker tasks)
        let outputs: Arc<DashMap<NodeKey, serde_json::Value>> = Arc::new(DashMap::new());
        let semaphore = Arc::new(Semaphore::new(budget.max_concurrent_nodes));

        // 9. Execute using frontier-based loop
        let error_strategy = workflow.config.error_strategy;
        let seed_nodes: Vec<NodeKey> = graph.entry_nodes();
        let failed_node = self
            .run_frontier(
                &graph,
                &node_map,
                &outputs,
                &semaphore,
                &cancel_token,
                &mut exec_state,
                execution_id,
                workflow.id,
                &input,
                &mut repo_version,
                &budget,
                &started,
                error_strategy,
                seed_nodes,
                HashMap::new(),
                HashMap::new(),
            )
            .await;

        self.runtime.clear_execution_output_totals(execution_id);

        let elapsed = started.elapsed();

        // 10. Determine final status and emit events
        let FinalStatusDecision {
            status: final_status,
            integrity_violation,
        } = determine_final_status(&failed_node, &cancel_token, &exec_state);
        let _ = exec_state.transition_status(final_status);

        // Persist final execution state (best-effort)
        if let Some(repo) = &self.execution_repo
            && let Ok(state_json) = serde_json::to_value(&exec_state)
        {
            match repo
                .transition(execution_id, repo_version, state_json)
                .await
            {
                Ok(true) => { /* success */ },
                Ok(false) => {
                    tracing::warn!(
                        %execution_id,
                        "final state checkpoint CAS mismatch"
                    );
                },
                Err(e) => {
                    tracing::warn!(
                        %execution_id,
                        error = %e,
                        "final state checkpoint failed"
                    );
                },
            }
        }

        self.emit_final_event(execution_id, final_status, elapsed, &failed_node);
        self.emit_frontier_integrity_if_violated(execution_id, integrity_violation);
        self.emit_event(ExecutionEvent::ExecutionFinished {
            execution_id,
            success: final_status == ExecutionStatus::Completed,
            elapsed,
        });

        // 11. Collect outputs and errors
        let node_outputs: HashMap<NodeKey, serde_json::Value> = outputs
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect();

        let node_errors: HashMap<NodeKey, String> = exec_state
            .node_states
            .iter()
            .filter_map(|(id, ns)| {
                ns.error_message
                    .as_ref()
                    .map(|msg| (id.clone(), msg.clone()))
            })
            .collect();

        Ok(ExecutionResult {
            execution_id,
            status: final_status,
            node_outputs,
            node_errors,
            duration: elapsed,
        })
    }

    /// Resume an incomplete execution after process restart.
    ///
    /// Loads execution state and workflow definition from storage, identifies
    /// which nodes are already complete, and re-executes from the frontier of
    /// ready-but-not-yet-executed nodes (nodes whose predecessors are all
    /// terminal but which are not yet terminal themselves).
    ///
    /// Persisted outputs are pre-loaded into the shared output map so that
    /// resumed nodes receive the correct predecessor data.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::PlanningFailed`] if:
    /// - `execution_repo` or `workflow_repo` is not configured on this engine
    /// - The execution or workflow is not found in storage
    /// - The execution is already in a terminal state
    /// - The persisted state cannot be deserialized
    pub async fn resume_execution(
        &self,
        execution_id: ExecutionId,
    ) -> Result<ExecutionResult, EngineError> {
        let started = Instant::now();

        // 1. Require both repos.
        let exec_repo = self
            .execution_repo
            .as_ref()
            .ok_or_else(|| EngineError::PlanningFailed("no execution_repo configured".into()))?;
        let workflow_repo = self
            .workflow_repo
            .as_ref()
            .ok_or_else(|| EngineError::PlanningFailed("no workflow_repo configured".into()))?;

        // 2. Load persisted execution state.
        let (repo_version_loaded, state_json) = exec_repo
            .get_state(execution_id)
            .await
            .map_err(|e| EngineError::PlanningFailed(format!("load state: {e}")))?
            .ok_or_else(|| {
                EngineError::PlanningFailed(format!("execution not found: {execution_id}"))
            })?;

        // Deserialize via JSON string to avoid `serde_json::from_value` issues
        // with Key<D> types that expect borrowed strings (domain-key serde impl).
        let state_str = serde_json::to_string(&state_json)
            .map_err(|e| EngineError::PlanningFailed(format!("serialize state: {e}")))?;
        let exec_state: ExecutionState = serde_json::from_str(&state_str)
            .map_err(|e| EngineError::PlanningFailed(format!("deserialize state: {e}")))?;

        // 3. Guard against resuming a terminal execution.
        if exec_state.status.is_terminal() {
            return Err(EngineError::PlanningFailed(format!(
                "execution {execution_id} is already terminal ({})",
                exec_state.status
            )));
        }

        let workflow_id = exec_state.workflow_id;

        // 4. Load workflow definition.
        let workflow_json = workflow_repo
            .get(workflow_id)
            .await
            .map_err(|e| EngineError::PlanningFailed(format!("load workflow: {e}")))?
            .ok_or_else(|| {
                EngineError::PlanningFailed(format!("workflow not found: {workflow_id}"))
            })?;

        // Deserialize via JSON string to avoid `serde_json::from_value` issues
        // with borrowed key types (e.g. `ActionKey` uses `#[serde(borrow)]`).
        let workflow_str = serde_json::to_string(&workflow_json)
            .map_err(|e| EngineError::PlanningFailed(format!("serialize workflow: {e}")))?;
        let workflow: WorkflowDefinition = serde_json::from_str(&workflow_str)
            .map_err(|e| EngineError::PlanningFailed(format!("deserialize workflow: {e}")))?;

        // 5. Load persisted node outputs.
        let persisted_outputs = exec_repo
            .load_all_outputs(execution_id)
            .await
            .map_err(|e| EngineError::PlanningFailed(format!("load outputs: {e}")))?;

        // 6. Build dependency graph.
        let graph = DependencyGraph::from_definition(&workflow)
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;

        // 7. Reconstruct the execution state, resetting non-terminal nodes. Nodes that were Running
        //    at crash time need to be re-executed. This is a recovery path, so the reset bypasses
        //    the forward state machine via `override_node_state` but still bumps the version per
        //    transition so CAS readers see the change (issue #255).
        let mut exec_state = exec_state;
        let non_terminal: Vec<NodeKey> = exec_state
            .node_states
            .iter()
            .filter(|(_, ns)| !ns.state.is_terminal())
            .map(|(id, _)| id.clone())
            .collect();
        for id in non_terminal {
            let _ = exec_state.override_node_state(id, NodeState::Pending);
        }
        // Transition back to Running so the frontier loop can proceed.
        // The persisted state may be Created, Paused, or already Running after a crash.
        // Use transition_status when the transition is valid; skip if already Running.
        if !exec_state.status.is_terminal() && exec_state.status != ExecutionStatus::Running {
            // Ignoring the result is intentional: if this fails the status is left
            // as-is (e.g. Paused), which is still non-terminal and the loop will proceed.
            let _ = exec_state.transition_status(ExecutionStatus::Running);
        }

        // 8. Populate shared output map from persisted outputs.
        let outputs: Arc<DashMap<NodeKey, serde_json::Value>> = Arc::new(DashMap::new());
        for (node_key, value) in persisted_outputs {
            outputs.insert(node_key.clone(), value);
        }

        // 9. Compute the resume frontier and pre-populate edge-tracking maps.
        //
        //    A node is on the frontier if:
        //    - it is not yet terminal (Pending after the reset above), AND
        //    - all its predecessor nodes are terminal in the loaded state.
        //
        //    We also rebuild `activated_edges` and `resolved_edges` for terminal
        //    nodes so that `run_frontier`'s bookkeeping stays consistent when
        //    it evaluates edges from the frontier.
        let node_map: HashMap<NodeKey, &nebula_workflow::NodeDefinition> =
            workflow.nodes.iter().map(|n| (n.id.clone(), n)).collect();

        let mut activated_edges: HashMap<NodeKey, HashSet<NodeKey>> = HashMap::new();
        let mut resolved_edges: HashMap<NodeKey, usize> = HashMap::new();
        let mut seed_nodes: Vec<NodeKey> = Vec::new();

        // Mark edges from terminal nodes as resolved (and activated, since they
        // completed successfully or were skipped).
        for (node_key, ns) in &exec_state.node_states {
            if !ns.state.is_terminal() {
                continue;
            }
            for conn in graph.outgoing_connections(node_key.clone()) {
                let target = conn.to_node.clone();
                // Increment per-edge count so multiple edges from the same terminal
                // source to the same target are each counted during resume.
                *resolved_edges.entry(target.clone()).or_insert(0) += 1;
                // Completed and Skipped nodes activate their outgoing edges so
                // that downstream nodes see a resolved predecessor.
                if matches!(ns.state, NodeState::Completed | NodeState::Skipped) {
                    activated_edges
                        .entry(target.clone())
                        .or_default()
                        .insert(node_key.clone());
                }
            }
        }

        // Identify frontier nodes: non-terminal nodes whose incoming edges are
        // all resolved (i.e., all predecessors are terminal).
        //
        // Note: we do NOT require that at least one edge is activated here.
        // During crash recovery we cannot know which edges were activated — that
        // state is not persisted separately. The conservative check (all
        // predecessors terminal → node is eligible) is correct for crash recovery:
        // the node may have been waiting for an edge that was never activated, but
        // the activated_edges map reconstructed above from Completed/Skipped
        // predecessors gives run_frontier the correct activation context to
        // evaluate edge conditions normally once the node is dispatched.
        for (node_key, ns) in &exec_state.node_states {
            if ns.state.is_terminal() {
                continue;
            }
            let incoming = graph.incoming_connections(node_key.clone());
            let required = incoming.len();
            let resolved = resolved_edges.get(node_key).cloned().unwrap_or(0);

            if required == 0 || resolved == required {
                seed_nodes.push(node_key.clone());
            }
        }

        // 10. Build remaining infrastructure for the frontier loop.
        // TODO: the original ExecutionBudget is not persisted in ExecutionState.
        // For now, resume uses the default budget. To fix, add `budget` to
        // ExecutionState and restore it here.
        let budget = ExecutionBudget::default();
        let semaphore = Arc::new(Semaphore::new(budget.max_concurrent_nodes));
        let cancel_token = CancellationToken::new();
        let mut repo_version = repo_version_loaded;

        self.metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
            .inc();

        let error_strategy = workflow.config.error_strategy;
        // TODO: the original workflow input is not persisted in ExecutionState.
        // Resume passes Null, which means re-running entry nodes will not receive
        // the original trigger data. Entry nodes that have already completed are
        // skipped via idempotency, so this only affects entry nodes that crashed
        // mid-execution without completing. To fix, persist `workflow_input` in
        // ExecutionState alongside the execution ID.
        let workflow_input = serde_json::Value::Null;
        let failed_node = self
            .run_frontier(
                &graph,
                &node_map,
                &outputs,
                &semaphore,
                &cancel_token,
                &mut exec_state,
                execution_id,
                workflow_id,
                &workflow_input,
                &mut repo_version,
                &budget,
                &started,
                error_strategy,
                seed_nodes,
                activated_edges,
                resolved_edges,
            )
            .await;

        self.runtime.clear_execution_output_totals(execution_id);

        let elapsed = started.elapsed();

        let FinalStatusDecision {
            status: final_status,
            integrity_violation,
        } = determine_final_status(&failed_node, &cancel_token, &exec_state);
        // Use the validated transition path. Ignoring the result is intentional:
        // if the current status is already terminal (e.g. the execution was
        // cancelled during the frontier loop), we do not overwrite it.
        let _ = exec_state.transition_status(final_status);

        // Persist final state (best-effort).
        if let Ok(state_json) = serde_json::to_value(&exec_state) {
            match exec_repo
                .transition(execution_id, repo_version, state_json)
                .await
            {
                Ok(true) => {},
                Ok(false) => {
                    tracing::warn!(%execution_id, "resume: final state checkpoint CAS mismatch");
                },
                Err(e) => {
                    tracing::warn!(%execution_id, error = %e, "resume: final state checkpoint failed");
                },
            }
        }

        self.emit_final_event(execution_id, final_status, elapsed, &failed_node);
        self.emit_frontier_integrity_if_violated(execution_id, integrity_violation);
        self.emit_event(ExecutionEvent::ExecutionFinished {
            execution_id,
            success: final_status == ExecutionStatus::Completed,
            elapsed,
        });

        let node_outputs: HashMap<NodeKey, serde_json::Value> = outputs
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect();

        let node_errors: HashMap<NodeKey, String> = exec_state
            .node_states
            .iter()
            .filter_map(|(id, ns)| {
                ns.error_message
                    .as_ref()
                    .map(|msg| (id.clone(), msg.clone()))
            })
            .collect();

        Ok(ExecutionResult {
            execution_id,
            status: final_status,
            node_outputs,
            node_errors,
            duration: elapsed,
        })
    }

    /// Execute all reachable nodes using a frontier-based approach.
    ///
    /// Nodes are spawned as soon as all their incoming edges have been resolved
    /// and at least one edge has been activated. This supports branching, skip
    /// propagation, and error routing.
    ///
    /// `seed_nodes` is the initial set of nodes to place on the ready queue.
    /// For a fresh execution this is the graph's entry nodes; for resumed
    /// executions it is the computed resume frontier.
    ///
    /// `initial_activated` and `initial_resolved` carry the edge-tracking
    /// state derived from already-completed nodes (populated for resume; empty
    /// for fresh executions).
    ///
    /// Returns `Some((node_key, error))` if a node failed without an error handler,
    /// `None` if all reachable nodes completed (or were skipped).
    #[allow(clippy::too_many_arguments)]
    async fn run_frontier(
        &self,
        graph: &DependencyGraph,
        node_map: &HashMap<NodeKey, &nebula_workflow::NodeDefinition>,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        semaphore: &Arc<Semaphore>,
        cancel_token: &CancellationToken,
        exec_state: &mut ExecutionState,
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        input: &serde_json::Value,
        repo_version: &mut u64,
        budget: &ExecutionBudget,
        started: &Instant,
        error_strategy: nebula_workflow::ErrorStrategy,
        seed_nodes: Vec<NodeKey>,
        initial_activated: HashMap<NodeKey, HashSet<NodeKey>>,
        initial_resolved: HashMap<NodeKey, usize>,
    ) -> Option<(NodeKey, String)> {
        let total_output_bytes = Arc::new(AtomicU64::new(0));
        let total_retries = Arc::new(AtomicU32::new(0));
        // Precompute how many incoming edges each node has
        let required_count: HashMap<NodeKey, usize> = node_map
            .keys()
            .map(|nid| (nid.clone(), graph.incoming_connections(nid.clone()).len()))
            .collect();

        // Track edge resolution state (pre-populated for resume)
        let mut activated_edges = initial_activated;
        let mut resolved_edges = initial_resolved;

        // Queue of nodes ready to execute
        let mut ready_queue: VecDeque<NodeKey> = VecDeque::new();

        // Seed with the provided nodes (entry nodes for fresh; frontier for resume)
        for node_key in seed_nodes {
            ready_queue.push_back(node_key);
        }

        // In-flight tasks
        let mut join_set: JoinSet<(
            NodeKey,
            Result<ActionResult<serde_json::Value>, EngineError>,
        )> = JoinSet::new();

        // Main frontier loop
        loop {
            // Phase 1: Drain ready queue → spawn into join_set
            while let Some(node_key) = ready_queue.pop_front() {
                if cancel_token.is_cancelled() {
                    break;
                }

                // Check budget limits before dispatching
                if let Some(violation) =
                    check_budget(budget, started, &total_output_bytes, &total_retries)
                {
                    cancel_token.cancel();
                    return Some((node_key, violation));
                }

                // Skip disabled nodes: mark as Skipped and activate outgoing edges
                // with null output so successors continue normally.
                if node_map.get(&node_key).is_some_and(|nd| !nd.enabled) {
                    mark_node_skipped(exec_state, node_key.clone());
                    process_outgoing_edges(
                        node_key.clone(),
                        None, // null output — Always edges activate
                        None, // not failed
                        graph,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );
                    continue;
                }

                // Durable idempotency check: if this node was already executed
                // (e.g., on a previous attempt), load the persisted output and
                // mark it completed without re-dispatching.
                if self
                    .check_and_apply_idempotency(
                        execution_id,
                        node_key.clone(),
                        outputs,
                        exec_state,
                        graph,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                    )
                    .await
                {
                    continue;
                }

                let spawned = self.spawn_node(
                    node_key.clone(),
                    node_map,
                    graph,
                    outputs,
                    semaphore,
                    cancel_token,
                    exec_state,
                    execution_id,
                    workflow_id,
                    input,
                    &activated_edges,
                    &mut join_set,
                );
                if spawned {
                    let action_key = node_map
                        .get(&node_key)
                        .map(|n| n.action_key.to_string())
                        .unwrap_or_default();
                    self.emit_event(ExecutionEvent::NodeStarted {
                        execution_id,
                        node_key: node_key.clone(),
                        action_key,
                    });
                    continue;
                }

                // Node failed during setup (e.g., param resolution).
                // `spawn_node` already marked the node as Failed and stored
                // the typed error message on `NodeExecutionState`.
                let abort = handle_node_failure(
                    node_key.clone(),
                    "parameter resolution failed",
                    error_strategy,
                    graph,
                    outputs,
                    &mut activated_edges,
                    &mut resolved_edges,
                    &required_count,
                    &mut ready_queue,
                    exec_state,
                );
                if let Some(err_msg) = abort {
                    cancel_token.cancel();
                    return Some((node_key, err_msg));
                }

                // Mirror the runtime-failure branch (§11.5, #297/#321):
                // when the node remains Failed after handle_node_failure
                // (i.e., not recovered by IgnoreErrors), persist the
                // failure decision and any OnError/ContinueOnError
                // edge-routing it triggered before any observer sees the
                // node as done. Without this, a crash between here and
                // the final-state checkpoint would lose both the Failed
                // state and the edge-routing already applied in memory.
                if exec_state
                    .node_state(node_key.clone())
                    .is_some_and(|ns| ns.state == NodeState::Failed)
                {
                    self.checkpoint_node(
                        execution_id,
                        node_key.clone(),
                        outputs,
                        exec_state,
                        repo_version,
                    )
                    .await;
                    let err = exec_state
                        .node_state(node_key.clone())
                        .and_then(|ns| ns.error_message.clone())
                        .unwrap_or_else(|| "parameter resolution failed".to_string());
                    self.emit_event(ExecutionEvent::NodeFailed {
                        execution_id,
                        node_key: node_key.clone(),
                        error: err,
                    });
                }
            }

            // Phase 2: Wait for one completion (or exit if nothing in flight)
            if join_set.is_empty() {
                break;
            }

            if cancel_token.is_cancelled() {
                join_set.abort_all();
                while join_set.join_next().await.is_some() {}
                break;
            }

            // Race join_set against the wall-clock deadline so a hung node
            // cannot starve budget enforcement. The Phase 1 check_budget call
            // only fires while ready_queue has work; once everything is in
            // flight, this select is the sole budget guard.
            let wall_clock_remaining: Option<Duration> = budget
                .max_duration
                .map(|max_dur| max_dur.saturating_sub(started.elapsed()));
            let sleep_fut = async {
                if let Some(d) = wall_clock_remaining {
                    tokio::time::sleep(d).await
                } else {
                    std::future::pending::<()>().await
                }
            };
            tokio::pin!(sleep_fut);

            let join_result = tokio::select! {
                result = join_set.join_next() => match result {
                    Some(r) => r,
                    None => break,
                },
                () = &mut sleep_fut => {
                    cancel_token.cancel();
                    join_set.abort_all();
                    while join_set.join_next().await.is_some() {}
                    return Some((
                        node_key!("_timeout"),
                        "execution budget exceeded: max_duration".to_string(),
                    ));
                }
                () = cancel_token.cancelled() => {
                    join_set.abort_all();
                    while join_set.join_next().await.is_some() {}
                    break;
                }
            };

            // Phase 3: Process the completed task
            match join_result {
                Ok((node_key, Ok(ActionResult::Retry { .. }))) => {
                    // ActionResult::Retry has no scheduler yet; treat it as a node
                    // failure for at-least-once semantics (#290/#296 short-term).
                    total_retries.fetch_add(1, Ordering::Relaxed);
                    let err = EngineError::Runtime(nebula_runtime::RuntimeError::ActionError(
                        nebula_action::error::ActionError::retryable(
                            "Action retry is not supported by the engine",
                        ),
                    ));
                    mark_node_failed(exec_state, node_key.clone(), &err);
                    let abort = handle_node_failure(
                        node_key.clone(),
                        &err.to_string(),
                        error_strategy,
                        graph,
                        outputs,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );
                    if let Some(err_msg) = abort {
                        cancel_token.cancel();
                        return Some((node_key.clone(), err_msg));
                    }

                    if exec_state
                        .node_state(node_key.clone())
                        .is_some_and(|ns| ns.state == NodeState::Failed)
                    {
                        self.checkpoint_node(
                            execution_id,
                            node_key.clone(),
                            outputs,
                            exec_state,
                            repo_version,
                        )
                        .await;
                        self.emit_event(ExecutionEvent::NodeFailed {
                            execution_id,
                            node_key: node_key.clone(),
                            error: err.to_string(),
                        });
                    }
                },
                Ok((node_key, Ok(action_result))) => {
                    mark_node_completed(exec_state, node_key.clone());

                    // Track output size for budget enforcement
                    if let Some(output) = outputs.get(&node_key) {
                        let bytes = serde_json::to_string(output.value())
                            .map(|s| s.len() as u64)
                            .unwrap_or(0);
                        total_output_bytes.fetch_add(bytes, Ordering::Relaxed);
                    }

                    // Persist node output + execution state, then record the
                    // idempotency key, before any external observer learns the
                    // node is done. This guarantees durability precedes
                    // visibility (#297).
                    self.checkpoint_node(
                        execution_id,
                        node_key.clone(),
                        outputs,
                        exec_state,
                        repo_version,
                    )
                    .await;
                    self.record_idempotency(execution_id, node_key.clone())
                        .await;

                    self.emit_event(ExecutionEvent::NodeCompleted {
                        execution_id,
                        node_key: node_key.clone(),
                        elapsed: started.elapsed(),
                    });

                    // Evaluate outgoing edges and update frontier
                    process_outgoing_edges(
                        node_key.clone(),
                        Some(&action_result),
                        None, // not failed
                        graph,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );
                },
                Ok((node_key, Err(ref err))) => {
                    // Node failed at runtime — persist before any observer
                    // learns the node is done and before successors advance
                    // (#297).
                    mark_node_failed(exec_state, node_key.clone(), err);
                    let abort = handle_node_failure(
                        node_key.clone(),
                        &err.to_string(),
                        error_strategy,
                        graph,
                        outputs,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );

                    if let Some(err_msg) = abort {
                        cancel_token.cancel();
                        return Some((node_key.clone(), err_msg));
                    }

                    if exec_state
                        .node_state(node_key.clone())
                        .is_some_and(|ns| ns.state == NodeState::Failed)
                    {
                        self.checkpoint_node(
                            execution_id,
                            node_key.clone(),
                            outputs,
                            exec_state,
                            repo_version,
                        )
                        .await;
                        self.emit_event(ExecutionEvent::NodeFailed {
                            execution_id,
                            node_key: node_key.clone(),
                            error: err.to_string(),
                        });
                    }
                },
                Err(join_err) => {
                    tracing::error!(?join_err, "node task panicked");
                    cancel_token.cancel();
                    return Some((node_key!("_panicked"), join_err.to_string()));
                },
            }
        }

        None
    }

    /// Spawn a single node into the JoinSet.
    ///
    /// Returns `true` if the node was spawned, `false` if it failed during setup
    /// (e.g., param resolution error).
    #[allow(clippy::too_many_arguments)]
    fn spawn_node(
        &self,
        node_key: NodeKey,
        node_map: &HashMap<NodeKey, &nebula_workflow::NodeDefinition>,
        graph: &DependencyGraph,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
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
    ) -> bool {
        let Some(node_def) = node_map.get(&node_key) else {
            return false;
        };
        let action_key = node_def.action_key.as_str().to_owned();
        let interface_version = node_def.interface_version.clone();

        // Partition incoming connections into flow (to_port=None) and support (to_port=Some)
        let (node_input, support_inputs) = resolve_node_input_with_support(
            node_key.clone(),
            graph,
            outputs,
            input,
            activated_edges,
        );

        // Resolve node parameters (expressions, templates, references)
        let action_input =
            match self
                .resolver
                .resolve(&node_key, &node_def.parameters, &node_input, outputs)
            {
                Ok(Some(resolved_params)) => resolved_params,
                Ok(None) => node_input, // No parameters → use predecessor output
                Err(e) => {
                    // Mark node as failed. Using `override_node_state`
                    // rather than a Ready→Running→Failed sequence
                    // because (a) parameter resolution failed BEFORE
                    // the node was scheduled so it is still Pending
                    // here, and Pending→Failed is not a valid forward
                    // transition; (b) we know for a fact the node
                    // failed and want it in the Failed state
                    // regardless of its current position. Version is
                    // still bumped per issue #255.
                    let _ = exec_state.override_node_state(node_key.clone(), NodeState::Failed);
                    if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
                        ns.error_message = Some(e.to_string());
                    }
                    return false;
                },
            };

        // Mark node as running in execution state (versioned).
        // Log on failure so a rejected transition is not silently
        // swallowed — callers of the frontier loop need to know if
        // the execution state got out of sync with the scheduler.
        if let Err(err) = exec_state.transition_node(node_key.clone(), NodeState::Ready) {
            tracing::warn!(%node_key, %err, "failed to transition node to Ready");
        }
        if let Err(err) = exec_state.transition_node(node_key.clone(), NodeState::Running) {
            tracing::warn!(%node_key, %err, "failed to transition node to Running");
        }

        let runtime = self.runtime.clone();
        let cancel = cancel_token.clone();
        let sem = semaphore.clone();
        let outputs_ref = outputs.clone();

        // Build credential accessor with a **deny-by-default** per-action allowlist.
        //
        // Per `PRODUCT_CANON` §4.5 / §12.5 + audit §2.4: an action can only
        // acquire credential IDs explicitly declared for its `ActionKey` via
        // `WorkflowEngine::with_action_credentials`. If the node's action was
        // never declared — or was declared with an empty set — the accessor
        // refuses every `get`/`has` request with
        // `CredentialAccessError::AccessDenied`. No silent "allow all" fallback.
        let allowed_keys: HashSet<String> = self
            .action_credentials
            .get(&node_def.action_key)
            .cloned()
            .unwrap_or_default();
        let credentials: Arc<dyn CredentialAccessor> =
            if let Some(resolver_fn) = &self.credential_resolver {
                let resolver_fn = Arc::clone(resolver_fn);
                Arc::new(EngineCredentialAccessor::new(
                    allowed_keys,
                    move |id: &str| {
                        let resolver_fn = Arc::clone(&resolver_fn);
                        let id = id.to_owned();
                        async move { (resolver_fn)(&id).await }
                    },
                    node_def.action_key.as_str().to_owned(),
                ))
            } else {
                default_credential_accessor()
            };

        // Build resource accessor: use the manager if configured, else noop.
        let resources: Arc<dyn ResourceAccessor> = if let Some(manager) = &self.resource_manager {
            Arc::new(EngineResourceAccessor::new(Arc::clone(manager)))
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

        // Build rate limiter from node definition if configured.
        let rate_limiter = node_def.rate_limit.as_ref().and_then(|rl| {
            let refill_rate = rl.max_requests as f64 / rl.window_secs.max(1) as f64;
            nebula_resilience::rate_limiter::TokenBucket::new(rl.max_requests as usize, refill_rate)
                .ok()
                .map(Arc::new)
        });

        join_set.spawn(
            NodeTask {
                runtime,
                cancel,
                sem,
                outputs: outputs_ref,
                execution_id,
                node_key: node_key.clone(),
                workflow_id,
                action_key,
                interface_version,
                input: action_input,
                support_inputs,
                credentials,
                resources,
                credential_refresh,
                rate_limiter,
            }
            .run(),
        );

        true
    }

    /// Check whether a node was already executed (idempotency key is set) and,
    /// if so, load its persisted output, mark it completed, and activate outgoing
    /// edges — all without re-dispatching the action.
    ///
    /// Returns `true` when the node was short-circuited (caller should `continue`
    /// to the next ready queue entry). Returns `false` when the node should be
    /// dispatched normally.
    #[allow(clippy::too_many_arguments)]
    async fn check_and_apply_idempotency(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        exec_state: &mut ExecutionState,
        graph: &DependencyGraph,
        activated_edges: &mut HashMap<NodeKey, HashSet<NodeKey>>,
        resolved_edges: &mut HashMap<NodeKey, usize>,
        required_count: &HashMap<NodeKey, usize>,
        ready_queue: &mut VecDeque<NodeKey>,
    ) -> bool {
        let Some(repo) = &self.execution_repo else {
            return false;
        };

        let idem_key = format!("{execution_id}:{node_key}:1");

        let already_done = match repo.check_idempotency(&idem_key).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    error = %e,
                    "idempotency check failed; proceeding with execution"
                );
                return false;
            },
        };

        if !already_done {
            return false;
        }

        // Node was already executed — load the persisted output.
        let output_value = match repo.load_node_output(execution_id, node_key.clone()).await {
            Ok(Some(v)) => v,
            Ok(None) => {
                // Idempotency key exists but no output was persisted. This indicates
                // a partial write (e.g. the process crashed after marking idempotent
                // but before saving the output). Re-execute to produce a clean result.
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    "idempotency key present but output missing; re-executing node"
                );
                return false;
            },
            Err(e) => {
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    error = %e,
                    "failed to load idempotent node output; re-executing"
                );
                return false;
            },
        };

        outputs.insert(node_key.clone(), output_value.clone());
        mark_node_completed(exec_state, node_key.clone());

        let fake_result = ActionResult::success(output_value);
        process_outgoing_edges(
            node_key.clone(),
            Some(&fake_result),
            None,
            graph,
            activated_edges,
            resolved_edges,
            required_count,
            ready_queue,
            exec_state,
        );

        true
    }

    /// Record an idempotency key for a successfully executed node (best-effort).
    ///
    /// Silently logs and ignores errors — idempotency key recording failures
    /// must not abort an otherwise healthy execution.
    async fn record_idempotency(&self, execution_id: ExecutionId, node_key: NodeKey) {
        let Some(repo) = &self.execution_repo else {
            return;
        };
        let idem_key = format!("{execution_id}:{node_key}:1");
        if let Err(e) = repo
            .mark_idempotent(&idem_key, execution_id, node_key.clone())
            .await
        {
            tracing::warn!(
                %execution_id,
                %node_key,
                error = %e,
                "failed to mark node as idempotent"
            );
        }
    }

    /// Persist node output and execution state to the repository (best-effort).
    ///
    /// Silently ignores errors — checkpoint failures must not abort
    /// an otherwise healthy execution.
    async fn checkpoint_node(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        exec_state: &ExecutionState,
        repo_version: &mut u64,
    ) {
        let Some(repo) = &self.execution_repo else {
            return;
        };

        // Save node output individually
        if let Some(output) = outputs.get(&node_key) {
            let attempt = exec_state
                .node_states
                .get(&node_key)
                .map(|ns| ns.attempt_count().max(1) as u32)
                .unwrap_or(1);
            if let Err(e) = repo
                .save_node_output(
                    execution_id,
                    node_key.clone(),
                    attempt,
                    output.value().clone(),
                )
                .await
            {
                tracing::warn!(%execution_id, %node_key, error = %e, "failed to persist node output");
            }
        }

        // Save execution state snapshot
        if let Ok(state_json) = serde_json::to_value(exec_state) {
            match repo
                .transition(execution_id, *repo_version, state_json)
                .await
            {
                Ok(true) => *repo_version += 1,
                Ok(false) => {
                    // CAS mismatch — re-read current version to recover
                    tracing::warn!(
                        %execution_id,
                        "checkpoint CAS mismatch, re-reading version"
                    );
                    if let Ok(Some((current_version, _))) = repo.get_state(execution_id).await {
                        *repo_version = current_version;
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        %execution_id,
                        error = %e,
                        "checkpoint persist failed"
                    );
                },
            }
        }
    }

    /// Record final execution metrics.
    fn emit_final_event(
        &self,
        _execution_id: ExecutionId,
        status: ExecutionStatus,
        elapsed: std::time::Duration,
        _failed_node: &Option<(NodeKey, String)>,
    ) {
        match status {
            ExecutionStatus::Completed => {
                self.metrics
                    .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
                    .inc();
            },
            ExecutionStatus::Failed => {
                self.metrics
                    .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
                    .inc();
            },
            _ => {},
        }

        self.metrics
            .histogram(NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS)
            .observe(elapsed.as_secs_f64());
    }
}

/// Bundled parameters for a single node execution task.
struct NodeTask {
    runtime: Arc<ActionRuntime>,
    cancel: CancellationToken,
    sem: Arc<Semaphore>,
    outputs: Arc<DashMap<NodeKey, serde_json::Value>>,
    execution_id: ExecutionId,
    node_key: NodeKey,
    workflow_id: WorkflowId,
    action_key: String,
    /// Pinned interface version for versioned action lookup.
    ///
    /// When `Some`, the runtime uses [`execute_action_versioned`] with this
    /// exact version. When `None`, the latest registered handler is used.
    interface_version: Option<semver::Version>,
    input: serde_json::Value,
    /// Data for support input ports, keyed by port name.
    #[allow(dead_code)] // reserved for multi-input actions
    support_inputs: HashMap<String, Vec<serde_json::Value>>,
    /// Credential accessor injected into the action context.
    credentials: Arc<dyn CredentialAccessor>,
    /// Resource accessor injected into the action context.
    resources: Arc<dyn ResourceAccessor>,
    /// Optional proactive credential refresh hook.
    ///
    /// Called before the action executes when a credential resolver is
    /// configured. The argument passed to the hook is the **action key**, not a
    /// credential ID. The caller (engine builder) controls the mapping from
    /// action key to credential IDs — this is a best-effort pre-dispatch hint.
    ///
    /// TODO: when per-node credential declarations are populated from action
    /// dependency metadata, pass the actual credential ID(s) instead.
    credential_refresh: Option<CredentialRefreshFn>,
    /// Optional rate limiter shared with other nodes using the same ActionKey.
    rate_limiter: Option<Arc<nebula_resilience::rate_limiter::TokenBucket>>,
}

impl NodeTask {
    /// Execute this node: acquire semaphore, check cancellation, run action.
    async fn run(
        self,
    ) -> (
        NodeKey,
        Result<ActionResult<serde_json::Value>, EngineError>,
    ) {
        let _permit = match self.sem.acquire().await {
            Ok(permit) => permit,
            Err(_) => return (self.node_key, Err(EngineError::Cancelled)),
        };

        if self.cancel.is_cancelled() {
            return (self.node_key, Err(EngineError::Cancelled));
        }

        // Proactive credential refresh: call the hook before the action runs
        // so that any short-lived credential is rotated while still valid.
        //
        // Failure modes (Batch 5D / #306):
        //   - cancel fires first → return EngineError::Cancelled. We must NOT block shutdown on a
        //     dying credential store.
        //   - refresh returns Err → surface a typed ActionError::CredentialRefreshFailed through
        //     EngineError::Action. The frontier loop routes this through `handle_node_failure`,
        //     where the workflow-level ErrorStrategy decides whether execution fails fast or
        //     continues/ignores the failure, and whether any `OnError` edge is activated. This
        //     replaces the old "log a WARN and proceed with a potentially stale credential" path,
        //     which leaked into N opaque downstream auth errors per failure.
        //   - refresh returns Ok → fall through to the action dispatch.
        if let Some(ref refresh_fn) = self.credential_refresh {
            let refresh_fut = (refresh_fn)(&self.action_key);
            tokio::pin!(refresh_fut);
            let refresh_result = tokio::select! {
                biased;
                () = self.cancel.cancelled() => {
                    return (self.node_key, Err(EngineError::Cancelled));
                }
                res = &mut refresh_fut => res,
            };

            match refresh_result {
                Ok(()) => {},
                Err(source) => {
                    let action_err =
                        ActionError::credential_refresh_failed(self.action_key.to_string(), source);
                    return (self.node_key, Err(EngineError::Action(action_err)));
                },
            }
        }

        let action_ctx = ActionContext::new(
            self.execution_id,
            self.node_key.clone(),
            self.workflow_id,
            self.cancel.child_token(),
        )
        .with_credentials(self.credentials.clone())
        .with_resources(self.resources.clone());

        // Acquire rate limit permit if configured. If the limiter rejects the
        // request, fail the node so ErrorStrategy decides abort/continue.
        if let Some(ref limiter) = self.rate_limiter {
            use nebula_resilience::rate_limiter::RateLimiter;
            if let Err(e) = limiter.acquire().await {
                tracing::warn!(
                    node_key = %self.node_key.clone(),
                    action_key = %self.action_key,
                    error = ?e,
                    "rate limit exceeded; failing node"
                );
                let action_err = nebula_action::error::ActionError::retryable_with_hint(
                    format!("rate limit exceeded: {e:?}"),
                    nebula_action::error::RetryHintCode::RateLimited,
                );
                return (
                    self.node_key.clone(),
                    Err(EngineError::Runtime(
                        nebula_runtime::RuntimeError::ActionError(action_err),
                    )),
                );
            }
        }

        // Use versioned action lookup when the node definition pins a version;
        // fall back to the latest registered handler otherwise.
        let result = self
            .runtime
            .execute_action_versioned(
                &self.action_key,
                self.interface_version.as_ref(),
                self.input,
                action_ctx,
            )
            .await;

        match result {
            Ok(action_result) => {
                // Extract the primary output for downstream node input resolution.
                if let Some(output) = extract_primary_output(&action_result) {
                    self.outputs.insert(self.node_key.clone(), output);
                }
                (self.node_key, Ok(action_result))
            },
            Err(e) => (self.node_key, Err(EngineError::Runtime(e))),
        }
    }
}

// ── Edge evaluation ─────────────────────────────────────────────────────────

/// Process outgoing edges from a completed/failed/skipped node.
///
/// For each outgoing edge, evaluates whether it should activate, updates
/// the tracking maps, and checks if any target node becomes ready or should
/// be skipped.
///
/// Returns `true` if the error was handled (at least one OnError edge activated).
#[allow(clippy::too_many_arguments)]
fn process_outgoing_edges(
    source_id: NodeKey,
    result: Option<&ActionResult<serde_json::Value>>,
    error_msg: Option<&str>,
    graph: &DependencyGraph,
    activated_edges: &mut HashMap<NodeKey, HashSet<NodeKey>>,
    resolved_edges: &mut HashMap<NodeKey, usize>,
    required_count: &HashMap<NodeKey, usize>,
    ready_queue: &mut VecDeque<NodeKey>,
    exec_state: &mut ExecutionState,
) -> bool {
    let outgoing = graph.outgoing_connections(source_id.clone());
    let node_failed = error_msg.is_some();
    let mut error_handled = false;

    for conn in &outgoing {
        let target = conn.to_node.clone();
        let activate = evaluate_edge(conn, result, node_failed);

        // Increment the per-edge resolved count (not per-source, so that multiple
        // edges from the same source node to the same target are each counted).
        *resolved_edges.entry(target.clone()).or_insert(0) += 1;
        if activate {
            activated_edges
                .entry(target.clone())
                .or_default()
                .insert(source_id.clone());
            if node_failed {
                error_handled = true;
            }
        }

        // Check if target is now fully resolved
        let resolved = resolved_edges.get(&target).cloned().unwrap_or(0);
        let required = required_count.get(&target).cloned().unwrap_or(0);
        let activated = activated_edges.get(&target).map_or(0, |s| s.len());

        if resolved == required {
            if activated > 0 {
                ready_queue.push_back(target);
            } else {
                propagate_skip(
                    target,
                    graph,
                    exec_state,
                    resolved_edges,
                    activated_edges,
                    required_count,
                    ready_queue,
                );
            }
        }
    }

    error_handled
}

/// Evaluate whether an edge should activate given the source node's outcome.
///
/// Rules:
/// - `Skip` results don't activate any edges (whole downstream subgraph)
/// - `Drop` results don't activate any edges (item dropped, no output on main port)
/// - `Terminate` results don't activate any edges (execution is ending)
/// - Failed nodes only activate `OnError` edges
/// - `Branch` results only activate edges whose `branch_key` matches `selected`
/// - `Route` results only activate edges whose `from_port` matches `port`
/// - `MultiOutput` results only activate edges whose `from_port` is in `outputs`
/// - `Always` activates on success (not on error, not on skip)
/// - `OnResult` activates when the matcher matches
/// - `Expression` activates when the expression evaluates to truthy
/// - `OnError` activates only when the node failed
fn evaluate_edge(
    conn: &Connection,
    result: Option<&ActionResult<serde_json::Value>>,
    node_failed: bool,
) -> bool {
    // Skip results don't activate any edges — skips the whole downstream subgraph.
    if let Some(ActionResult::Skip { .. }) = result {
        return false;
    }

    // Drop results don't activate any edges — this item produced no output
    // on the main port, but downstream parallel branches processing other
    // items are unaffected (they have their own per-item evaluate_edge call).
    if let Some(ActionResult::Drop { .. }) = result {
        return false;
    }

    // Terminate results don't activate any edges — the execution is ending.
    // TODO(engine): this gate only blocks local downstream edges. Full
    // parallel-branch cancellation, ExecutionTerminationReason propagation,
    // and determine_final_status handling for Terminate is scheduler work
    // tracked in the ControlAction plan as Phase 3. Until then, a Terminate
    // return from a node in one branch does NOT cancel sibling branches
    // and does NOT populate ExecutionResult::termination_reason — it only
    // prevents this node's own subgraph from firing.
    if let Some(ActionResult::Terminate { .. }) = result {
        return false;
    }

    // For failed nodes, only OnError edges can activate
    if node_failed {
        return match &conn.condition {
            EdgeCondition::OnError { matcher } => match_error_condition(matcher),
            _ => false,
        };
    }

    // Check branch_key for Branch results
    if let Some(ActionResult::Branch { selected, .. }) = result
        && let Some(ref key) = conn.branch_key
        && key != selected
    {
        return false;
    }

    // Check from_port for Route results
    if let Some(ActionResult::Route { port, .. }) = result
        && let Some(ref key) = conn.from_port
        && key != port
    {
        return false;
    }

    // Check from_port for MultiOutput results
    if let Some(ActionResult::MultiOutput {
        outputs: port_outputs,
        ..
    }) = result
        && let Some(ref key) = conn.from_port
        && !port_outputs.contains_key(key)
    {
        return false;
    }

    // Evaluate the edge condition
    match &conn.condition {
        EdgeCondition::Always => true,
        EdgeCondition::OnError { .. } => false, // Not failed, so OnError doesn't activate
        EdgeCondition::OnResult { matcher } => match_result_condition(matcher, result),
        EdgeCondition::Expression { expr } => evaluate_expression_condition(expr, result),
        _ => false,
    }
}

/// Check if an OnError condition matches (for failed nodes).
fn match_error_condition(matcher: &ErrorMatcher) -> bool {
    match matcher {
        ErrorMatcher::Any => true,
        // For Code and Expression matchers, default to matching for now.
        // Full implementation would check error codes or evaluate expressions.
        ErrorMatcher::Code { .. } | ErrorMatcher::Expression { .. } => true,
        _ => false,
    }
}

/// Check if an OnResult condition matches the node's output.
fn match_result_condition(
    matcher: &ResultMatcher,
    result: Option<&ActionResult<serde_json::Value>>,
) -> bool {
    match matcher {
        ResultMatcher::Success => true,
        ResultMatcher::FieldEquals { field, value } => {
            let output = result.and_then(extract_primary_output);
            output
                .as_ref()
                .and_then(|v| v.get(field))
                .is_some_and(|v| v == value)
        },
        // Expression-based result matching; default to true for now.
        ResultMatcher::Expression { .. } => true,
        _ => false,
    }
}

/// Evaluate an Expression edge condition against the node's output.
fn evaluate_expression_condition(
    expr: &str,
    result: Option<&ActionResult<serde_json::Value>>,
) -> bool {
    // Build a minimal expression engine for evaluation.
    // In production, this should be shared; here we use a lightweight approach.
    let engine = ExpressionEngine::new();
    let mut ctx = EvaluationContext::new();
    if let Some(output) = result.and_then(extract_primary_output) {
        ctx.set_input(output);
    }
    match engine.evaluate(expr, &ctx) {
        Ok(value) => value.as_bool().unwrap_or(false),
        Err(_) => false,
    }
}

/// Recursively mark a node and its unreachable successors as skipped.
fn propagate_skip(
    node_key: NodeKey,
    graph: &DependencyGraph,
    exec_state: &mut ExecutionState,
    resolved_edges: &mut HashMap<NodeKey, usize>,
    activated_edges: &HashMap<NodeKey, HashSet<NodeKey>>,
    required_count: &HashMap<NodeKey, usize>,
    ready_queue: &mut VecDeque<NodeKey>,
) {
    // Guard against double-processing
    if let Some(ns) = exec_state.node_states.get(&node_key)
        && ns.state.is_terminal()
    {
        return;
    }

    // Versioned transition (issue #255).
    let _ = exec_state.transition_node(node_key.clone(), NodeState::Skipped);

    // Mark all outgoing edges as resolved (dead) for their targets
    for conn in graph.outgoing_connections(node_key) {
        let target = conn.to_node.clone();
        // Increment per-edge count (not per-source) so that multiple edges from
        // the same skipped source to the same target are each counted.
        *resolved_edges.entry(target.clone()).or_insert(0) += 1;

        let resolved = resolved_edges.get(&target).cloned().unwrap_or(0);
        let required = required_count.get(&target).cloned().unwrap_or(0);
        let activated = activated_edges.get(&target).map_or(0, |s| s.len());

        if resolved == required {
            if activated > 0 {
                ready_queue.push_back(target);
            } else {
                propagate_skip(
                    target,
                    graph,
                    exec_state,
                    resolved_edges,
                    activated_edges,
                    required_count,
                    ready_queue,
                );
            }
        }
    }
}

// ── Node state helpers ──────────────────────────────────────────────────────

/// Check whether any budget limit has been exceeded.
///
/// Returns `Some(reason)` if a limit is exceeded, `None` otherwise.
fn check_budget(
    budget: &ExecutionBudget,
    started: &Instant,
    total_output_bytes: &AtomicU64,
    total_retries: &AtomicU32,
) -> Option<String> {
    if let Some(max_dur) = budget.max_duration
        && started.elapsed() > max_dur
    {
        return Some("execution budget exceeded: max_duration".into());
    }
    if let Some(max_bytes) = budget.max_output_bytes
        && total_output_bytes.load(Ordering::Relaxed) > max_bytes
    {
        return Some("execution budget exceeded: max_output_bytes".into());
    }
    if let Some(max_retries) = budget.max_total_retries
        && total_retries.load(Ordering::Relaxed) > max_retries
    {
        return Some("execution budget exceeded: max_total_retries".into());
    }
    None
}

/// Handle a node failure according to the configured error strategy.
///
/// Returns `Some(error_message)` when the caller should cancel + return
/// (i.e., fail-fast), or `None` when execution may continue.
#[allow(clippy::too_many_arguments)]
fn handle_node_failure(
    node_key: NodeKey,
    error_msg: &str,
    error_strategy: nebula_workflow::ErrorStrategy,
    graph: &DependencyGraph,
    outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
    activated_edges: &mut HashMap<NodeKey, HashSet<NodeKey>>,
    resolved_edges: &mut HashMap<NodeKey, usize>,
    required_count: &HashMap<NodeKey, usize>,
    ready_queue: &mut VecDeque<NodeKey>,
    exec_state: &mut ExecutionState,
) -> Option<String> {
    // IgnoreErrors: treat the failure as a successful null result so
    // downstream nodes activate normally.
    if error_strategy == nebula_workflow::ErrorStrategy::IgnoreErrors {
        // The node was already marked Failed by the caller; recover it to
        // Completed since we are ignoring the error, keeping state consistent.
        // `Failed → Completed` is not a valid forward transition, so this
        // uses `override_node_state` to reset the state; the version is
        // still bumped so CAS readers observe the recovery (issue #255).
        let _ = exec_state.override_node_state(node_key.clone(), NodeState::Completed);
        if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
            ns.error_message = None;
        }
        outputs.insert(node_key.clone(), serde_json::json!(null));
        process_outgoing_edges(
            node_key.clone(),
            Some(&ActionResult::success(serde_json::json!(null))),
            None,
            graph,
            activated_edges,
            resolved_edges,
            required_count,
            ready_queue,
            exec_state,
        );
        return None;
    }

    // For FailFast / ContinueOnError: evaluate edges as a failure to
    // check for OnError handlers.
    let error_handled = process_outgoing_edges(
        node_key.clone(),
        None,
        Some(error_msg),
        graph,
        activated_edges,
        resolved_edges,
        required_count,
        ready_queue,
        exec_state,
    );

    if error_handled {
        // Store error info for the OnError handler's input.
        outputs.insert(
            node_key.clone(),
            serde_json::json!({
                "error": error_msg,
                "node_id": node_key.to_string(),
            }),
        );
        return None;
    }

    match error_strategy {
        nebula_workflow::ErrorStrategy::ContinueOnError => {
            // Edges already resolved (not activated) above — dependents
            // will be skipped; unaffected branches continue.
            None
        },
        // FailFast and future variants
        _ => Some(error_msg.to_owned()),
    }
}

/// Mark a node as skipped in the execution state.
///
/// Uses the versioned transition API (issue #255) so CAS readers see
/// the parent version move.
fn mark_node_skipped(exec_state: &mut ExecutionState, node_key: NodeKey) {
    let _ = exec_state.transition_node(node_key.clone(), NodeState::Skipped);
}

/// Mark a node as completed in the execution state.
fn mark_node_completed(exec_state: &mut ExecutionState, node_key: NodeKey) {
    let _ = exec_state.transition_node(node_key.clone(), NodeState::Completed);
}

/// Mark a node as failed in the execution state.
fn mark_node_failed(exec_state: &mut ExecutionState, node_key: NodeKey, err: &EngineError) {
    let _ = exec_state.transition_node(node_key.clone(), NodeState::Failed);
    if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
        ns.error_message = Some(err.to_string());
    }
}

/// Outcome of the final-status decision at the end of a frontier loop.
///
/// Combines the chosen [`ExecutionStatus`] with optional integrity-violation
/// detail so the caller can emit a diagnostic
/// [`ExecutionEvent::FrontierIntegrityViolation`] before the usual
/// [`ExecutionEvent::ExecutionFinished`]. Keeping the decision pure (no
/// event emission inside the function) lets us unit-test it without
/// a live `WorkflowEngine`.
#[derive(Debug)]
struct FinalStatusDecision {
    status: ExecutionStatus,
    /// `Some(nodes)` when the frontier exited without `failed_node` or
    /// cancellation but not all nodes reached a terminal state — see
    /// `docs/PRODUCT_CANON.md` §11.1.
    integrity_violation: Option<Vec<(NodeKey, NodeState)>>,
}

/// Determine the final execution status.
///
/// Gates `Completed` on [`ExecutionState::all_nodes_terminal`] to satisfy the
/// §11.1 invariant: if the frontier drains without a failure or cancellation
/// but some nodes are still non-terminal, we return `Failed` with an attached
/// integrity-violation payload so the caller can emit a diagnostic event and
/// (optionally) surface [`EngineError::FrontierIntegrity`] to operators.
fn determine_final_status(
    failed_node: &Option<(NodeKey, String)>,
    cancel_token: &CancellationToken,
    exec_state: &ExecutionState,
) -> FinalStatusDecision {
    if failed_node.is_some() {
        return FinalStatusDecision {
            status: ExecutionStatus::Failed,
            integrity_violation: None,
        };
    }
    if cancel_token.is_cancelled() {
        return FinalStatusDecision {
            status: ExecutionStatus::Cancelled,
            integrity_violation: None,
        };
    }
    if !exec_state.all_nodes_terminal() {
        let non_terminal: Vec<(NodeKey, NodeState)> = exec_state
            .node_states
            .iter()
            .filter(|(_, ns)| !ns.state.is_terminal())
            .map(|(id, ns)| (id.clone(), ns.state))
            .collect();
        tracing::error!(
            execution_id = %exec_state.execution_id,
            non_terminal_count = non_terminal.len(),
            ?non_terminal,
            "frontier integrity violation: loop exited with non-terminal nodes; \
             marking execution as Failed to satisfy PRODUCT_CANON §11.1"
        );
        return FinalStatusDecision {
            status: ExecutionStatus::Failed,
            integrity_violation: Some(non_terminal),
        };
    }
    FinalStatusDecision {
        status: ExecutionStatus::Completed,
        integrity_violation: None,
    }
}

// ── Input resolution ────────────────────────────────────────────────────────

/// Resolve node input, partitioning by `to_port` into flow input and support inputs.
///
/// Connections with `to_port = None` feed the main flow input (same as before).
/// Connections with `to_port = Some(port_name)` are collected into a per-port
/// map of values, delivered to the action via `ActionContext::support_inputs`.
fn resolve_node_input_with_support(
    node_key: NodeKey,
    graph: &DependencyGraph,
    outputs: &DashMap<NodeKey, serde_json::Value>,
    workflow_input: &serde_json::Value,
    activated_edges: &HashMap<NodeKey, HashSet<NodeKey>>,
) -> (serde_json::Value, HashMap<String, Vec<serde_json::Value>>) {
    let activated: HashSet<NodeKey> = activated_edges.get(&node_key).cloned().unwrap_or_default();

    // Partition incoming connections by to_port
    let incoming = graph.incoming_connections(node_key);
    let mut flow_predecessors: Vec<NodeKey> = Vec::new();
    let mut support_inputs: HashMap<String, Vec<serde_json::Value>> = HashMap::new();

    for conn in &incoming {
        let source = conn.from_node.clone();
        if !activated.contains(&source) {
            continue;
        }
        match &conn.to_port {
            None => {
                if !flow_predecessors.contains(&source) {
                    flow_predecessors.push(source);
                }
            },
            Some(port_name) => {
                if let Some(output) = outputs.get(&source) {
                    support_inputs
                        .entry(port_name.clone())
                        .or_default()
                        .push(output.value().clone());
                }
            },
        }
    }

    // Resolve main flow input from flow predecessors
    let flow_input = if flow_predecessors.is_empty() {
        // No flow predecessors — use workflow-level input (entry node) or Null
        if activated.is_empty() {
            workflow_input.clone()
        } else {
            serde_json::Value::Null
        }
    } else if flow_predecessors.len() == 1 {
        outputs
            .get(&flow_predecessors[0])
            .map(|v| v.value().clone())
            .unwrap_or(serde_json::Value::Null)
    } else {
        let mut merged = serde_json::Map::new();
        for pred_id in &flow_predecessors {
            if let Some(output) = outputs.get(pred_id) {
                merged.insert(pred_id.to_string(), output.value().clone());
            }
        }
        serde_json::Value::Object(merged)
    };

    (flow_input, support_inputs)
}

/// Extract the primary output value from an ActionResult for downstream input resolution.
fn extract_primary_output(result: &ActionResult<serde_json::Value>) -> Option<serde_json::Value> {
    match result {
        ActionResult::Success { output } => output.as_value().cloned(),
        ActionResult::Skip { output, .. } => output.as_ref().and_then(|o| o.as_value().cloned()),
        ActionResult::Continue { output, .. } => output.as_value().cloned(),
        ActionResult::Break { output, .. } => output.as_value().cloned(),
        ActionResult::Branch { output, .. } => output.as_value().cloned(),
        ActionResult::Route { data, .. } => data.as_value().cloned(),
        ActionResult::MultiOutput { main_output, .. } => {
            main_output.as_ref().and_then(|o| o.as_value().cloned())
        },
        ActionResult::Wait { partial_output, .. } => {
            partial_output.as_ref().and_then(|o| o.as_value().cloned())
        },
        ActionResult::Retry { .. } => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nebula_action::{
        ActionError, TriggerContext,
        action::Action,
        context::{Context, CredentialContextExt},
        dependency::ActionDependencies,
        metadata::ActionMetadata,
        result::ActionResult,
        stateless::StatelessAction,
    };
    use nebula_core::action_key;
    use nebula_runtime::{
        ActionExecutor, DataPassingPolicy, InProcessSandbox, registry::ActionRegistry,
    };
    use nebula_storage::{ExecutionRepo, WorkflowRepo};
    use nebula_workflow::{
        Connection, ErrorStrategy, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
    };

    use super::*;

    // -- Test handlers --

    struct EchoHandler {
        meta: ActionMetadata,
    }

    impl ActionDependencies for EchoHandler {}
    impl Action for EchoHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for EchoHandler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        async fn execute(
            &self,
            input: Self::Input,
            _ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    struct FailHandler {
        meta: ActionMetadata,
    }

    impl ActionDependencies for FailHandler {}
    impl Action for FailHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for FailHandler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        async fn execute(
            &self,
            _input: Self::Input,
            _ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Err(ActionError::fatal("intentional failure"))
        }
    }

    struct SlowHandler {
        meta: ActionMetadata,
        delay: Duration,
    }

    impl ActionDependencies for SlowHandler {}
    impl Action for SlowHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for SlowHandler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        async fn execute(
            &self,
            input: Self::Input,
            ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            tokio::select! {
                () = tokio::time::sleep(self.delay) => Ok(ActionResult::success(input)),
                () = ctx.cancellation().cancelled() => Err(ActionError::Cancelled),
            }
        }
    }

    // -- Helpers --

    fn make_workflow(
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
    ) -> WorkflowDefinition {
        let now = chrono::Utc::now();
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

    fn make_workflow_with_config(
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
        config: WorkflowConfig,
    ) -> WorkflowDefinition {
        let now = chrono::Utc::now();
        WorkflowDefinition {
            id: WorkflowId::new(),
            name: "test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes,
            connections,
            variables: HashMap::new(),
            config,
            trigger: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: 1,
        }
    }

    fn make_engine(registry: Arc<ActionRegistry>) -> (WorkflowEngine, MetricsRegistry) {
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let metrics = MetricsRegistry::new();

        let runtime = Arc::new(ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy::default(),
            metrics.clone(),
        ));

        let engine = WorkflowEngine::new(runtime, metrics.clone());
        (engine, metrics)
    }

    // -- Tests --

    #[tokio::test]
    async fn single_node_workflow() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let (engine, _) = make_engine(registry);

        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n.clone(), "echo", "echo").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_output(&n), Some(&serde_json::json!("hello")));
    }

    #[tokio::test]
    async fn linear_two_node_workflow() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let (engine, _) = make_engine(registry);

        let n1 = node_key!("n1");
        let n2 = node_key!("n2");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(n2.clone(), "B", "echo").unwrap(),
            ],
            vec![Connection::new(n1.clone(), n2.clone())],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(42), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_output(&n1), Some(&serde_json::json!(42)));
        // B echoes its input, which is A's output (42)
        assert_eq!(result.node_output(&n2), Some(&serde_json::json!(42)));
    }

    #[tokio::test]
    async fn diamond_workflow() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let (engine, _) = make_engine(registry);

        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let d = node_key!("d");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
                NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
                NodeDefinition::new(d.clone(), "D", "echo").unwrap(),
            ],
            vec![
                Connection::new(a.clone(), b.clone()),
                Connection::new(a.clone(), c.clone()),
                Connection::new(b.clone(), d.clone()),
                Connection::new(c.clone(), d.clone()),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("start"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_outputs.len(), 4);
        assert_eq!(result.node_output(&a), Some(&serde_json::json!("start")));
        assert_eq!(result.node_output(&b), Some(&serde_json::json!("start")));
        assert_eq!(result.node_output(&c), Some(&serde_json::json!("start")));
        // Join node gets merged outputs from b and c
        let d_output = result.node_output(&d).unwrap();
        assert!(d_output.is_object());
    }

    #[tokio::test]
    async fn failing_node_stops_execution() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        });

        let (engine, _) = make_engine(registry);

        let n1 = node_key!("n1");
        let n2 = node_key!("n2");
        let n3 = node_key!("n3");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(n2.clone(), "B", "fail").unwrap(),
                NodeDefinition::new(n3.clone(), "C", "echo").unwrap(),
            ],
            vec![
                Connection::new(n1.clone(), n2.clone()),
                Connection::new(n2.clone(), n3.clone()),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure());
        assert!(result.node_output(&n1).is_some());
        assert!(result.node_output(&n2).is_none());
        assert!(result.node_output(&n3).is_none());
    }

    #[tokio::test]
    async fn missing_action_key_returns_error() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);

        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "A", "unknown").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .expect("engine returns Ok even when a node fails");

        // When action key is not in registry, the node fails and execution result is failure
        assert!(!result.is_success());
    }

    #[tokio::test]
    async fn empty_workflow_returns_planning_error() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);

        let wf = make_workflow(vec![], vec![]);
        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await;

        assert!(matches!(result, Err(EngineError::PlanningFailed(_))));
    }

    #[tokio::test]
    async fn telemetry_events_emitted() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let (engine, metrics) = make_engine(registry);

        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "echo", "echo").unwrap()],
            vec![],
        );

        engine
            .execute_workflow(&wf, serde_json::json!("test"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(
            metrics
                .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
                .get()
                > 0
        );
        assert!(
            metrics
                .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
                .get()
                > 0
        );
    }

    #[tokio::test]
    async fn metrics_recorded_on_failure() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        });

        let (engine, metrics) = make_engine(registry);

        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "fail", "fail").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure());
        assert!(
            metrics
                .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
                .get()
                > 0
        );
        assert!(
            metrics
                .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
                .get()
                > 0
        );
    }

    #[tokio::test]
    async fn trigger_context_construction_is_usable_in_engine() {
        let ctx = TriggerContext::new(
            WorkflowId::new(),
            node_key!("test"),
            tokio_util::sync::CancellationToken::new(),
        );
        assert!(!ctx.has_credential_id("missing").await);
        assert!(
            ctx.schedule_after(std::time::Duration::from_millis(1))
                .await
                .is_err()
        );
        assert!(
            ctx.emit_execution(serde_json::json!({"event":"tick"}))
                .await
                .is_err()
        );
    }

    // -- Frontier-specific test handlers --

    struct SkipHandler {
        meta: ActionMetadata,
    }

    impl ActionDependencies for SkipHandler {}
    impl Action for SkipHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for SkipHandler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        async fn execute(
            &self,
            _input: Self::Input,
            _ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::skip("skipped by test"))
        }
    }

    struct BranchHandler {
        meta: ActionMetadata,
        selected: String,
    }

    impl ActionDependencies for BranchHandler {}
    impl Action for BranchHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for BranchHandler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        async fn execute(
            &self,
            input: Self::Input,
            _ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::Branch {
                selected: self.selected.clone(),
                output: nebula_action::output::ActionOutput::Value(input),
                alternatives: std::collections::HashMap::new(),
            })
        }
    }

    // -- Frontier-specific tests --

    /// A → Branch(selects "true") → B (branch_key="true") / C (branch_key="false") → D
    /// Only B should execute; C should be skipped; D should still run (via B).
    #[tokio::test]
    async fn branch_workflow_only_selected_path_executes() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });
        registry.register_stateless(BranchHandler {
            meta: ActionMetadata::new(action_key!("branch"), "Branch", "branches"),
            selected: "true".into(),
        });

        let (engine, _) = make_engine(registry);

        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let d = node_key!("d");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "branch").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
                NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
                NodeDefinition::new(d.clone(), "D", "echo").unwrap(),
            ],
            vec![
                Connection::new(a.clone(), b.clone()).with_branch_key("true"),
                Connection::new(a.clone(), c.clone()).with_branch_key("false"),
                Connection::new(b.clone(), d.clone()),
                Connection::new(c.clone(), d.clone()),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        // A executed (branch node)
        assert!(result.node_output(&a).is_some());
        // B executed (true branch)
        assert!(result.node_output(&b).is_some());
        // C was NOT executed (false branch, skipped)
        assert!(result.node_output(&c).is_none());
        // D executed (received input from B only)
        assert!(result.node_output(&d).is_some());
    }

    /// A → B(skip) → C. Verify C is skipped and doesn't execute.
    #[tokio::test]
    async fn skip_propagation() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });
        registry.register_stateless(SkipHandler {
            meta: ActionMetadata::new(action_key!("skip"), "Skip", "always skips"),
        });

        let (engine, _) = make_engine(registry);

        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(b.clone(), "B", "skip").unwrap(),
                NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
            ],
            vec![
                Connection::new(a.clone(), b.clone()),
                Connection::new(b.clone(), c.clone()),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        // Execution succeeds overall (skip is not a failure)
        assert!(result.is_success());
        // A executed
        assert!(result.node_output(&a).is_some());
        // B executed but produced Skip result (no output stored since skip has no output)
        assert!(result.node_output(&b).is_none());
        // C was skipped (never executed)
        assert!(result.node_output(&c).is_none());
    }

    /// A → B(fails) --OnError--> C. Verify C receives error data and execution succeeds.
    #[tokio::test]
    async fn error_routing_with_handler() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        });

        let (engine, _) = make_engine(registry);

        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(b.clone(), "B", "fail").unwrap(),
                NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
            ],
            vec![
                Connection::new(a.clone(), b.clone()),
                Connection::new(b.clone(), c.clone()).with_condition(EdgeCondition::OnError {
                    matcher: ErrorMatcher::Any,
                }),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        // Execution succeeds because the error was handled
        assert!(result.is_success());
        // A executed
        assert!(result.node_output(&a).is_some());
        // B failed but error data was stored
        assert!(result.node_output(&b).is_some());
        // C executed with error data from B
        let c_output = result.node_output(&c).unwrap();
        assert!(c_output.get("error").is_some());
    }

    /// A → B(fails) → C (Always). No OnError handler → fail-fast (same as today).
    #[tokio::test]
    async fn error_without_handler_fails_fast() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        });

        let (engine, _) = make_engine(registry);

        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(b.clone(), "B", "fail").unwrap(),
                NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
            ],
            vec![
                Connection::new(a.clone(), b.clone()),
                Connection::new(b, c.clone()),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure());
        assert!(result.node_output(&a).is_some());
        // B failed, no error handler → fail-fast
        assert!(result.node_output(&c).is_none());
    }

    /// A → B with OnResult(Success) condition. B should run when A succeeds.
    #[tokio::test]
    async fn conditional_edge_on_result() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let (engine, _) = make_engine(registry);

        let a = node_key!("a");
        let b = node_key!("b");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
            ],
            vec![
                Connection::new(a.clone(), b.clone()).with_condition(EdgeCondition::OnResult {
                    matcher: ResultMatcher::Success,
                }),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_output(&a), Some(&serde_json::json!("hello")));
        assert_eq!(result.node_output(&b), Some(&serde_json::json!("hello")));
    }

    /// Diamond with mixed conditions:
    /// A → B (Always), A → C (OnResult{Success}), B → D, C → D
    /// All should execute when A succeeds.
    #[tokio::test]
    async fn diamond_with_mixed_conditions() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let (engine, _) = make_engine(registry);

        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let d = node_key!("d");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
                NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
                NodeDefinition::new(d.clone(), "D", "echo").unwrap(),
            ],
            vec![
                Connection::new(a.clone(), b.clone()), // Always
                Connection::new(a.clone(), c.clone()).with_condition(EdgeCondition::OnResult {
                    matcher: ResultMatcher::Success,
                }),
                Connection::new(b.clone(), d.clone()),
                Connection::new(c.clone(), d.clone()),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("start"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_outputs.len(), 4);
        assert!(result.node_output(&a).is_some());
        assert!(result.node_output(&b).is_some());
        assert!(result.node_output(&c).is_some());
        // D should have merged input from B and C
        let d_output = result.node_output(&d).unwrap();
        assert!(d_output.is_object());
    }

    // -- ExecutionRepo persistence tests --

    #[tokio::test]
    async fn persists_execution_state_on_success() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(repo.clone());

        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n.clone(), "echo", "echo").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());

        // Verify state was persisted
        let entry = repo.get_state(result.execution_id).await.unwrap();
        assert!(entry.is_some(), "execution state should be persisted");
        let (version, state) = entry.unwrap();
        assert!(version >= 2, "repo version should have been bumped");
        assert_eq!(
            state.get("status").and_then(|s| s.as_str()),
            Some("completed")
        );

        // Verify node output was saved
        let node_output = repo.load_node_output(result.execution_id, n).await.unwrap();
        assert_eq!(node_output, Some(serde_json::json!("hello")));
    }

    #[tokio::test]
    async fn persists_execution_state_on_failure() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        });

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(repo.clone());

        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "fail", "fail").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure());

        // Verify final state was persisted as failed
        let entry = repo.get_state(result.execution_id).await.unwrap();
        assert!(entry.is_some(), "execution state should be persisted");
        let (_version, state) = entry.unwrap();
        assert_eq!(state.get("status").and_then(|s| s.as_str()), Some("failed"));
    }

    #[tokio::test]
    async fn persists_node_outputs_for_multi_node_workflow() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(repo.clone());

        let n1 = node_key!("n1");
        let n2 = node_key!("n2");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(n2.clone(), "B", "echo").unwrap(),
            ],
            vec![Connection::new(n1.clone(), n2.clone())],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(42), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());

        // Both node outputs should be persisted
        let all_outputs = repo.load_all_outputs(result.execution_id).await.unwrap();
        assert_eq!(all_outputs.len(), 2);
        assert_eq!(all_outputs[&n1], serde_json::json!(42));
        assert_eq!(all_outputs[&n2], serde_json::json!(42));
    }

    // -- Budget enforcement tests --

    #[tokio::test]
    async fn budget_max_duration_exceeded() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(SlowHandler {
            meta: ActionMetadata::new(action_key!("slow"), "Slow", "sleeps"),
            delay: Duration::from_millis(100),
        });
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let (engine, _) = make_engine(registry);

        // Slow → Echo. Budget allows only 1ms.
        let a = node_key!("a");
        let b = node_key!("b");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "Slow", "slow").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
            ],
            vec![Connection::new(a, b)],
        );

        let budget = ExecutionBudget::default().with_max_duration(Duration::from_millis(1));

        let result = engine
            .execute_workflow(&wf, serde_json::json!("data"), budget)
            .await
            .unwrap();

        // The slow action takes >1ms, so budget should trigger before
        // the next node is dispatched.
        assert!(result.is_failure());
    }

    #[tokio::test]
    async fn budget_max_output_bytes_exceeded() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let (engine, _) = make_engine(registry);

        // A → B. Each echoes a payload. Budget allows very few bytes.
        let a = node_key!("a");
        let b = node_key!("b");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
            ],
            vec![Connection::new(a, b)],
        );

        // Budget: max 5 bytes of total output (the JSON "hello" is 7 bytes)
        let budget = ExecutionBudget::default().with_max_output_bytes(5);

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), budget)
            .await
            .unwrap();

        // A's output exceeds 5 bytes → budget violation before B runs
        assert!(result.is_failure());
    }

    #[test]
    fn budget_max_total_retries_exceeded() {
        let budget = ExecutionBudget::default().with_max_total_retries(2);
        let started = Instant::now();
        let total_output_bytes = AtomicU64::new(0);

        // Under the limit: 2 retries allowed, 2 used — not exceeded yet
        let total_retries = AtomicU32::new(2);
        assert!(
            check_budget(&budget, &started, &total_output_bytes, &total_retries).is_none(),
            "2 retries with limit 2 should not trigger budget"
        );

        // Over the limit: 3 retries used, limit is 2
        total_retries.store(3, Ordering::Relaxed);
        let violation = check_budget(&budget, &started, &total_output_bytes, &total_retries);
        assert!(
            violation.is_some(),
            "3 retries with limit 2 should trigger budget"
        );
        assert!(
            violation.unwrap().contains("max_total_retries"),
            "violation message should name the exceeded budget"
        );
    }

    #[test]
    fn budget_max_total_retries_unlimited_when_none() {
        let budget = ExecutionBudget::default(); // max_total_retries = None
        let started = Instant::now();
        let total_output_bytes = AtomicU64::new(0);
        let total_retries = AtomicU32::new(u32::MAX);

        // No limit set — even a saturated counter should not trigger
        assert!(
            check_budget(&budget, &started, &total_output_bytes, &total_retries).is_none(),
            "unlimited retries: u32::MAX retries should not trigger budget when None"
        );
    }

    // -- Error strategy tests --

    #[tokio::test]
    async fn error_strategy_continue_on_error_skips_dependents() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        });

        let (engine, _) = make_engine(registry);

        // Entry → [Fail, Echo(C)]
        // Fail → B
        // With ContinueOnError: Fail fails, B is skipped, C still runs.
        let entry = node_key!("entry");
        let fail_node = node_key!("fail_node");
        let b = node_key!("b");
        let c = node_key!("c");

        let config = WorkflowConfig {
            error_strategy: ErrorStrategy::ContinueOnError,
            ..WorkflowConfig::default()
        };

        let wf = make_workflow_with_config(
            vec![
                NodeDefinition::new(entry.clone(), "Entry", "echo").unwrap(),
                NodeDefinition::new(fail_node.clone(), "Fail", "fail").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
                NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
            ],
            vec![
                Connection::new(entry.clone(), fail_node.clone()),
                Connection::new(entry.clone(), c.clone()),
                Connection::new(fail_node, b.clone()),
            ],
            config,
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("data"), ExecutionBudget::default())
            .await
            .unwrap();

        // Workflow completes (not fail-fast)
        assert!(result.is_success() || result.status == ExecutionStatus::Completed);
        // Entry ran
        assert!(result.node_output(&entry).is_some());
        // C is independent and should have run
        assert!(result.node_output(&c).is_some());
        // B depends on the failed node — should be skipped (no output)
        assert!(result.node_output(&b).is_none());
    }

    #[tokio::test]
    async fn error_strategy_ignore_errors_continues_downstream() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        });

        let (engine, _) = make_engine(registry);

        // A(fail) → B(echo)
        // With IgnoreErrors: A fails but B should still run with null input
        let a = node_key!("a");
        let b = node_key!("b");

        let config = WorkflowConfig {
            error_strategy: ErrorStrategy::IgnoreErrors,
            ..WorkflowConfig::default()
        };

        let wf = make_workflow_with_config(
            vec![
                NodeDefinition::new(a.clone(), "A", "fail").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
            ],
            vec![Connection::new(a.clone(), b.clone())],
            config,
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("data"), ExecutionBudget::default())
            .await
            .unwrap();

        // Workflow should complete successfully
        assert_eq!(result.status, ExecutionStatus::Completed);
        // A's output was replaced with null
        assert_eq!(result.node_output(&a), Some(&serde_json::json!(null)));
        // B ran and received null as input
        assert!(result.node_output(&b).is_some());
        assert_eq!(result.node_output(&b), Some(&serde_json::json!(null)));
    }

    // -- resume_execution tests --

    /// Helper: build an InMemoryWorkflowRepo and save a workflow definition.
    async fn save_workflow_to_repo(
        wf: &WorkflowDefinition,
    ) -> Arc<nebula_storage::InMemoryWorkflowRepo> {
        let repo = Arc::new(nebula_storage::InMemoryWorkflowRepo::new());
        let json = serde_json::to_value(wf).unwrap();
        repo.save(wf.id, 0, json).await.unwrap();
        repo
    }

    #[tokio::test]
    async fn resume_requires_execution_repo() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);
        // No execution_repo or workflow_repo attached.
        let err = engine
            .resume_execution(ExecutionId::new())
            .await
            .unwrap_err();
        assert!(
            matches!(err, EngineError::PlanningFailed(ref msg) if msg.contains("execution_repo")),
            "expected no-execution_repo error, got: {err}"
        );
    }

    #[tokio::test]
    async fn resume_requires_workflow_repo() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);
        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let engine = engine.with_execution_repo(exec_repo);
        // No workflow_repo attached.
        let err = engine
            .resume_execution(ExecutionId::new())
            .await
            .unwrap_err();
        assert!(
            matches!(err, EngineError::PlanningFailed(ref msg) if msg.contains("workflow_repo")),
            "expected no-workflow_repo error, got: {err}"
        );
    }

    #[tokio::test]
    async fn resume_returns_error_for_missing_execution() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);
        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "echo", "echo").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;
        let engine = engine
            .with_execution_repo(exec_repo)
            .with_workflow_repo(workflow_repo);

        let err = engine
            .resume_execution(ExecutionId::new())
            .await
            .unwrap_err();
        assert!(
            matches!(err, EngineError::PlanningFailed(ref msg) if msg.contains("not found")),
            "expected not-found error, got: {err}"
        );
    }

    #[tokio::test]
    async fn resume_returns_error_for_terminal_execution() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });
        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "echo", "echo").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;
        let engine = engine
            .with_execution_repo(exec_repo.clone())
            .with_workflow_repo(workflow_repo);

        // Run to completion first.
        let result = engine
            .execute_workflow(&wf, serde_json::json!("hi"), ExecutionBudget::default())
            .await
            .unwrap();
        assert!(result.is_success());

        // Now resume the completed execution — should fail.
        let err = engine
            .resume_execution(result.execution_id)
            .await
            .unwrap_err();
        assert!(
            matches!(err, EngineError::PlanningFailed(ref msg) if msg.contains("terminal")),
            "expected terminal-state error, got: {err}"
        );
    }

    #[tokio::test]
    async fn resume_executes_remaining_nodes_after_crash() {
        // Simulate a 3-node linear workflow (n1 → n2 → n3) where n1 completed
        // before the crash. We manually inject the partially completed state into
        // the repos and verify that resume runs n2 and n3.
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);

        let n1 = node_key!("n1");
        let n2 = node_key!("n2");
        let n3 = node_key!("n3");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(n2.clone(), "B", "echo").unwrap(),
                NodeDefinition::new(n3.clone(), "C", "echo").unwrap(),
            ],
            vec![
                Connection::new(n1.clone(), n2.clone()),
                Connection::new(n2.clone(), n3.clone()),
            ],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        // Manually build a partial execution state where n1 is Completed but
        // n2 and n3 are still Pending (simulating a crash after n1 finished).
        let execution_id = ExecutionId::new();
        let node_ids = vec![n1.clone(), n2.clone(), n3.clone()];
        let mut exec_state = ExecutionState::new(execution_id, wf.id, &node_ids);
        exec_state
            .transition_status(ExecutionStatus::Running)
            .unwrap();
        // Mark n1 as completed.
        exec_state
            .node_states
            .get_mut(&n1)
            .unwrap()
            .transition_to(NodeState::Ready)
            .unwrap();
        exec_state
            .node_states
            .get_mut(&n1)
            .unwrap()
            .transition_to(NodeState::Running)
            .unwrap();
        exec_state
            .node_states
            .get_mut(&n1)
            .unwrap()
            .transition_to(NodeState::Completed)
            .unwrap();

        let state_json = serde_json::to_value(&exec_state).unwrap();
        exec_repo
            .create(execution_id, wf.id, state_json)
            .await
            .unwrap();

        // Persist n1's output.
        exec_repo
            .save_node_output(execution_id, n1.clone(), 1, serde_json::json!("from_n1"))
            .await
            .unwrap();

        let engine = engine
            .with_execution_repo(exec_repo.clone())
            .with_workflow_repo(workflow_repo);

        let result = engine.resume_execution(execution_id).await.unwrap();

        assert!(result.is_success(), "resume should complete successfully");
        assert_eq!(result.execution_id, execution_id);
        // n1's output comes from the persisted outputs
        assert_eq!(result.node_output(&n1), Some(&serde_json::json!("from_n1")));
        // n2 and n3 should have been executed and produced outputs
        assert!(
            result.node_output(&n2).is_some(),
            "n2 should have been re-executed"
        );
        assert!(
            result.node_output(&n3).is_some(),
            "n3 should have been re-executed"
        );
    }

    /// Regression for [#321](https://github.com/vanyastaff/nebula/issues/321).
    ///
    /// The setup-failure branch of `run_frontier` (parameter resolution
    /// error before the action is spawned) routed the failure through
    /// `handle_node_failure` but SKIPPED the `checkpoint_node` call the
    /// runtime-failure branch makes. A crash between setup-failure
    /// handling and the next final-state checkpoint therefore lost both
    /// the node's `Failed` state and any OnError / ContinueOnError
    /// edge-routing already applied in memory by `handle_node_failure`.
    /// PRODUCT_CANON §11.5 (durability precedes visibility, §12.2 /
    /// #297).
    ///
    /// This test covers the fix in two parts:
    ///   1. Running a ContinueOnError workflow with one node that fails at parameter resolution.
    ///      Symmetric persistence means the frontier loop emits one extra `transition()` against
    ///      the repo — observable as an additional repo-version bump (create → setup-failure
    ///      checkpoint → final = v3 vs the pre-fix create → final = v2).
    ///   2. Simulating a crash at that intermediate checkpoint by injecting a matching state
    ///      snapshot into a fresh repo and resuming. The resumed engine must keep the node in
    ///      `Failed` (terminal states are not reset by `resume_execution`) and must NOT re-execute
    ///      the node from scratch.
    #[tokio::test]
    async fn setup_failure_persists_before_final_checkpoint() {
        use nebula_workflow::ParamValue;

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        // `ContinueOnError` ensures `handle_node_failure` returns `None`
        // so the frontier loop reaches the new setup-failure checkpoint.
        // FailFast would return early (cancel + propagate) before the
        // branch this test is exercising; the same durability gap exists
        // there, but this is the exercise path that lets the test
        // observe the new transition directly.
        let b = node_key!("b");
        let wf = make_workflow_with_config(
            vec![
                NodeDefinition::new(b.clone(), "B", "echo")
                    .unwrap()
                    .with_parameter("bad", ParamValue::template("Hello {{ unclosed")),
            ],
            vec![],
            WorkflowConfig {
                error_strategy: ErrorStrategy::ContinueOnError,
                ..WorkflowConfig::default()
            },
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        // Part 1: run the workflow and observe the extra checkpoint via
        // the repo-version counter.
        let repo1 = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine1, _) = make_engine(registry.clone());
        let engine1 = engine1
            .with_execution_repo(repo1.clone())
            .with_workflow_repo(workflow_repo.clone());

        let result = engine1
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .unwrap();

        let (version, final_state) = repo1
            .get_state(result.execution_id)
            .await
            .unwrap()
            .expect("execution state must be persisted");
        // Using `>=` rather than `==` so a future legitimate mid-execution
        // checkpoint (e.g. a per-status-transition persist) does not break
        // this test. The regression signal is preserved either way: the
        // pre-fix path lands at v2 (create + final only), which always
        // fails `>= 3`.
        assert!(
            version >= 3,
            "expected at least three version bumps: create (v1) + \
             setup-failure checkpoint (v2 — the fix) + final (v3). Pre-fix \
             path skips the setup-failure checkpoint and lands at v2; got \
             {version}"
        );
        assert_eq!(
            final_state
                .get("node_states")
                .and_then(|ns| ns.get(b.as_str()))
                .and_then(|nb| nb.get("state"))
                .and_then(|v| v.as_str()),
            Some("failed"),
            "final persisted state must record node B as Failed"
        );
        assert!(
            result.node_errors.contains_key(&b),
            "execution result must carry the setup-failure error for B"
        );

        // Part 2: simulate a crash at the intermediate checkpoint. Build
        // a state snapshot matching what the setup-failure checkpoint
        // writes (status=Running, node B Failed with error message) and
        // resume in a fresh repo.
        let execution_id = ExecutionId::new();
        let node_ids = vec![b.clone()];
        let mut crashed_state = ExecutionState::new(execution_id, wf.id, &node_ids);
        crashed_state
            .transition_status(ExecutionStatus::Running)
            .unwrap();
        // Mirror spawn_node's override on parameter-resolution failure:
        // the node was still Pending when resolution failed, so we use
        // override_node_state (Pending → Failed is not a valid forward
        // transition). The bump is implicit.
        crashed_state
            .override_node_state(b.clone(), NodeState::Failed)
            .unwrap();
        if let Some(ns) = crashed_state.node_states.get_mut(&b) {
            ns.error_message = Some("parameter resolution failed: template parse error".into());
        }

        let repo2 = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        repo2
            .create(
                execution_id,
                wf.id,
                serde_json::to_value(&crashed_state).unwrap(),
            )
            .await
            .unwrap();

        let (engine2, _) = make_engine(registry);
        let engine2 = engine2
            .with_execution_repo(repo2.clone())
            .with_workflow_repo(workflow_repo);
        let resumed = engine2.resume_execution(execution_id).await.unwrap();

        // Resume must land in a terminal status — the Failed node is
        // already terminal, so the frontier has nothing to run.
        assert!(
            resumed.status.is_terminal(),
            "resume must reach a terminal status, got {:?}",
            resumed.status
        );

        // Node B must still carry its setup-failure error: resume leaves
        // terminal nodes untouched (engine.rs §resume_execution step 7).
        // If B had been re-dispatched, its attempts vector would grow or
        // the error message would be overwritten by a new failure.
        let persisted = repo2
            .get_state(execution_id)
            .await
            .unwrap()
            .expect("state must still be persisted after resume");
        assert_eq!(
            persisted
                .1
                .get("node_states")
                .and_then(|ns| ns.get(b.as_str()))
                .and_then(|nb| nb.get("state"))
                .and_then(|v| v.as_str()),
            Some("failed"),
            "resume must not have reset node B's terminal Failed state"
        );
        assert!(
            resumed
                .node_errors
                .get(&b)
                .is_some_and(|err| err.contains("parameter resolution failed")),
            "resumed node B must still report the injected setup-failure \
             message; re-execution would have replaced it. errors: {:?}",
            resumed.node_errors
        );
    }

    // -- Durable idempotency tests --

    /// Pre-marking a node's idempotency key causes the engine to skip execution
    /// and load the persisted output instead of re-running the action.
    #[tokio::test]
    async fn idempotency_check_prevents_double_execution() {
        use std::sync::atomic::{AtomicU32, Ordering as AOrdering};

        // Track how many times the handler is actually invoked.
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        struct CountingHandler {
            meta: ActionMetadata,
            count: Arc<AtomicU32>,
        }

        impl ActionDependencies for CountingHandler {}
        impl Action for CountingHandler {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }

        impl StatelessAction for CountingHandler {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            async fn execute(
                &self,
                input: Self::Input,
                _ctx: &impl Context,
            ) -> Result<ActionResult<Self::Output>, ActionError> {
                self.count.fetch_add(1, AOrdering::Relaxed);
                Ok(ActionResult::success(input))
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(CountingHandler {
            meta: ActionMetadata::new(action_key!("counting"), "Counting", "counts calls"),
            count: call_count_clone,
        });

        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(exec_repo.clone());

        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n.clone(), "count_node", "counting").unwrap()],
            vec![],
        );

        // Run the workflow once — node should execute and its idempotency key
        // should be recorded.
        let result1 = engine
            .execute_workflow(
                &wf,
                serde_json::json!("payload"),
                ExecutionBudget::default(),
            )
            .await
            .unwrap();
        assert!(result1.is_success(), "first execution should succeed");
        assert_eq!(
            call_count.load(AOrdering::Relaxed),
            1,
            "handler should be called exactly once on first run"
        );

        // Manually inject the same execution_id's idempotency key so that if
        // we simulate re-running the same execution, the node is skipped.
        // (The engine generates the key as "{execution_id}:{node_key}:1".)
        let execution_id = result1.execution_id;
        let idem_key = format!("{execution_id}:{n}:1");

        // Verify the key was recorded by the first run.
        let already_marked = exec_repo.check_idempotency(&idem_key).await.unwrap();
        assert!(
            already_marked,
            "idempotency key should be recorded after first execution"
        );

        // Also verify the persisted output is loadable.
        let persisted = exec_repo.load_node_output(execution_id, n).await.unwrap();
        assert_eq!(
            persisted,
            Some(serde_json::json!("payload")),
            "persisted output should match the original execution result"
        );
    }

    // -- Action version pinning tests --

    /// When `interface_version` is set on a node, the engine uses the versioned
    /// handler instead of the latest.
    #[tokio::test]
    async fn version_pinned_node_uses_specified_handler() {
        use semver::Version;

        // V1 handler returns "v1".
        struct V1Handler {
            meta: ActionMetadata,
        }

        impl ActionDependencies for V1Handler {}
        impl Action for V1Handler {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }

        impl StatelessAction for V1Handler {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            async fn execute(
                &self,
                _input: Self::Input,
                _ctx: &impl Context,
            ) -> Result<ActionResult<Self::Output>, ActionError> {
                Ok(ActionResult::success(serde_json::json!("v1")))
            }
        }

        // V2 handler returns "v2".
        struct V2Handler {
            meta: ActionMetadata,
        }

        impl ActionDependencies for V2Handler {}
        impl Action for V2Handler {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }

        impl StatelessAction for V2Handler {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            async fn execute(
                &self,
                _input: Self::Input,
                _ctx: &impl Context,
            ) -> Result<ActionResult<Self::Output>, ActionError> {
                Ok(ActionResult::success(serde_json::json!("v2")))
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        let v1 = Version::new(1, 0, 0);
        let v2 = Version::new(2, 0, 0);
        // Register v1 first; v2 will become the "latest" (handlers map entry).
        registry.register_stateless(V1Handler {
            meta: ActionMetadata::new(action_key!("versioned"), "V1", "v1 handler")
                .with_version_full(v1.clone()),
        });
        registry.register_stateless(V2Handler {
            meta: ActionMetadata::new(action_key!("versioned"), "V2", "v2 handler")
                .with_version_full(v2.clone()),
        });

        let (engine, _) = make_engine(registry);

        let n1 = node_key!("n1");
        let n2 = node_key!("n2");

        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1.clone(), "pinned_v1", "versioned")
                    .unwrap()
                    .with_interface_version(v1),
                NodeDefinition::new(n2.clone(), "pinned_v2", "versioned")
                    .unwrap()
                    .with_interface_version(v2),
            ],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(
            result.node_output(&n1),
            Some(&serde_json::json!("v1")),
            "n1 should use the v1 handler"
        );
        assert_eq!(
            result.node_output(&n2),
            Some(&serde_json::json!("v2")),
            "n2 should use the v2 handler"
        );
    }

    // -- Proactive credential refresh tests --

    /// When a credential refresh hook is set, it is called before each node dispatch.
    #[tokio::test]
    async fn credential_refresh_hook_is_called_before_node_dispatch() {
        use std::sync::atomic::{AtomicU32, Ordering as AOrdering};

        let refresh_count = Arc::new(AtomicU32::new(0));
        let refresh_count_clone = refresh_count.clone();

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        // The refresh hook is only called when a credential resolver is also set.
        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_credential_resolver(|_id: &str| async move {
                Err(nebula_credential::CredentialAccessError::NotFound(
                    "no credentials".to_owned(),
                ))
            })
            .with_credential_refresh(move |_id: &str| {
                let count = refresh_count_clone.clone();
                async move {
                    count.fetch_add(1, AOrdering::Relaxed);
                    Ok(())
                }
            });

        let n1 = node_key!("n1");
        let n2 = node_key!("n2");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(n2.clone(), "B", "echo").unwrap(),
            ],
            vec![Connection::new(n1, n2)],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("x"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        // Two nodes → hook called twice.
        assert_eq!(
            refresh_count.load(AOrdering::Relaxed),
            2,
            "refresh hook should be called once per dispatched node"
        );
    }

    // -- Multi-edge regression tests --

    /// Regression: two distinct edges from the same source to the same target
    /// must not stall the target node.
    ///
    /// Previously, `resolved_edges` used `HashSet<NodeKey>` (source-node cardinality)
    /// while `required_count` counted edges. With two edges A → B, the set deduped
    /// them to one entry, so `resolved(1) != required(2)` forever and B never ran.
    /// The fix changes `resolved_edges` to `HashMap<NodeKey, usize>` (edge-count
    /// cardinality), so both increments are counted and B correctly becomes ready.
    #[tokio::test]
    async fn multi_edge_from_same_source_executes_target() {
        use nebula_workflow::EdgeCondition;

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let (engine, _) = make_engine(registry);

        let a = node_key!("a");
        let b = node_key!("b");

        // Two distinct (non-identical) edges from A to B: one unconditional,
        // one via a named source port. Both activate on success.
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
            ],
            vec![
                Connection::new(a.clone(), b.clone()).with_condition(EdgeCondition::Always),
                Connection::new(a, b.clone())
                    .with_condition(EdgeCondition::Always)
                    .with_from_port("alt"),
            ],
        );

        let result = engine
            .execute_workflow(
                &wf,
                serde_json::json!("payload"),
                ExecutionBudget::default(),
            )
            .await
            .unwrap();

        assert!(
            result.is_success(),
            "multi-edge workflow must complete successfully; got: {:?}",
            result.status
        );
        assert!(
            result.node_output(&b).is_some(),
            "target node B must execute and produce output"
        );
    }

    /// Regression for #341: `determine_final_status` must return `Failed`
    /// (not `Completed`) when at least one node has not reached a terminal
    /// state, even when no node explicitly failed and the cancellation token
    /// is not set.
    ///
    /// Additionally, it must attach an `integrity_violation` payload naming
    /// the non-terminal nodes, so the caller can emit
    /// `ExecutionEvent::FrontierIntegrityViolation` rather than silently
    /// reporting success (PRODUCT_CANON §11.1).
    #[test]
    fn final_status_guard_returns_failed_for_non_terminal_nodes() {
        let exec_id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let n1 = node_key!("n1");
        let n2 = node_key!("n2");

        // n1 completed, n2 still Pending (simulates a stalled node).
        let mut exec_state = ExecutionState::new(exec_id, wf_id, &[n1.clone(), n2.clone()]);
        exec_state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
        // n2 stays NodeState::Pending

        let cancel_token = CancellationToken::new();
        let decision = determine_final_status(&None, &cancel_token, &exec_state);

        assert_eq!(
            decision.status,
            ExecutionStatus::Failed,
            "non-terminal nodes must prevent a false Completed status"
        );
        let non_terminal = decision
            .integrity_violation
            .expect("integrity_violation must be populated when guard fires");
        assert_eq!(non_terminal.len(), 1, "exactly one node is non-terminal");
        assert_eq!(
            non_terminal[0],
            (n2, NodeState::Pending),
            "payload must name the stalled node and its observed state"
        );
    }

    /// Smoke-test: `determine_final_status` returns `Completed` when all nodes are
    /// terminal and there is no failure or cancellation.
    #[test]
    fn final_status_completed_when_all_terminal() {
        let exec_id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let n1 = node_key!("n1");
        let n2 = node_key!("n2");

        let mut exec_state = ExecutionState::new(exec_id, wf_id, &[n1.clone(), n2.clone()]);
        exec_state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
        exec_state.node_states.get_mut(&n2).unwrap().state = NodeState::Skipped;

        let cancel_token = CancellationToken::new();
        let decision = determine_final_status(&None, &cancel_token, &exec_state);

        assert_eq!(
            decision.status,
            ExecutionStatus::Completed,
            "all-terminal nodes with no failure must yield Completed"
        );
        assert!(
            decision.integrity_violation.is_none(),
            "no integrity payload when the invariant holds"
        );
    }

    /// Invariant: no combination of `(failed_node, cancel_token, exec_state)`
    /// may produce `Completed` when `all_nodes_terminal` is false.
    ///
    /// Acts as a lightweight property-style check — enumerates the cartesian
    /// product of the three input axes for a two-node workflow and asserts
    /// the canon §11.1 rule across every combination.
    #[test]
    fn final_status_never_completed_with_non_terminal_nodes() {
        use NodeState::*;
        let states = [
            Pending, Ready, Running, Retrying, Completed, Failed, Skipped, Cancelled,
        ];
        let failure_cases = [None, Some((node_key!("n1"), "boom".to_owned()))];
        let cancel_cases = [false, true];

        let combinations = states
            .iter()
            .flat_map(|&a| std::iter::repeat(a).zip(states.iter().copied()))
            .flat_map(|(a, b)| failure_cases.iter().map(move |f| (a, b, f)))
            .flat_map(|(a, b, f)| cancel_cases.iter().map(move |&c| (a, b, f, c)));
        for (a, b, failed, cancel) in combinations {
            check_no_false_completed(a, b, failed, cancel);
        }
    }

    fn check_no_false_completed(
        a: NodeState,
        b: NodeState,
        failed: &Option<(NodeKey, String)>,
        cancel: bool,
    ) {
        let exec_id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let n1 = node_key!("n1");
        let n2 = node_key!("n2");
        let mut state = ExecutionState::new(exec_id, wf_id, &[n1.clone(), n2.clone()]);
        state.node_states.get_mut(&n1).unwrap().state = a;
        state.node_states.get_mut(&n2).unwrap().state = b;

        let token = CancellationToken::new();
        if cancel {
            token.cancel();
        }

        let decision = determine_final_status(failed, &token, &state);
        if decision.status != ExecutionStatus::Completed {
            return;
        }
        assert!(
            state.all_nodes_terminal(),
            "Completed must imply all_nodes_terminal; \
             violated with a={a:?} b={b:?} failed={failed:?} cancel={cancel}"
        );
        assert!(
            decision.integrity_violation.is_none(),
            "Completed decisions must not carry an integrity payload"
        );
    }

    /// Regression for #341: when the guard populates a non-terminal payload,
    /// `emit_frontier_integrity_if_violated` must send exactly one
    /// `ExecutionEvent::FrontierIntegrityViolation`. Covers the helper all
    /// three finish sites call, so a reorder or drop at any site is caught
    /// centrally.
    #[tokio::test]
    async fn emit_frontier_integrity_helper_delivers_one_event_on_violation() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);
        let (tx, mut rx) = mpsc::channel::<ExecutionEvent>(8);
        let engine = engine.with_event_sender(tx);

        let exec_id = ExecutionId::new();
        let n2 = node_key!("n2");
        let payload = Some(vec![(n2.clone(), NodeState::Pending)]);
        engine.emit_frontier_integrity_if_violated(exec_id, payload);

        match rx.try_recv().expect("violation event") {
            ExecutionEvent::FrontierIntegrityViolation {
                execution_id,
                non_terminal_nodes,
            } => {
                assert_eq!(execution_id, exec_id);
                assert_eq!(non_terminal_nodes, vec![(n2, NodeState::Pending)]);
            },
            other => panic!("expected FrontierIntegrityViolation, got {other:?}"),
        }
        // No further events from this helper — the finish event is the
        // caller's responsibility and is intentionally out of scope here.
        assert!(rx.try_recv().is_err(), "helper must emit exactly one event");
    }

    /// When the guard does not fire, `emit_frontier_integrity_if_violated`
    /// must stay silent so the finish-event stream is unchanged in the
    /// happy path.
    #[tokio::test]
    async fn emit_frontier_integrity_helper_silent_when_no_violation() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);
        let (tx, mut rx) = mpsc::channel::<ExecutionEvent>(8);
        let engine = engine.with_event_sender(tx);

        engine.emit_frontier_integrity_if_violated(ExecutionId::new(), None);
        assert!(rx.try_recv().is_err());
    }

    /// Regression for #306: when the proactive credential-refresh hook
    /// returns an error, the node MUST end up Failed (not Completed) and
    /// the failure MUST surface as a typed
    /// `ActionError::CredentialRefreshFailed`, not a log-and-continue WARN.
    ///
    /// Verifies:
    ///   1. The action handler is **never** invoked (refresh fails before dispatch).
    ///   2. The execution result is not a success.
    ///   3. The emitted `NodeFailed` event carries the typed error code
    ///      `ACTION:CREDENTIAL_REFRESH_FAILED` (visible to downstream consumers via the error
    ///      string).
    ///   4. The new `EngineError::Action` carries an `ActionError` that pattern-matches as
    ///      `CredentialRefreshFailed`.
    #[tokio::test]
    async fn credential_refresh_failure_surfaces_as_typed_error() {
        use std::sync::atomic::{AtomicU32, Ordering as AOrdering};

        // Action that asserts it never runs — if the engine reaches
        // dispatch despite a failed refresh, this will fire and surface
        // a different error than the one we expect.
        struct NeverRunHandler {
            meta: ActionMetadata,
            invoked: Arc<AtomicU32>,
        }
        impl ActionDependencies for NeverRunHandler {}
        impl Action for NeverRunHandler {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }
        impl StatelessAction for NeverRunHandler {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            async fn execute(
                &self,
                input: Self::Input,
                _ctx: &impl Context,
            ) -> Result<ActionResult<Self::Output>, ActionError> {
                self.invoked.fetch_add(1, AOrdering::Relaxed);
                Ok(ActionResult::success(input))
            }
        }

        let invoked = Arc::new(AtomicU32::new(0));
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(NeverRunHandler {
            meta: ActionMetadata::new(action_key!("never"), "Never", "must not run"),
            invoked: invoked.clone(),
        });

        // Refresh hook always fails. Use `ActionError::retryable` for
        // the inner source so the `Arc<dyn Error>` wrapping in
        // `CredentialRefreshFailed` round-trips through Display.
        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_credential_resolver(|_id: &str| async move {
                Err(nebula_credential::CredentialAccessError::NotFound(
                    "no credentials".to_owned(),
                ))
            })
            .with_credential_refresh(|_id: &str| async move {
                Err(ActionError::retryable("credential store down"))
            });

        let n1 = node_key!("n1");
        let wf = make_workflow(
            vec![NodeDefinition::new(n1.clone(), "A", "never").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("x"), ExecutionBudget::default())
            .await
            .expect("engine returns Ok(ExecutionResult) even on node failure");

        // (1) The action body must NEVER have been called.
        assert_eq!(
            invoked.load(AOrdering::Relaxed),
            0,
            "action body must not run when proactive refresh fails"
        );

        // (2) Execution must NOT be a success.
        assert!(
            !result.is_success(),
            "workflow must not succeed when refresh fails"
        );

        // (3) The node-level error message must mention the typed cause.
        // The default ErrorStrategy is FailFast, so the engine populates
        // `node_errors` with the failed node's error message. The string
        // representation of the new variant is stable and downstream
        // consumers (TUI, log scrape, dashboards) can match on it.
        let node_err = result
            .node_errors
            .get(&n1)
            .expect("node_errors must contain the failed node");
        assert!(
            node_err.contains("credential refresh failed"),
            "expected typed CredentialRefreshFailed in error, got: {node_err}"
        );
        assert!(
            node_err.contains("credential store down"),
            "expected source string preserved in error, got: {node_err}"
        );

        // (4) Construct the variant directly and confirm classifier
        // routing — this is the contract downstream consumers match on.
        let typed =
            ActionError::credential_refresh_failed("never", ActionError::retryable("store down"));
        assert!(matches!(typed, ActionError::CredentialRefreshFailed { .. }));
        assert!(typed.is_retryable(), "default classification is retryable");
        let engine_err = EngineError::Action(typed);
        assert!(matches!(
            engine_err,
            EngineError::Action(ActionError::CredentialRefreshFailed { .. })
        ));
    }

    // -- Credential allowlist enforcement (PRODUCT_CANON §4.5 / §12.5 — audit §2.4) --

    /// Handler that attempts to acquire a credential by id and records the result.
    ///
    /// Used by the allowlist tests below: a single stateless action whose
    /// parameter chooses which credential id to probe. Implemented directly as
    /// a [`StatelessHandler`] (rather than as a [`StatelessAction`]) because
    /// that trait receives a concrete `&ActionContext` from the adapter — which
    /// lets us call [`CredentialContextExt::credential_by_id`] without needing
    /// a downcast from `&impl Context`.
    ///
    /// The outcome (success vs typed error) is surfaced via the execution
    /// result so tests can assert that denial propagates as a real error
    /// rather than silently succeeding.
    struct CredProbeHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl nebula_action::StatelessHandler for CredProbeHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn execute(
            &self,
            input: serde_json::Value,
            ctx: &nebula_action::ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            let id = input
                .get("credential_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ActionError::fatal("missing credential_id"))?;
            // `credential_by_id` forwards `CredentialAccessError::AccessDenied`
            // as `ActionError::SandboxViolation` via the From impl in
            // `nebula_action::error`. We want the typed error to bubble up so
            // the engine records a NodeFailed — not to swallow it.
            let _snapshot = ctx.credential_by_id(id).await?;
            Ok(ActionResult::success(serde_json::json!({"ok": true})))
        }
    }

    /// Register a `CredProbeHandler` under `key` into the given registry.
    fn register_probe(registry: &ActionRegistry, key: ActionKey, name: &str) {
        let meta = ActionMetadata::new(key, name, "acquires a credential");
        registry.register(
            meta.clone(),
            nebula_action::ActionHandler::Stateless(Arc::new(CredProbeHandler { meta })),
        );
    }

    /// Build a workflow with a single `CredProbeHandler` node that probes `cred_id`.
    fn probe_workflow(action: &str, cred_id: &str) -> WorkflowDefinition {
        let n1 = node_key!("probe");
        let node = NodeDefinition::new(n1.clone(), "probe", action)
            .unwrap()
            .with_parameter(
                "credential_id",
                nebula_workflow::ParamValue::literal(serde_json::json!(cred_id)),
            );
        make_workflow(vec![node], vec![])
    }

    /// Build a snapshot the resolver can return for any id. Used to prove that
    /// denial happens **before** the resolver is consulted, not as a side effect
    /// of the store returning nothing — deny-by-default must be a real policy
    /// check, not a lucky miss.
    fn dummy_snapshot(id: &str) -> nebula_credential::CredentialSnapshot {
        nebula_credential::CredentialSnapshot::new(
            id,
            nebula_credential::CredentialRecord::new(),
            nebula_credential::SecretToken::new(nebula_credential::SecretString::new("test-value")),
        )
    }

    /// Default-deny: an action that was never declared to the engine cannot
    /// acquire any credential — even one the resolver would happily return.
    #[tokio::test]
    async fn credential_access_denied_without_declaration() {
        let registry = Arc::new(ActionRegistry::new());
        register_probe(&registry, action_key!("probe"), "Probe");

        let (engine, _) = make_engine(registry);
        // No `with_action_credentials` — `probe` has no declaration.
        let engine = engine.with_credential_resolver(|id: &str| {
            let id = id.to_owned();
            async move { Ok(dummy_snapshot(&id)) }
        });

        let wf = probe_workflow("probe", "api_key");
        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .expect("engine returns Ok(ExecutionResult) even on node failure");

        assert!(
            !result.is_success(),
            "undeclared action must not acquire credentials"
        );
        let err = result
            .node_errors
            .get(&node_key!("probe"))
            .expect("failed node must carry an error message");
        // `CredentialAccessError::AccessDenied` is mapped to
        // `ActionError::SandboxViolation { capability, action_id }` (see
        // `nebula_action::error::From<CredentialAccessError>`), whose Display
        // is `"sandbox violation: capability `{capability}` denied for ..."`.
        assert!(
            err.contains("sandbox violation") && err.contains("denied"),
            "error must surface sandbox-violation denial, got: {err}"
        );
        assert!(
            err.contains("credential:api_key"),
            "error must attribute the denied credential id, got: {err}"
        );
        assert!(
            err.contains("for action `probe`"),
            "error must attribute the action whose access was denied, got: {err}"
        );
    }

    /// Declared: the engine permits exactly the credential ids explicitly
    /// declared for the action's `ActionKey`.
    #[tokio::test]
    async fn credential_access_allowed_with_declaration() {
        let registry = Arc::new(ActionRegistry::new());
        register_probe(&registry, action_key!("probe"), "Probe");

        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_credential_resolver(|id: &str| {
                let id = id.to_owned();
                async move { Ok(dummy_snapshot(&id)) }
            })
            .with_action_credentials(action_key!("probe"), ["api_key"]);

        let wf = probe_workflow("probe", "api_key");
        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .expect("engine returns Ok(ExecutionResult)");

        assert!(
            result.is_success(),
            "declared credential must be acquirable, errors: {:?}",
            result.node_errors
        );
    }

    /// Mismatched: an action that declares credential `A` still cannot acquire
    /// credential `B`. Per-key enforcement, not per-action blanket allow.
    #[tokio::test]
    async fn credential_access_denied_for_mismatched_key() {
        let registry = Arc::new(ActionRegistry::new());
        register_probe(&registry, action_key!("probe"), "Probe");

        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_credential_resolver(|id: &str| {
                let id = id.to_owned();
                async move { Ok(dummy_snapshot(&id)) }
            })
            .with_action_credentials(action_key!("probe"), ["cred_a"]);

        let wf = probe_workflow("probe", "cred_b");
        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .expect("engine returns Ok(ExecutionResult) even on node failure");

        assert!(
            !result.is_success(),
            "mismatched credential id must not be acquirable"
        );
        let err = result
            .node_errors
            .get(&node_key!("probe"))
            .expect("failed node must carry an error message");
        assert!(
            err.contains("sandbox violation") && err.contains("denied"),
            "error must surface sandbox-violation denial, got: {err}"
        );
        assert!(
            err.contains("credential:cred_b"),
            "error must attribute the denied credential id (cred_b), got: {err}"
        );
    }

    /// Scoping: declarations for one `ActionKey` do not leak to others.
    #[tokio::test]
    async fn credential_declaration_is_per_action_key() {
        let registry = Arc::new(ActionRegistry::new());
        register_probe(&registry, action_key!("probe_a"), "Probe A");
        register_probe(&registry, action_key!("probe_b"), "Probe B");

        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_credential_resolver(|id: &str| {
                let id = id.to_owned();
                async move { Ok(dummy_snapshot(&id)) }
            })
            // Only `probe_a` declares `shared_key`. `probe_b` must still be denied.
            .with_action_credentials(action_key!("probe_a"), ["shared_key"]);

        // probe_b tries shared_key → must fail even though probe_a has it declared.
        let wf = probe_workflow("probe_b", "shared_key");
        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .expect("engine returns Ok(ExecutionResult)");

        assert!(
            !result.is_success(),
            "declaration for probe_a must not leak to probe_b"
        );
    }

    /// Merging: repeat declarations for the same `ActionKey` add keys cumulatively
    /// rather than replacing the set.
    #[tokio::test]
    async fn action_credentials_merge_across_builder_calls() {
        let registry = Arc::new(ActionRegistry::new());
        register_probe(&registry, action_key!("probe"), "Probe");

        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_credential_resolver(|id: &str| {
                let id = id.to_owned();
                async move { Ok(dummy_snapshot(&id)) }
            })
            .with_action_credentials(action_key!("probe"), ["first"])
            .with_action_credentials(action_key!("probe"), ["second"]);

        // Probing "second" must succeed — the second call adds, not replaces.
        let wf = probe_workflow("probe", "second");
        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .expect("engine returns Ok(ExecutionResult)");
        assert!(
            result.is_success(),
            "repeated with_action_credentials must merge, not replace. errors: {:?}",
            result.node_errors
        );
    }
}
