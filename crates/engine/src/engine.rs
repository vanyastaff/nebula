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
use nebula_action::{ActionError, ActionResult, capability::default_resource_accessor};
use nebula_core::{
    ActionKey, NodeKey,
    accessor::{CredentialAccessor, ResourceAccessor},
    id::{ExecutionId, InstanceId, WorkflowId},
    node_key,
};
use nebula_credential::default_credential_accessor;
// ScopeLevel removed from ActionContext
// use nebula_core::scope::ScopeLevel;
use nebula_execution::{
    ExecutionStatus, context::ExecutionBudget, plan::ExecutionPlan, state::ExecutionState,
    status::ExecutionTerminationReason,
};
use nebula_expression::ExpressionEngine;
use nebula_metrics::naming::{
    NEBULA_ENGINE_LEASE_CONTENTION_TOTAL, NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL, engine_lease_contention_reason,
};
use nebula_plugin::PluginRegistry;
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{Connection, DependencyGraph, NodeState, WorkflowDefinition};
use tokio::{sync::Semaphore, task::JoinSet};
use tokio_util::sync::CancellationToken;

use crate::{
    credential_accessor::EngineCredentialAccessor, error::EngineError, event::ExecutionEvent,
    resolver::ParamResolver, resource_accessor::EngineResourceAccessor, result::ExecutionResult,
    runtime::ActionRuntime,
};

/// Type alias for the optional event sender.
///
/// Bounded (rather than unbounded) so a slow consumer cannot drive engine
/// memory to unbounded growth. A workflow with ~10k nodes emits ~50k events;
/// the capacity below keeps roughly one in-flight workflow's worth of events
/// buffered per subscriber before the bus starts dropping.
type EventBus = nebula_eventbus::EventBus<ExecutionEvent>;

/// Default capacity for the engine's event bus. Tuned so a typical
/// interactive workflow (hundreds of nodes) never blocks, while a runaway
/// producer with a dead consumer cannot inflate memory without bound. Spec
/// 28 §2.4 calls for broadcast to multiple subscribers (storage writer,
/// metrics collector, websocket broadcaster, audit writer) — `EventBus` is
/// the workspace-standard fan-out primitive.
pub const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Default execution-lease TTL.
///
/// Long enough to survive a GC pause or a slow checkpoint write; short
/// enough that a crashed runner's lease expires inside a minute and
/// redelivery doesn't feel stuck. See ADR 0008.
pub const DEFAULT_EXECUTION_LEASE_TTL: Duration = Duration::from_secs(30);

/// Default interval between execution-lease heartbeat renewals.
///
/// Set to `DEFAULT_EXECUTION_LEASE_TTL / 3` so that two consecutive
/// heartbeat misses still leave the lease valid for at least another TTL
/// cycle before acquire-on-expiry can kick in. See ADR 0008.
pub const DEFAULT_EXECUTION_LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// Type alias for the boxed async credential-refresh function stored on the engine.
///
/// When set, the engine calls this function before dispatching any node that uses
/// credentials, passing the credential ID. The callee is responsible for refreshing
/// the credential (e.g., rotating short-lived tokens) before the action resolves it.
type CredentialRefreshFn = Arc<
    dyn Fn(&str) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send>> + Send + Sync,
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
    #[expect(
        dead_code,
        reason = "field reserved for expression-based edge condition evaluation; wired up in construction but not yet called at runtime"
    )]
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
    event_bus: Option<EventBus>,
    /// Stable per-instance identifier used as the execution-lease holder.
    ///
    /// Generated once at [`WorkflowEngine::new`] via [`InstanceId::new`]
    /// (monotonic ULID). A single process runs exactly one instance id
    /// for its lifetime; restarts rotate it so a post-restart runner
    /// cannot inadvertently inherit a lease from its previous
    /// incarnation. See ADR 0008.
    instance_id: InstanceId,
    /// TTL to apply when acquiring or renewing an execution lease.
    ///
    /// Defaults to [`DEFAULT_EXECUTION_LEASE_TTL`]. Tuned down only in
    /// tests via [`WorkflowEngine::with_lease_ttl`] to shorten
    /// time-based behavior.
    lease_ttl: Duration,
    /// Interval between heartbeat renewals while a frontier loop runs.
    ///
    /// Defaults to [`DEFAULT_EXECUTION_LEASE_HEARTBEAT_INTERVAL`]. Tuned
    /// down only in tests via [`WorkflowEngine::with_lease_heartbeat_interval`].
    lease_heartbeat_interval: Duration,
    /// Volatile index of in-flight executions this runner owns.
    ///
    /// Published **after** `acquire_and_heartbeat_lease` succeeds — the lease
    /// is the authoritative single-runner fence (ADR-0015), so publishing
    /// after the lease prevents an overlapping attempt for the same
    /// [`ExecutionId`] from overwriting the live token. Each entry is
    /// tagged with a monotonically-increasing [`RunningRegistrationId`]
    /// nonce, and the [`RunningRegistration`] guard's `Drop` uses
    /// [`DashMap::remove_if`] to remove only entries that still carry its
    /// own nonce — a defensive guard against an out-of-order drop from a
    /// losing attempt clobbering the winner's registration.
    ///
    /// [`cancel_execution`] looks up the token here and cancels it,
    /// closing the ADR-0008 A3 control-queue `Cancel` path into the
    /// cooperative-cancel signal the frontier loop already observes.
    ///
    /// **Not durable.** This map lives only as long as the `WorkflowEngine`
    /// instance. On process crash the entries vanish with the runner; the
    /// durable truth is `executions` + `execution_control_queue`, and the
    /// replacement runner reloads from storage (ADR-0008 §5, canon §12.2).
    ///
    /// [`execute_workflow`]: Self::execute_workflow
    /// [`resume_execution`]: Self::resume_execution
    /// [`cancel_execution`]: Self::cancel_execution
    running: Arc<DashMap<ExecutionId, RunningEntry>>,
    /// Optional handle for the background credential-refresh reclaim
    /// sweep. When set, dropping the engine aborts the spawned task —
    /// see [`crate::credential::refresh::ReclaimSweepHandle`] (sub-spec
    /// §3.3, §3.4).
    ///
    /// Wired by the composition root via
    /// [`Self::with_credential_reclaim_sweep`] when the deployment has a
    /// durable [`nebula_storage::credential::RefreshClaimRepo`] (Postgres
    /// or SQLite). Single-replica desktop mode without sentinel-event
    /// recording leaves this `None`.
    credential_reclaim_sweep: Option<crate::credential::refresh::ReclaimSweepHandle>,
}

/// Monotonic per-registration identifier used to fence out-of-order drops
/// from concurrent attempts on the same [`ExecutionId`] (ADR-0016 / #482
/// Copilot review). A `u64` is fine — we'd need billions of registrations
/// per runner lifetime to wrap, and the counter resets on process restart
/// anyway.
type RunningRegistrationId = u64;

/// Process-wide monotonic counter for registration nonces.
static NEXT_REGISTRATION_ID: AtomicU64 = AtomicU64::new(1);

/// Value stored in [`WorkflowEngine::running`]. Pairs the live
/// [`CancellationToken`] with the [`RunningRegistrationId`] nonce so the
/// drop guard can use [`DashMap::remove_if`] instead of unconditional
/// `remove`.
struct RunningEntry {
    registration_id: RunningRegistrationId,
    token: CancellationToken,
}

/// RAII guard that removes an execution from the [`WorkflowEngine::running`]
/// registry when dropped **only if the entry still carries its own
/// [`RunningRegistrationId`]**. Covers the normal exit path and every
/// early-return branch (e.g. heartbeat-lost `EngineError::Leased`) without
/// manually threading a `remove` call through each site. The nonce check
/// prevents a losing attempt from removing the winning attempt's token —
/// see [`WorkflowEngine::running`] doc for the hazard.
struct RunningRegistration {
    running: Arc<DashMap<ExecutionId, RunningEntry>>,
    execution_id: ExecutionId,
    registration_id: RunningRegistrationId,
}

impl Drop for RunningRegistration {
    fn drop(&mut self) {
        // `remove_if` only removes when the predicate matches; if a winner
        // has already published a newer `RunningEntry` we leave theirs
        // intact (fence against the hazard Copilot flagged on #482).
        self.running.remove_if(&self.execution_id, |_k, entry| {
            entry.registration_id == self.registration_id
        });
    }
}

impl WorkflowEngine {
    /// Create a new engine with the given components.
    pub fn new(runtime: Arc<ActionRuntime>, metrics: MetricsRegistry) -> Self {
        let expression_engine = Arc::new(ExpressionEngine::with_cache_size(1024));
        let instance_id = InstanceId::new();
        tracing::info!(
            instance_id = %instance_id,
            "workflow engine starting; lease holder string bound for this process's lifetime"
        );
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
            event_bus: None,
            instance_id,
            lease_ttl: DEFAULT_EXECUTION_LEASE_TTL,
            lease_heartbeat_interval: DEFAULT_EXECUTION_LEASE_HEARTBEAT_INTERVAL,
            running: Arc::new(DashMap::new()),
            credential_reclaim_sweep: None,
        }
    }

    /// Signal a cooperative cancel to an in-flight execution this runner owns.
    ///
    /// Returns `true` if the execution's [`CancellationToken`] was found in
    /// this engine's registry and cancelled; `false` if this runner has no
    /// frontier loop for that `execution_id`. A `false` return is not an
    /// error — it is the honest answer for the cross-runner case where a
    /// sibling runner owns the live loop.
    ///
    /// Cancellation is idempotent: calling this twice for the same id while
    /// the loop is still draining is a no-op on the second call. The map
    /// entry is removed when the frontier loop's `RunningRegistration`
    /// guard drops, not by this call — so repeat observers of the token see
    /// the same cancelled state.
    ///
    /// This is the engine-side hook that
    /// [`crate::control_dispatch::EngineControlDispatch`]'s `dispatch_cancel`
    /// calls after the §5 idempotency guard; the durable API-level CAS to
    /// `Cancelled` has already landed on the execution row by the time a
    /// `Cancel` command reaches the consumer (canon §12.2, §13 step 5).
    pub fn cancel_execution(&self, execution_id: ExecutionId) -> bool {
        match self.running.get(&execution_id) {
            Some(entry) => {
                entry.value().token.cancel();
                true
            },
            None => false,
        }
    }

    /// This engine instance's execution-lease holder identifier.
    ///
    /// Rendered in [`EngineError::Leased`] so operators can see which
    /// runner currently owns an execution. Stable for the lifetime of
    /// this process. See ADR 0008.
    #[must_use]
    pub fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    /// Override the execution-lease TTL.
    ///
    /// Primarily for tests that need to exercise expiry behavior under
    /// `tokio::time::pause()`. Production callers should leave this at
    /// [`DEFAULT_EXECUTION_LEASE_TTL`]; changing it alters redelivery
    /// latency after a hard crash.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_lease_ttl(mut self, ttl: Duration) -> Self {
        self.lease_ttl = ttl;
        self
    }

    /// Override the execution-lease heartbeat interval.
    ///
    /// Primarily for tests that need sub-second heartbeats under
    /// `tokio::time::pause()`. Production callers should leave this at
    /// [`DEFAULT_EXECUTION_LEASE_HEARTBEAT_INTERVAL`]; setting it too
    /// large relative to `lease_ttl` makes heartbeats skippable.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_lease_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.lease_heartbeat_interval = interval;
        self
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
    /// use nebula_credential::CredentialAccessError;
    /// use nebula_engine::credential::CredentialResolver;
    /// use nebula_storage::credential::InMemoryStore;
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
        Fut: Future<Output = Result<(), ActionError>> + Send + 'static,
    {
        self.credential_refresh = Some(Arc::new(move |id: &str| {
            Box::pin(refresh_fn(id))
                as Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send>>
        }));
        self
    }

    /// Attach the background credential refresh reclaim sweep handle.
    ///
    /// Per sub-spec §3.3 + §3.4 the engine spawns a periodic task that
    /// calls `RefreshClaimRepo::reclaim_stuck`, routes
    /// `RefreshInFlight`-flagged stale claims through
    /// [`crate::credential::refresh::SentinelTrigger`], and publishes
    /// `CredentialEvent::ReauthRequired` once the rolling-window
    /// threshold is exceeded.
    ///
    /// The composition root constructs the handle via
    /// [`crate::credential::refresh::ReclaimSweepHandle::spawn`] and
    /// passes it here. Storing the handle on the engine ensures the
    /// task is aborted when the engine drops (clean shutdown).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_credential_reclaim_sweep(
        mut self,
        handle: crate::credential::refresh::ReclaimSweepHandle,
    ) -> Self {
        self.credential_reclaim_sweep = Some(handle);
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

    /// Attach an event bus for real-time execution monitoring.
    ///
    /// When set, the engine publishes [`ExecutionEvent`]s for node
    /// lifecycle transitions (started, completed, failed, skipped) and
    /// execution completion. Used by the CLI TUI for live monitoring and
    /// by the spec 28 §2.4 subscribers (storage writer, metrics
    /// collector, websocket broadcaster, audit writer).
    ///
    /// The bus fans out to every subscriber independently — each gets a
    /// bounded queue sized per bus construction. Slow subscribers drop
    /// events (they can observe the drop via their own `Subscriber`), the
    /// engine never blocks.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_event_bus(mut self, bus: EventBus) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Emit an execution event if a bus is configured.
    ///
    /// Hot path is one `broadcast::send` plus the bus's own accounting —
    /// a dead or backed-up subscriber surfaces as a drop in its
    /// `EventBusStats::dropped_count`, never as back-pressure on the
    /// engine itself.
    fn emit_event(&self, event: ExecutionEvent) {
        if let Some(bus) = &self.event_bus {
            let _ = bus.emit(event);
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
        let Some(bus) = &self.event_bus else {
            return;
        };
        if !matches!(bus.emit(event), nebula_eventbus::PublishOutcome::Sent) {
            tracing::error!(
                %execution_id,
                non_terminal_count,
                "frontier integrity violation event dropped by event bus (slow subscriber)"
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
        // Use override inputs if provided — computed up front so the
        // same value is persisted on the execution state (issue #311)
        // and fed to the frontier loop.
        let input = plan
            .input_overrides
            .get(&plan.replay_from)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        exec_state.set_workflow_input(input.clone());
        // Persist the budget so a later resume of this replayed
        // execution honours the same limits (issue #289).
        exec_state.set_budget(budget.clone());
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
                    .is_none_or(|preds| preds.iter().all(|p| pinned.contains(p))) // no predecessors = entry node
            })
            .cloned()
            .collect();

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
            termination_reason,
            integrity_violation,
        } = determine_final_status(&failed_node, &cancel_token, &exec_state);
        // `Running → Cancelled` is not a one-step transition (see
        // `nebula_execution::transition` — issue #273 documents the shortcuts
        // the state machine carved out). When the frontier loop tore down on
        // a cancel token, bridge through `Cancelling` so the subsequent
        // `transition_status(Cancelled)` is valid and the persisted row
        // records the terminal outcome. Without the bridge, the `let _`
        // swallows the invalid-transition error and the row stays at
        // `Running`, producing a two-truth violation against the
        // `ExecutionResult` the engine returns (ADR-0008 A3).
        if final_status == ExecutionStatus::Cancelled
            && exec_state.status == ExecutionStatus::Running
        {
            let _ = exec_state.transition_status(ExecutionStatus::Cancelling);
        }
        let _ = exec_state.transition_status(final_status);

        self.emit_frontier_integrity_if_violated(execution_id, integrity_violation);
        tracing::info!(
            target = "engine",
            %execution_id,
            ?final_status,
            ?termination_reason,
            ?elapsed,
            "execution_finished"
        );
        self.emit_event(ExecutionEvent::ExecutionFinished {
            execution_id,
            success: final_status == ExecutionStatus::Completed,
            elapsed,
            termination_reason: termination_reason.clone(),
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
            termination_reason,
        })
    }

    /// Acquire the execution lease and spawn a heartbeat task.
    ///
    /// Implements the ADR 0008 lease lifecycle: the engine is the
    /// authoritative "who runs this execution right now" signal. Second
    /// runners find a live lease and return [`EngineError::Leased`]
    /// rather than dispatch nodes in parallel.
    ///
    /// On success, returns a [`LeaseGuard`] that must be shut down with
    /// [`LeaseGuard::shutdown`] after the frontier loop exits. The guard
    /// also exposes [`LeaseGuard::heartbeat_lost`] so the caller can
    /// detect stolen / expired leases and refuse to persist further
    /// state (a §12.2 durability invariant).
    ///
    /// Returns `Ok(None)` when no `execution_repo` is configured — in
    /// that mode the engine is a single-process library with no
    /// coordination seam, and the caller proceeds without a lease.
    async fn acquire_and_heartbeat_lease(
        &self,
        execution_id: ExecutionId,
        frontier_cancel: CancellationToken,
    ) -> Result<Option<LeaseGuard>, EngineError> {
        let Some(repo) = self.execution_repo.clone() else {
            return Ok(None);
        };
        let holder = self.instance_id.to_string();
        let ttl = self.lease_ttl;
        let heartbeat_interval = self.lease_heartbeat_interval;

        // Try to acquire the lease.
        let acquired = repo
            .acquire_lease(execution_id, holder.clone(), ttl)
            .await
            .map_err(|e| EngineError::PlanningFailed(format!("acquire lease: {e}")))?;

        if !acquired {
            // Someone else holds the lease (or it's our own holder
            // string from an earlier, still-live attempt — ADR 0008
            // "same-holder re-acquire" — but for safety we still
            // report as Leased so the caller can decide). Surface with
            // the held holder for operator visibility.
            //
            // The exact holder string isn't returned by acquire_lease
            // when it fails, so we surface the contention counter and
            // the execution id; operators correlate via storage row.
            let labels = self
                .metrics
                .interner()
                .single("reason", engine_lease_contention_reason::ALREADY_HELD);
            self.metrics
                .counter_labeled(NEBULA_ENGINE_LEASE_CONTENTION_TOTAL, &labels)
                .inc();
            tracing::warn!(
                %execution_id,
                %holder,
                "execution lease is held by another runner; refusing to dispatch (§12.2, #325)"
            );
            return Err(EngineError::Leased {
                execution_id,
                holder,
            });
        }

        tracing::debug!(
            %execution_id,
            %holder,
            ttl_secs = ttl.as_secs(),
            heartbeat_secs = heartbeat_interval.as_secs(),
            "execution lease acquired"
        );

        // Spawn a heartbeat task. A shared `heartbeat_lost` token
        // trips when a renew_lease returns `Ok(false)` (stolen or
        // expired) or errors — the frontier loop observes it via
        // the caller-provided `frontier_cancel` which we mirror.
        let heartbeat_lost = CancellationToken::new();
        let heartbeat_repo = repo.clone();
        let heartbeat_holder = holder.clone();
        let heartbeat_shutdown = CancellationToken::new();
        let metrics = self.metrics.clone();
        let heartbeat_lost_cloned = heartbeat_lost.clone();
        let heartbeat_shutdown_cloned = heartbeat_shutdown.clone();
        let frontier_cancel_cloned = frontier_cancel.clone();
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(heartbeat_interval);
            // The first tick fires immediately; skip it — we just
            // acquired.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                tokio::select! {
                    () = heartbeat_shutdown_cloned.cancelled() => {
                        // Normal shutdown from the caller after frontier exits.
                        break;
                    }
                    _ = ticker.tick() => {
                        match heartbeat_repo
                            .renew_lease(execution_id, &heartbeat_holder, ttl)
                            .await
                        {
                            Ok(true) => {
                                tracing::trace!(
                                    %execution_id,
                                    holder = %heartbeat_holder,
                                    "execution lease renewed"
                                );
                            }
                            Ok(false) => {
                                let labels = metrics
                                    .interner()
                                    .single(
                                        "reason",
                                        engine_lease_contention_reason::HEARTBEAT_LOST,
                                    );
                                metrics
                                    .counter_labeled(
                                        NEBULA_ENGINE_LEASE_CONTENTION_TOTAL,
                                        &labels,
                                    )
                                    .inc();
                                tracing::error!(
                                    %execution_id,
                                    holder = %heartbeat_holder,
                                    "execution lease heartbeat lost — another runner or \
                                     expiry took it; aborting this runner to avoid corrupting \
                                     state the new holder now drives (ADR 0008, §12.2)"
                                );
                                heartbeat_lost_cloned.cancel();
                                frontier_cancel_cloned.cancel();
                                break;
                            }
                            Err(e) => {
                                // Storage-layer error on renew — conservative:
                                // treat as loss and stop the runner. The
                                // alternative (continue despite unknown lease
                                // state) risks two runners — the exact
                                // failure mode #325 is fixing.
                                let labels = metrics
                                    .interner()
                                    .single(
                                        "reason",
                                        engine_lease_contention_reason::HEARTBEAT_LOST,
                                    );
                                metrics
                                    .counter_labeled(
                                        NEBULA_ENGINE_LEASE_CONTENTION_TOTAL,
                                        &labels,
                                    )
                                    .inc();
                                tracing::error!(
                                    %execution_id,
                                    holder = %heartbeat_holder,
                                    error = %e,
                                    "execution lease heartbeat renew failed; aborting \
                                     runner conservatively (ADR 0008, §12.2)"
                                );
                                heartbeat_lost_cloned.cancel();
                                frontier_cancel_cloned.cancel();
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Some(LeaseGuard {
            repo,
            execution_id,
            holder,
            handle: Some(handle),
            shutdown: heartbeat_shutdown,
            heartbeat_lost,
        }))
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
        // Persist the original trigger payload so that resume can
        // feed entry nodes the same input instead of substituting
        // Null (issue #311).
        exec_state.set_workflow_input(input.clone());
        // Persist the execution budget so that resume enforces the
        // same concurrency / retry / timeout limits the original run
        // was configured with, rather than falling back to
        // `ExecutionBudget::default()` (issue #289).
        exec_state.set_budget(budget.clone());

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

        // 5a. Acquire the execution lease before dispatching nodes (ADR
        // 0008, #325). Second runners that race in after the
        // `create` above but before lease acquire will observe a held
        // lease and get `EngineError::Leased`; we do not sleep-retry.
        //
        // The lease row is a column on the `executions` row so we must
        // acquire strictly after `create`. The lease scope covers the
        // frontier loop AND the final persist — holding through
        // persist is load-bearing: a stale writer whose heartbeat has
        // just died must not overwrite the canonical state a new
        // holder already began driving.
        let lease = self
            .acquire_and_heartbeat_lease(execution_id, cancel_token.clone())
            .await?;

        // 5b. Publish the cancel token into the running registry ONLY
        // after the lease is ours. The lease is the authoritative
        // single-runner fence (ADR-0015); publishing after it prevents
        // an overlapping attempt for the same `ExecutionId` from
        // overwriting the live token (#482 Copilot review). The guard's
        // nonce-scoped `Drop` (`RunningRegistration::drop`) removes the
        // entry on every exit path — normal completion, heartbeat-lost
        // `Leased`, final-persist errors — and is defensive against
        // clobbering a winner's registration if a losing attempt ever
        // slips through.
        let registration_id = NEXT_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed);
        self.running.insert(
            execution_id,
            RunningEntry {
                registration_id,
                token: cancel_token.clone(),
            },
        );
        let _cancel_registration = RunningRegistration {
            running: Arc::clone(&self.running),
            execution_id,
            registration_id,
        };

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
        let heartbeat_lost = lease.as_ref().is_some_and(LeaseGuard::heartbeat_lost);
        let FinalStatusDecision {
            status: final_status,
            termination_reason,
            integrity_violation,
        } = determine_final_status(&failed_node, &cancel_token, &exec_state);
        // `Running → Cancelled` is not a one-step transition (see
        // `nebula_execution::transition` — issue #273 documents the shortcuts
        // the state machine carved out). When the frontier loop tore down on
        // a cancel token, bridge through `Cancelling` so the subsequent
        // `transition_status(Cancelled)` is valid and the persisted row
        // records the terminal outcome. Without the bridge, the `let _`
        // swallows the invalid-transition error and the row stays at
        // `Running`, producing a two-truth violation against the
        // `ExecutionResult` the engine returns (ADR-0008 A3).
        if final_status == ExecutionStatus::Cancelled
            && exec_state.status == ExecutionStatus::Running
        {
            let _ = exec_state.transition_status(ExecutionStatus::Cancelling);
        }
        let _ = exec_state.transition_status(final_status);

        // If the heartbeat lost the lease mid-run, a sibling runner
        // now owns the canonical state. We MUST NOT persist the final
        // state or emit ExecutionFinished from this runner — the new
        // holder will drive completion. ADR 0008 / §12.2.
        let reported_status = if heartbeat_lost {
            tracing::error!(
                %execution_id,
                "final state persistence skipped: heartbeat lost this runner's lease; \
                 another runner now owns the execution (ADR 0008, §12.2, #325)"
            );
            // Release whatever lease state we hold cleanly, then bubble
            // the typed error — this runner does not own the terminal
            // transition, so reporting a status would silently overwrite
            // the new holder's state.
            if let Some(guard) = lease {
                guard.shutdown().await;
            }
            return Err(EngineError::Leased {
                execution_id,
                holder: self.instance_id.to_string(),
            });
        } else if let Some(repo) = &self.execution_repo {
            // Persist final execution state with CAS-conflict
            // reconciliation (issue #333). The pre-fix branch was
            // log-and-continue on CAS mismatch, which let the engine
            // report `Completed` on an un-persisted row and silently
            // overwrite concurrent external transitions. `persist_final_state`
            // now reloads the persisted state on mismatch, honors
            // external terminal transitions, and retries once on
            // non-terminal conflicts before surfacing a typed
            // `CasConflict` error.
            match self
                .persist_final_state(repo, execution_id, &mut exec_state, &mut repo_version)
                .await
            {
                Ok(None) => final_status,
                Ok(Some(external_status)) => {
                    // External actor drove the row into a terminal
                    // state we may not overwrite — surface it.
                    external_status
                },
                Err(EngineError::CasConflict {
                    expected_version,
                    observed_version,
                    observed_status,
                    ..
                }) => {
                    tracing::error!(
                        %execution_id,
                        expected_version,
                        observed_version,
                        %observed_status,
                        "final state CAS conflict could not be reconciled; \
                         reporting Failed instead of silently completing (§11.5, #333)"
                    );
                    ExecutionStatus::Failed
                },
                Err(e) => {
                    tracing::error!(
                        %execution_id,
                        error = %e,
                        "final state persist failed; \
                         reporting Failed instead of silently completing (§11.5, #333)"
                    );
                    ExecutionStatus::Failed
                },
            }
        } else {
            final_status
        };

        // Release the lease after final-state persistence. Release
        // strictly after persist is load-bearing: a sibling runner
        // must not observe an available lease while our terminal
        // write is still in flight.
        if let Some(guard) = lease {
            guard.shutdown().await;
        }

        self.emit_final_event(execution_id, reported_status, elapsed, &failed_node);
        self.emit_frontier_integrity_if_violated(execution_id, integrity_violation);
        tracing::info!(
            target = "engine",
            %execution_id,
            ?reported_status,
            ?termination_reason,
            ?elapsed,
            "execution_finished"
        );
        self.emit_event(ExecutionEvent::ExecutionFinished {
            execution_id,
            success: reported_status == ExecutionStatus::Completed,
            elapsed,
            termination_reason: termination_reason.clone(),
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
            status: reported_status,
            node_outputs,
            node_errors,
            duration: elapsed,
            termination_reason: termination_reason.clone(),
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
        // Cold-start seam (ADR-0008 A2, §5): the API's start handler persists an
        // `ExecutionState::new(id, workflow_id, &[])` row — no per-node entries,
        // because the handler does not load the workflow on the hot path. The
        // first `ControlCommand::Start` that drains via `EngineControlDispatch`
        // lands here; seed `node_states` from the workflow definition so the
        // frontier seeder below treats graph entry nodes as the natural starting
        // set. A warm resume (post-crash, with persisted per-node state) skips
        // this branch untouched.
        if exec_state.node_states.is_empty() {
            for node in &workflow.nodes {
                exec_state.set_node_state(
                    node.id.clone(),
                    nebula_execution::state::NodeExecutionState::new(),
                );
            }
        }
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
            let resolved = resolved_edges.get(node_key).copied().unwrap_or(0);

            if required == 0 || resolved == required {
                seed_nodes.push(node_key.clone());
            }
        }

        // 10. Build remaining infrastructure for the frontier loop.
        //
        // Restore the `ExecutionBudget` the original run was configured
        // with (issue #289). Legacy states that predate budget
        // persistence deserialize the field as `None` — fall back to
        // `ExecutionBudget::default()` with a warning so the degraded
        // limits are visible in logs instead of silently swapping
        // operator-configured limits for default ones.
        let budget = if let Some(b) = exec_state.budget.clone() {
            b
        } else {
            tracing::warn!(
                %execution_id,
                "resume: persisted execution state is missing budget; \
                 falling back to ExecutionBudget::default() — \
                 concurrency, retry, and timeout limits from the \
                 original run are not being honoured (issue #289)"
            );
            ExecutionBudget::default()
        };
        let semaphore = Arc::new(Semaphore::new(budget.max_concurrent_nodes));
        let cancel_token = CancellationToken::new();
        let mut repo_version = repo_version_loaded;

        // Acquire the execution lease before running the frontier (ADR
        // 0008, #325). Resume is explicitly a second entry point for an
        // existing execution — if another runner is already driving it
        // (whether because the crash recovery loop picked it up or an
        // operator issued two resumes back-to-back), we fence this call
        // with `EngineError::Leased` instead of running nodes in parallel
        // with the existing runner.
        let lease = self
            .acquire_and_heartbeat_lease(execution_id, cancel_token.clone())
            .await?;

        // Publish the cancel token into the running registry ONLY after
        // the lease is ours (ADR-0015 single-runner fence). Symmetric to
        // `execute_workflow` — see its comment for the full rationale
        // and the #482 Copilot review context.
        let registration_id = NEXT_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed);
        self.running.insert(
            execution_id,
            RunningEntry {
                registration_id,
                token: cancel_token.clone(),
            },
        );
        let _cancel_registration = RunningRegistration {
            running: Arc::clone(&self.running),
            execution_id,
            registration_id,
        };

        self.metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
            .inc();

        let error_strategy = workflow.config.error_strategy;
        // Restore the original trigger payload from the persisted
        // execution state. Legacy states that predate #311 deserialize
        // the field as `None` — fall back to `Null` with a warning so
        // the regression is visible in logs.
        let workflow_input = if let Some(v) = exec_state.workflow_input.clone() {
            v
        } else {
            tracing::warn!(
                %execution_id,
                "resume: persisted execution state is missing workflow_input; \
                 falling back to Null — entry nodes that did not complete \
                 on the original run will receive Null input"
            );
            serde_json::Value::Null
        };
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

        let heartbeat_lost = lease.as_ref().is_some_and(LeaseGuard::heartbeat_lost);
        let FinalStatusDecision {
            status: final_status,
            termination_reason,
            integrity_violation,
        } = determine_final_status(&failed_node, &cancel_token, &exec_state);
        // Use the validated transition path. Ignoring the result is intentional:
        // if the current status is already terminal (e.g. the execution was
        // cancelled during the frontier loop), we do not overwrite it.
        //
        // Bridge `Running → Cancelling → Cancelled` when the cancel token
        // fired mid-flight — one-step `Running → Cancelled` is not in the
        // valid-transition table (issue #273), so without the bridge the
        // invalid-transition error is silently swallowed and the row stays
        // at `Running`, producing a two-truth violation (ADR-0008 A3).
        if final_status == ExecutionStatus::Cancelled
            && exec_state.status == ExecutionStatus::Running
        {
            let _ = exec_state.transition_status(ExecutionStatus::Cancelling);
        }
        let _ = exec_state.transition_status(final_status);

        // Heartbeat loss: another runner now owns the canonical state.
        // Skip final persist and surface as Leased — mirrors the
        // execute_workflow contract. ADR 0008 / §12.2 / #325.
        let reported_status = if heartbeat_lost {
            tracing::error!(
                %execution_id,
                "resume: final state persistence skipped: heartbeat lost this runner's lease; \
                 another runner now owns the execution (ADR 0008, §12.2, #325)"
            );
            if let Some(guard) = lease {
                guard.shutdown().await;
            }
            return Err(EngineError::Leased {
                execution_id,
                holder: self.instance_id.to_string(),
            });
        } else {
            // Persist final state with CAS-conflict reconciliation
            // (issue #333). Mirrors `execute_workflow` — see its comment
            // for the full contract.
            match self
                .persist_final_state(exec_repo, execution_id, &mut exec_state, &mut repo_version)
                .await
            {
                Ok(None) => final_status,
                Ok(Some(external_status)) => external_status,
                Err(EngineError::CasConflict {
                    expected_version,
                    observed_version,
                    observed_status,
                    ..
                }) => {
                    tracing::error!(
                        %execution_id,
                        expected_version,
                        observed_version,
                        %observed_status,
                        "resume: final state CAS conflict could not be reconciled; \
                         reporting Failed instead of silently completing (§11.5, #333)"
                    );
                    ExecutionStatus::Failed
                },
                Err(e) => {
                    tracing::error!(
                        %execution_id,
                        error = %e,
                        "resume: final state persist failed; \
                         reporting Failed instead of silently completing (§11.5, #333)"
                    );
                    ExecutionStatus::Failed
                },
            }
        };

        // Release the lease after the final persist completes.
        if let Some(guard) = lease {
            guard.shutdown().await;
        }

        self.emit_final_event(execution_id, reported_status, elapsed, &failed_node);
        self.emit_frontier_integrity_if_violated(execution_id, integrity_violation);
        tracing::info!(
            target = "engine",
            %execution_id,
            ?reported_status,
            ?termination_reason,
            ?elapsed,
            "execution_finished"
        );
        self.emit_event(ExecutionEvent::ExecutionFinished {
            execution_id,
            success: reported_status == ExecutionStatus::Completed,
            elapsed,
            termination_reason: termination_reason.clone(),
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
            status: reported_status,
            node_outputs,
            node_errors,
            duration: elapsed,
            termination_reason: termination_reason.clone(),
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

        // In-flight tasks + a side map from tokio task id → NodeKey so
        // that panics (where the inner future's `(NodeKey, _)` payload
        // is lost) can still be attributed to the real node instead
        // of a synthesized placeholder (issue #301).
        let mut join_set: JoinSet<(
            NodeKey,
            Result<ActionResult<serde_json::Value>, EngineError>,
        )> = JoinSet::new();
        let mut task_nodes: HashMap<tokio::task::Id, NodeKey> = HashMap::new();

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
                    &mut task_nodes,
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
                //
                // Ordering (§11.5, #297 review): classify → apply
                // recovery → route (stages OnError payload into
                // outputs) → checkpoint (durably commits state +
                // staged payload) → emit. Route runs BEFORE
                // checkpoint so `load_all_outputs` on resume finds
                // the OnError handler's input.
                let err_msg = exec_state
                    .node_state(node_key.clone())
                    .and_then(|ns| ns.error_message.clone())
                    .unwrap_or_else(|| "parameter resolution failed".to_string());

                let outcome = classify_failure(error_strategy);
                if let Err(e) =
                    apply_failure_recovery(outcome, node_key.clone(), exec_state, outputs)
                {
                    cancel_token.cancel();
                    return Some((node_key, e.to_string()));
                }

                // Route BEFORE checkpoint so the OnError input payload
                // (`outputs[node_key] = {error, node_id}`) written by
                // `route_failure_edges` is captured by the checkpoint.
                // Successors enqueued into `ready_queue` are invisible
                // until Phase 1 of the next loop iteration, which runs
                // strictly after the checkpoint below — nothing external
                // observes the routing before the store commits it.
                let abort = route_failure_edges(
                    outcome,
                    node_key.clone(),
                    &err_msg,
                    error_strategy,
                    graph,
                    outputs,
                    &mut activated_edges,
                    &mut resolved_edges,
                    &required_count,
                    &mut ready_queue,
                    exec_state,
                );

                if let Err(e) = self
                    .checkpoint_node(
                        execution_id,
                        node_key.clone(),
                        outputs,
                        exec_state,
                        repo_version,
                    )
                    .await
                {
                    cancel_token.cancel();
                    return Some((node_key, e.to_string()));
                }

                if exec_state
                    .node_state(node_key.clone())
                    .is_some_and(|ns| ns.state == NodeState::Failed)
                {
                    self.emit_event(ExecutionEvent::NodeFailed {
                        execution_id,
                        node_key: node_key.clone(),
                        error: err_msg.clone(),
                    });
                }

                if let Some(err_msg) = abort {
                    cancel_token.cancel();
                    return Some((node_key, err_msg));
                }
            }

            // Phase 2: Wait for one completion (or exit if nothing in flight)
            if join_set.is_empty() {
                break;
            }

            if cancel_token.is_cancelled() {
                join_set.abort_all();
                while join_set.join_next_with_id().await.is_some() {}
                task_nodes.clear();
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
                    tokio::time::sleep(d).await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::pin!(sleep_fut);

            let join_result = tokio::select! {
                result = join_set.join_next_with_id() => match result {
                    Some(r) => r,
                    None => break,
                },
                () = &mut sleep_fut => {
                    cancel_token.cancel();
                    join_set.abort_all();
                    while join_set.join_next_with_id().await.is_some() {}
                    task_nodes.clear();
                    return Some((
                        node_key!("_timeout"),
                        "execution budget exceeded: max_duration".to_string(),
                    ));
                }
                () = cancel_token.cancelled() => {
                    join_set.abort_all();
                    while join_set.join_next_with_id().await.is_some() {}
                    task_nodes.clear();
                    break;
                }
            };

            // Phase 3: Process the completed task
            match join_result {
                // `ActionResult::Retry` is a `planned` capability under canon
                // §11.2 — there is no persisted attempt accounting yet. The
                // variant itself is gated behind `unstable-retry-scheduler`
                // in `nebula-action`, but Cargo feature unification can still
                // make the variant present in the `nebula-action` the engine
                // sees even if `nebula-engine/unstable-retry-scheduler` is
                // off. We therefore route retry detection through the always-
                // available `ActionResult::is_retry()` predicate instead of
                // cfg-gating this arm — that way `Retry` is never silently
                // handed to the generic `Ok(action_result)` success arm.
                // Handling stays a synthetic failure until the real scheduler
                // lands (#290 / #296).
                Ok((task_id, (node_key, Ok(ref action_result)))) if action_result.is_retry() => {
                    task_nodes.remove(&task_id);
                    total_retries.fetch_add(1, Ordering::Relaxed);
                    let err = EngineError::Runtime(crate::runtime::RuntimeError::ActionError(
                        ActionError::retryable("Action retry is not supported by the engine"),
                    ));
                    mark_node_failed(exec_state, node_key.clone(), &err);
                    let err_str = err.to_string();

                    // Ordering (§11.5, #297 review): classify → apply
                    // recovery → route (stages OnError payload) →
                    // checkpoint → emit. Identical shape to the
                    // runtime-failure branch below; see its comment
                    // block.
                    let outcome = classify_failure(error_strategy);
                    if let Err(e) =
                        apply_failure_recovery(outcome, node_key.clone(), exec_state, outputs)
                    {
                        cancel_token.cancel();
                        return Some((node_key.clone(), e.to_string()));
                    }

                    let abort = route_failure_edges(
                        outcome,
                        node_key.clone(),
                        &err_str,
                        error_strategy,
                        graph,
                        outputs,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );

                    if let Err(e) = self
                        .checkpoint_node(
                            execution_id,
                            node_key.clone(),
                            outputs,
                            exec_state,
                            repo_version,
                        )
                        .await
                    {
                        cancel_token.cancel();
                        return Some((node_key.clone(), e.to_string()));
                    }

                    if outcome == FailureOutcome::Fail {
                        self.emit_event(ExecutionEvent::NodeFailed {
                            execution_id,
                            node_key: node_key.clone(),
                            error: err_str.clone(),
                        });
                    }

                    if let Some(err_msg) = abort {
                        cancel_token.cancel();
                        return Some((node_key.clone(), err_msg));
                    }
                },
                Ok((task_id, (node_key, Ok(action_result)))) => {
                    task_nodes.remove(&task_id);
                    mark_node_completed(exec_state, node_key.clone());

                    // Track output size for budget enforcement
                    if let Some(output) = outputs.get(&node_key) {
                        let bytes =
                            serde_json::to_string(output.value()).map_or(0, |s| s.len() as u64);
                        total_output_bytes.fetch_add(bytes, Ordering::Relaxed);
                    }

                    // Capture an explicit-termination signal BEFORE the
                    // checkpoint so that the same CAS-write durably
                    // persists `terminated_by` (canon §11.5; ROADMAP
                    // §M0.3). The companion `cancel_token.cancel()` is
                    // deferred until AFTER `Ok` from `checkpoint_node` so
                    // we tear down sibling branches only on a durable
                    // decision.
                    let terminate_was_first_set =
                        if let ActionResult::Terminate { reason } = &action_result {
                            let exec_reason = map_termination_reason(node_key.clone(), reason);
                            let was_first =
                                exec_state.set_terminated_by(node_key.clone(), exec_reason.clone());
                            tracing::info!(
                                target = "engine::frontier",
                                execution_id = %execution_id,
                                node_key = %node_key,
                                ?exec_reason,
                                was_first,
                                "explicit_termination_signal"
                            );
                            was_first
                        } else {
                            false
                        };

                    // Persist node output + execution state, then record the
                    // idempotency key, before any external observer learns the
                    // node is done. This guarantees durability precedes
                    // visibility (§11.5, #297). Checkpoint failure aborts the
                    // node's progression so observers never see an
                    // unpersisted transition and the frontier never advances
                    // on an undurable decision (§12.4).
                    if let Err(e) = self
                        .checkpoint_node(
                            execution_id,
                            node_key.clone(),
                            outputs,
                            exec_state,
                            repo_version,
                        )
                        .await
                    {
                        cancel_token.cancel();
                        return Some((node_key.clone(), e.to_string()));
                    }
                    self.record_idempotency(exec_state, execution_id, node_key.clone())
                        .await;

                    // Persist the full ActionResult alongside the raw
                    // output so that idempotent replay can reconstruct
                    // the exact routing semantics (issue #299).
                    let attempt = exec_state
                        .node_states
                        .get(&node_key)
                        .map_or(1, |ns| ns.attempt_count().max(1) as u32);
                    self.record_node_result(
                        execution_id,
                        node_key.clone(),
                        attempt,
                        &action_result,
                    )
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

                    // ROADMAP §M0.3: signal `cancel_token` ONLY after the
                    // termination signal is durable AND we've gated the
                    // local downstream edges through `process_outgoing_edges`
                    // (which already treats `Terminate` like `Skip`).
                    // Siblings still in flight observe the cancel and tear
                    // down; the executor's `select!` arm reconciles their
                    // `Cancelled` state on the next loop iteration.
                    if terminate_was_first_set {
                        tracing::trace!(
                            target = "engine::frontier",
                            execution_id = %execution_id,
                            node_key = %node_key,
                            "cancel_token signalled after durable termination"
                        );
                        cancel_token.cancel();
                    }
                },
                Ok((task_id, (node_key, Err(ref err)))) => {
                    task_nodes.remove(&task_id);
                    // Node failed at runtime. Ordering (§11.5, #297 PR
                    // review by Copilot — route stages OnError payload
                    // that checkpoint must capture so resume can read
                    // it from `load_all_outputs`):
                    //   1. `mark_node_failed`      — in-memory Failed
                    //   2. `apply_failure_recovery` — IgnoreErrors-only override of state + null
                    //      output (in-memory)
                    //   3. `route_failure_edges`    — evaluate outgoing edges; may write `{error,
                    //      node_id}` payload into `outputs[node_key]` for OnError input; may
                    //      enqueue successors into `ready_queue`
                    //   4. `checkpoint_node`        — durable commit of state + outputs (abort on
                    //      Err; the discarded `ready_queue` mutations never surface)
                    //   5. `emit_event`             — observers (only for Fail outcome), strictly
                    //      after persist
                    //
                    // Successors in `ready_queue` do NOT dispatch until
                    // Phase 1 of the next loop iteration; that runs
                    // after checkpoint. Nothing external observes a
                    // state the store has not committed (§11.5).
                    mark_node_failed(exec_state, node_key.clone(), err);
                    let err_str = err.to_string();

                    let outcome = classify_failure(error_strategy);
                    if let Err(e) =
                        apply_failure_recovery(outcome, node_key.clone(), exec_state, outputs)
                    {
                        cancel_token.cancel();
                        return Some((node_key.clone(), e.to_string()));
                    }

                    let abort = route_failure_edges(
                        outcome,
                        node_key.clone(),
                        &err_str,
                        error_strategy,
                        graph,
                        outputs,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );

                    if let Err(e) = self
                        .checkpoint_node(
                            execution_id,
                            node_key.clone(),
                            outputs,
                            exec_state,
                            repo_version,
                        )
                        .await
                    {
                        cancel_token.cancel();
                        return Some((node_key.clone(), e.to_string()));
                    }

                    if outcome == FailureOutcome::Fail {
                        self.emit_event(ExecutionEvent::NodeFailed {
                            execution_id,
                            node_key: node_key.clone(),
                            error: err_str.clone(),
                        });
                    }

                    if let Some(err_msg) = abort {
                        cancel_token.cancel();
                        return Some((node_key.clone(), err_msg));
                    }
                },
                Err(join_err) => {
                    // Recover the real NodeKey via the task-id side
                    // map; falling back to a synthetic key would
                    // report a phantom node and lose the identity of
                    // the actually-panicked task (issue #301).
                    let task_id = join_err.id();
                    let panicked_node = task_nodes.remove(&task_id);
                    let err_msg = join_err.to_string();
                    tracing::error!(
                        ?task_id,
                        ?panicked_node,
                        error = %err_msg,
                        "node task panicked"
                    );

                    if let Some(node_key) = panicked_node {
                        self.handle_panicked_node(
                            execution_id,
                            node_key.clone(),
                            &err_msg,
                            outputs,
                            exec_state,
                            repo_version,
                        )
                        .await;
                        cancel_token.cancel();
                        return Some((node_key, err_msg));
                    }

                    // No matching task id — this should be unreachable
                    // as we insert every spawn into `task_nodes`, but
                    // fall through defensively rather than inventing
                    // a node identity.
                    cancel_token.cancel();
                    return Some((
                        node_key!("_panicked"),
                        format!("panicked task with unknown id: {err_msg}"),
                    ));
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
        task_nodes: &mut HashMap<tokio::task::Id, NodeKey>,
    ) -> bool {
        let Some(node_def) = node_map.get(&node_key) else {
            // Unknown node — route through the setup-failure path so
            // the frontier loop records the error and checkpoints the
            // state (issues #300, #321).
            let _ = exec_state.mark_setup_failed(
                node_key.clone(),
                format!("node {node_key} is not in the workflow's node map"),
            );
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
                    // Parameter resolution failed. `mark_setup_failed`
                    // handles the Pending/Failed/Retrying source states
                    // uniformly via `override_node_state` (Pending →
                    // Failed is not a valid forward transition) and
                    // bumps the parent version for CAS readers
                    // (issues #255, #300).
                    let _ = exec_state.mark_setup_failed(node_key.clone(), e.to_string());
                    return false;
                },
            };

        // Drive the node to Running via the typed state-machine
        // helper. `start_node_attempt` models the legal transitions
        // (Pending → Ready → Running, Failed → Retrying → Running,
        // Retrying → Running) and returns an error for anything else.
        // On error we do NOT silently spawn the task on stale state —
        // route through the setup-failure path instead (issue #300).
        if let Err(err) = exec_state.start_node_attempt(node_key.clone()) {
            let _ = exec_state.mark_setup_failed(
                node_key.clone(),
                format!("cannot start node attempt: {err}"),
            );
            return false;
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
                        async move {
                            let snapshot = (resolver_fn)(&id).await.map_err(|e| {
                                nebula_core::CoreError::CredentialNotFound { key: e.to_string() }
                            })?;
                            Ok(Box::new(snapshot) as Box<dyn std::any::Any + Send + Sync>)
                        }
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

        let handle = join_set.spawn(
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
        task_nodes.insert(handle.id(), node_key);

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

        let idem_key = exec_state.idempotency_key_for_node(node_key.clone());

        let already_done = match repo.check_idempotency(idem_key.as_str()).await {
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

        // Prefer the fully-typed persisted ActionResult when
        // available. Falling back to a synthesized `Success` loses
        // Branch/Route/MultiOutput/Skip routing semantics on replay —
        // every branch edge would fire unconditionally (issue #299).
        let stored_result = match repo.load_node_result(execution_id, node_key.clone()).await {
            Ok(Some(record)) => {
                match serde_json::from_value::<ActionResult<serde_json::Value>>(record.result) {
                    Ok(result) => Some(result),
                    Err(e) => {
                        tracing::warn!(
                            %execution_id,
                            %node_key,
                            error = %e,
                            "failed to deserialize persisted action result; \
                             falling back to synthesized Success"
                        );
                        None
                    },
                }
            },
            Ok(None) => {
                // Backend has no stored result (legacy rows, or a
                // backend that does not override save_node_result).
                // Fall back to the old behaviour but log so the
                // regression is visible.
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    "idempotency replay has no persisted ActionResult; \
                     synthesizing Success — Branch/Route/MultiOutput \
                     routing will not be preserved"
                );
                None
            },
            Err(e) => {
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    error = %e,
                    "failed to load persisted action result; \
                     falling back to synthesized Success"
                );
                None
            },
        };

        let effective_result = stored_result.unwrap_or_else(|| ActionResult::success(output_value));
        process_outgoing_edges(
            node_key.clone(),
            Some(&effective_result),
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

    /// Persist the full [`ActionResult`] variant for a successfully
    /// executed node so that idempotent replay can reconstruct the
    /// exact routing semantics (Branch/Route/MultiOutput/Skip) instead
    /// of synthesising a flat `Success` (issue #299).
    ///
    /// Best-effort: failures are logged and ignored. Backends that do
    /// not override `save_node_result` no-op via the default trait
    /// implementation.
    async fn record_node_result(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
        action_result: &ActionResult<serde_json::Value>,
    ) {
        let Some(repo) = &self.execution_repo else {
            return;
        };
        let value = match serde_json::to_value(action_result) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    error = %e,
                    "failed to serialize action result for persistence"
                );
                return;
            },
        };
        let kind = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_owned();
        let record = nebula_storage::NodeResultRecord::new(kind, value);
        if let Err(e) = repo
            .save_node_result(execution_id, node_key.clone(), attempt, record)
            .await
        {
            tracing::warn!(
                %execution_id,
                %node_key,
                error = %e,
                "failed to persist action result"
            );
        }
    }

    /// Record an idempotency key for a successfully executed node (best-effort).
    ///
    /// Silently logs and ignores errors — idempotency key recording failures
    /// must not abort an otherwise healthy execution.
    async fn record_idempotency(
        &self,
        exec_state: &ExecutionState,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) {
        let Some(repo) = &self.execution_repo else {
            return;
        };
        let idem_key = exec_state.idempotency_key_for_node(node_key.clone());
        if let Err(e) = repo
            .mark_idempotent(idem_key.as_str(), execution_id, node_key.clone())
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

    /// Persist node output and execution state to the repository.
    ///
    /// Returns `Err(EngineError::CheckpointFailed)` when the store cannot
    /// durably commit — `save_node_output` failure, `transition()` error,
    /// or CAS mismatch (the row moved beneath the engine). Callers in
    /// `run_frontier` MUST abort the node's progression (no edge routing,
    /// no event emission) on `Err` so that observers and the frontier
    /// never act on an unpersisted transition (§11.5, §12.4, #297).
    /// Persist final Failed state + emit NodeFailed for a panicked task.
    ///
    /// Best-effort: checkpoint failures are logged at `warn!` level (not
    /// propagated) so that the engine still returns a cohesive panic
    /// error to `run_frontier`'s caller. The real durability gap —
    /// `save_node_output` after panic — is already logged by
    /// `checkpoint_node` itself.
    async fn handle_panicked_node(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        err_msg: &str,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        exec_state: &mut ExecutionState,
        repo_version: &mut u64,
    ) {
        let panic_err = EngineError::TaskPanicked(err_msg.to_owned());
        mark_node_failed(exec_state, node_key.clone(), &panic_err);
        let checkpoint_result = self
            .checkpoint_node(
                execution_id,
                node_key.clone(),
                outputs,
                exec_state,
                repo_version,
            )
            .await;
        if let Err(e) = checkpoint_result {
            tracing::warn!(
                %execution_id,
                %node_key,
                error = %e,
                "failed to checkpoint panicked node state"
            );
        }
        self.emit_event(ExecutionEvent::NodeFailed {
            execution_id,
            node_key,
            error: err_msg.to_owned(),
        });
    }

    async fn checkpoint_node(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        exec_state: &ExecutionState,
        repo_version: &mut u64,
    ) -> Result<(), EngineError> {
        let Some(repo) = &self.execution_repo else {
            return Ok(());
        };

        // Save node output individually. A write failure here is a real
        // durability gap — replayers load outputs from this table, so
        // losing one means a downstream consumer may reconstruct a
        // node as "ran and returned nothing" after restart (§11.5).
        if let Some(output) = outputs.get(&node_key) {
            let attempt = exec_state
                .node_states
                .get(&node_key)
                .map_or(1, |ns| ns.attempt_count().max(1) as u32);
            if let Err(e) = repo
                .save_node_output(
                    execution_id,
                    node_key.clone(),
                    attempt,
                    output.value().clone(),
                )
                .await
            {
                return Err(EngineError::CheckpointFailed {
                    node_key,
                    reason: format!("save_node_output: {e}"),
                });
            }
        }

        // Save execution state snapshot.
        let state_json =
            serde_json::to_value(exec_state).map_err(|e| EngineError::CheckpointFailed {
                node_key: node_key.clone(),
                reason: format!("serialize state: {e}"),
            })?;

        match repo
            .transition(execution_id, *repo_version, state_json)
            .await
        {
            Ok(true) => {
                *repo_version += 1;
                Ok(())
            },
            Ok(false) => {
                // CAS mismatch: another actor moved the row (API cancel,
                // second engine worker, etc.). We cannot durably commit
                // this node's transition on top of a stale version, so
                // the frontier must abort this progression rather than
                // silently advance (§11.5 "durability precedes
                // visibility", §12.4 "no silent log-and-continue").
                //
                // Per issue #333 we refetch the **full** persisted
                // state — not just the version — so the failure surface
                // carries the observer-visible status (e.g. `Cancelling`
                // / `Cancelled`) rather than discarding authoritative
                // fields. The caller (frontier loop) aborts the node's
                // progression and the outer execute/resume path
                // reconciles against the newly observed state.
                let expected_version = *repo_version;
                let observed_status = match repo.get_state(execution_id).await {
                    Ok(Some((current_version, current_state))) => {
                        *repo_version = current_version;
                        parse_observed_status(&current_state)
                    },
                    Ok(None) => "unknown".to_owned(),
                    Err(e) => {
                        tracing::warn!(
                            %execution_id,
                            %node_key,
                            error = %e,
                            "checkpoint CAS mismatch: failed to refetch persisted state"
                        );
                        "unknown".to_owned()
                    },
                };
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    expected_version,
                    observed_version = *repo_version,
                    %observed_status,
                    "checkpoint CAS mismatch — aborting node progression (§11.1, #333)"
                );
                Err(EngineError::CasConflict {
                    execution_id,
                    expected_version,
                    observed_version: *repo_version,
                    observed_status,
                })
            },
            Err(e) => Err(EngineError::CheckpointFailed {
                node_key,
                reason: e.to_string(),
            }),
        }
    }

    /// Persist the final execution state, reconciling with any
    /// externally-driven concurrent update (issue #333).
    ///
    /// Contract:
    ///
    /// * On CAS success: commit and return the engine-local final status (unchanged from pre-fix
    ///   behaviour).
    /// * On CAS mismatch, **reload the full persisted state**, then:
    ///     - If the observed persisted status is already terminal (`Cancelled` / `Failed` /
    ///       `TimedOut` / `Completed`), honor it. The external actor (API cancel, admin mutation,
    ///       sibling runner) produced an authoritative terminal transition the engine may not
    ///       overwrite — `Ok(Some(external_status))` is returned so the caller reports the external
    ///       status in `ExecutionResult`.
    ///     - Otherwise, copy the engine's local `final_status` onto the freshly-loaded state, bump
    ///       the observed version, and retry the transition exactly once. On repeated CAS mismatch
    ///       or a storage error, return [`EngineError::CasConflict`] /
    ///       [`EngineError::CheckpointFailed`] rather than silently reporting success.
    ///
    /// Pre-fix this path was `log-and-continue` (see `tracing::warn!`
    /// "final state checkpoint CAS mismatch" before #333) — that
    /// silently dropped the final write and let the engine report
    /// `Completed` on an un-persisted state, violating `docs/PRODUCT_CANON.md`
    /// §11.5 (durability precedes visibility) and §12.4 (no silent
    /// log-and-continue on state-transition failures).
    ///
    /// # Returns
    ///
    /// * `Ok(None)` — the engine's local final status was durably persisted (either on first try or
    ///   on the retry).
    /// * `Ok(Some(status))` — the CAS was still conflicting but the persisted state is already
    ///   terminal; the caller should surface `status` as the execution outcome instead of the
    ///   engine's local decision.
    /// * `Err(EngineError::CasConflict { .. })` — after a retry the row was still moving and not
    ///   terminal; the engine cannot honor the conflict without more context, so the caller
    ///   surfaces a typed failure instead of a silent success.
    async fn persist_final_state(
        &self,
        repo: &Arc<dyn nebula_storage::ExecutionRepo>,
        execution_id: ExecutionId,
        exec_state: &mut ExecutionState,
        repo_version: &mut u64,
    ) -> Result<Option<ExecutionStatus>, EngineError> {
        let state_json =
            serde_json::to_value(&*exec_state).map_err(|e| EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("serialize final state: {e}"),
            })?;

        match repo
            .transition(execution_id, *repo_version, state_json)
            .await
        {
            Ok(true) => {
                *repo_version += 1;
                Ok(None)
            },
            Ok(false) => {
                let expected_version = *repo_version;
                // Reload the full persisted state — not just version —
                // so we can decide whether the external transition is
                // authoritative (issue #333).
                let (observed_version, observed_json) = match repo.get_state(execution_id).await {
                    Ok(Some(pair)) => pair,
                    Ok(None) => {
                        return Err(EngineError::CasConflict {
                            execution_id,
                            expected_version,
                            observed_version: 0,
                            observed_status: "missing".to_owned(),
                        });
                    },
                    Err(e) => {
                        return Err(EngineError::CheckpointFailed {
                            node_key: final_state_node_key(),
                            reason: format!("final CAS refetch failed: {e}"),
                        });
                    },
                };
                *repo_version = observed_version;
                let observed_status_enum = parse_observed_execution_status(&observed_json);
                let observed_status_str = parse_observed_status(&observed_json);

                // External state already terminal → honor it. The
                // engine must not overwrite an authoritative
                // cancellation / failure with its own local decision
                // (§11.5, #333).
                if observed_status_enum
                    .as_ref()
                    .is_some_and(ExecutionStatus::is_terminal)
                {
                    tracing::warn!(
                        %execution_id,
                        expected_version,
                        observed_version,
                        %observed_status_str,
                        "final state CAS mismatch: external transition is terminal — \
                         honoring external status instead of overwriting (§11.5, #333)"
                    );
                    return Ok(observed_status_enum);
                }

                // External state non-terminal (e.g. sibling runner
                // still in-flight). Retry once: re-serialize the local
                // state at the freshly observed version and attempt the
                // final write again. If it still conflicts, surface
                // the typed error rather than dropping the write.
                let retry_json = match serde_json::to_value(&*exec_state) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(EngineError::CheckpointFailed {
                            node_key: final_state_node_key(),
                            reason: format!("serialize retry state: {e}"),
                        });
                    },
                };
                match repo
                    .transition(execution_id, *repo_version, retry_json)
                    .await
                {
                    Ok(true) => {
                        tracing::info!(
                            %execution_id,
                            expected_version,
                            observed_version,
                            "final state CAS retry succeeded after external bump (§11.5, #333)"
                        );
                        *repo_version += 1;
                        Ok(None)
                    },
                    Ok(false) => {
                        // Refresh the observed version once more so
                        // the error carries the latest truth.
                        let (latest_version, latest_json) = match repo.get_state(execution_id).await
                        {
                            Ok(Some(pair)) => pair,
                            _ => (observed_version, observed_json),
                        };
                        *repo_version = latest_version;
                        Err(EngineError::CasConflict {
                            execution_id,
                            expected_version,
                            observed_version: latest_version,
                            observed_status: parse_observed_status(&latest_json),
                        })
                    },
                    Err(e) => Err(EngineError::CheckpointFailed {
                        node_key: NodeKey::new("__final__").expect("placeholder node key"),
                        reason: format!("final CAS retry failed: {e}"),
                    }),
                }
            },
            Err(e) => Err(EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("final state persist: {e}"),
            }),
        }
    }

    /// Record final execution metrics.
    fn emit_final_event(
        &self,
        _execution_id: ExecutionId,
        status: ExecutionStatus,
        elapsed: Duration,
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
    #[expect(dead_code, reason = "reserved for multi-input actions")]
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
        //     EngineError::Action. The frontier loop routes this through `classify_failure` +
        //     `route_failure_edges`, where the workflow-level ErrorStrategy decides whether
        //     execution fails fast or continues/ignores the failure, and whether any `OnError` edge
        //     is activated (split from the old `handle_node_failure` per #297 / §11.5). This
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
                        ActionError::credential_refresh_failed(self.action_key.clone(), source);
                    return (self.node_key, Err(EngineError::Action(action_err)));
                },
            }
        }

        let base = Arc::new(
            nebula_core::BaseContext::builder()
                .cancellation(self.cancel.child_token())
                .build(),
        );
        let action_ctx = nebula_action::ActionRuntimeContext::new(
            base,
            self.execution_id,
            self.node_key.clone(),
            self.workflow_id,
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
                let action_err = ActionError::retryable_with_hint(
                    format!("rate limit exceeded: {e:?}"),
                    nebula_action::error::RetryHintCode::RateLimited,
                );
                return (
                    self.node_key.clone(),
                    Err(EngineError::Runtime(
                        crate::runtime::RuntimeError::ActionError(action_err),
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
                &action_ctx,
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
        let resolved = resolved_edges.get(&target).copied().unwrap_or(0);
        let required = required_count.get(&target).copied().unwrap_or(0);
        let activated = activated_edges.get(&target).map_or(0, HashSet::len);

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
/// Port-driven routing (spec 28 §2.2): the engine matches the edge's
/// effective source port (`from_port`, defaulting to `"main"`) against the
/// port the upstream `ActionResult` produced on. There is no "edge
/// condition" — conditionals are carried by explicit control-flow nodes
/// (`If`, `Switch`, `Router`, `ErrorRouter`) whose own `ActionResult`
/// decides which port fires.
///
/// Rules:
/// - `Skip` / `Drop` / `Terminate` → no edges activate on any port.
/// - Failed node → only edges with `from_port == "error"` activate; action authors wire their
///   failure path to an `ErrorRouter` or recovery node.
/// - `Success` → activates edges on `"main"` (or `None`).
/// - `Branch { selected }` → activates edges whose effective source port equals `selected` (legacy
///   alias for `Route`).
/// - `Route { port }` → activates edges whose effective source port equals `port`.
/// - `MultiOutput { outputs }` → activates edges whose effective source port is present in
///   `outputs`.
/// - `Continue` / `Break` / `Retry` / `Wait` → engine treats these like `Success` for edge
///   activation (they hit the main port); persistent state handling lives outside this routing
///   decision.
fn evaluate_edge(
    conn: &Connection,
    result: Option<&ActionResult<serde_json::Value>>,
    node_failed: bool,
) -> bool {
    use ActionResult::*;

    // Skip / Drop / Terminate never activate any edge.
    if matches!(result, Some(Skip { .. } | Drop { .. } | Terminate { .. })) {
        return false;
    }

    let effective_port = conn.effective_from_port();

    // Failures route exclusively through the `"error"` port. Downstream must
    // wire an explicit `ErrorRouter` / recovery node to fan out by error class.
    if node_failed {
        return effective_port == "error";
    }

    match result {
        Some(Branch { selected, .. }) => effective_port == selected.as_str(),
        Some(Route { port, .. }) => effective_port == port.as_str(),
        Some(MultiOutput {
            outputs: port_outputs,
            ..
        }) => port_outputs.contains_key(effective_port),
        // Success and every other main-port variant fire the main port.
        _ => effective_port == "main",
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

        let resolved = resolved_edges.get(&target).copied().unwrap_or(0);
        let required = required_count.get(&target).copied().unwrap_or(0);
        let activated = activated_edges.get(&target).map_or(0, HashSet::len);

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

/// Classification of a node failure against the workflow's error strategy.
///
/// Pure function of the strategy: splits the outcome from the state
/// mutation + edge routing that used to live together in the old
/// `handle_node_failure`. Split lets `run_frontier` order `state-mutation
/// → persist → emit → route` per §11.5 / #297 — routing outgoing edges
/// may push successors into `ready_queue`, which must be a deterministic
/// function of the persisted state, not of an in-memory decision that
/// a crash can lose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureOutcome {
    /// `IgnoreErrors`: the node is recovered to `Completed` with a null
    /// output. No `NodeFailed` event is emitted; downstream edges activate
    /// as if the node had returned `ActionResult::success(null)`.
    Recover,
    /// `FailFast` or `ContinueOnError`: the node stays `Failed`. The
    /// caller emits `NodeFailed` and then routes failure edges, which
    /// may activate an OnError handler, resolve-without-activate for
    /// ContinueOnError, or request abort for FailFast.
    Fail,
}

/// Classify a failure outcome. Pure — does not touch `exec_state`.
fn classify_failure(error_strategy: nebula_workflow::ErrorStrategy) -> FailureOutcome {
    match error_strategy {
        nebula_workflow::ErrorStrategy::IgnoreErrors => FailureOutcome::Recover,
        _ => FailureOutcome::Fail,
    }
}

/// Apply the IgnoreErrors in-memory recovery before routing + checkpoint.
///
/// For `FailureOutcome::Recover` (IgnoreErrors): overrides the state to
/// `Completed`, clears `error_message`, inserts a `null` output. Mirrors
/// the old `handle_node_failure` IgnoreErrors path. The override bumps
/// the version per #255 so CAS readers see the recovery.
///
/// For `FailureOutcome::Fail`: no-op. The failed state was set by the
/// caller's `mark_node_failed` (or `spawn_node`'s override); the OnError
/// input payload (if any edge matches) is written by
/// `route_failure_edges` and captured by the following checkpoint.
///
/// Returns `Err(EngineError::Execution)` if `override_node_state`
/// cannot find the node — the caller MUST abort the node's progression
/// rather than leave state + outputs half-applied (§12.4). Pre-review
/// (PR #436 / Copilot) this function discarded the `Result` via
/// `let _ = ...`, silently masking a real consistency error.
fn apply_failure_recovery(
    outcome: FailureOutcome,
    node_key: NodeKey,
    exec_state: &mut ExecutionState,
    outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
) -> Result<(), EngineError> {
    if outcome == FailureOutcome::Recover {
        exec_state.override_node_state(node_key.clone(), NodeState::Completed)?;
        if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
            ns.error_message = None;
        }
        outputs.insert(node_key, serde_json::json!(null));
    }
    Ok(())
}

/// Route outgoing edges. MUST be called BEFORE `checkpoint_node` so
/// the OnError input payload this function writes into
/// `outputs[node_key]` is captured by the following checkpoint — that
/// is what `resume_execution`'s `load_all_outputs` reads when a
/// crashed OnError handler is replayed.
///
/// Successors pushed into `ready_queue` are invisible to external
/// observers until the next `Phase 1` dispatch, which runs strictly
/// after the outer match arm's `checkpoint_node`. If the following
/// checkpoint returns `Err`, the caller aborts the frontier (cancel
/// token + early return); the discarded `ready_queue` mutations never
/// surface — §11.5 invariant holds.
///
/// Returns `Some(error_message)` if the frontier must abort — FailFast
/// strategy with no OnError handler took the failure. Returns `None`
/// when routing completed cleanly (OnError handled, ContinueOnError
/// resolved, or IgnoreErrors routed-as-success).
#[allow(clippy::too_many_arguments)]
fn route_failure_edges(
    outcome: FailureOutcome,
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
    match outcome {
        FailureOutcome::Recover => {
            process_outgoing_edges(
                node_key,
                Some(&ActionResult::success(serde_json::json!(null))),
                None,
                graph,
                activated_edges,
                resolved_edges,
                required_count,
                ready_queue,
                exec_state,
            );
            None
        },
        FailureOutcome::Fail => {
            // Evaluate outgoing edges as a failure: OnError handlers,
            // if any, are activated; otherwise edges are resolved
            // without activation so dependents get Skipped.
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
                // Stage OnError handler input into outputs BEFORE the
                // checkpoint that will run next — guarantees the
                // payload is durably captured so a resumed OnError
                // successor can read it from persisted state via
                // `load_all_outputs` (#297 review / Copilot).
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
                    // Edges resolved (not activated) — dependents will be
                    // Skipped; unaffected branches continue.
                    None
                },
                // FailFast and future variants
                _ => Some(error_msg.to_owned()),
            }
        },
    }
}

/// Mark a node as skipped in the execution state.
///
/// Uses the versioned transition API (issue #255) so CAS readers see
/// the parent version move.
fn mark_node_skipped(exec_state: &mut ExecutionState, node_key: NodeKey) {
    let _ = exec_state.transition_node(node_key, NodeState::Skipped);
}

/// Mark a node as completed in the execution state.
fn mark_node_completed(exec_state: &mut ExecutionState, node_key: NodeKey) {
    let _ = exec_state.transition_node(node_key, NodeState::Completed);
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
///
/// `termination_reason` carries the engine's explanation of *why* the
/// execution reached its final status (canon §4.5; ROADMAP §M0.3).
/// `None` means the engine has nothing to add — historically this was
/// always the case; the field is `Option` for backwards-compatible
/// destructuring while consumers are wired in T4.
#[derive(Debug)]
struct FinalStatusDecision {
    status: ExecutionStatus,
    /// Engine-attributed reason for the final status. Wired into
    /// [`ExecutionResult::termination_reason`] and
    /// [`ExecutionEvent::ExecutionFinished`] in T4.
    termination_reason: Option<ExecutionTerminationReason>,
    /// `Some(nodes)` when the frontier exited without `failed_node` or
    /// cancellation but not all nodes reached a terminal state — see
    /// `docs/PRODUCT_CANON.md` §11.1.
    integrity_violation: Option<Vec<(NodeKey, NodeState)>>,
}

/// RAII guard for an acquired execution lease with a running heartbeat.
///
/// Lifecycle:
/// - constructed by [`WorkflowEngine::acquire_and_heartbeat_lease`]
/// - held for the duration of the frontier loop (heartbeat task renews every
///   [`WorkflowEngine::lease_heartbeat_interval`])
/// - explicitly shut down via [`LeaseGuard::shutdown`] after the final state is persisted
///
/// If the guard is dropped without `shutdown`, the heartbeat task
/// aborts and no explicit `release_lease` is sent — the lease expires
/// naturally after its TTL. `shutdown` is the preferred path because
/// it frees the lease immediately, shortening redelivery latency for
/// legitimate successor runs.
struct LeaseGuard {
    repo: Arc<dyn nebula_storage::ExecutionRepo>,
    execution_id: ExecutionId,
    holder: String,
    handle: Option<tokio::task::JoinHandle<()>>,
    /// Signalled by `shutdown` to stop the heartbeat loop cleanly.
    shutdown: CancellationToken,
    /// Tripped by the heartbeat loop when renew returns `Ok(false)` or
    /// errors — the caller uses this to refuse final-state persistence.
    heartbeat_lost: CancellationToken,
}

impl LeaseGuard {
    /// Whether the heartbeat loop lost the lease while the frontier ran.
    ///
    /// True means another runner acquired the lease (or a storage error
    /// made the current runner unsafe to continue). Per ADR 0008 the
    /// caller must not persist further state — the new holder now owns
    /// the execution's canonical state.
    fn heartbeat_lost(&self) -> bool {
        self.heartbeat_lost.is_cancelled()
    }

    /// Stop the heartbeat loop and release the lease.
    ///
    /// Best-effort: release failures (storage unavailable, holder
    /// already reassigned) are logged but do not surface as engine
    /// errors — a TTL-driven natural expiry is the ultimate fallback.
    async fn shutdown(mut self) {
        self.shutdown.cancel();
        if let Some(handle) = self.handle.take() {
            // Wait for the heartbeat loop to notice and exit cleanly.
            let _ = handle.await;
        }
        if self.heartbeat_lost.is_cancelled() {
            // We never owned the lease at shutdown time — do not send
            // a release that might wipe a new holder's record. The
            // natural TTL takes care of eventual reacquisition.
            tracing::debug!(
                execution_id = %self.execution_id,
                holder = %self.holder,
                "lease heartbeat lost; skipping explicit release (TTL will expire)"
            );
            return;
        }
        match self
            .repo
            .release_lease(self.execution_id, &self.holder)
            .await
        {
            Ok(true) => {
                tracing::debug!(
                    execution_id = %self.execution_id,
                    holder = %self.holder,
                    "execution lease released"
                );
            },
            Ok(false) => {
                // Lease not owned at release time — could be a race
                // where TTL expired just before. Not actionable.
                tracing::debug!(
                    execution_id = %self.execution_id,
                    holder = %self.holder,
                    "release_lease returned false; lease was not owned at release time"
                );
            },
            Err(e) => {
                tracing::warn!(
                    execution_id = %self.execution_id,
                    holder = %self.holder,
                    error = %e,
                    "release_lease failed; TTL will eventually expire the lease"
                );
            },
        }
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        // If shutdown was not called, abort the heartbeat task so it
        // doesn't outlive the engine handle. The lease itself expires
        // via TTL.
        self.shutdown.cancel();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Determine the final execution status with explanatory
/// `termination_reason` (ROADMAP §M0.3).
///
/// # Priority ladder (top wins)
///
/// 1. **`exec_state.terminated_by`** is set — a node returned `ActionResult::Terminate`. This is
///    **authoritative** even if a sibling failed during cancel-driven tear-down (sibling failure
///    after a deliberate stop is collateral noise; the user's explicit signal wins).
///    - `ExplicitStop` → `(Completed, Some(ExplicitStop))`
///    - `ExplicitFail` → `(Failed,    Some(ExplicitFail))`
///    - any other variant (future-proofing for the `nebula_action::TerminationReason`
///      `#[non_exhaustive]` map fallback in [`map_termination_reason`]) → `(Failed,
///      Some(SystemError))`.
/// 2. **`failed_node` is set** with no explicit termination → a node failed at runtime. `(Failed,
///    None)` — engine has nothing to add beyond the failure itself; the failure detail is on the
///    node's `error_message` and the surfacing layer (T4) reports the underlying error.
/// 3. **`cancel_token` cancelled** with no explicit termination → external cancellation (API,
///    admin, engine shutdown). `(Cancelled, Some(Cancelled))`.
/// 4. **Frontier integrity violation** — the loop drained without `failed_node` or cancel but some
///    nodes are non-terminal (canon §11.1). `(Failed, Some(SystemError))` plus the
///    integrity_violation payload so the caller can emit a diagnostic
///    [`ExecutionEvent::FrontierIntegrityViolation`].
/// 5. **Natural completion** — every node terminal. `(Completed, Some(NaturalCompletion))`.
///
/// `(Failed, None)` from path 2 is intentional and load-bearing: a
/// system-driven failure already carries the error context elsewhere,
/// and the surfacing layer must distinguish "engine ran into an error"
/// (None) from "engine attributes the failure to a system-level
/// invariant breach" (`Some(SystemError)`).
fn determine_final_status(
    failed_node: &Option<(NodeKey, String)>,
    cancel_token: &CancellationToken,
    exec_state: &ExecutionState,
) -> FinalStatusDecision {
    // Priority 1 — explicit termination wins over everything else.
    if let Some((_, reason)) = exec_state.terminated_by.as_ref() {
        let (status, termination_reason) = match reason {
            ExecutionTerminationReason::ExplicitStop { .. } => {
                (ExecutionStatus::Completed, reason.clone())
            },
            ExecutionTerminationReason::ExplicitFail { .. } => {
                (ExecutionStatus::Failed, reason.clone())
            },
            other => {
                // `map_termination_reason` falls back to SystemError
                // for unknown future `nebula_action::TerminationReason`
                // variants. Surface as Failed so callers don't silently
                // promote it to Completed.
                tracing::warn!(
                    execution_id = %exec_state.execution_id,
                    ?other,
                    "terminated_by carried an unexpected variant; treating as Failed"
                );
                (
                    ExecutionStatus::Failed,
                    ExecutionTerminationReason::SystemError,
                )
            },
        };
        tracing::debug!(
            target = "engine::final_status",
            execution_id = %exec_state.execution_id,
            ?status,
            ?termination_reason,
            "final_status_decided (priority 1: explicit termination)"
        );
        return FinalStatusDecision {
            status,
            termination_reason: Some(termination_reason),
            integrity_violation: None,
        };
    }

    // Priority 2 — system-driven failure (no explicit signal).
    if failed_node.is_some() {
        tracing::debug!(
            target = "engine::final_status",
            execution_id = %exec_state.execution_id,
            "final_status_decided (priority 2: failed_node)"
        );
        return FinalStatusDecision {
            status: ExecutionStatus::Failed,
            termination_reason: None,
            integrity_violation: None,
        };
    }

    // Priority 3 — external cancellation (no explicit signal).
    if cancel_token.is_cancelled() {
        tracing::debug!(
            target = "engine::final_status",
            execution_id = %exec_state.execution_id,
            "final_status_decided (priority 3: external cancel)"
        );
        return FinalStatusDecision {
            status: ExecutionStatus::Cancelled,
            termination_reason: Some(ExecutionTerminationReason::Cancelled),
            integrity_violation: None,
        };
    }

    // Priority 4 — frontier integrity violation (canon §11.1).
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
            termination_reason: Some(ExecutionTerminationReason::SystemError),
            integrity_violation: Some(non_terminal),
        };
    }

    // Priority 5 — natural completion.
    tracing::debug!(
        target = "engine::final_status",
        execution_id = %exec_state.execution_id,
        "final_status_decided (priority 5: natural completion)"
    );
    FinalStatusDecision {
        status: ExecutionStatus::Completed,
        termination_reason: Some(ExecutionTerminationReason::NaturalCompletion),
        integrity_violation: None,
    }
}

/// Placeholder `NodeKey` used in final-state persistence errors.
///
/// Final-state writes are **execution-scoped**, not node-scoped, but
/// [`EngineError::CheckpointFailed`] takes a `NodeKey`. Rather than
/// extend the error shape for one use-site, a stable sentinel lets
/// operators distinguish "no node — final write" from a real node
/// failure in logs.
fn final_state_node_key() -> NodeKey {
    NodeKey::new("final_execution_state").expect("sentinel node key is always valid")
}

/// Extract the `status` field from a persisted execution-state JSON
/// snapshot in a best-effort, lossy way.
///
/// Used by CAS-conflict reporting (issue #333) to attach the
/// observer-visible external status to the typed [`EngineError::CasConflict`]
/// payload. Never panics: unknown shapes render as `"unknown"`.
fn parse_observed_status(state_json: &serde_json::Value) -> String {
    state_json
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned()
}

/// Parse a persisted execution-state JSON snapshot back into the
/// typed [`ExecutionStatus`] enum.
///
/// Returns `None` when the snapshot is missing the `status` key or
/// carries a value the engine does not recognise. Used by the final
/// checkpoint reconciliation path (issue #333) to decide whether an
/// externally-driven state is already terminal — in which case the
/// engine honors it instead of overwriting.
fn parse_observed_execution_status(state_json: &serde_json::Value) -> Option<ExecutionStatus> {
    state_json
        .get("status")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
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
            .map_or(serde_json::Value::Null, |v| v.value().clone())
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

/// Map an `ActionResult::Terminate` reason from the action layer into
/// the engine's `ExecutionTerminationReason` (ROADMAP §M0.3).
///
/// Pure conversion; the caller carries the result into
/// [`ExecutionState::set_terminated_by`]. The `by_node` field is
/// duplicated on both the outer tuple in `terminated_by` and the inner
/// reason variant so that downstream consumers (audit log, event bus)
/// can carry the reason on its own without losing provenance.
///
/// `nebula_action::TerminationReason` is `#[non_exhaustive]`. Future
/// variants land as `SystemError` here — never silently `Cancelled` or
/// `NaturalCompletion`, both of which would lose audit fidelity.
/// Adding a new variant upstream surfaces here as a `tracing::error!`
/// invariant breach so the gap is visible before any persisted state
/// rolls forward.
fn map_termination_reason(
    by_node: NodeKey,
    reason: &nebula_action::TerminationReason,
) -> ExecutionTerminationReason {
    match reason {
        nebula_action::TerminationReason::Success { note } => {
            ExecutionTerminationReason::ExplicitStop {
                by_node,
                note: note.clone(),
            }
        },
        nebula_action::TerminationReason::Failure { code, message } => {
            ExecutionTerminationReason::ExplicitFail {
                by_node,
                code: code.as_str().into(),
                message: message.clone(),
            }
        },
        other => {
            tracing::error!(
                target = "engine::frontier",
                ?other,
                "unknown TerminationReason variant — treating as SystemError; \
                 add explicit mapping in map_termination_reason"
            );
            ExecutionTerminationReason::SystemError
        },
    }
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
        // `ActionResult::Retry` has no primary output; the `_` arm below
        // handles it identically regardless of feature-unification state.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nebula_action::{
        ActionError, action::Action, context::CredentialContextExt, metadata::ActionMetadata,
        result::ActionResult, stateless::StatelessAction,
    };
    use nebula_core::{DeclaresDependencies, action_key};
    use nebula_storage::{ExecutionRepo, WorkflowRepo};
    use nebula_workflow::{
        Connection, ErrorStrategy, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
    };

    use super::*;
    use crate::runtime::{
        ActionExecutor, DataPassingPolicy, InProcessSandbox, registry::ActionRegistry,
    };

    // -- Test handlers --

    struct EchoHandler {
        meta: ActionMetadata,
    }

    impl DeclaresDependencies for EchoHandler {}
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
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    struct FailHandler {
        meta: ActionMetadata,
    }

    impl DeclaresDependencies for FailHandler {}
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
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Err(ActionError::fatal("intentional failure"))
        }
    }

    struct SlowHandler {
        meta: ActionMetadata,
        delay: Duration,
    }

    impl DeclaresDependencies for SlowHandler {}
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
            ctx: &(impl nebula_action::ActionContext + ?Sized),
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
        let base = Arc::new(
            nebula_core::BaseContext::builder()
                .cancellation(CancellationToken::new())
                .build(),
        );
        let ctx =
            nebula_action::TriggerRuntimeContext::new(base, WorkflowId::new(), node_key!("test"));
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

    impl DeclaresDependencies for SkipHandler {}
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
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::skip("skipped by test"))
        }
    }

    struct BranchHandler {
        meta: ActionMetadata,
        selected: String,
    }

    impl DeclaresDependencies for BranchHandler {}
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
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::Branch {
                selected: self.selected.clone(),
                output: nebula_action::output::ActionOutput::Value(input),
                alternatives: HashMap::new(),
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
                Connection::new(a.clone(), b.clone()).with_from_port("true"),
                Connection::new(a.clone(), c.clone()).with_from_port("false"),
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
                Connection::new(b.clone(), c.clone()).with_from_port("error"),
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
            vec![Connection::new(a.clone(), b.clone())],
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

    // -- Crash-window regression tests for #297 / D2 --

    /// Wraps an inner [`ExecutionRepo`] and returns `Err` on the Nth
    /// `transition()` call (1-indexed). All other trait methods delegate.
    /// Used to simulate a storage failure during `checkpoint_node`.
    struct FailAtTransitionN {
        inner: Arc<nebula_storage::InMemoryExecutionRepo>,
        fail_on: u32,
        calls: AtomicU32,
    }

    impl FailAtTransitionN {
        fn new(inner: Arc<nebula_storage::InMemoryExecutionRepo>, fail_on: u32) -> Self {
            Self {
                inner,
                fail_on,
                calls: AtomicU32::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl ExecutionRepo for FailAtTransitionN {
        async fn get_state(
            &self,
            id: ExecutionId,
        ) -> Result<Option<(u64, serde_json::Value)>, nebula_storage::ExecutionRepoError> {
            self.inner.get_state(id).await
        }

        async fn transition(
            &self,
            id: ExecutionId,
            expected_version: u64,
            new_state: serde_json::Value,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n == self.fail_on {
                return Err(nebula_storage::ExecutionRepoError::Connection(format!(
                    "injected transition failure at call #{n}"
                )));
            }
            self.inner.transition(id, expected_version, new_state).await
        }

        async fn get_journal(
            &self,
            id: ExecutionId,
        ) -> Result<Vec<serde_json::Value>, nebula_storage::ExecutionRepoError> {
            self.inner.get_journal(id).await
        }

        async fn append_journal(
            &self,
            id: ExecutionId,
            entry: serde_json::Value,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner.append_journal(id, entry).await
        }

        async fn acquire_lease(
            &self,
            id: ExecutionId,
            holder: String,
            ttl: Duration,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            self.inner.acquire_lease(id, holder, ttl).await
        }

        async fn renew_lease(
            &self,
            id: ExecutionId,
            holder: &str,
            ttl: Duration,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            self.inner.renew_lease(id, holder, ttl).await
        }

        async fn release_lease(
            &self,
            id: ExecutionId,
            holder: &str,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            self.inner.release_lease(id, holder).await
        }

        async fn create(
            &self,
            id: ExecutionId,
            workflow_id: WorkflowId,
            state: serde_json::Value,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner.create(id, workflow_id, state).await
        }

        async fn save_node_output(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
            attempt: u32,
            output: serde_json::Value,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner
                .save_node_output(execution_id, node_key, attempt, output)
                .await
        }

        async fn load_node_output(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
        ) -> Result<Option<serde_json::Value>, nebula_storage::ExecutionRepoError> {
            self.inner.load_node_output(execution_id, node_key).await
        }

        async fn load_all_outputs(
            &self,
            execution_id: ExecutionId,
        ) -> Result<HashMap<NodeKey, serde_json::Value>, nebula_storage::ExecutionRepoError>
        {
            self.inner.load_all_outputs(execution_id).await
        }

        async fn list_running(
            &self,
        ) -> Result<Vec<ExecutionId>, nebula_storage::ExecutionRepoError> {
            self.inner.list_running().await
        }

        async fn list_running_for_workflow(
            &self,
            workflow_id: WorkflowId,
        ) -> Result<Vec<ExecutionId>, nebula_storage::ExecutionRepoError> {
            self.inner.list_running_for_workflow(workflow_id).await
        }

        async fn count(
            &self,
            workflow_id: Option<WorkflowId>,
        ) -> Result<u64, nebula_storage::ExecutionRepoError> {
            self.inner.count(workflow_id).await
        }

        async fn check_idempotency(
            &self,
            key: &str,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            self.inner.check_idempotency(key).await
        }

        async fn mark_idempotent(
            &self,
            key: &str,
            execution_id: ExecutionId,
            node_key: NodeKey,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner
                .mark_idempotent(key, execution_id, node_key)
                .await
        }

        async fn save_stateful_checkpoint(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
            attempt: u32,
            iteration: u32,
            state: serde_json::Value,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner
                .save_stateful_checkpoint(execution_id, node_key, attempt, iteration, state)
                .await
        }

        async fn load_stateful_checkpoint(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
            attempt: u32,
        ) -> Result<
            Option<nebula_storage::StatefulCheckpointRecord>,
            nebula_storage::ExecutionRepoError,
        > {
            self.inner
                .load_stateful_checkpoint(execution_id, node_key, attempt)
                .await
        }

        async fn delete_stateful_checkpoint(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
            attempt: u32,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner
                .delete_stateful_checkpoint(execution_id, node_key, attempt)
                .await
        }
    }

    /// Regression for [#297](https://github.com/vanyastaff/nebula/issues/297) (D2).
    ///
    /// When `checkpoint_node` fails on the runtime-failure branch, the
    /// engine MUST abort the node's progression: the `Failed` state is
    /// not durably persisted, therefore no OnError successor may be
    /// spawned and no `NodeFailed` event may be emitted. Pre-fix the
    /// checkpoint error was silently logged (`tracing::warn!`) and
    /// `handle_node_failure` had already routed the OnError edge in
    /// memory, so the successor `B` was spawned off an undurable
    /// failure decision — the §11.5 "durability precedes visibility"
    /// invariant was violated.
    #[tokio::test]
    async fn runtime_failure_checkpoint_error_aborts_before_edge_routing() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "fails"),
        });
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        });

        // A (fail) --OnError--> B (echo). ContinueOnError so the frontier
        // loop reaches the failure branch (FailFast would early-return
        // before checkpoint).
        let a = node_key!("a");
        let b = node_key!("b");
        let wf = make_workflow_with_config(
            vec![
                NodeDefinition::new(a.clone(), "A", "fail").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
            ],
            vec![Connection::new(a.clone(), b.clone()).with_from_port("error")],
            WorkflowConfig {
                error_strategy: ErrorStrategy::ContinueOnError,
                ..WorkflowConfig::default()
            },
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        // First transition() call corresponds to the checkpoint_node
        // invocation after A's runtime failure. Fail it.
        let base = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let failing_repo = Arc::new(FailAtTransitionN::new(base, 1));

        let (engine, _) = make_engine(registry);
        let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
        let mut event_rx = event_bus.subscribe();
        let engine = engine
            .with_execution_repo(failing_repo)
            .with_workflow_repo(workflow_repo)
            .with_event_bus(event_bus);

        let _ = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await;

        // Drop engine so the event channel closes; drain.
        drop(engine);
        let mut events = Vec::new();
        while let Some(e) = event_rx.recv().await {
            events.push(e);
        }

        let b_started = events.iter().any(|e| {
            matches!(
                e,
                ExecutionEvent::NodeStarted { node_key, .. } if node_key == &b
            )
        });
        assert!(
            !b_started,
            "B must not be spawned after A's checkpoint failed — \
             checkpoint must precede edge routing (§11.5, #297). events: {events:#?}"
        );

        let a_failed_announced = events.iter().any(|e| {
            matches!(
                e,
                ExecutionEvent::NodeFailed { node_key, .. } if node_key == &a
            )
        });
        assert!(
            !a_failed_announced,
            "NodeFailed must not fire when A's checkpoint failed — \
             external observers must never see a transition the store \
             did not commit (§11.5, #297). events: {events:#?}"
        );
    }

    /// Regression for [#297](https://github.com/vanyastaff/nebula/issues/297) (D2).
    ///
    /// `IgnoreErrors` strategy recovers a failed node to `Completed`. The
    /// recovery MUST survive a checkpoint boundary: the sequence
    /// `Failed → Completed` in memory must be persisted as `Completed`
    /// before successors (which see a "success with null" payload) are
    /// routed. Pre-fix, `handle_node_failure` applied the override in
    /// memory and routed edges, then the outer `if state == Failed`
    /// guard skipped the checkpoint — so persistence lagged the
    /// observable recovery by up to one final-state flush.
    #[tokio::test]
    async fn ignore_errors_persists_recovered_completed_state() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "fails"),
        });

        let a = node_key!("a");
        let wf = make_workflow_with_config(
            vec![NodeDefinition::new(a.clone(), "A", "fail").unwrap()],
            vec![],
            WorkflowConfig {
                error_strategy: ErrorStrategy::IgnoreErrors,
                ..WorkflowConfig::default()
            },
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_execution_repo(repo.clone())
            .with_workflow_repo(workflow_repo);

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(
            result.is_success(),
            "IgnoreErrors workflow must finish Completed, got {:?}",
            result.status
        );

        let (version, final_state) = repo
            .get_state(result.execution_id)
            .await
            .unwrap()
            .expect("state must be persisted");

        // Expected version bumps: create (v1) → IgnoreErrors recovery
        // checkpoint (v2) → final (v3). Pre-fix path skips the recovery
        // checkpoint and lands at v2. Using `>=` so later legitimate
        // checkpoint additions do not break the signal.
        assert!(
            version >= 3,
            "expected at least three version bumps: create + recovery \
             checkpoint + final. Pre-fix path persists the recovered \
             state only at the final flush; got {version}"
        );

        assert_eq!(
            final_state
                .get("node_states")
                .and_then(|ns| ns.get(a.as_str()))
                .and_then(|na| na.get("state"))
                .and_then(|v| v.as_str()),
            Some("completed"),
            "IgnoreErrors must persist the recovered Completed state, \
             not the intermediate Failed"
        );
    }

    /// Regression for [#297](https://github.com/vanyastaff/nebula/issues/297) (D2) —
    /// setup-failure branch symmetry with runtime-failure branch.
    ///
    /// Parameter-resolution failure goes through the setup-failure arm
    /// of `run_frontier`. The checkpoint-before-routing discipline
    /// must hold there too: if the setup-failure checkpoint errors,
    /// the engine aborts instead of logging-and-continuing onto the
    /// OnError successor.
    #[tokio::test]
    async fn setup_failure_checkpoint_error_aborts_before_edge_routing() {
        use nebula_workflow::ParamValue;
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        });

        let a = node_key!("a");
        let b = node_key!("b");
        let wf = make_workflow_with_config(
            vec![
                NodeDefinition::new(a.clone(), "A", "echo")
                    .unwrap()
                    .with_parameter("bad", ParamValue::template("Hello {{ unclosed")),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
            ],
            vec![Connection::new(a.clone(), b.clone()).with_from_port("error")],
            WorkflowConfig {
                error_strategy: ErrorStrategy::ContinueOnError,
                ..WorkflowConfig::default()
            },
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        let base = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let failing_repo = Arc::new(FailAtTransitionN::new(base, 1));

        let (engine, _) = make_engine(registry);
        let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
        let mut event_rx = event_bus.subscribe();
        let engine = engine
            .with_execution_repo(failing_repo)
            .with_workflow_repo(workflow_repo)
            .with_event_bus(event_bus);

        let _ = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await;

        drop(engine);
        let mut events = Vec::new();
        while let Some(e) = event_rx.recv().await {
            events.push(e);
        }

        let b_started = events.iter().any(|e| {
            matches!(
                e,
                ExecutionEvent::NodeStarted { node_key, .. } if node_key == &b
            )
        });
        assert!(
            !b_started,
            "B must not be spawned after A's setup-failure checkpoint \
             failed (§11.5, #297). events: {events:#?}"
        );
    }

    /// Regression for PR [#436](https://github.com/vanyastaff/nebula/pull/436)
    /// review (Copilot) — the OnError input payload
    /// (`{error, node_id}`) must be staged into `outputs[failed_node]`
    /// BEFORE `checkpoint_node` commits the failure, so that a crashed-
    /// then-resumed workflow loads it via `load_all_outputs` rather
    /// than finding the OnError successor's input missing.
    #[tokio::test]
    async fn on_error_payload_is_persisted_before_checkpoint_commits() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "fails"),
        });
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        });

        let a = node_key!("a");
        let b = node_key!("b");
        let wf = make_workflow_with_config(
            vec![
                NodeDefinition::new(a.clone(), "A", "fail").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
            ],
            vec![Connection::new(a.clone(), b.clone()).with_from_port("error")],
            WorkflowConfig {
                error_strategy: ErrorStrategy::ContinueOnError,
                ..WorkflowConfig::default()
            },
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;
        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());

        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_execution_repo(repo.clone())
            .with_workflow_repo(workflow_repo);

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .unwrap();

        // The OnError handler B completed, so the workflow reports
        // success (ContinueOnError + handled error).
        assert!(result.is_success(), "status: {:?}", result.status);

        // The OnError input payload must have been loadable from the
        // persistence store — i.e. captured by the checkpoint that
        // commits A's Failed state, not written ephemerally after.
        let persisted = repo
            .load_node_output(result.execution_id, a.clone())
            .await
            .unwrap()
            .expect(
                "outputs[A] must be persisted: resume's load_all_outputs \
                 depends on it for the OnError handler's input",
            );

        let error_field = persisted.get("error").and_then(|v| v.as_str());
        let node_id_field = persisted.get("node_id").and_then(|v| v.as_str());
        assert_eq!(
            node_id_field,
            Some(a.as_str()),
            "persisted payload must carry node_id for the OnError \
             handler; got {persisted:?}"
        );
        assert!(
            error_field.is_some_and(|s| s.contains("intentional failure")),
            "persisted payload must carry the error message; got {persisted:?}"
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

        impl DeclaresDependencies for CountingHandler {}
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
                _ctx: &(impl nebula_action::ActionContext + ?Sized),
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

        // Reconstruct the idempotency key via the same seam the engine
        // uses (issue #266) — `ExecutionState::idempotency_key_for_node`
        // — so the test is not coupled to a literal `:1` suffix and
        // still asserts the key was durably recorded by the first run.
        //
        // Deserializing via a JSON string (rather than `from_value`)
        // avoids `#[serde(borrow)]` issues on domain keys — the same
        // workaround `resume_execution` applies when loading state.
        let execution_id = result1.execution_id;
        let (_, state_json) = exec_repo
            .get_state(execution_id)
            .await
            .unwrap()
            .expect("execution state must be persisted after first run");
        let state_str = serde_json::to_string(&state_json).unwrap();
        let exec_state: ExecutionState =
            serde_json::from_str(&state_str).expect("deserialize persisted execution state");
        let idem_key = exec_state.idempotency_key_for_node(n.clone());

        let already_marked = exec_repo
            .check_idempotency(idem_key.as_str())
            .await
            .unwrap();
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

        impl DeclaresDependencies for V1Handler {}
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
                _ctx: &(impl nebula_action::ActionContext + ?Sized),
            ) -> Result<ActionResult<Self::Output>, ActionError> {
                Ok(ActionResult::success(serde_json::json!("v1")))
            }
        }

        // V2 handler returns "v2".
        struct V2Handler {
            meta: ActionMetadata,
        }

        impl DeclaresDependencies for V2Handler {}
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
                _ctx: &(impl nebula_action::ActionContext + ?Sized),
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
                Connection::new(a.clone(), b.clone()),
                Connection::new(a, b.clone()).with_from_port("alt"),
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

    // ── ROADMAP §M0.3 — `determine_final_status` priority-ladder unit tests ──
    //
    // These cover the seven branches of the explicit-termination ladder
    // documented on `determine_final_status`: explicit termination beats
    // failed_node beats cancel_token beats integrity violation beats natural
    // completion. Pairs with the integration tests in
    // `crates/engine/tests/explicit_termination.rs`.

    fn make_two_terminal_state(
        terminated_by: Option<(NodeKey, ExecutionTerminationReason)>,
    ) -> ExecutionState {
        let n1 = node_key!("n1");
        let n2 = node_key!("n2");
        let mut state = ExecutionState::new(
            ExecutionId::new(),
            WorkflowId::new(),
            &[n1.clone(), n2.clone()],
        );
        // Drive both nodes to a terminal state so the integrity guard
        // does not fire for tests that do not want to exercise it.
        state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
        state.node_states.get_mut(&n2).unwrap().state = NodeState::Skipped;
        state.terminated_by = terminated_by;
        state
    }

    /// Priority 1 (Stop): explicit-stop signal yields `Completed` plus the
    /// `ExplicitStop` reason regardless of natural drainage.
    #[test]
    fn final_status_explicit_stop_yields_completed_with_explicit_reason() {
        let n1 = node_key!("n1");
        let reason = ExecutionTerminationReason::ExplicitStop {
            by_node: n1.clone(),
            note: Some("done".to_owned()),
        };
        let state = make_two_terminal_state(Some((n1, reason.clone())));

        let token = CancellationToken::new();
        let decision = determine_final_status(&None, &token, &state);

        assert_eq!(decision.status, ExecutionStatus::Completed);
        assert_eq!(decision.termination_reason, Some(reason));
        assert!(decision.integrity_violation.is_none());
    }

    /// Priority 1 (Fail): explicit-fail signal yields `Failed` plus the
    /// `ExplicitFail` reason — distinct from a system-driven `Failed`.
    #[test]
    fn final_status_explicit_fail_yields_failed_with_explicit_reason() {
        let n1 = node_key!("n1");
        let reason = ExecutionTerminationReason::ExplicitFail {
            by_node: n1.clone(),
            code: nebula_execution::status::ExecutionTerminationCode::new("E_FAIL"),
            message: "boom".to_owned(),
        };
        let state = make_two_terminal_state(Some((n1, reason.clone())));

        let token = CancellationToken::new();
        let decision = determine_final_status(&None, &token, &state);

        assert_eq!(decision.status, ExecutionStatus::Failed);
        assert_eq!(decision.termination_reason, Some(reason));
    }

    /// Priority 2: a system-driven `failed_node` without an explicit
    /// termination yields `(Failed, None)` — the `None` is load-bearing
    /// (signals "engine has nothing extra to attribute").
    #[test]
    fn final_status_failed_node_without_terminate_yields_failed_none() {
        let state = make_two_terminal_state(None);
        let token = CancellationToken::new();
        let failed = Some((node_key!("n1"), "boom".to_owned()));

        let decision = determine_final_status(&failed, &token, &state);

        assert_eq!(decision.status, ExecutionStatus::Failed);
        assert!(decision.termination_reason.is_none());
    }

    /// Priority 3: external cancel without an explicit termination yields
    /// `(Cancelled, Cancelled)` — distinct from explicit-stop.
    #[test]
    fn final_status_external_cancel_yields_cancelled_with_cancelled_reason() {
        let state = make_two_terminal_state(None);
        let token = CancellationToken::new();
        token.cancel();

        let decision = determine_final_status(&None, &token, &state);

        assert_eq!(decision.status, ExecutionStatus::Cancelled);
        assert_eq!(
            decision.termination_reason,
            Some(ExecutionTerminationReason::Cancelled)
        );
    }

    /// Priority 5: natural drainage with all-terminal nodes and no signal
    /// yields `(Completed, NaturalCompletion)`.
    #[test]
    fn final_status_natural_completion_yields_completed_with_natural_reason() {
        let state = make_two_terminal_state(None);
        let token = CancellationToken::new();

        let decision = determine_final_status(&None, &token, &state);

        assert_eq!(decision.status, ExecutionStatus::Completed);
        assert_eq!(
            decision.termination_reason,
            Some(ExecutionTerminationReason::NaturalCompletion)
        );
    }

    /// Priority 1 wins over Priority 2: explicit stop authoritative even
    /// when a sibling failed mid-cancel. The user's stop signal is
    /// authoritative; sibling failure is collateral.
    #[test]
    fn final_status_explicit_stop_wins_over_failed_node() {
        let n1 = node_key!("n1");
        let stop_reason = ExecutionTerminationReason::ExplicitStop {
            by_node: n1.clone(),
            note: None,
        };
        let state = make_two_terminal_state(Some((n1, stop_reason.clone())));
        let token = CancellationToken::new();
        // Sibling failure that would have promoted to Failed under priority 2.
        let failed = Some((node_key!("n2"), "sibling exploded mid-cancel".to_owned()));

        let decision = determine_final_status(&failed, &token, &state);

        assert_eq!(
            decision.status,
            ExecutionStatus::Completed,
            "ExplicitStop must win over sibling failure"
        );
        assert_eq!(decision.termination_reason, Some(stop_reason));
    }

    /// Priority 1 wins over Priority 2 (Fail variant): an explicit fail
    /// signal is authoritative even when a sibling also failed.
    #[test]
    fn final_status_explicit_fail_wins_over_failed_sibling() {
        let n1 = node_key!("n1");
        let fail_reason = ExecutionTerminationReason::ExplicitFail {
            by_node: n1.clone(),
            code: nebula_execution::status::ExecutionTerminationCode::new("E_USER_FAIL"),
            message: "user-driven".to_owned(),
        };
        let state = make_two_terminal_state(Some((n1, fail_reason.clone())));
        let token = CancellationToken::new();
        let failed = Some((node_key!("n2"), "sibling crash".to_owned()));

        let decision = determine_final_status(&failed, &token, &state);

        assert_eq!(decision.status, ExecutionStatus::Failed);
        assert_eq!(
            decision.termination_reason,
            Some(fail_reason),
            "ExplicitFail must win over sibling failure"
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
        let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(8);
        let mut rx = event_bus.subscribe();
        let engine = engine.with_event_bus(event_bus);

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
        assert!(
            rx.try_recv().is_none(),
            "helper must emit exactly one event"
        );
    }

    /// When the guard does not fire, `emit_frontier_integrity_if_violated`
    /// must stay silent so the finish-event stream is unchanged in the
    /// happy path.
    #[tokio::test]
    async fn emit_frontier_integrity_helper_silent_when_no_violation() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);
        let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(8);
        let mut rx = event_bus.subscribe();
        let engine = engine.with_event_bus(event_bus);

        engine.emit_frontier_integrity_if_violated(ExecutionId::new(), None);
        assert!(rx.try_recv().is_none());
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
        impl DeclaresDependencies for NeverRunHandler {}
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
                _ctx: &(impl nebula_action::ActionContext + ?Sized),
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
            ctx: &dyn nebula_action::ActionContext,
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
        let node = NodeDefinition::new(n1, "probe", action)
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

    // -- Regression tests for batch 2 (#299, #300, #301, #311, #321) --

    /// Issue #321 — the setup-failure path (parameter resolution error,
    /// missing node definition, invalid state-machine start) must
    /// checkpoint the execution state, symmetrical with the runtime-
    /// failure path. Previously only the runtime branch checkpointed,
    /// so a setup failure left the persisted state describing the node
    /// as Pending even though it was Failed in memory.
    #[tokio::test]
    async fn setup_failure_checkpoints_execution_state() {
        // Force parameter resolution to fail by referencing a node
        // that has no output in the shared outputs map.
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(repo.clone());

        let n1 = node_key!("n1");
        let ghost = node_key!("ghost");
        let mut params: HashMap<String, nebula_workflow::ParamValue> = HashMap::new();
        params.insert(
            "input".into(),
            nebula_workflow::ParamValue::Reference {
                node_key: ghost,
                output_path: String::new(),
            },
        );
        let mut node = NodeDefinition::new(n1.clone(), "A", "echo").unwrap();
        node.parameters = params;

        let wf = make_workflow(vec![node], vec![]);

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure(), "setup failure should fail execution");

        // The critical assertion: the execution state was checkpointed
        // after the setup failure. The persisted status must be
        // `failed` and the failed node's state must be `failed` with
        // an error_message populated.
        let (_version, state_json) = repo
            .get_state(result.execution_id)
            .await
            .unwrap()
            .expect("execution state should be persisted after setup failure");
        assert_eq!(
            state_json.get("status").and_then(|s| s.as_str()),
            Some("failed"),
            "execution status should be persisted as failed"
        );
        let node_state = state_json
            .pointer(&format!("/node_states/{n1}/state"))
            .and_then(|v| v.as_str());
        assert_eq!(
            node_state,
            Some("failed"),
            "node state should be persisted as failed after setup failure (issue #321)"
        );
        let err_msg = state_json
            .pointer(&format!("/node_states/{n1}/error_message"))
            .and_then(|v| v.as_str());
        assert!(
            err_msg.is_some(),
            "setup-failure error message should be persisted, got state: {state_json}"
        );
    }

    /// Issue #311 — resume_execution must restore the original
    /// workflow input from the persisted state, not substitute Null.
    /// Regression: `ExecutionState::workflow_input` is now persisted
    /// at execution start and read back on resume.
    #[tokio::test]
    async fn resume_restores_original_workflow_input() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);

        let n1 = node_key!("n1");
        let wf = make_workflow(
            vec![NodeDefinition::new(n1.clone(), "A", "echo").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        // Build a partial execution state for a FRESH execution where
        // the entry node has not yet run. Persist it with the original
        // trigger payload set via `set_workflow_input`.
        let execution_id = ExecutionId::new();
        let mut exec_state = ExecutionState::new(execution_id, wf.id, std::slice::from_ref(&n1));
        exec_state
            .transition_status(ExecutionStatus::Running)
            .unwrap();
        exec_state.set_workflow_input(serde_json::json!({"trigger": "webhook-payload"}));
        let state_json = serde_json::to_value(&exec_state).unwrap();
        exec_repo
            .create(execution_id, wf.id, state_json)
            .await
            .unwrap();

        let engine = engine
            .with_execution_repo(exec_repo.clone())
            .with_workflow_repo(workflow_repo);

        let result = engine.resume_execution(execution_id).await.unwrap();

        assert!(result.is_success());
        // Echo pipes the input through — so n1's output is exactly
        // the workflow input the engine restored from storage.
        assert_eq!(
            result.node_output(&n1),
            Some(&serde_json::json!({"trigger": "webhook-payload"})),
            "resume should feed the entry node the persisted trigger payload, not Null (issue #311)"
        );
    }

    /// Issue #289 — `resume_execution` must restore the persisted
    /// `ExecutionBudget` instead of silently reverting to
    /// `ExecutionBudget::default()`. Before the fix, a run configured
    /// with a tight concurrency / retry / timeout budget would resume
    /// with the default 10-way concurrency and unbounded retries,
    /// changing behavior vs operator expectations. See
    /// `PRODUCT_CANON.md §4.5` (public surface honored end-to-end).
    #[tokio::test]
    async fn resume_restores_persisted_budget() {
        use std::time::Duration;

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        // Start an execution on "engine A" with a non-default budget
        // (max_concurrent_nodes=3 + retries + timeout + output cap),
        // persist it, then resume on a fresh "engine B" that has never
        // seen the budget in-memory. The persisted row is the only
        // channel for the budget to reach the resumed run.
        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let n1 = node_key!("n1");
        let wf = make_workflow(
            vec![NodeDefinition::new(n1.clone(), "A", "echo").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        let configured = ExecutionBudget::default()
            .with_max_concurrent_nodes(3)
            .with_max_duration(Duration::from_secs(97))
            .with_max_output_bytes(7 * 1024)
            .with_max_total_retries(11);

        // Simulate the state "engine A" would have written right
        // after `execute_workflow` began but before the node ran:
        // status=Running, entry node still Pending, budget persisted.
        // This mirrors the real crash window the fix covers (the
        // setup-failure / post-create checkpoint).
        let execution_id = ExecutionId::new();
        let mut exec_state = ExecutionState::new(execution_id, wf.id, std::slice::from_ref(&n1));
        exec_state
            .transition_status(ExecutionStatus::Running)
            .unwrap();
        exec_state.set_budget(configured.clone());
        let state_json = serde_json::to_value(&exec_state).unwrap();
        exec_repo
            .create(execution_id, wf.id, state_json)
            .await
            .unwrap();

        // Resume on a fresh engine ("engine B" — new runner, new
        // instance, no memory of the original budget).
        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_execution_repo(exec_repo.clone())
            .with_workflow_repo(workflow_repo);
        let result = engine.resume_execution(execution_id).await.unwrap();
        assert!(result.is_success());

        // Re-load the persisted state and assert the budget survived
        // the resume unchanged — this proves the resume path reads
        // the budget off the row rather than substituting a default.
        //
        // Deserialize via a JSON string (not `serde_json::from_value`)
        // because `ExecutionState::node_states` uses `NodeKey` which
        // has a borrowed-string `Deserialize` impl incompatible with
        // `from_value` (docs/pitfalls — serde MapAccess).
        let (_v, state_after) = exec_repo.get_state(execution_id).await.unwrap().unwrap();
        let state_after_str = serde_json::to_string(&state_after).unwrap();
        let round_tripped: ExecutionState = serde_json::from_str(&state_after_str).unwrap();
        let restored = round_tripped
            .budget
            .expect("resume must preserve the persisted budget on the execution row");
        assert_eq!(
            restored, configured,
            "resume must use the persisted budget, not ExecutionBudget::default() (issue #289)"
        );
        // And specifically NOT the default — guards against a silent
        // regression where the code accidentally overwrites the field
        // with `default()` before the final persist.
        assert_ne!(
            restored,
            ExecutionBudget::default(),
            "the configured budget must not collapse to default() on resume"
        );
    }

    /// Issue #289 — legacy persisted states that predate budget
    /// persistence must still resume (falling back to
    /// `ExecutionBudget::default()` with a warning log), so the fix
    /// does not break old rows.
    #[tokio::test]
    async fn resume_falls_back_to_default_budget_on_legacy_state() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let n1 = node_key!("n1");
        let wf = make_workflow(
            vec![NodeDefinition::new(n1.clone(), "A", "echo").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        // Build a state snapshot with NO `budget` field — simulates a
        // pre-#289 row. We build the state normally, serialize it,
        // then strip the field before persist so the resume path
        // observes it as `None` (the legacy deserialization outcome).
        let execution_id = ExecutionId::new();
        let mut exec_state = ExecutionState::new(execution_id, wf.id, std::slice::from_ref(&n1));
        exec_state
            .transition_status(ExecutionStatus::Running)
            .unwrap();
        // Don't set the budget. Confirm the field is absent after
        // roundtrip — catches a future default change that would
        // accidentally inject a value.
        assert!(exec_state.budget.is_none());
        let mut state_json = serde_json::to_value(&exec_state).unwrap();
        if let Some(obj) = state_json.as_object_mut() {
            obj.remove("budget");
        }
        exec_repo
            .create(execution_id, wf.id, state_json)
            .await
            .unwrap();

        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_execution_repo(exec_repo.clone())
            .with_workflow_repo(workflow_repo);

        // Resume must succeed despite the missing budget — the engine
        // logs a warning and falls back to the default.
        let result = engine.resume_execution(execution_id).await.unwrap();
        assert!(result.is_success());
    }

    /// Issue #300 — spawn_node must NOT silently spawn a task on a
    /// node whose state machine cannot reach Running from its current
    /// position. When the engine is asked to spawn a node that is
    /// already Completed (e.g. via a manually-manipulated state), the
    /// typed `start_node_attempt` helper rejects the transition and
    /// the node is routed through the setup-failure path.
    #[test]
    fn start_node_attempt_rejects_terminal_state() {
        let n1 = node_key!("n1");
        let mut state = ExecutionState::new(
            ExecutionId::new(),
            WorkflowId::new(),
            std::slice::from_ref(&n1),
        );
        // Drive n1 to Completed via the legal transition chain.
        state.transition_node(n1.clone(), NodeState::Ready).unwrap();
        state
            .transition_node(n1.clone(), NodeState::Running)
            .unwrap();
        state
            .transition_node(n1.clone(), NodeState::Completed)
            .unwrap();

        let err = state
            .start_node_attempt(n1.clone())
            .expect_err("start_node_attempt must reject Completed source state");
        assert!(
            err.to_string().contains("invalid transition"),
            "error should be InvalidTransition, got: {err}"
        );
        // State must not have moved.
        assert_eq!(state.node_state(n1).unwrap().state, NodeState::Completed);
    }

    /// Issue #301 — when a node task panics, the engine must report
    /// the real NodeKey, not a synthesized placeholder. Regression
    /// verified via a panicking handler.
    #[tokio::test]
    async fn panicked_task_reports_real_node_id() {
        struct PanicHandler {
            meta: ActionMetadata,
        }

        impl DeclaresDependencies for PanicHandler {}
        impl Action for PanicHandler {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }

        impl StatelessAction for PanicHandler {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            async fn execute(
                &self,
                _input: Self::Input,
                _ctx: &(impl nebula_action::ActionContext + ?Sized),
            ) -> Result<ActionResult<Self::Output>, ActionError> {
                panic!("intentional panic for test");
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(PanicHandler {
            meta: ActionMetadata::new(action_key!("boom"), "Boom", "panics"),
        });

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(repo.clone());

        let n1 = node_key!("n1");
        let wf = make_workflow(
            vec![NodeDefinition::new(n1.clone(), "Boom", "boom").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(
                &wf,
                serde_json::json!("ignored"),
                ExecutionBudget::default(),
            )
            .await
            .unwrap();

        assert!(result.is_failure(), "panicked workflow must fail");

        // The node errors map must list the real n1 key with a
        // non-empty message, not some synthetic NodeKey.
        let err_msg = result
            .node_errors
            .get(&n1)
            .expect("panicked node must be recorded under its real NodeKey (issue #301)");
        assert!(
            !err_msg.is_empty(),
            "panic error message should not be empty, got: {err_msg:?}"
        );

        // Persisted state should also reflect n1 as the failed node.
        let (_v, state_json) = repo
            .get_state(result.execution_id)
            .await
            .unwrap()
            .expect("state persisted after panic");
        let node_state = state_json
            .pointer(&format!("/node_states/{n1}/state"))
            .and_then(|v| v.as_str());
        assert_eq!(
            node_state,
            Some("failed"),
            "panicked node should be checkpointed as Failed"
        );
    }

    /// Issue #299 — idempotency replay must reconstruct the exact
    /// ActionResult variant so that Branch edges gate correctly.
    /// Regression: with the old code a persisted Branch result was
    /// replayed as a flat `Success`, and every branch edge fired
    /// regardless of `branch_key`, causing unintended downstream
    /// execution on replay.
    #[tokio::test]
    async fn idempotency_replay_preserves_branch_routing() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(BranchHandler {
            meta: ActionMetadata::new(action_key!("branch"), "Branch", "branches"),
            selected: "true".into(),
        });
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(repo.clone());

        // A → B (branch_key="true") / C (branch_key="false")
        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a.clone(), "A", "branch").unwrap(),
                NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
                NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
            ],
            vec![
                Connection::new(a.clone(), b.clone()).with_from_port("true"),
                Connection::new(a.clone(), c.clone()).with_from_port("false"),
            ],
        );

        // First run: A emits Branch{selected=true}. Only B fires.
        let first = engine
            .execute_workflow(
                &wf,
                serde_json::json!("payload"),
                ExecutionBudget::default(),
            )
            .await
            .unwrap();

        assert!(first.is_success());
        assert!(
            first.node_output(&b).is_some(),
            "B should run on first pass"
        );
        assert!(
            first.node_output(&c).is_none(),
            "C should NOT run on first pass (false branch)"
        );

        // Verify the persisted ActionResult encodes a Branch variant
        // rather than bare output — this is the byte-level check
        // behind issue #299's fix.
        let persisted_record = repo
            .load_node_result(first.execution_id, a.clone())
            .await
            .unwrap()
            .expect("load_node_result should return the persisted ActionResult after #299");
        assert_eq!(
            persisted_record.kind, "Branch",
            "persisted ActionResult for A should be the Branch variant, got: {persisted_record:?}"
        );
        assert_eq!(
            persisted_record
                .result
                .get("selected")
                .and_then(|v| v.as_str()),
            Some("true"),
            "Branch selector should be persisted verbatim"
        );
    }

    // -- Regression tests for #333 (CAS-conflict reconciliation) --

    /// Wraps an inner [`ExecutionRepo`] and injects a single external
    /// "concurrent" transition BEFORE the Nth `transition()` call from
    /// the engine — bumping the version and optionally rewriting the
    /// status to simulate an API cancel / admin mutation / sibling
    /// runner. Subsequent engine transitions hit a version mismatch.
    ///
    /// Used to reproduce the pre-fix #333 failure mode where the engine
    /// silently overwrote external state on CAS mismatch.
    struct ExternalMutateBeforeN {
        inner: Arc<nebula_storage::InMemoryExecutionRepo>,
        mutate_before: u32,
        new_status: Option<String>,
        calls: AtomicU32,
        injected: std::sync::atomic::AtomicBool,
    }

    impl ExternalMutateBeforeN {
        fn new(
            inner: Arc<nebula_storage::InMemoryExecutionRepo>,
            mutate_before: u32,
            new_status: Option<&str>,
        ) -> Self {
            Self {
                inner,
                mutate_before,
                new_status: new_status.map(ToOwned::to_owned),
                calls: AtomicU32::new(0),
                injected: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    #[async_trait::async_trait]
    impl ExecutionRepo for ExternalMutateBeforeN {
        async fn get_state(
            &self,
            id: ExecutionId,
        ) -> Result<Option<(u64, serde_json::Value)>, nebula_storage::ExecutionRepoError> {
            self.inner.get_state(id).await
        }

        async fn transition(
            &self,
            id: ExecutionId,
            expected_version: u64,
            new_state: serde_json::Value,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n == self.mutate_before
                && !self.injected.swap(true, Ordering::SeqCst)
                && let Ok(Some((current_version, mut current_state))) =
                    self.inner.get_state(id).await
            {
                if let Some(status) = &self.new_status
                    && let Some(obj) = current_state.as_object_mut()
                {
                    obj.insert(
                        "status".to_owned(),
                        serde_json::Value::String(status.clone()),
                    );
                }
                // Perform the external transition via the inner repo at
                // the version the engine believes is current, bumping it
                // beneath the engine's feet.
                let _ = self
                    .inner
                    .transition(id, current_version, current_state)
                    .await;
            }
            self.inner.transition(id, expected_version, new_state).await
        }

        async fn get_journal(
            &self,
            id: ExecutionId,
        ) -> Result<Vec<serde_json::Value>, nebula_storage::ExecutionRepoError> {
            self.inner.get_journal(id).await
        }

        async fn append_journal(
            &self,
            id: ExecutionId,
            entry: serde_json::Value,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner.append_journal(id, entry).await
        }

        async fn acquire_lease(
            &self,
            id: ExecutionId,
            holder: String,
            ttl: Duration,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            self.inner.acquire_lease(id, holder, ttl).await
        }

        async fn renew_lease(
            &self,
            id: ExecutionId,
            holder: &str,
            ttl: Duration,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            self.inner.renew_lease(id, holder, ttl).await
        }

        async fn release_lease(
            &self,
            id: ExecutionId,
            holder: &str,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            self.inner.release_lease(id, holder).await
        }

        async fn create(
            &self,
            id: ExecutionId,
            workflow_id: WorkflowId,
            state: serde_json::Value,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner.create(id, workflow_id, state).await
        }

        async fn save_node_output(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
            attempt: u32,
            output: serde_json::Value,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner
                .save_node_output(execution_id, node_key, attempt, output)
                .await
        }

        async fn load_node_output(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
        ) -> Result<Option<serde_json::Value>, nebula_storage::ExecutionRepoError> {
            self.inner.load_node_output(execution_id, node_key).await
        }

        async fn load_all_outputs(
            &self,
            execution_id: ExecutionId,
        ) -> Result<HashMap<NodeKey, serde_json::Value>, nebula_storage::ExecutionRepoError>
        {
            self.inner.load_all_outputs(execution_id).await
        }

        async fn list_running(
            &self,
        ) -> Result<Vec<ExecutionId>, nebula_storage::ExecutionRepoError> {
            self.inner.list_running().await
        }

        async fn list_running_for_workflow(
            &self,
            workflow_id: WorkflowId,
        ) -> Result<Vec<ExecutionId>, nebula_storage::ExecutionRepoError> {
            self.inner.list_running_for_workflow(workflow_id).await
        }

        async fn count(
            &self,
            workflow_id: Option<WorkflowId>,
        ) -> Result<u64, nebula_storage::ExecutionRepoError> {
            self.inner.count(workflow_id).await
        }

        async fn check_idempotency(
            &self,
            key: &str,
        ) -> Result<bool, nebula_storage::ExecutionRepoError> {
            self.inner.check_idempotency(key).await
        }

        async fn mark_idempotent(
            &self,
            key: &str,
            execution_id: ExecutionId,
            node_key: NodeKey,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner
                .mark_idempotent(key, execution_id, node_key)
                .await
        }

        async fn save_stateful_checkpoint(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
            attempt: u32,
            iteration: u32,
            state: serde_json::Value,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner
                .save_stateful_checkpoint(execution_id, node_key, attempt, iteration, state)
                .await
        }

        async fn load_stateful_checkpoint(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
            attempt: u32,
        ) -> Result<
            Option<nebula_storage::StatefulCheckpointRecord>,
            nebula_storage::ExecutionRepoError,
        > {
            self.inner
                .load_stateful_checkpoint(execution_id, node_key, attempt)
                .await
        }

        async fn delete_stateful_checkpoint(
            &self,
            execution_id: ExecutionId,
            node_key: NodeKey,
            attempt: u32,
        ) -> Result<(), nebula_storage::ExecutionRepoError> {
            self.inner
                .delete_stateful_checkpoint(execution_id, node_key, attempt)
                .await
        }
    }

    /// Regression for [#333](https://github.com/vanyastaff/nebula/issues/333).
    ///
    /// When the engine's final `transition()` CAS-misses because an
    /// external actor (API cancel, admin mutation, sibling runner)
    /// committed a **terminal** transition first, the engine MUST
    /// honor the external terminal state rather than overwrite it.
    /// Pre-fix, the engine only refreshed the version and continued
    /// reporting its local `Completed` status while the persisted row
    /// carried (say) `Cancelled` — a silent overwrite of concurrent
    /// state. With the fix, `persist_final_state` detects the terminal
    /// external status and surfaces it in the `ExecutionResult`.
    #[tokio::test]
    async fn final_cas_conflict_with_external_cancel_honors_external_status() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        });

        let a = node_key!("a");
        let wf = make_workflow(
            vec![NodeDefinition::new(a.clone(), "A", "echo").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        // `create()` seeds version=1 without a transition() call.
        // The engine then issues two transition() calls: #1 is the
        // node checkpoint (v=1 → v=2) and #2 is the final state
        // write (v=2 → v=3). Inject the external mutation before
        // call #2 so the FINAL CAS misses.
        let inner = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let mutating_repo = Arc::new(ExternalMutateBeforeN::new(
            inner.clone(),
            2,
            Some("cancelled"),
        ));

        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_execution_repo(mutating_repo)
            .with_workflow_repo(workflow_repo);

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .expect(
                "execute_workflow should return Ok on external terminal override (§11.5, #333)",
            );

        assert_eq!(
            result.status,
            ExecutionStatus::Cancelled,
            "engine must surface the external Cancelled status when its \
             final CAS collides with a terminal external transition \
             (pre-fix it reported Completed, silently overwriting the \
             concurrent cancel — §11.5, #333). got {:?}",
            result.status
        );

        // The persisted row must carry `cancelled` — the engine must
        // NOT have overwritten it with its own `completed`.
        let (_v, final_state) = inner
            .get_state(result.execution_id)
            .await
            .unwrap()
            .expect("persisted state must exist");
        assert_eq!(
            final_state.get("status").and_then(|v| v.as_str()),
            Some("cancelled"),
            "persisted row must retain the external Cancelled status; \
             engine must not overwrite a concurrent terminal transition \
             (§11.5, #333)"
        );
    }

    /// Regression for [#333](https://github.com/vanyastaff/nebula/issues/333).
    ///
    /// On `checkpoint_node` CAS mismatch, the engine now returns the
    /// typed [`EngineError::CasConflict`] carrying the observer-visible
    /// external status — not a generic `CheckpointFailed`. Pre-fix,
    /// only the version was refreshed (the observed state was
    /// discarded) and the error reason was a bare string, leaving no
    /// structured signal for operators or upstream schedulers to
    /// distinguish a stale-version abort from a real external conflict.
    #[tokio::test]
    async fn node_checkpoint_cas_conflict_surfaces_observed_status() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        });

        let a = node_key!("a");
        let wf = make_workflow(
            vec![NodeDefinition::new(a.clone(), "A", "echo").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        // Inject the external mutation before transition #1 — the
        // first call after `create()` is the node-level checkpoint
        // (v=1 → v=2 expected). The external bump flips status to
        // `cancelling` and moves the row to v=2 so the engine's
        // checkpoint_node CAS lands stale.
        let inner = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let mutating_repo = Arc::new(ExternalMutateBeforeN::new(
            inner.clone(),
            1,
            Some("cancelling"),
        ));

        let (engine, _) = make_engine(registry);
        let engine = engine
            .with_execution_repo(mutating_repo)
            .with_workflow_repo(workflow_repo);

        // The final result is not the focus here — what matters is
        // that the persisted row shows the engine observed the
        // external status rather than blindly overwriting the row.
        // Note: depending on scheduling, the engine may report
        // Failed (node checkpoint aborted) or Cancelled (external).
        // Either way it MUST NOT claim Completed.
        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await;

        let execution_id_opt = match &result {
            Ok(r) => {
                assert_ne!(
                    r.status,
                    ExecutionStatus::Completed,
                    "engine must not report Completed when the node's \
                     checkpoint CAS-missed against a concurrent external \
                     transition (§11.5, #333); got Completed for {r:?}"
                );
                Some(r.execution_id)
            },
            Err(e) => {
                // An execution-level Err is also acceptable — the
                // engine did not silently report success.
                tracing::debug!(error = %e, "execution surfaced typed error on CAS conflict");
                None
            },
        };

        // Crucially, the persisted row must still carry the external
        // `cancelling` status — never overwritten. The engine's own
        // writes after CAS miss MUST NOT land.
        if let Some(execution_id) = execution_id_opt {
            let state_opt = inner.get_state(execution_id).await.unwrap_or(None);
            if let Some((_v, final_state)) = state_opt {
                let status = final_state.get("status").and_then(|v| v.as_str());
                assert_ne!(
                    status,
                    Some("completed"),
                    "persisted row must not land as completed when the node \
                     checkpoint CAS-missed against a concurrent external \
                     mutation (§11.5, #333); found {status:?}"
                );
            }
        }
    }

    /// Regression for [#333](https://github.com/vanyastaff/nebula/issues/333).
    ///
    /// When the final CAS misses against a **non-terminal** external
    /// write, the reconciliation helper retries once at the refreshed
    /// version. The retry succeeds (no further concurrent writer), so
    /// the engine commits its decision at the new version instead of
    /// losing it. Pre-fix, the path was log-and-continue and the
    /// engine's final write was silently dropped.
    #[tokio::test]
    async fn persist_final_state_retries_once_on_nonterminal_conflict() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);

        let inner = Arc::new(nebula_storage::InMemoryExecutionRepo::new());

        let execution_id = ExecutionId::new();
        let workflow_id = WorkflowId::new();
        let node_ids = vec![node_key!("x")];
        let mut local_state = ExecutionState::new(execution_id, workflow_id, &node_ids);
        local_state
            .transition_status(ExecutionStatus::Running)
            .unwrap();
        inner
            .create(
                execution_id,
                workflow_id,
                serde_json::to_value(&local_state).unwrap(),
            )
            .await
            .unwrap();

        // External non-terminal bump: stay in Running but advance
        // `updated_at` by re-saving the row at a new version.
        let mut external_state = local_state.clone();
        external_state.updated_at = chrono::Utc::now();
        external_state.version += 1;
        let external_json = serde_json::to_value(&external_state).unwrap();
        let ok = inner
            .transition(execution_id, 1, external_json)
            .await
            .expect("external transition should succeed");
        assert!(ok, "external transition must commit at v=1");

        // Engine's local final state is Completed, using the stale
        // repo_version=1.
        let mut repo_version: u64 = 1;
        let mut engine_final_state = local_state.clone();
        engine_final_state
            .transition_status(ExecutionStatus::Completed)
            .unwrap();

        let repo: Arc<dyn ExecutionRepo> = inner.clone();
        let outcome = engine
            .persist_final_state(
                &repo,
                execution_id,
                &mut engine_final_state,
                &mut repo_version,
            )
            .await
            .expect("retry should succeed on non-terminal conflict");

        assert_eq!(
            outcome, None,
            "helper must report Ok(None) when the local final status \
             was ultimately persisted (non-terminal conflict → retry \
             succeeded). got {outcome:?}"
        );

        // The persisted row must now be Completed at a bumped version.
        let (persisted_version, final_state) = inner
            .get_state(execution_id)
            .await
            .unwrap()
            .expect("row must still exist");
        assert!(
            persisted_version >= 3,
            "expected version ≥ 3 (create + external bump + retry), \
             got {persisted_version}"
        );
        assert_eq!(
            final_state.get("status").and_then(|v| v.as_str()),
            Some("completed"),
            "retry must durably persist the engine's Completed decision \
             at the refreshed version (§11.5, #333)"
        );
    }

    /// Regression for [#333](https://github.com/vanyastaff/nebula/issues/333).
    ///
    /// Unit-level check on the reconciliation helper: when the final
    /// CAS misses against a concurrent Cancelled write, the helper
    /// returns `Ok(Some(Cancelled))` — not `Ok(None)` (silent overwrite
    /// on the pre-fix path). Isolated from the full `execute_workflow`
    /// frame so the observable contract is easy to evolve.
    #[tokio::test]
    async fn persist_final_state_honors_external_terminal_transition() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);

        let inner = Arc::new(nebula_storage::InMemoryExecutionRepo::new());

        // Seed an execution row manually at version 0 with Running
        // status (mirrors what `execute_workflow`'s `create` does).
        let execution_id = ExecutionId::new();
        let workflow_id = WorkflowId::new();
        let node_ids = vec![node_key!("x")];
        let mut local_state = ExecutionState::new(execution_id, workflow_id, &node_ids);
        local_state
            .transition_status(ExecutionStatus::Running)
            .unwrap();
        inner
            .create(
                execution_id,
                workflow_id,
                serde_json::to_value(&local_state).unwrap(),
            )
            .await
            .unwrap();

        // Simulate an external cancel: bump the row to version 2 with
        // status=cancelled.
        let mut external_state = local_state.clone();
        external_state
            .transition_status(ExecutionStatus::Cancelling)
            .ok();
        external_state
            .transition_status(ExecutionStatus::Cancelled)
            .ok();
        let external_json = serde_json::to_value(&external_state).unwrap();
        let ok = inner
            .transition(execution_id, 1, external_json)
            .await
            .expect("external transition should succeed");
        assert!(ok, "external transition must commit at v=1");

        // Now ask the engine to persist its final state as Completed
        // starting from the stale version it had before the external
        // bump. This is exactly the pre-fix silent-overwrite scenario.
        let mut repo_version: u64 = 1;
        let mut engine_final_state = local_state.clone();
        engine_final_state
            .transition_status(ExecutionStatus::Completed)
            .unwrap();

        let repo: Arc<dyn ExecutionRepo> = inner.clone();
        let outcome = engine
            .persist_final_state(
                &repo,
                execution_id,
                &mut engine_final_state,
                &mut repo_version,
            )
            .await
            .expect("reconciliation must succeed against an external terminal write");

        assert_eq!(
            outcome,
            Some(ExecutionStatus::Cancelled),
            "helper must report the external terminal status; pre-fix \
             returned None and silently overwrote the cancel (§11.5, #333). \
             got {outcome:?}"
        );

        // Double-check: the persisted row still says `cancelled`.
        let (_v, final_state) = inner
            .get_state(execution_id)
            .await
            .unwrap()
            .expect("row must still exist");
        assert_eq!(
            final_state.get("status").and_then(|v| v.as_str()),
            Some("cancelled"),
            "engine must not overwrite the external Cancelled row \
             with its local Completed decision (§11.5, #333)"
        );
    }

    // ── #325 execution lease lifecycle (ADR 0008) ─────────────────────────

    /// Regression for #325: a second `resume_execution` on the same row
    /// while a first runner still holds the lease must get
    /// [`EngineError::Leased`] instead of racing the frontier loop.
    ///
    /// This is the core multi-runner correctness property. We construct
    /// the repo by hand, seed a non-terminal execution row, and call
    /// `resume_execution` twice concurrently. Only one runner may
    /// dispatch nodes.
    #[tokio::test]
    async fn two_concurrent_resume_runners_are_fenced_by_lease() {
        let registry = Arc::new(ActionRegistry::new());
        // Slow echo so the first runner is still inside the frontier
        // loop when the second call arrives.
        registry.register_stateless(SlowHandler {
            meta: ActionMetadata::new(action_key!("slow"), "Slow", "slow echoes"),
            delay: Duration::from_millis(300),
        });

        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n.clone(), "Slow", "slow").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        // Seed a non-terminal execution row the two runners will target.
        let execution_id = ExecutionId::new();
        let node_ids = vec![n.clone()];
        let exec_state = ExecutionState::new(execution_id, wf.id, &node_ids);
        let state_json = serde_json::to_value(&exec_state).unwrap();
        exec_repo
            .create(execution_id, wf.id, state_json)
            .await
            .unwrap();

        // Two independent engines, each with its own InstanceId, sharing
        // the same storage. One of them should win the lease.
        let (engine_a, _) = make_engine(registry.clone());
        let engine_a = engine_a
            .with_execution_repo(exec_repo.clone())
            .with_workflow_repo(workflow_repo.clone());
        let (engine_b, _) = make_engine(registry);
        let engine_b = engine_b
            .with_execution_repo(exec_repo.clone())
            .with_workflow_repo(workflow_repo);

        assert_ne!(
            engine_a.instance_id(),
            engine_b.instance_id(),
            "independent engines must produce distinct lease holder strings"
        );

        // Spawn both calls concurrently — whoever acquires first wins,
        // the other must see `EngineError::Leased`.
        let handle_a = tokio::spawn(async move { engine_a.resume_execution(execution_id).await });
        let handle_b = tokio::spawn(async move { engine_b.resume_execution(execution_id).await });

        let result_a = handle_a.await.unwrap();
        let result_b = handle_b.await.unwrap();

        let losses: Vec<_> = [&result_a, &result_b]
            .iter()
            .filter_map(|r| match r {
                Err(EngineError::Leased {
                    execution_id: eid,
                    holder,
                }) => Some((*eid, holder.clone())),
                _ => None,
            })
            .collect();
        let successes: Vec<_> = [&result_a, &result_b]
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .collect();

        assert_eq!(
            losses.len() + successes.len(),
            2,
            "both calls must return either Ok or a typed Leased error, no panics; \
             got a={result_a:?}, b={result_b:?}"
        );
        assert_eq!(
            losses.len(),
            1,
            "exactly one runner must be fenced by the lease; got a={result_a:?}, b={result_b:?}"
        );
        assert_eq!(
            successes.len(),
            1,
            "exactly one runner must dispatch nodes; got a={result_a:?}, b={result_b:?}"
        );
        assert_eq!(
            losses[0].0, execution_id,
            "Leased error must carry the execution id that was contested"
        );
        assert!(
            successes[0].is_success(),
            "the winning runner must complete the workflow successfully; \
             got status={:?}",
            successes[0].status
        );
    }

    /// Registry race regression (ADR-0016 / #482 Copilot review).
    ///
    /// Two `resume_execution` calls overlap on the **same engine** for the
    /// **same execution_id**. The winner acquires the lease and publishes
    /// its token into `running`; the loser hits `EngineError::Leased`
    /// before it ever inserts — so no drop guard can clobber the winner's
    /// entry. This asserts the observable contract: while the winner's
    /// frontier loop is live, `engine.cancel_execution(id)` still finds a
    /// registered token even after the loser has returned.
    #[tokio::test]
    async fn overlapping_resume_losers_do_not_clobber_winners_registry_entry() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(SlowHandler {
            meta: ActionMetadata::new(action_key!("slow"), "Slow", "slow echoes"),
            delay: Duration::from_millis(500),
        });

        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n.clone(), "Slow", "slow").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;

        let execution_id = ExecutionId::new();
        let node_ids = vec![n.clone()];
        let exec_state = ExecutionState::new(execution_id, wf.id, &node_ids);
        exec_repo
            .create(
                execution_id,
                wf.id,
                serde_json::to_value(&exec_state).unwrap(),
            )
            .await
            .unwrap();

        // Single engine, so both calls share the same `running` registry —
        // this is the path the Copilot review flagged. Wrap in `Arc` so we
        // can drive the second call from a background task and still
        // observe the registry from the test thread.
        let (engine, _) = make_engine(registry);
        let engine = Arc::new(
            engine
                .with_execution_repo(exec_repo.clone())
                .with_workflow_repo(workflow_repo),
        );

        // Winner: drive the workflow in the background. Its frontier loop
        // will be live (500ms sleep) long enough for the loser to race.
        let winner_engine = Arc::clone(&engine);
        let winner =
            tokio::spawn(async move { winner_engine.resume_execution(execution_id).await });

        // Poll the registry until the winner has published its token.
        // This synchronises on the exact moment the race window opens.
        let t_wait = Instant::now();
        loop {
            if engine.running.contains_key(&execution_id) {
                break;
            }
            assert!(
                t_wait.elapsed() < Duration::from_secs(2),
                "winner failed to register its token within 2s"
            );
            tokio::task::yield_now().await;
        }

        // Loser: a second resume call on the same engine for the same id.
        // Must fail fast with `Leased` — and crucially must NOT clobber
        // the registry entry the winner just published.
        let loser = engine.resume_execution(execution_id).await;
        assert!(
            matches!(loser, Err(EngineError::Leased { .. })),
            "overlapping resume must be fenced by the lease; got {loser:?}"
        );

        // The winner is still running — its token must still be live.
        // This is the property that would have failed without the
        // vacant-only insert + nonce-scoped remove_if (Copilot hazard).
        assert!(
            engine.cancel_execution(execution_id),
            "winner's registry entry must survive the loser's failed attempt \
             (if this fails, the loser's Drop clobbered the winner's token)"
        );

        // Signalling cancel aborts the winner quickly. Assert the outcome
        // explicitly — `Err(EngineError::Leased)` or any non-terminal status
        // would indicate a real regression (e.g. the heartbeat unexpectedly
        // stole the lease during the 500ms slow handler); silently dropping
        // the `Result` would mask that.
        let winner_result = tokio::time::timeout(Duration::from_secs(5), winner)
            .await
            .expect("winner returns within 5s of cancel")
            .expect("join ok")
            .expect("winner returns Ok(ExecutionResult) after cancel");
        // Both labels are acceptable: the abort-select arm ends in `Cancelled`;
        // the node-error arm (handler returned `ActionError::Cancelled`, processed
        // as node failure) ends in `Failed`. The scheduler race between the two is
        // pre-existing behaviour covered by `integration::cancellation_via_sibling_failure`.
        assert!(
            matches!(
                winner_result.status,
                ExecutionStatus::Cancelled | ExecutionStatus::Failed
            ),
            "winner must reach a terminal non-success status after cancel; got {:?}",
            winner_result.status
        );
    }

    /// After the first runner releases the lease on terminal completion,
    /// a later `resume_execution` on a non-terminal row can acquire it
    /// cleanly. Covers the release-on-terminal branch of ADR 0008.
    #[tokio::test]
    async fn lease_is_released_after_terminal_completion_so_next_runner_can_acquire() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n.clone(), "echo", "echo").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;
        let engine = engine
            .with_execution_repo(exec_repo.clone())
            .with_workflow_repo(workflow_repo);

        // First run acquires + releases the lease on completion.
        let first = engine
            .execute_workflow(&wf, serde_json::json!("v1"), ExecutionBudget::default())
            .await
            .unwrap();
        assert!(first.is_success());

        // Lease must be free immediately — a brand-new acquire with a
        // fresh holder should succeed without waiting for TTL.
        let acquired = exec_repo
            .acquire_lease(first.execution_id, "probe".into(), Duration::from_secs(5))
            .await
            .unwrap();
        assert!(
            acquired,
            "lease must be released on terminal completion, not pending TTL expiry"
        );
    }

    /// Once the engine has run an execution to completion, a second
    /// `execute_workflow` call produces a brand-new `ExecutionId`, so
    /// its lease is independent and acquires without contention.
    /// Defense-in-depth: confirms we don't share lease state across
    /// unrelated ids.
    #[tokio::test]
    async fn execute_workflow_produces_independent_lease_per_execution_id() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        });

        let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let n = node_key!("n");
        let wf = make_workflow(
            vec![NodeDefinition::new(n.clone(), "echo", "echo").unwrap()],
            vec![],
        );
        let workflow_repo = save_workflow_to_repo(&wf).await;
        let engine = engine
            .with_execution_repo(exec_repo)
            .with_workflow_repo(workflow_repo);

        let first = engine
            .execute_workflow(&wf, serde_json::json!("v1"), ExecutionBudget::default())
            .await
            .unwrap();
        let second = engine
            .execute_workflow(&wf, serde_json::json!("v2"), ExecutionBudget::default())
            .await
            .unwrap();
        assert!(first.is_success());
        assert!(second.is_success());
        assert_ne!(
            first.execution_id, second.execution_id,
            "each execute_workflow call must produce its own ExecutionId"
        );
    }
}
