//! Workflow execution engine.
//!
//! Executes workflows using a frontier-based approach: each node is spawned
//! as soon as all its incoming edges are resolved and at least one is activated,
//! rather than waiting for an entire topological level. This enables branching,
//! skip propagation, error routing, and conditional edges.

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use nebula_action::{
    ActionError, ActionResult, capability::default_resource_accessor, result::WaitCondition,
};
use nebula_core::{
    ActionKey, CredentialKey, NodeKey, ResourceKey,
    accessor::{Clock, CredentialAccessor, ResourceAccessor, SystemClock},
    id::{ExecutionId, InstanceId, WorkflowId},
    node_key,
};
use nebula_credential::default_credential_accessor;
// ScopeLevel removed from ActionContext
// use nebula_core::scope::ScopeLevel;
use nebula_execution::output::ExecutionOutput;
use nebula_execution::{
    ExecutionStatus,
    context::ExecutionBudget,
    plan::ExecutionPlan,
    state::{AttemptOutcome, ExecutionState, WaitSignal, WaitWake},
    status::ExecutionTerminationReason,
};
use nebula_expression::ExpressionEngine;
use nebula_metrics::naming::{
    NEBULA_ENGINE_LEASE_CONTENTION_TOTAL, NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL, engine_lease_contention_reason,
};
use nebula_metrics::{Counter, Histogram, MetricsRegistry};
use nebula_plugin::PluginRegistry;
use nebula_workflow::{Connection, DependencyGraph, NodeState, WorkflowDefinition};
use tokio::{
    sync::{Semaphore, mpsc, oneshot},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use nebula_storage_port::Scope;
use nebula_storage_port::dto::ResumeTarget;
use nebula_storage_port::dto::resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash};

// W-S3c: token minting — SHA-256 hash-at-rest, base64 bearer, zeroizing plaintext.
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use rand::Rng as _;
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{
    credential_accessor::EngineCredentialAccessor,
    error::EngineError,
    event::{ExecutionEvent, NodeFailedDetails},
    resolver::ParamResolver,
    resource::ResourceActivatorRegistry,
    resource_accessor::EngineResourceAccessor,
    result::ExecutionResult,
    runtime::ActionRuntime,
    scoped_resources::LayeredResourceAccessor,
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
/// 28 calls for broadcast to multiple subscribers (storage writer,
/// metrics collector, websocket broadcaster, audit writer) — `EventBus` is
/// the workspace-standard fan-out primitive.
pub const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Outcome of [`WorkflowEngine::satisfy_signal_waits`].
///
/// Distinguishes "signal waits armed for completion" from "nothing to do"
/// so callers can log counts accurately without treating zero as an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SatisfyOutcome {
    /// `n` signal-driven `Waiting` nodes were armed for completion (their
    /// `next_attempt_at` was set so the follow-up drive's Phase-0b completes
    /// them on the main port).
    Satisfied(usize),
    /// No signal-driven `Waiting` nodes existed — execution was already
    /// satisfied, cancelled, or never had wait nodes.
    NothingToSatisfy,
    /// The execution left the `Paused` state (terminal or `Cancelling`) between
    /// the caller's pre-lease status read and the under-lease reload — a
    /// concurrent Cancel/Terminate won the race. No nodes were touched; the
    /// Resume is moot and the caller should ack it without driving.
    ExecutionNotResumable,
}

/// Outcome of [`WorkflowEngine::cancel_dangling_nodes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelDanglingOutcome {
    /// `n` non-terminal nodes were transitioned to `Cancelled`.
    Cancelled(usize),
    /// Nothing to do — every node is already terminal (idempotent re-delivery),
    /// or the execution reached a non-cancel terminal outcome before the cancel
    /// landed. The caller may ack.
    NothingToCancel,
    /// The cancel is NOT yet durably recorded on the execution (status is still
    /// non-terminal and not `Cancelling`). The API writes `Cancelled` before
    /// enqueuing `Cancel`, so this is a producer-ordering anomaly — the caller
    /// must DEFER (not ack), so B1 reclaim redelivers until the status reflects
    /// the cancel and the node cleanup can run, rather than silently dropping it.
    StatusNotCancelled,
}

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

/// How long [`WorkflowEngine::resume_live`] waits for the live frontier loop
/// to confirm a durable self-arm checkpoint before treating the Resume as
/// undelivered (ADR-0099 W-S2b, P1#1).
///
/// Generously larger than the live loop's longest non-`await` span between two
/// `recv()` polls (one frontier iteration), so a healthy loop always acks
/// inside the window; well under [`DEFAULT_EXECUTION_LEASE_TTL`], so a runner
/// that is genuinely wedged times out here long before its lease expires and
/// B1 reclaim takes over. A timeout resolves to a `Deferred` control-queue
/// outcome, which is safe because Resume redelivery is idempotent — the arm
/// either has not landed (redelivery re-arms) or has landed and a duplicate
/// Resume is a no-op on the already-armed node.
const RESUME_ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Bounded capacity of a live frontier loop's resume channel (ADR-0099
/// W-S2b, P1#1). A Resume is rare relative to a loop iteration, so the loop
/// drains each request well before another arrives; the small buffer absorbs
/// a brief burst, and a full channel is treated by [`WorkflowEngine::resume_live`]
/// as no-delivery (defer for B1 reclaim) rather than a blocking send.
const RESUME_CHANNEL_CAPACITY: usize = 8;

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
    workflow_executions_started: Counter,
    workflow_executions_completed: Counter,
    workflow_executions_failed: Counter,
    workflow_execution_duration_seconds: Histogram,
    /// Node registry for node-level metadata and versioning.
    plugin_registry: PluginRegistry,
    /// Closed `kind → typed factory` allowlist for stored-resource-row
    /// activation.
    ///
    /// A persisted resource row carries only a `kind` string plus opaque
    /// JSON; turning it into a typed `Manager::register_resolved::<R>`
    /// call requires an erased [`ResourceFactory`](crate::ResourceFactory)
    /// that already knows its concrete `R` and `R::Topology`
    /// (see [`KindActivator`](crate::KindActivator)). The map is
    /// **closed**: a kind is registrable only if a factory was explicitly
    /// inserted; an unknown kind is a wiring fault caught at activation,
    /// never a silent no-op.
    ///
    /// Post ADR-0095 D2, `Plugin::resources()` returns
    /// `Vec<Arc<dyn ResourceFactory>>` — each factory carries both
    /// introspection (`key`, `metadata`, `validate`) and construction
    /// (`register`). The composition root can populate this allowlist
    /// directly from `plugin.resources()` without a separate wiring step.
    ///
    /// Defaults empty (fail-closed: every kind rejected) for engines wired
    /// without resource activation.
    ///
    /// [`plugin_registry`]: Self::plugin_registry
    resource_registrars: ResourceActivatorRegistry,
    /// Engine-owned reverse index `CredentialId → resolved resource rows`
    /// for the per-slot rotation fan-out.
    ///
    /// Always constructed (an empty index is a no-op fan-out), held here
    /// next to [`resource_registrars`] and the
    /// [`resource_manager`](Self::resource_manager) it is driven against
    /// because the three are the resource-rotation triad: the registrar
    /// path [`bind`](nebula_credential_rotation_index_bind)s a row when a
    /// credential resolves into a `#[credential]` slot, and the
    /// `ResourceFanoutDriver` drains it on a rotation/revoke event into the
    /// `Manager` slot ports. No `nebula-resource → nebula-engine` edge: the
    /// index now lives in `nebula-resource` and the rotation signal arrives
    /// via `nebula-eventbus`.
    ///
    /// Feature-gated with the index itself (`rotation`).
    ///
    /// [`resource_registrars`]: Self::resource_registrars
    #[cfg(feature = "rotation")]
    resource_fanout_index: Arc<nebula_resource::ResourceFanoutIndex>,
    /// Single-shot guard for
    /// [`spawn_resource_rotation_fanout`](Self::spawn_resource_rotation_fanout).
    ///
    /// The driver subscribes the credential/lease buses; spawning it
    /// twice would subscribe twice and **double-dispatch every
    /// refresh/revoke** to the resource fan-out (non-idempotent hooks
    /// fire 2×, `RotationOutcome` metrics inflated). The composition root
    /// owns the single spawn, but a defensive structural guard (not a
    /// "remember to call it once" convention) makes a second call a
    /// no-op `None` rather than a silent double-subscribe. An
    /// `AtomicBool` (flipped via `compare_exchange`) so the `&self`
    /// method is single-shot even under a concurrent double-call.
    ///
    /// Feature-gated with the index itself (`rotation`).
    #[cfg(feature = "rotation")]
    resource_fanout_spawned: std::sync::atomic::AtomicBool,
    /// Resolves node parameters (expressions, templates, references) to JSON.
    resolver: ParamResolver,
    /// Optional resource manager for providing resources to actions.
    resource_manager: Option<Arc<nebula_resource::Manager>>,
    /// Extra scope fields merged into each node's resource acquire context
    /// (`org_id`, `workspace_id`) when resources are registered above execution
    /// scope.
    resource_acquire_scope: Option<nebula_core::scope::Scope>,
    /// Per-execution acquire scope (`org_id` / `workspace_id` for this run).
    execution_acquire_scopes: DashMap<ExecutionId, nebula_core::scope::Scope>,
    /// Per-resource resolved **collision-free structural** slot identities
    /// recorded at activation (pre-run).
    resource_slot_identities: RwLock<HashMap<ResourceKey, nebula_resource::SlotIdentity>>,
    /// Frozen slot-identity map per execution (snapshot at run start).
    resource_slot_identities_by_execution:
        DashMap<ExecutionId, Arc<HashMap<ResourceKey, nebula_resource::SlotIdentity>>>,
    /// Optional spec-16 port bundle (execution-state / lease / journal /
    /// node-result / idempotency / checkpoint). `None` puts the engine in
    /// single-process library mode (no coordination seam, no lease).
    stores: Option<crate::store_seam::ExecutionStores>,
    /// Optional spec-16 workflow-definition port bundle for the resume
    /// path (workflow-row + version stores).
    workflow_stores: Option<crate::store_seam::WorkflowStores>,
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
    /// See product canon (operational honesty — no false capabilities; secrets and auth).
    /// Populated via [`WorkflowEngine::with_action_credentials`].
    action_credentials: HashMap<ActionKey, HashSet<String>>,
    /// Optional event sender for real-time execution monitoring (TUI, logging).
    event_bus: Option<EventBus>,
    /// Injectable clock for deterministic durable-timing paths (retry
    /// deadlines, wait expirations, token issue timestamps).
    ///
    /// Defaults to [`SystemClock`] at construction; override with
    /// [`WorkflowEngine::with_clock`] in tests to make wall-clock-sensitive
    /// timing behaviour deterministic and inspection-ready.
    clock: Arc<dyn Clock>,
    /// Stable per-instance identifier used as the execution-lease holder.
    ///
    /// Generated once when constructing [`WorkflowEngine`] via [`InstanceId::new`]
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
    /// is the authoritative single-runner fence, so publishing
    /// after the lease prevents an overlapping attempt for the same
    /// [`ExecutionId`] from overwriting the live token. Each entry is
    /// tagged with a monotonically-increasing [`RunningRegistrationId`]
    /// nonce, and the [`RunningRegistration`] guard's `Drop` uses
    /// [`DashMap::remove_if`] to remove only entries that still carry its
    /// own nonce — a defensive guard against an out-of-order drop from a
    /// losing attempt clobbering the winner's registration.
    ///
    /// [`cancel_execution`] looks up the token here and cancels it,
    /// closing the A3 control-queue `Cancel` path into the
    /// cooperative-cancel signal the frontier loop already observes.
    ///
    /// **Not durable.** This map lives only as long as the `WorkflowEngine`
    /// instance. On process crash the entries vanish with the runner; the
    /// durable truth is `executions` + `execution_control_queue`, and the
    /// replacement runner reloads from storage.
    ///
    /// [`execute_workflow`]: Self::execute_workflow
    /// [`resume_execution`]: Self::resume_execution
    /// [`cancel_execution`]: Self::cancel_execution
    running: Arc<DashMap<ExecutionId, RunningEntry>>,
    /// Optional handle for the background credential-refresh reclaim
    /// sweep. When set, dropping the engine aborts the spawned task —
    /// see [`nebula_credential::runtime::ReclaimSweepHandle`] (sub-spec
    /// , ).
    ///
    /// Wired by the composition root via
    /// [`Self::with_credential_reclaim_sweep`] when the deployment has a
    /// durable [`nebula_storage::credential::RefreshClaimRepo`] (Postgres
    /// or SQLite). Single-replica desktop mode without sentinel-event
    /// recording leaves this `None`.
    credential_reclaim_sweep: Option<nebula_credential::runtime::ReclaimSweepHandle>,
}

/// Monotonic per-registration identifier used to fence out-of-order drops
/// from concurrent attempts on the same [`ExecutionId`] ( / #482
/// Copilot review). A `u64` is fine — we'd need billions of registrations
/// per runner lifetime to wrap, and the counter resets on process restart
/// anyway.
type RunningRegistrationId = u64;

/// Process-wide monotonic counter for registration nonces.
static NEXT_REGISTRATION_ID: AtomicU64 = AtomicU64::new(1);

/// A request to a live frontier loop to self-arm its parked signal waits for
/// completion (ADR-0099 W-S2b, P1#1).
///
/// Carries a one-shot `ack` channel the loop fires AFTER it durably
/// checkpoints the self-arm (or learns the arm failed), plus the
/// `resume_target` that selects which parked signal wait(s) to arm (W-S3a).
struct ResumeRequest {
    /// The loop sends exactly one [`ResumeOutcome`] here once the self-arm
    /// has durably landed or failed. A dropped sender (the loop exited
    /// before replying) resolves the awaiting receiver to `Err`, which
    /// [`WorkflowEngine::resume_live`] maps to [`ResumeDelivery::LoopGone`].
    ack: oneshot::Sender<ResumeOutcome>,
    /// Which parked signal wait this Resume targets (W-S3a). `Some(target)`
    /// arms only the kind+identity match; `None` arms every signal wait (the
    /// W-S2b untargeted behavior).
    resume_target: Option<ResumeTarget>,
}

/// The durable result of a live loop processing a [`ResumeRequest`].
///
/// Sent on the request's `ack` channel STRICTLY after the self-arm
/// checkpoint resolves, so an `Armed` outcome is a durability guarantee, not
/// a "the loop woke up" signal.
#[derive(Debug)]
pub(crate) enum ResumeOutcome {
    /// The loop armed `count` signal waits and the arm checkpoint landed
    /// durably. The control-queue row may be acked.
    Armed {
        /// Number of signal-`Waiting` nodes armed in this pass.
        count: usize,
    },
    /// The loop woke but found no signal-`Waiting` node to arm (spurious or
    /// already-armed wake). Nothing to do; the row may be acked.
    NothingToArm,
    /// The self-arm checkpoint failed (e.g. the loop lost its lease
    /// mid-iteration: `FencedOut` / `CasConflict`). The arm did NOT land; the
    /// row must NOT be acked.
    ArmFailed,
}

/// The outcome of [`WorkflowEngine::resume_live`] delivering a Resume to a
/// live frontier loop.
///
/// Distinguishes the durable-ack cases (`Acked`) from the no-delivery cases
/// (`NoLiveEntry` / `LoopGone` / `AckTimeout`) so `dispatch_resume` can ack
/// only when the arm is durable and otherwise defer for B1 reclaim.
#[derive(Debug)]
pub(crate) enum ResumeDelivery {
    /// The live loop received the request and replied with a durable
    /// [`ResumeOutcome`].
    Acked(ResumeOutcome),
    /// No live `RunningEntry` exists on this runner (cross-runner or
    /// just-paused), or its resume channel is closed/full — treated as
    /// no-delivery so the Resume defers rather than blocks.
    NoLiveEntry,
    /// A live entry existed and the request was sent, but the loop dropped
    /// the `ack` sender before replying (it exited / panicked) — the arm did
    /// not durably land.
    LoopGone,
    /// The loop did not reply within [`RESUME_ACK_TIMEOUT`] — it is wedged or
    /// far behind. Treated as no-delivery (defer); redelivery is idempotent.
    AckTimeout,
}

/// Value stored in [`WorkflowEngine::running`]. Pairs the live
/// [`CancellationToken`] with the [`RunningRegistrationId`] nonce so the
/// drop guard can use [`DashMap::remove_if`] instead of unconditional
/// `remove`.
///
/// `resume_tx` is the live-frontier resume channel (ADR-0099 W-S2b): a
/// bounded [`mpsc::Sender<ResumeRequest>`] the running loop selects on. A
/// `Resume` command for a still-`Running` execution (a signal wait parked
/// with a `timeout`, so the row never reached `Paused`) cannot use the
/// durable satisfy-CAS — that path acquires the lease the live loop already
/// holds (two-writers-one-row). Instead [`WorkflowEngine::resume_live`] sends
/// a [`ResumeRequest`] here; the loop wakes, self-arms the parked signal
/// node(s) under its OWN lease, durably checkpoints the arm, then replies on
/// the request's `ack` channel. The control-queue ack is gated on that
/// durable reply (P1#1), so a crash between the notify and the checkpoint
/// can no longer drop an acked-but-not-landed Resume — the durable row
/// remains the sole source of truth and an un-acked Resume is redelivered.
///
/// [`mpsc::Sender<ResumeRequest>`]: tokio::sync::mpsc::Sender
struct RunningEntry {
    registration_id: RunningRegistrationId,
    token: CancellationToken,
    resume_tx: mpsc::Sender<ResumeRequest>,
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

mod frontier;
mod persistence;
mod resume;

impl WorkflowEngine {
    /// Create a new engine with the given components.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Telemetry`] if the shared registry rejects
    /// registration for the canonical workflow metric identities.
    pub fn new(runtime: Arc<ActionRuntime>, metrics: MetricsRegistry) -> Result<Self, EngineError> {
        let workflow_executions_started =
            metrics.counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)?;
        let workflow_executions_completed =
            metrics.counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)?;
        let workflow_executions_failed =
            metrics.counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)?;
        let workflow_execution_duration_seconds =
            metrics.histogram(NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS)?;

        let expression_engine = Arc::new(ExpressionEngine::with_cache_size(1024));
        let instance_id = InstanceId::new();
        tracing::info!(
            instance_id = %instance_id,
            "workflow engine starting; lease holder string bound for this process's lifetime"
        );
        Ok(Self {
            runtime,
            metrics,
            workflow_executions_started,
            workflow_executions_completed,
            workflow_executions_failed,
            workflow_execution_duration_seconds,
            plugin_registry: PluginRegistry::new(),
            resource_registrars: ResourceActivatorRegistry::new(),
            #[cfg(feature = "rotation")]
            resource_fanout_index: Arc::new(nebula_resource::ResourceFanoutIndex::new()),
            #[cfg(feature = "rotation")]
            resource_fanout_spawned: std::sync::atomic::AtomicBool::new(false),
            resolver: ParamResolver::new(expression_engine),
            resource_manager: None,
            resource_acquire_scope: None,
            execution_acquire_scopes: DashMap::new(),
            resource_slot_identities: RwLock::new(HashMap::new()),
            resource_slot_identities_by_execution: DashMap::new(),
            stores: None,
            workflow_stores: None,
            credential_resolver: None,
            credential_refresh: None,
            action_credentials: HashMap::new(),
            event_bus: None,
            clock: Arc::new(SystemClock),
            instance_id,
            lease_ttl: DEFAULT_EXECUTION_LEASE_TTL,
            lease_heartbeat_interval: DEFAULT_EXECUTION_LEASE_HEARTBEAT_INTERVAL,
            running: Arc::new(DashMap::new()),
            credential_reclaim_sweep: None,
        })
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
    /// calls after the idempotency guard; the durable API-level CAS to
    /// `Cancelled` has already landed on the execution row by the time a
    /// `Cancel` command reaches the consumer (control-queue cancel enqueue path).
    pub fn cancel_execution(&self, execution_id: ExecutionId) -> bool {
        match self.running.get(&execution_id) {
            Some(entry) => {
                entry.value().token.cancel();
                true
            },
            None => false,
        }
    }

    /// Deliver a `Resume` to a LIVE frontier loop owned by THIS runner and
    /// wait for the loop's durable self-arm result (ADR-0099 W-S2b, P1#1).
    ///
    /// Sends a [`ResumeRequest`] to the live `RunningEntry` for
    /// `execution_id`; the loop wakes, self-arms its signal-`Waiting` node(s)
    /// under its OWN lease, durably checkpoints the arm, then replies on the
    /// request's `ack` channel. The returned [`ResumeDelivery`] reports
    /// whether that durable arm landed:
    ///
    /// - [`ResumeDelivery::Acked`] — the loop replied; the inner
    ///   [`ResumeOutcome`] says whether the arm landed (`Armed` /
    ///   `NothingToArm` — durably resolved, may ack) or failed (`ArmFailed` —
    ///   the loop lost its lease mid-iteration; must defer).
    /// - [`ResumeDelivery::NoLiveEntry`] — no live loop on this runner
    ///   (cross-runner / just-paused), or the channel is closed/full. A full
    ///   channel is treated as no-delivery (defer) rather than a blocking
    ///   send, so a backed-up loop never stalls the control consumer.
    /// - [`ResumeDelivery::LoopGone`] — the loop dropped the `ack` sender
    ///   before replying (it exited / panicked); the arm did not land.
    /// - [`ResumeDelivery::AckTimeout`] — no reply within
    ///   [`RESUME_ACK_TIMEOUT`]; the loop is wedged or far behind.
    ///
    /// Routing the Resume through the live loop (rather than writing the row
    /// directly) is load-bearing: a `Running` execution's lease is held by its
    /// own loop, so the durable satisfy-CAS would deadlock/race that owner
    /// (two-writers-one-row). The live loop remains the sole writer of its own
    /// row.
    ///
    /// `Acked(Armed)` is returned ONLY after the live loop durably
    /// checkpointed the arm; every other variant means the arm is not durable,
    /// so the caller must defer (the control-queue row stays un-acked for B1
    /// reclaim). Deferring is always safe: Resume redelivery is idempotent — a
    /// duplicate Resume to an already-armed node is a [`ResumeOutcome::NothingToArm`]
    /// no-op.
    ///
    /// No-live-owner recovery (a `Running` execution with no live loop on ANY
    /// runner) and cross-runner Resume routing/affinity remain a future
    /// (W-S3) slice: today a `NoLiveEntry` simply defers for B1 reclaim.
    pub(crate) async fn resume_live(
        &self,
        execution_id: ExecutionId,
        resume_target: Option<ResumeTarget>,
    ) -> ResumeDelivery {
        // Build the reply channel, then drop the `running` map guard BEFORE
        // awaiting the ack — holding a `DashMap` ref across `.await` could
        // block other shards' access to the same bucket.
        let ack_rx = {
            let (ack_tx, ack_rx) = oneshot::channel();
            let Some(entry) = self.running.get(&execution_id) else {
                return ResumeDelivery::NoLiveEntry;
            };
            // `try_send` (not `send().await`): a full or closed channel is a
            // no-delivery signal, not something to block the control consumer
            // on. Capacity is small (a Resume is rare relative to a loop
            // iteration); a full channel means a prior Resume is still
            // un-drained, so deferring this one and letting B1 redeliver is
            // both safe and idempotent.
            if entry
                .value()
                .resume_tx
                .try_send(ResumeRequest {
                    ack: ack_tx,
                    resume_target,
                })
                .is_err()
            {
                return ResumeDelivery::NoLiveEntry;
            }
            ack_rx
        };
        match tokio::time::timeout(RESUME_ACK_TIMEOUT, ack_rx).await {
            Ok(Ok(outcome)) => ResumeDelivery::Acked(outcome),
            // The loop dropped the `ack` sender before replying (exited /
            // panicked) — the self-arm checkpoint did not land.
            Ok(Err(_)) => ResumeDelivery::LoopGone,
            Err(_) => ResumeDelivery::AckTimeout,
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

    /// Override the wall-clock source used for durable timing paths.
    ///
    /// The engine uses this clock for retry deadlines, wait expirations,
    /// and resume-token issue timestamps — all values that must be
    /// deterministic and comparable across retries and process restarts.
    ///
    /// The default is [`SystemClock`] (real wall time). Inject a
    /// `FakeClock` in unit tests to assert timing behaviour without
    /// sleeping.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Access the node registry.
    pub fn plugin_registry(&self) -> &PluginRegistry {
        &self.plugin_registry
    }

    /// The closed `kind → typed registrar` allowlist.
    ///
    /// This is the only path from a stored resource row (a `kind` string
    /// plus opaque JSON) to a typed `Manager::register_resolved::<R>`
    /// call. A kind is registrable only if a registrar was explicitly
    /// wired in (via [`with_resource_registrars`](Self::with_resource_registrars));
    /// an unknown kind is a wiring fault surfaced at activation, never a
    /// silent no-op . Defaults empty
    /// (fail-closed) when the engine is built without resource activation.
    ///
    /// The activation path (resolving a persisted row's `kind` through
    /// this allowlist) is built on top of this accessor; this method is
    /// the live producer the §M11.5 fan-out / resource activation consumes.
    #[must_use]
    pub fn resource_registrars(&self) -> &ResourceActivatorRegistry {
        &self.resource_registrars
    }

    /// The engine-owned per-slot rotation reverse index.
    ///
    /// This is the `bind` producer for the §M11.5 fan-out: the
    /// resource-activation path (`ResourceActivatorRegistry::register` →
    /// `Manager::register_resolved`, which resolves a credential into a
    /// `#[credential]` slot) records a row here so a later rotation /
    /// revoke fans to exactly that resolved row. It is also the index the
    /// [`spawn_resource_rotation_fanout`](Self::spawn_resource_rotation_fanout)
    /// driver drains. Empty until a row binds — an empty index is a
    /// no-op fan-out, never an error.
    #[cfg(feature = "rotation")]
    #[must_use]
    pub fn resource_fanout_index(&self) -> &Arc<nebula_resource::ResourceFanoutIndex> {
        &self.resource_fanout_index
    }

    /// Spawn the production rotation fan-out driver wiring the engine's
    /// [`resource_fanout_index`](Self::resource_fanout_index) +
    /// [`resource_manager`](Self::with_resource_manager) to the
    /// credential-rotation / lease-revoke event streams the
    /// credential-runtime composition root publishes.
    ///
    /// `credential_bus` / `lease_bus` are the buses the credential-runtime
    /// composition root publishes on after a refresh CAS-persists fresh
    /// material or a lease is revoked. A `CredentialEvent::Refreshed` drives
    /// `ResourceFanoutIndex::dispatch_refresh`, and
    /// `CredentialEvent::Revoked` / `LeaseEvent::LeaseRevoked` drive
    /// `dispatch_revoke`, for every resolved resource row that bound the
    /// credential.
    ///
    /// Returns a [`nebula_resource::ResourceFanoutDriver`] handle; **hold
    /// it** for as long as fan-out should run — dropping it aborts the
    /// driver task.
    ///
    /// Returns `None` (spawning nothing) when either:
    ///
    /// - no resource manager was wired via
    ///   [`with_resource_manager`](Self::with_resource_manager) (there is
    ///   nothing to fan rotations *to*); or
    /// - the driver was **already spawned** by a prior call. This method
    ///   is **single-shot / idempotent**: each spawn subscribes the
    ///   credential + lease buses, so spawning twice would subscribe
    ///   twice and double-dispatch every refresh/revoke to the resource
    ///   fan-out. The first call that has a manager spawns and returns
    ///   `Some(driver)`; every later call is a no-op `None` and does
    ///   **not** subscribe again (the guard is claimed atomically, so a
    ///   concurrent double-call still yields exactly one driver). A
    ///   no-manager call spawns nothing and does **not** consume the
    ///   single-shot — a later call once a manager is wired can still
    ///   spawn.
    ///
    /// No `nebula-resource → nebula-engine` edge: the index is now owned
    /// by `nebula-resource` and rotation signals arrive via
    /// `nebula-eventbus`.
    #[cfg(feature = "rotation")]
    #[must_use = "the returned driver handle must be held; dropping it aborts the fan-out"]
    pub fn spawn_resource_rotation_fanout(
        &self,
        credential_bus: Arc<nebula_eventbus::EventBus<nebula_credential::CredentialEvent>>,
        lease_bus: Option<Arc<nebula_eventbus::EventBus<nebula_credential::LeaseEvent>>>,
    ) -> Option<nebula_resource::ResourceFanoutDriver> {
        use std::sync::atomic::Ordering;

        // Only a deployment with a resource manager has anything to fan
        // rotations to. Resolve it *before* claiming the single-shot so a
        // no-manager call does not burn the guard.
        let manager = Arc::clone(self.resource_manager.as_ref()?);

        // Single-shot: claim the guard atomically. If it was already set,
        // the driver is already running on its own subscriber pair —
        // spawning again would double-subscribe and double-dispatch every
        // event, so return `None` and subscribe nothing.
        if self
            .resource_fanout_spawned
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            tracing::debug!(
                target: "nebula_engine::credential::rotation",
                "spawn_resource_rotation_fanout called again; fan-out driver \
                 already running — no second subscriber spawned (idempotent)"
            );
            return None;
        }

        Some(nebula_resource::ResourceFanoutDriver::spawn(
            Arc::clone(&self.resource_fanout_index),
            manager,
            credential_bus,
            lease_bus,
        ))
    }

    /// Attach a resource manager for providing resources to actions.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resource_manager(mut self, manager: Arc<nebula_resource::Manager>) -> Self {
        self.resource_manager = Some(manager);
        self
    }

    /// Merge `org_id` / `workspace_id` into the scope used for resource acquire.
    ///
    /// Prefer [`execute_workflow_with_acquire_scope`](Self::execute_workflow_with_acquire_scope)
    /// when each run carries its own tenant; this builder sets the engine default.
    #[must_use]
    pub fn with_resource_acquire_scope(mut self, scope: nebula_core::scope::Scope) -> Self {
        self.resource_acquire_scope = Some(scope);
        self
    }

    /// Merges engine-default and per-run tenant fields for resource acquire.
    ///
    /// Per-run values override the engine default field-by-field; unset run
    /// fields fall back to the engine default.
    fn merged_acquire_scope(
        engine_default: &Option<nebula_core::scope::Scope>,
        run_scope: Option<&nebula_core::scope::Scope>,
    ) -> nebula_core::scope::Scope {
        let default = engine_default.as_ref();
        let run = run_scope;
        nebula_core::scope::Scope {
            org_id: run
                .and_then(|s| s.org_id)
                .or_else(|| default.and_then(|s| s.org_id)),
            workspace_id: run
                .and_then(|s| s.workspace_id)
                .or_else(|| default.and_then(|s| s.workspace_id)),
            ..Default::default()
        }
    }

    fn install_execution_resource_context(
        &self,
        execution_id: ExecutionId,
        run_acquire_scope: Option<nebula_core::scope::Scope>,
    ) {
        let slot_snap = self
            .resource_slot_identities
            .read()
            .map(|ids| Arc::new(ids.clone()))
            .unwrap_or_else(|err| {
                tracing::error!(
                    target: "nebula_engine",
                    ?err,
                    "resource_slot_identities lock poisoned; empty slot map for execution"
                );
                Arc::new(HashMap::new())
            });
        self.resource_slot_identities_by_execution
            .insert(execution_id, slot_snap);

        let acquire_scope =
            Self::merged_acquire_scope(&self.resource_acquire_scope, run_acquire_scope.as_ref());
        self.execution_acquire_scopes
            .insert(execution_id, acquire_scope);
    }

    /// Record a resolved **collision-free structural** slot identity for a
    /// resource key (activation-time).
    ///
    /// The recorded [`SlotIdentity`](nebula_resource::SlotIdentity) is the
    /// exact structural key `Manager::register_resolved` derived for the same
    /// resolved `(slot, credential)` bindings, so the action-time acquire
    /// path addresses the *same* registry row (no digest aliasing across
    /// tenants).
    pub fn record_resource_slot_identity(
        &self,
        key: ResourceKey,
        slot_identity: nebula_resource::SlotIdentity,
    ) {
        match self.resource_slot_identities.write() {
            Ok(mut ids) => {
                ids.insert(key, slot_identity);
            },
            Err(err) => {
                tracing::error!(
                    target: "nebula_engine",
                    ?err,
                    %key,
                    ?slot_identity,
                    "resource_slot_identities lock poisoned; slot identity not recorded"
                );
            },
        }
    }

    /// Live-register a resource kind and record its slot identity for acquire.
    ///
    /// Callers should prefer this over
    /// [`ResourceActivatorRegistry::register`](crate::ResourceActivatorRegistry::register)
    /// alone so action-time `acquire_any` uses the same `slot_identity` as the
    /// manager registry row.
    ///
    /// # Errors
    ///
    /// Same as [`ResourceActivatorRegistry::register`].
    pub async fn register_resource(
        &self,
        kind: &str,
        manager: &nebula_resource::Manager,
        request: crate::RegisterRequest<'_>,
    ) -> Result<(), crate::RegistrarError> {
        let outcome = self
            .resource_registrars
            .register(kind, manager, request)
            .await?;
        self.record_resource_slot_identity(outcome.resource_key, outcome.slot_identity);
        Ok(())
    }

    /// Live-register under `rotation`, bind the fan-out index, and record slot identity.
    ///
    /// # Errors
    ///
    /// Same as [`ResourceActivatorRegistry::register_and_bind`].
    #[cfg(feature = "rotation")]
    pub async fn register_resource_and_bind(
        &self,
        kind: &str,
        manager: &nebula_resource::Manager,
        request: crate::RegisterRequest<'_>,
        fanout_index: Option<&nebula_resource::ResourceFanoutIndex>,
    ) -> Result<(), crate::RegistrarError> {
        let outcome = self
            .resource_registrars
            .register_and_bind(kind, manager, request, fanout_index)
            .await?;
        self.record_resource_slot_identity(outcome.resource_key, outcome.slot_identity);
        Ok(())
    }

    /// Thread in the closed `kind → typed registrar` allowlist.
    ///
    /// `impl Plugin` is the runtime source of truth for *what* a plugin
    /// contributes (`actions()` / `resources()` / `credentials()` —
    /// INTEGRATION_MODEL, "Plugin packaging" §). But `Plugin::resources()`
    /// yields `Vec<Arc<dyn nebula_resource::AnyResource>>`, and
    /// `AnyResource` is **metadata-only** (`key()` + `metadata()`, no
    /// associated types, no constructor); `#[derive(Resource)]` emits
    /// only slot plumbing (`DeclaresDependencies`, slot accessors,
    /// `HasCredentialSlots`) — it emits no per-`R` value factory and no
    /// `R::Topology` factory. The typed
    /// `Manager::register_resolved::<R>` consumes a `resource: R` and a
    /// `R::Topology` by value, monomorphized, so neither is
    /// recoverable from `dyn AnyResource`.
    ///
    /// The engine therefore cannot synthesize this allowlist by reflecting
    /// over `Plugin::resources()`. The composition root pairs each
    /// plugin-declared resource `kind` (taken from the resource's own
    /// catalog key — never guessed) with the concrete-`R`
    /// resource/topology constructors it holds (the shape
    /// [`crate::KindActivator`] takes) and threads the assembled
    /// [`ResourceActivatorRegistry`] in here — mirroring how Actions are
    /// registered by the caller (typed registration), not auto-pulled from
    /// the plugin registry. Scope is row/activation context, not a
    /// plugin-declaration field, and is supplied per-call via
    /// [`RegisterRequest`](crate::RegisterRequest) — never defaulted, since
    /// a wrong scope is an isolation hole.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resource_registrars(mut self, registrars: ResourceActivatorRegistry) -> Self {
        self.resource_registrars = registrars;
        self
    }

    /// Wire a resolved plugin's actions into the engine's executable registry.
    ///
    /// Registers every action factory declared by `plugin` into the engine's
    /// live [`ActionRegistry`](crate::ActionRegistry) (making those actions
    /// dispatchable) **and** records the plugin in the engine's
    /// [`PluginRegistry`] (making its metadata queryable).
    ///
    /// # Idempotency / duplicate policy
    ///
    /// - Registering the **same plugin key** twice is rejected with
    ///   `PluginWiringError::DuplicatePlugin`.
    /// - If any **action key** contributed by the plugin already exists in the
    ///   `ActionRegistry`, wiring is aborted before any mutation and
    ///   `PluginWiringError::DuplicateActionKey` is returned.
    ///
    /// # Out of scope (explicit deferrals)
    ///
    /// Resource and credential wiring are not handled here; see
    /// `crate::plugin_wiring::PluginWiringError` doc for rationale. Actions
    /// that require no resources or credentials work end-to-end via this path.
    ///
    /// # Errors
    ///
    /// Returns `PluginWiringError` on duplicate plugin or duplicate action key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use std::sync::Arc;
    /// use nebula_engine::{WorkflowEngine, PluginWiringError};
    /// use nebula_plugin::ResolvedPlugin;
    ///
    /// let plugin = Arc::new(ResolvedPlugin::from(MyPlugin::new())?);
    /// let engine = WorkflowEngine::new(runtime, metrics)?
    ///     .with_plugin(plugin)?;
    /// ```
    pub fn with_plugin(
        mut self,
        plugin: Arc<nebula_plugin::ResolvedPlugin>,
    ) -> Result<Self, crate::plugin_wiring::PluginWiringError> {
        use crate::plugin_wiring::PluginWiringError;

        let plugin_key = plugin.key().clone();

        // Guard: duplicate plugin key in the plugin registry.
        if self.plugin_registry.contains(&plugin_key) {
            tracing::warn!(
                target: "nebula_engine::plugin_wiring",
                plugin_key = %plugin_key,
                "with_plugin: rejected duplicate plugin key"
            );
            return Err(PluginWiringError::DuplicatePlugin { plugin_key });
        }

        // Guard: pre-flight check for duplicate action keys before any mutation.
        // `ActionRegistry::get_factory` uses `&self` (interior-mutable DashMap),
        // so we can probe without taking ownership of the registry.
        for (action_key, _factory) in plugin.actions() {
            if self.runtime.registry().get_factory(action_key).is_some() {
                tracing::warn!(
                    target: "nebula_engine::plugin_wiring",
                    plugin_key = %plugin_key,
                    action_key = %action_key,
                    "with_plugin: rejected duplicate action key"
                );
                return Err(PluginWiringError::DuplicateActionKey {
                    plugin_key,
                    action_key: action_key.clone(),
                });
            }
        }

        // on_load runs before any mutation: a failing hook aborts wiring with
        // nothing registered (atomicity). ADR-0095 D2 load contract.
        plugin.plugin().on_load().map_err(|source| {
            tracing::warn!(
                target: "nebula_engine::plugin_wiring",
                plugin_key = %plugin_key,
                error = %source,
                "with_plugin: on_load hook failed — wiring aborted"
            );
            PluginWiringError::OnLoad {
                plugin_key: plugin_key.clone(),
                source,
            }
        })?;

        // on_load succeeded — commit the registrations.
        let span = tracing::info_span!(
            "nebula_engine::plugin_wiring::with_plugin",
            plugin_key = %plugin_key,
        );
        let _guard = span.enter();

        for (action_key, factory) in plugin.actions() {
            let metadata = factory.metadata().clone();
            self.runtime
                .registry()
                .register_factory(metadata, Arc::clone(factory));
            tracing::debug!(
                target: "nebula_engine::plugin_wiring",
                %action_key,
                "registered action factory from plugin"
            );
        }

        // Register into the plugin registry for metadata / catalog queries.
        // Both guards above confirmed the key is absent; surface any unexpected
        // registry fault as a typed error rather than a panic.
        self.plugin_registry
            .register(plugin)
            .map_err(|_| PluginWiringError::DuplicatePlugin {
                plugin_key: plugin_key.clone(),
            })?;

        tracing::info!(
            target: "nebula_engine::plugin_wiring",
            plugin_key = %plugin_key,
            "plugin wired into engine"
        );

        Ok(self)
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
    /// use nebula_credential::runtime::CredentialResolver;
    /// use nebula_storage::credential::SqliteCredentialStore;
    ///
    /// let store = Arc::new(SqliteCredentialStore::connect("sqlite://creds.db").await?);
    /// let coord = Arc::new(nebula_engine::credential::default_in_memory_coordinator()?);
    /// let transport = Arc::new(nebula_api::ports::ReqwestRefreshTransport::default());
    /// let resolver = Arc::new(CredentialResolver::with_dependencies(store, coord, transport));
    ///
    /// let engine = WorkflowEngine::new(runtime, metrics)?
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
    /// let engine = WorkflowEngine::new(runtime, metrics)?
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
    /// Per sub-spec + the engine spawns a periodic task that
    /// calls `RefreshClaimRepo::reclaim_stuck`, routes
    /// `RefreshInFlight`-flagged stale claims through
    /// [`nebula_credential::runtime::SentinelTrigger`], and publishes
    /// `CredentialEvent::ReauthRequired` once the rolling-window
    /// threshold is exceeded.
    ///
    /// The composition root constructs the handle via
    /// [`nebula_credential::runtime::ReclaimSweepHandle::spawn`] and
    /// passes it here. Storing the handle on the engine ensures the
    /// task is aborted when the engine drops (clean shutdown).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_credential_reclaim_sweep(
        mut self,
        handle: nebula_credential::runtime::ReclaimSweepHandle,
    ) -> Self {
        self.credential_reclaim_sweep = Some(handle);
        self
    }

    /// Declare the credential IDs an action is permitted to acquire.
    ///
    /// The engine enforces a **deny-by-default** allowlist (see `PRODUCT_CANON` /// and ). When a node whose `action_key == action` runs, only the
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
    /// let engine = WorkflowEngine::new(runtime, metrics)?
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

    /// Set the spec-16 storage-port bundle for persistent execution state.
    ///
    /// When set, the engine persists execution state after creation and
    /// after each node completes through these scoped port handles (state
    /// CAS via [`nebula_storage_port::TransitionBatch`], lease fencing,
    /// node results, idempotency) and threads its lease
    /// [`nebula_storage_port::FencingToken`] into every committed batch.
    /// Without it, state is in-memory only (single-process library mode).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_execution_stores(mut self, stores: crate::store_seam::ExecutionStores) -> Self {
        self.stores = Some(stores);
        self
    }

    /// Set the spec-16 workflow-definition port bundle for the resume
    /// path. Required for [`resume_execution`]; when not set,
    /// `resume_execution` returns an error.
    ///
    /// [`resume_execution`]: Self::resume_execution
    #[must_use = "builder methods must be chained or built"]
    pub fn with_workflow_stores(mut self, stores: crate::store_seam::WorkflowStores) -> Self {
        self.workflow_stores = Some(stores);
        self
    }

    /// Attach an event bus for real-time execution monitoring.
    ///
    /// When set, the engine publishes [`ExecutionEvent`]s for node
    /// lifecycle transitions (started, completed, failed, skipped) and
    /// execution completion. Used by the CLI TUI for live monitoring and
    /// by the spec 28 subscribers (storage writer, metrics
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
    pub(crate) fn emit_event(&self, event: ExecutionEvent) {
        if let Some(bus) = &self.event_bus {
            let _ = bus.emit(event);
        }
    }

    /// Emit [`ExecutionEvent::FrontierIntegrityViolation`] when the    /// guard has populated a non-terminal payload. Called at every finish
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
    ///
    /// # Lease semantics (intentional, ROADMAP §M2.2 / T5)
    ///
    /// `replay_execution` mints a **fresh** `ExecutionId` per call and
    /// runs the frontier loop without acquiring a lease — there is no
    /// `acquire_and_heartbeat_lease` call on this path. The replay is
    /// a new logical execution: it does not contend with a sibling
    /// runner that holds the lease for the source execution, and it
    /// has its own (fresh) lease-less identity. This invariant is
    /// locked down by the `replay_does_not_contend_for_held_lease`
    /// integration test in `crates/engine/tests/lease_takeover.rs`.
    pub async fn replay_execution(
        &self,
        scope: &Scope,
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
        // Replay is lease-less and not published into the `running` registry,
        // so no `Resume` can target it. Drop the Sender immediately: the
        // receiver's first `recv()` then yields `None` (the
        // `ResumeChannelClosed` no-op arm), satisfying `run_frontier`'s
        // contract without ever delivering a Resume.
        let mut resume_rx = {
            let (_resume_tx, resume_rx) = mpsc::channel::<ResumeRequest>(1);
            resume_rx
        };
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
        let workflow_retry_policy = workflow.config.retry_policy.clone();

        // Run the frontier loop — same as execute_workflow, just different seed + pre-populated
        // outputs.
        let failed_node = self
            .run_frontier(
                scope,
                &graph,
                &node_map,
                &outputs,
                &semaphore,
                &cancel_token,
                &mut resume_rx,
                &mut exec_state,
                execution_id,
                workflow.id,
                &input,
                &mut repo_version,
                // replay_execution is intentionally lease-less (no
                // fencing token); checkpoints take the legacy path.
                None,
                &budget,
                &started,
                error_strategy,
                workflow_retry_policy,
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
        // `ExecutionResult` the engine returns.
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
    /// state (a durability invariant).
    ///
    /// Returns `Ok(None)` when no `execution_repo` is configured — in
    /// that mode the engine is a single-process library with no
    /// coordination seam, and the caller proceeds without a lease.
    async fn acquire_and_heartbeat_lease(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        frontier_cancel: CancellationToken,
    ) -> Result<Option<LeaseGuard>, EngineError> {
        let holder = self.instance_id.to_string();
        let ttl = self.lease_ttl;
        let heartbeat_interval = self.lease_heartbeat_interval;

        // Dual-dispatch lease acquisition: spec-16 port (returns the
        // fencing token threaded into every commit) when stores are
        // configured, else the legacy `ExecutionRepo` (holder-string
        // lease, no fencing). A live lease held by another runner
        // surfaces as `EngineError::Leased` on both paths.
        let backend: crate::store_seam::LeaseBackend = if let Some(stores) = self.stores.clone() {
            let token = stores
                .execution
                .acquire_lease(scope, &execution_id.to_string(), &holder, ttl)
                .await
                .map_err(|e| EngineError::PlanningFailed(format!("acquire lease: {e}")))?;
            let Some(token) = token else {
                let labels = self
                    .metrics
                    .interner()
                    .single("reason", engine_lease_contention_reason::ALREADY_HELD);
                self.metrics
                    .counter_labeled(NEBULA_ENGINE_LEASE_CONTENTION_TOTAL, &labels)?
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
            };
            crate::store_seam::LeaseBackend::new(stores.execution.clone(), scope.clone(), token)
        } else {
            // No storage seam configured — single-process library mode,
            // proceed without a lease.
            return Ok(None);
        };

        tracing::debug!(
            %execution_id,
            %holder,
            ttl_secs = ttl.as_secs(),
            heartbeat_secs = heartbeat_interval.as_secs(),
            "execution lease acquired"
        );

        // Spawn a heartbeat task. A shared `heartbeat_lost` token
        // trips when a renew returns `Ok(false)` (stolen or expired) or
        // errors — the frontier loop observes it via the
        // caller-provided `frontier_cancel` which we mirror.
        let heartbeat_lost = CancellationToken::new();
        let heartbeat_backend = backend.clone();
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
                        match heartbeat_backend
                            .renew(execution_id, ttl)
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
                                if let Ok(c) = metrics.counter_labeled(
                                    NEBULA_ENGINE_LEASE_CONTENTION_TOTAL,
                                    &labels,
                                ) {
                                    c.inc();
                                }
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
                                if let Ok(c) = metrics.counter_labeled(
                                    NEBULA_ENGINE_LEASE_CONTENTION_TOTAL,
                                    &labels,
                                ) {
                                    c.inc();
                                }
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
            backend,
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
    ///
    /// This method is used in tests and local library mode. Production code
    /// enters the engine via [`Self::resume_execution`] (which carries the
    /// real per-message tenant scope from the control-queue / job-dispatch row).
    /// Tests and library-mode callers pass [`crate::store_seam::single_tenant_scope`]
    /// (or a real scope) explicitly — this method does not manufacture one.
    pub async fn execute_workflow(
        &self,
        scope: &Scope,
        workflow: &WorkflowDefinition,
        input: serde_json::Value,
        budget: ExecutionBudget,
    ) -> Result<ExecutionResult, EngineError> {
        self.execute_workflow_scoped(scope, workflow, input, budget, None)
            .await
    }

    /// Like [`execute_workflow`](Self::execute_workflow) with per-run resource-acquire scope.
    ///
    /// `scope` is the storage tenant scope; pass
    /// [`crate::store_seam::single_tenant_scope`] for tests and library mode.
    ///
    /// `run_acquire_scope` supplies `org_id` / `workspace_id` for resource acquisition.
    /// Fields set here override the engine default from
    /// [`with_resource_acquire_scope`](Self::with_resource_acquire_scope).
    ///
    /// Production code uses [`resume_execution`](Self::resume_execution).
    pub async fn execute_workflow_with_acquire_scope(
        &self,
        scope: &Scope,
        workflow: &WorkflowDefinition,
        input: serde_json::Value,
        budget: ExecutionBudget,
        run_acquire_scope: Option<nebula_core::scope::Scope>,
    ) -> Result<ExecutionResult, EngineError> {
        self.execute_workflow_scoped(scope, workflow, input, budget, run_acquire_scope)
            .await
    }

    /// Internal implementation — thread `scope` through the storage port calls.
    ///
    /// Called by [`execute_workflow`], [`execute_workflow_with_acquire_scope`], and
    /// [`replay_execution`] with a caller-supplied scope; called by
    /// [`resume_execution`] with the per-message tenant scope from the
    /// control-queue / job-dispatch row.
    async fn execute_workflow_scoped(
        &self,
        scope: &Scope,
        workflow: &WorkflowDefinition,
        input: serde_json::Value,
        budget: ExecutionBudget,
        run_acquire_scope: Option<nebula_core::scope::Scope>,
    ) -> Result<ExecutionResult, EngineError> {
        budget
            .validate_for_execution()
            .map_err(|msg| EngineError::PlanningFailed(msg.to_string()))?;

        let execution_id = ExecutionId::new();
        let started = Instant::now();

        self.install_execution_resource_context(execution_id, run_acquire_scope);
        let slot_by_exec = &self.resource_slot_identities_by_execution;
        let acquire_by_exec = &self.execution_acquire_scopes;
        let _execution_resource_guard = scopeguard::guard(execution_id, move |id| {
            slot_by_exec.remove(&id);
            acquire_by_exec.remove(&id);
        });

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
        // same concurrency, timeout, and output-size limits the
        // original run was configured with, rather than falling back
        // to `ExecutionBudget::default()` (issue #289).
        exec_state.set_budget(budget.clone());

        // 4b. Persist initial execution state. The spec-16 port `create`
        // starts the row at version 0 (the first `commit` CASes against
        // `expected_version == 0`); the checkpoints below track from this
        // baseline.
        let mut repo_version: u64 = 0;
        if let Some(stores) = &self.stores {
            let state_json = serde_json::to_value(&exec_state)
                .map_err(|e| EngineError::PlanningFailed(format!("serialize state: {e}")))?;
            stores
                .execution
                .create(
                    scope,
                    &execution_id.to_string(),
                    &workflow.id.to_string(),
                    state_json,
                )
                .await
                .map_err(|e| EngineError::PlanningFailed(format!("persist initial state: {e}")))?;
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
            .acquire_and_heartbeat_lease(scope, execution_id, cancel_token.clone())
            .await?;

        // Fencing token threaded into every checkpoint / final-state
        // commit. `Some` when a port lease is held; `None` only in
        // single-process library mode (no lease).
        let fencing = lease.as_ref().and_then(LeaseGuard::fencing_token);

        // 5b. Publish the cancel token into the running registry ONLY
        // after the lease is ours. The lease is the authoritative
        // single-runner fence ; publishing after it prevents
        // an overlapping attempt for the same `ExecutionId` from
        // overwriting the live token (#482 Copilot review). The guard's
        // nonce-scoped `Drop` (`RunningRegistration::drop`) removes the
        // entry on every exit path — normal completion, heartbeat-lost
        // `Leased`, final-persist errors — and is defensive against
        // clobbering a winner's registration if a losing attempt ever
        // slips through.
        let registration_id = NEXT_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed);
        // Live-frontier resume channel (W-S2b): the Sender is published on the
        // `RunningEntry` alongside the cancel token so a `Resume` for this
        // still-`Running` execution (a signal wait parked with a timeout)
        // reaches the live loop; the Receiver is owned by `run_frontier`. The
        // ack on each `ResumeRequest` gates the control-queue ack on the
        // durable self-arm checkpoint (P1#1).
        let (resume_tx, mut resume_rx) = mpsc::channel::<ResumeRequest>(RESUME_CHANNEL_CAPACITY);
        self.running.insert(
            execution_id,
            RunningEntry {
                registration_id,
                token: cancel_token.clone(),
                resume_tx,
            },
        );
        let _cancel_registration = RunningRegistration {
            running: Arc::clone(&self.running),
            execution_id,
            registration_id,
        };

        // 6. Record start metric
        self.workflow_executions_started.inc();

        // 7. Build node lookup map
        let node_map: HashMap<NodeKey, &nebula_workflow::NodeDefinition> =
            workflow.nodes.iter().map(|n| (n.id.clone(), n)).collect();

        // 8. Shared output storage (concurrent access from worker tasks)
        let outputs: Arc<DashMap<NodeKey, serde_json::Value>> = Arc::new(DashMap::new());
        let semaphore = Arc::new(Semaphore::new(budget.max_concurrent_nodes));

        // 9. Execute using frontier-based loop
        let error_strategy = workflow.config.error_strategy;
        let workflow_retry_policy = workflow.config.retry_policy.clone();
        let seed_nodes: Vec<NodeKey> = graph.entry_nodes();
        let failed_node = self
            .run_frontier(
                scope,
                &graph,
                &node_map,
                &outputs,
                &semaphore,
                &cancel_token,
                &mut resume_rx,
                &mut exec_state,
                execution_id,
                workflow.id,
                &input,
                &mut repo_version,
                fencing,
                &budget,
                &started,
                error_strategy,
                workflow_retry_policy,
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
        // `ExecutionResult` the engine returns.
        if final_status == ExecutionStatus::Cancelled
            && exec_state.status == ExecutionStatus::Running
        {
            let _ = exec_state.transition_status(ExecutionStatus::Cancelling);
        }
        let _ = exec_state.transition_status(final_status);

        // If the heartbeat lost the lease mid-run, a sibling runner
        // now owns the canonical state. We MUST NOT persist the final
        // state or emit ExecutionFinished from this runner — the new
        // holder will drive completion — control-queue lease handoff.
        let reported_status = if heartbeat_lost {
            tracing::error!(
                %execution_id,
                "final state persistence skipped: heartbeat lost this runner's lease; \
                 another runner now owns the execution (#325)"
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
        } else if self.stores.is_some() {
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
                .persist_final_state(
                    scope,
                    execution_id,
                    &mut exec_state,
                    &mut repo_version,
                    fencing,
                )
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

    /// Test-only accessor: ask the injected clock for the current time.
    ///
    /// Used by tests that call [`with_clock`] with a fake clock to assert
    /// that the injected clock is actually stored and consulted — without
    /// needing to drive a full workflow execution.
    #[cfg(test)]
    pub(crate) fn clock_now(&self) -> DateTime<Utc> {
        self.clock.now()
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
    /// Workflow node being executed. Passed through to
    /// [`ActionRuntime::execute_action_with_node`] so the dispatch path
    /// can hand the [`NodeDefinition`] to
    /// [`nebula_action::ActionFactory::instantiate`] (slot bindings,
    /// parameters, version pinning live on the node).
    node: Arc<nebula_workflow::NodeDefinition>,
    /// Pinned interface version for versioned action lookup.
    ///
    /// When `Some`, the runtime uses [`execute_action_with_node`] with this
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
        // is activated (split from the old `handle_node_failure` per #297). This
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

        let base = match nebula_core::BaseContext::builder(nebula_core::scope::Scope {
            execution_id: Some(self.execution_id),
            workflow_id: Some(self.workflow_id),
            ..nebula_core::scope::Scope::default()
        })
        .principal(nebula_core::scope::Principal::System)
        .cancellation(self.cancel.child_token())
        .build()
        {
            Ok(ctx) => Arc::new(ctx),
            Err(e) => {
                return (
                    self.node_key,
                    Err(EngineError::PlanningFailed(e.to_string())),
                );
            },
        };
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

        // Production dispatch via the factory path: the runtime calls
        // `factory.instantiate(node, ctx)` to build a fresh erased
        // action, then dispatches the matching variant. The factory
        // spine is the sole dispatch path as of ADR-0098 D0 PR3.
        let result = self
            .runtime
            .execute_action_with_node(
                &self.node,
                self.interface_version.as_ref(),
                self.input,
                &action_ctx,
                None,
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
/// Port-driven routing (spec 28 ): the engine matches the edge's
/// effective source port (`from_port`, defaulting to `"main"`) against the
/// port the upstream `ActionResult` produced on. There is no "edge
/// condition" — conditionals are carried by explicit `ControlAction` nodes
/// (e.g. `If`, `Switch`, `Router` — the canonical 7 from
/// `nebula_action::control`) whose own `ActionResult` decides which port
/// fires.
///
/// Rules:
/// - `Skip` / `Drop` / `Terminate` → no edges activate on any port.
/// - `Wait` → no edges activate from the `Wait` result; downstream edges
///   are gated until the node reaches `Completed` (when the wait condition
///   is satisfied). This is defense-in-depth: the park path in Phase 3
///   structurally skips `process_outgoing_edges` on `Wait`, so this guard
///   fires only if that skip is somehow bypassed.
/// - Failed node → only edges with `from_port == "error"` activate; action authors wire their
///   failure path to whichever `ControlAction` fits (typically a `Switch` keyed on error class) or
///   to a recovery node.
/// - `Success` → activates edges on `"main"` (or `None`).
/// - `Branch { selected }` → activates edges whose effective source port equals `selected` (legacy
///   alias for `Route`).
/// - `Route { port }` → activates edges whose effective source port equals `port`.
/// - `MultiOutput { outputs }` → activates edges whose effective source port is present in
///   `outputs`.
/// - `Continue` / `Break` → engine treats these like `Success` for edge activation (they
///   hit the main port); persistent state handling lives outside this routing decision.
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

    // Wait gates all downstream edges until the wait condition is
    // satisfied (the park path in Phase 3 structurally skips
    // `process_outgoing_edges`, so this is defense-in-depth).
    if matches!(result, Some(Wait { .. })) {
        return false;
    }

    let effective_port = conn.effective_from_port();

    // Failures route exclusively through the `"error"` port. Downstream
    // must wire an explicit `ControlAction` (typically a `Switch` keyed on
    // error class) or recovery node to fan out by error class.
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
    None
}

/// Drain both the retry-pending min-heap and the ready_queue,
/// transitioning every parked / queued node to `Cancelled` and
/// clearing any `next_attempt_at`.
///
/// Called by all three frontier-loop teardown paths (Phase 2 cancel
/// short-circuit, `WakeReason::Cancel`, `WakeReason::WallClock`) per
/// T5 acceptance: cancel/terminate/budget breach must
/// not leave non-terminal nodes behind, otherwise the frontier integrity (CAS on version)
/// frontier-integrity guard fires `FrontierIntegrityViolation`
/// instead of the honest `Cancelled` / `TimedOut` final status.
///
/// Best-effort: transition errors are logged and ignored. The
/// outer `persist_final_state` covers the durable persist; failures
/// here only affect post-mortem log fidelity.
fn drain_pending_to_cancelled(
    retry_heap: &mut BinaryHeap<Reverse<(DateTime<Utc>, NodeKey)>>,
    wait_heap: &mut BinaryHeap<Reverse<(DateTime<Utc>, NodeKey)>>,
    ready_queue: &mut VecDeque<NodeKey>,
    exec_state: &mut ExecutionState,
    execution_id: ExecutionId,
) {
    while let Some(Reverse((_, parked))) = retry_heap.pop() {
        let cancelled = exec_state.transition_node(parked.clone(), NodeState::Cancelled);
        if let Some(ns) = exec_state.node_states.get_mut(&parked) {
            ns.next_attempt_at = None;
        }
        match cancelled {
            Ok(()) => tracing::debug!(
                target = "engine::retry",
                %execution_id,
                node_key = %parked,
                "WaitingRetry → Cancelled (cancel observed during backoff)"
            ),
            Err(e) => tracing::warn!(
                target = "engine::retry",
                %execution_id,
                node_key = %parked,
                error = %e,
                "WaitingRetry → Cancelled rejected during cancel drain"
            ),
        }
    }
    // Drain parked wait nodes — `Waiting → Cancelled` lets a cancelled
    // execution tear down nodes that were parked for an external
    // condition without observing a phantom `Completed` step.
    while let Some(Reverse((_, waiting))) = wait_heap.pop() {
        let cancelled = exec_state.transition_node(waiting.clone(), NodeState::Cancelled);
        if let Some(ns) = exec_state.node_states.get_mut(&waiting) {
            // Clear both timer fields together: a signal+timeout wait carries
            // `wait_wake = Some(..)` as well as `next_attempt_at`, and the
            // next_attempt_at/wait_wake invariant must hold even on a terminal
            // Cancelled node. `clear_wait_timer` is safe here — wait_heap nodes
            // are `Waiting` (not WaitingRetry), so `next_attempt_at` is a park
            // timer, not a retry timer.
            ns.clear_wait_timer();
        }
        match cancelled {
            Ok(()) => tracing::debug!(
                target = "engine::wait",
                %execution_id,
                node_key = %waiting,
                "Waiting → Cancelled (cancel observed while parked)"
            ),
            Err(e) => tracing::warn!(
                target = "engine::wait",
                %execution_id,
                node_key = %waiting,
                error = %e,
                "Waiting → Cancelled rejected during cancel drain"
            ),
        }
    }
    // Signal-driven waits (`next_attempt_at == None`) AND their blocked
    // downstream `Pending` nodes are intentionally NOT represented in the timer
    // heaps or the ready_queue, so the drains above never visit them. Scan
    // `node_states` for ANY remaining non-terminal node and cancel it —
    // otherwise a cancel observed while a signal wait is parked would leave the
    // wait node and/or its blocked downstream non-terminal under a `Cancelled`
    // execution (a state-machine leak). Nodes already cancelled above (heaps /
    // ready_queue) are terminal and skipped.
    let stranded_non_terminal: Vec<NodeKey> = exec_state
        .node_states
        .iter()
        .filter(|(_, ns)| !ns.state.is_terminal())
        .map(|(id, _)| id.clone())
        .collect();
    for node_key in stranded_non_terminal {
        let cancelled = exec_state.transition_node(node_key.clone(), NodeState::Cancelled);
        if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
            ns.next_attempt_at = None;
        }
        match cancelled {
            Ok(()) => tracing::debug!(
                target = "engine::wait",
                %execution_id,
                %node_key,
                "stranded non-terminal node → Cancelled (cancel teardown; not on heap/queue)"
            ),
            Err(e) => tracing::warn!(
                target = "engine::wait",
                %execution_id,
                %node_key,
                error = %e,
                "stranded non-terminal node → Cancelled rejected during cancel drain"
            ),
        }
    }
    while let Some(queued) = ready_queue.pop_front() {
        // `Ready → Cancelled` is in the canonical transition table.
        // Pending nodes that landed here straight from the seed
        // (no Phase 0 promotion) also accept `Pending → Cancelled`.
        match exec_state.transition_node(queued.clone(), NodeState::Cancelled) {
            Ok(()) => tracing::debug!(
                target = "engine::retry",
                %execution_id,
                node_key = %queued,
                "ready_queue node cancelled before dispatch"
            ),
            Err(e) => tracing::warn!(
                target = "engine::retry",
                %execution_id,
                node_key = %queued,
                error = %e,
                "ready_queue node → Cancelled rejected before dispatch"
            ),
        }
    }
}

/// Resolve the effective per-node retry policy per T4.
///
/// Resolution order (more specific wins):
/// 1. `NodeDefinition.retry_policy` — operator-declared per-node policy.
/// 2. `workflow_default` — `WorkflowConfig.retry_policy`, the workflow-wide default applied to
///    nodes that do not declare their own.
/// 3. `None` — no engine-level retry; the failure flows straight to the existing
///    classify+route+checkpoint path.
fn effective_retry_policy<'a>(
    node_def: &'a nebula_workflow::NodeDefinition,
    workflow_default: Option<&'a nebula_workflow::RetryConfig>,
) -> Option<&'a nebula_workflow::RetryConfig> {
    node_def.retry_policy.as_ref().or(workflow_default)
}

/// The retry decision for a just-failed node attempt.
///
/// Pure: depends only on the per-node attempt count, the resolved
/// policy, and the execution-level budget. Does not mutate any state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryDecision {
    /// No retry — fall through to the existing
    /// classify+route+checkpoint path.
    Finalize,
    /// Schedule a retry after `delay`. The frontier loop transitions
    /// the node to `WaitingRetry`, stamps `next_attempt_at = now() + delay`,
    /// increments the global counter, and parks the node on the
    /// retry-pending heap.
    Retry { delay: Duration },
}

/// Decide whether the just-failed dispatch of `node_key` should be
/// retried per T4 acceptance.
///
/// Ordering of checks (whichever caps first wins):
///
/// 0. **Fatal-error short-circuit** — if the just-recorded attempt's typed
///    [`ActionError`] is fatal ([`ActionError::is_fatal`]), finalize
///    immediately, *before* any policy/budget check. Retry is otherwise a
///    pure attempts/budget/backoff policy that never consulted error
///    fatality at all — so a `Fatal` action error (or a runner
///    after-send close, which maps to fatal) used to be re-dispatched
///    under policy. This early-return makes "bytes reached the plugin ⇒
///    never re-dispatch" structural for *all* actions and closes that
///    pre-existing, dispatch-independent gap.
/// 1. **Global budget cap** — `ExecutionState::has_exhausted_retry_budget` consults
///    `ExecutionBudget.max_total_retries`. A `Some(0)` cap disables retry entirely; a `None` cap
///    leaves the per-node policy as the only gate.
/// 2. **Per-node policy presence** — no policy → no retry.
/// 3. **Per-node policy max_attempts** — once `attempts.len()` (the number of completed attempts at
///    the time of this decision) has reached `policy.max_attempts`, the retry budget is exhausted.
/// 4. **Backoff calc** — `policy.delay_for_attempt(attempt_count - 1)` where `attempt_count` is the
///    just-finished attempt number (1-indexed). This yields the same wait the
///    `nebula-resilience::retry` crate would for the same `RetryConfig`.
///
/// `recorded_error` is the typed error of the attempt just pushed to
/// history (the runtime-failure path supplies it; the setup-failure path
/// passes `None` because a param-resolution failure has no `ActionError`
/// and stays retry-eligible — "the action never started").
/// Whether a just-recorded node failure is **terminal** — never re-dispatched,
/// whatever the retry policy says.
///
/// A fatal [`ActionError`] (direct, or wrapped in `RuntimeError::ActionError`)
/// is terminal, preserving the long-standing fatal-action short-circuit. A
/// non-`ActionError` runtime condition is terminal unless it is explicitly
/// retryable: the iteration cap, stuck-state, data-limit, agent turn-budget,
/// and unsupported-wait variants finalize, while `AgentTurnTimeout` stays
/// retryable.
///
/// The ActionError case routes through `is_fatal` rather than
/// `EngineError::is_retryable`: the `RuntimeError::ActionError` variant carries
/// `retryable = false` in its own classify metadata, which would otherwise
/// shadow the inner action error's real retryability and stop a legitimately
/// retryable action error from retrying.
fn error_is_terminal(err: &EngineError) -> bool {
    match err.as_action_error() {
        Some(action_err) => action_err.is_fatal(),
        None => !nebula_error::Classify::is_retryable(err),
    }
}

fn compute_retry_decision(
    node_key: &NodeKey,
    exec_state: &ExecutionState,
    retry_policy: Option<&nebula_workflow::RetryConfig>,
    recorded_error_is_terminal: bool,
) -> RetryDecision {
    if recorded_error_is_terminal {
        tracing::debug!(
            target = "engine::retry",
            execution_id = %exec_state.execution_id,
            %node_key,
            "retry skipped: just-recorded attempt error is terminal \
             (fatal action error or non-retryable runtime condition) — no re-dispatch"
        );
        return RetryDecision::Finalize;
    }

    if exec_state.has_exhausted_retry_budget() {
        tracing::debug!(
            target = "engine::retry",
            execution_id = %exec_state.execution_id,
            %node_key,
            total_retries = exec_state.total_retries,
            "retry skipped: ExecutionBudget.max_total_retries cap reached"
        );
        return RetryDecision::Finalize;
    }

    let Some(policy) = retry_policy else {
        return RetryDecision::Finalize;
    };

    // Missing node state means the engine has no history we can
    // base a retry decision on (programming error: only nodes the
    // engine has dispatched are eligible). Refuse the retry rather
    // than fabricating an `attempts_used = 0` for a stranger node —
    // that would let a programming bug schedule retries on
    // unbounded state (hot-path safety).
    let Some(ns) = exec_state.node_states.get(node_key) else {
        tracing::warn!(
            target = "engine::retry",
            execution_id = %exec_state.execution_id,
            %node_key,
            "retry skipped: node state missing for retry decision"
        );
        return RetryDecision::Finalize;
    };
    // `attempts.len()` is the count of *completed* attempts at the
    // moment of decision (post-push of the just-failed attempt). Once
    // it reaches `max_attempts`, the budget is spent.
    let attempts_used = ns.attempts.len() as u32;

    if attempts_used >= policy.max_attempts {
        tracing::debug!(
            target = "engine::retry",
            execution_id = %exec_state.execution_id,
            %node_key,
            attempts_used,
            max_attempts = policy.max_attempts,
            "retry skipped: per-node max_attempts reached"
        );
        return RetryDecision::Finalize;
    }

    // `delay_for_attempt(0)` = initial delay (after attempt #1 fails);
    // `delay_for_attempt(1)` = after attempt #2 fails; etc.
    // The just-finished attempt index is `attempts_used - 1` (0-based).
    let delay = policy.delay_for_attempt(attempts_used.saturating_sub(1));
    RetryDecision::Retry { delay }
}

/// Translate a Tokio-style retry delay to a chrono wall-clock
/// timestamp without panicking on extreme inputs.
///
/// `chrono::Duration::from_std` rejects values larger than
/// `i64::MAX` milliseconds (~292 million years). A naive
/// `.unwrap_or_default()` would silently substitute zero — turning
/// a misconfigured huge backoff into a hot-loop retry. Instead, we
/// log + clamp to `DateTime::<Utc>::MAX_UTC` so the next attempt is
/// effectively never scheduled, but the engine remains responsive
/// to cancel/wall-clock teardown signals. Using the absolute ceiling
/// avoids the `now + chrono::Duration::MAX` overflow that occurs for
/// any `now` value beyond the epoch.
///
/// `now` is supplied by the caller from an injectable [`Clock`] so
/// retry deadlines are deterministic and testable without real wall time.
fn next_retry_at(
    execution_id: ExecutionId,
    node_key: &NodeKey,
    delay: Duration,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    match chrono::Duration::from_std(delay) {
        Ok(d) => now + d,
        Err(e) => {
            tracing::warn!(
                target = "engine::retry",
                %execution_id,
                %node_key,
                error = %e,
                delay_ms = delay.as_millis() as u64,
                "retry delay is not representable by chrono; clamping retry deadline to MAX_UTC"
            );
            // `now + chrono::Duration::MAX` overflows for any `now` near the epoch.
            // Use the absolute ceiling of `DateTime<Utc>` so the node is effectively
            // never retried while the engine remains responsive to cancel/shutdown.
            DateTime::<Utc>::MAX_UTC
        },
    }
}

/// Classification of a node failure against the workflow's error strategy.
///
/// Pure function of the strategy: splits the outcome from the state
/// mutation + edge routing that used to live together in the old
/// `handle_node_failure`. Split lets `run_frontier` order `state-mutation
/// → persist → emit → route` per / #297 — routing outgoing edges
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
/// rather than leave state + outputs half-applied . Pre-review
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
/// surface — invariant holds.
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

/// Decode a persisted node-result JSON blob into a typed [`ActionResult`].
///
/// Returns `None` (caller synthesizes a flat `Success`) on a decode
/// failure, logging the regression so a lost Branch/Route/MultiOutput
/// routing is visible rather than silent (issue #299). Shared by the
/// port and legacy idempotency-replay paths so both decode identically.
fn deserialize_stored_result(
    json: serde_json::Value,
    execution_id: ExecutionId,
    node_key: &NodeKey,
) -> Option<ActionResult<serde_json::Value>> {
    match serde_json::from_value::<ActionResult<serde_json::Value>>(json) {
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
}

/// Whether a parked [`NodeState::Waiting`] node is a **signal** wait — one a
/// `Resume` can satisfy, as opposed to a pure timer wait that only a timer can.
///
/// Two shapes:
/// - signal-only: `next_attempt_at == None` (case-a; the row went `Paused`).
/// - signal + timeout: `wait_wake == Some(Timeout)` (W-S2b; the row stays
///   `Running` with the timeout deadline in `next_attempt_at`).
///
/// A timer-driven completion wait (`Until` / `Duration`) carries
/// `wait_wake == Some(Completion)` with a `next_attempt_at` — that is NOT a
/// signal wait and a Resume must never re-arm it.
fn is_signal_wait(ns: &nebula_execution::state::NodeExecutionState) -> bool {
    ns.next_attempt_at.is_none() || ns.wait_wake == Some(WaitWake::Timeout)
}

/// Whether a parked node's persisted [`WaitSignal`] identity matches a Resume's
/// [`ResumeTarget`] — the KIND-AWARE targeting rule (ADR-0099 W-S3a).
///
/// A target matches ONLY a same-kind signal with an equal identity: a
/// `Webhook` target matches a `WaitSignal::Webhook` whose `callback_id` is
/// equal, and NEVER an `Approval` or `Execution` wait. This is the structural
/// safety control that closes the kind-confusion confused-deputy bug — a
/// webhook Resume can never satisfy an approval gate, regardless of string
/// collisions.
///
/// A node with no persisted `wait_signal` (a legacy row, or a timer wait) never
/// matches a target — only an untargeted Resume (no `ResumeTarget`) arms it.
fn matches_resume_target(
    ns: &nebula_execution::state::NodeExecutionState,
    target: &ResumeTarget,
) -> bool {
    match (ns.wait_signal.as_ref(), target) {
        (
            Some(WaitSignal::Webhook { callback_id }),
            ResumeTarget::Webhook { callback_id: want },
        ) => callback_id == want,
        (Some(WaitSignal::Approval { approver }), ResumeTarget::Approval { approver: want }) => {
            approver == want
        },
        (
            Some(WaitSignal::Execution { execution_id }),
            ResumeTarget::Execution { execution_id: want },
        ) => {
            // Compare typed-to-typed: parse the target's opaque string form into
            // an `ExecutionId` (no allocation on the persisted side) and match by
            // value. A malformed target string simply never matches.
            want.parse::<ExecutionId>()
                .is_ok_and(|want_id| *execution_id == want_id)
        },
        // Any cross-kind pair, or a node with no persisted identity, never
        // matches a target — the kind-confusion safety rule.
        _ => false,
    }
}

/// Arm — for Phase-0b completion — every signal-`Waiting` node selected by
/// `resume_target`, returning the armed node keys (ADR-0099 W-S3a).
///
/// Shared by the two Resume arm sites — the `Paused` no-live-runner satisfy-CAS
/// (under the freshly-acquired lease) and the live-frontier `ResumeSignalled`
/// self-arm (under the loop's own lease). A node is selected when it is
/// `Waiting`, [`is_signal_wait`] (a signal-only OR signal+timeout wait, never a
/// timer-driven completion wait), AND matched by the target:
///
/// - `Some(target)` → KIND-AWARE match via [`matches_resume_target`]: arms only
///   the node whose persisted [`WaitSignal`] equals the target by kind +
///   identity. This is the structural close of both confused-deputy bugs (one
///   Resume satisfying a sibling wait; a webhook Resume satisfying an approval
///   gate).
/// - `None` → the legacy untargeted Resume: arms every signal-`Waiting` node
///   (preserves the W-S2b behavior).
///
/// Mutates only the in-memory `exec_state` (via [`arm_wait_completion`], the
/// paired `next_attempt_at`/`wait_wake` write); the CALLER owns the lease and
/// the durable checkpoint that commits the arm, preserving the
/// own-the-lease-before-RMW invariant (#856). Does NOT bump the version — the
/// caller does (mirroring each existing site's single bump).
///
/// [`arm_wait_completion`]: nebula_execution::state::NodeExecutionState::arm_wait_completion
/// [`WaitSignal`]: nebula_execution::state::WaitSignal
fn arm_signal_waits_under_lease(
    exec_state: &mut ExecutionState,
    resume_target: Option<&ResumeTarget>,
    now: DateTime<Utc>,
) -> Vec<NodeKey> {
    let to_arm: Vec<NodeKey> = exec_state
        .node_states
        .iter()
        .filter(|(_, ns)| {
            ns.state == NodeState::Waiting
                && is_signal_wait(ns)
                && match resume_target {
                    Some(target) => matches_resume_target(ns, target),
                    None => true,
                }
        })
        .map(|(id, _)| id.clone())
        .collect();
    for node_key in &to_arm {
        if let Some(ns) = exec_state.node_states.get_mut(node_key) {
            // Arm for COMPLETION: a satisfied signal wait wakes to complete the
            // node (`Waiting → Completed`), never to fail. The paired write
            // keeps the next_attempt_at/wait_wake invariant intact.
            ns.arm_wait_completion(now);
        }
    }
    to_arm
}

/// Mark a node as failed in the execution state.
///
/// The error message is attached **only if** the node actually transitioned to
/// `Failed`. A rejected transition (the node was already terminal — e.g.
/// `Completed` — or the key is unknown) must NOT stamp a failure message onto a
/// node that did not fail; the rejection is surfaced via `WARN` rather than
/// silently dropped.
fn mark_node_failed(exec_state: &mut ExecutionState, node_key: NodeKey, err: &EngineError) {
    if exec_state
        .transition_node(node_key.clone(), NodeState::Failed)
        .is_ok()
    {
        if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
            ns.error_message = Some(err.to_string());
        }
    } else {
        tracing::warn!(
            target = "engine",
            node_key = %node_key,
            error = %err,
            "mark_node_failed: transition to Failed rejected; error_message not written"
        );
    }
}

/// Mint a single-use resume token for a signal-park and return the minted row
/// plus the plaintext bearer `SecretString`.
///
/// ## Security invariants
///
/// - The 32-byte raw secret is generated with `rand::rng().fill_bytes` (OS
///   CSPRNG).  It is never copied into a `String` or `Vec<u8>` outside of
///   the stack frame.
/// - The **plaintext bearer** is `BASE64_STANDARD.encode(raw)` wrapped
///   directly into `SecretString::new` — no intermediate `String` binding.
///   `SecretString` zeroizes the heap allocation on drop.
/// - The **hash stored at rest** is SHA-256(raw bytes).  The preimage is the
///   32 raw bytes, NOT the base64 encoding, so the stored hash reveals nothing
///   about the bearer.
/// - The `SecretString` is returned to the caller and dropped there; it is
///   NEVER logged, traced, or formatted.
///
/// ## Idempotency
///
/// Backends insert with `ON CONFLICT(execution_id, node_key) DO NOTHING`.
/// A crash-and-redelivery that re-parks the same node sees the conflict and
/// skips the insert; the existing live token is preserved.
fn mint_park_token(
    scope: &Scope,
    execution_id: ExecutionId,
    node_key: &NodeKey,
    wait_kind: ResumeTokenWaitKind,
    callback_label: String,
    wake_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<(ResumeTokenRow, SecretString), EngineError> {
    // Wrap the raw preimage in `Zeroizing` so it is wiped on drop — the
    // 32 raw bytes are the actual secret (the base64 bearer is derived from
    // them), so they must not linger on the stack after this function returns.
    let mut raw = Zeroizing::new([0u8; 32]);
    rand::rng().fill_bytes(raw.as_mut());

    // Hash BEFORE encoding — preimage is 32 raw bytes, hash reveals nothing
    // about the base64 bearer.
    let hash_bytes = Sha256::digest(raw.as_ref());
    let token_hash = TokenHash::try_from_bytes(hash_bytes.to_vec()).map_err(|e| {
        EngineError::CheckpointFailed {
            node_key: node_key.clone(),
            reason: format!("SHA-256 output length mismatch: {e}"),
        }
    })?;

    // Plaintext bearer: base64-encoded raw bytes, wrapped directly into
    // SecretString — no intermediate String allocation stays alive.
    let plaintext_bearer = SecretString::new(BASE64_STANDARD.encode(raw.as_ref()).into());

    // Mirror the wait's timeout deadline so W-S3d can reject a token
    // presented after the node's own timeout fired.
    let expires_at = wake_at.map(|dt| dt.to_rfc3339());

    let row = ResumeTokenRow::new(
        token_hash,
        scope.clone(),
        execution_id.to_string(),
        node_key.to_string(),
        wait_kind,
        callback_label,
        now.to_rfc3339(),
        expires_at,
    );

    Ok((row, plaintext_bearer))
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
/// execution reached its final status (operational honesty).
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
    /// `docs/PRODUCT_CANON.md`.
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
    /// Which storage backend owns this lease (legacy repo vs spec-16
    /// port). Carries the fencing token on the port path.
    backend: crate::store_seam::LeaseBackend,
    execution_id: ExecutionId,
    /// Holder string for diagnostics (the legacy backend also uses it
    /// for renew/release internally).
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

    /// The fencing token this runner holds, or `None` on the legacy
    /// lease path. Threaded into every committed transition batch so the
    /// store rejects a write from a superseded holder.
    fn fencing_token(&self) -> Option<nebula_storage_port::FencingToken> {
        self.backend.fencing_token()
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
        match self.backend.release(self.execution_id).await {
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
///    nodes are non-terminal (frontier integrity (CAS on version)). `(Failed, Some(SystemError))` plus the
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

    // Priority 4a — at least one signal-driven `Waiting` node exists and no
    // non-terminal node is actively in-flight (`Running` / `WaitingRetry`).
    //
    // A signal-driven wait has `next_attempt_at == None`; the node holds no
    // worker and will not be driven by the timer arm. The frontier exits
    // naturally (all heaps empty, join-set empty) with these nodes still
    // non-terminal — which the old Priority-4 arm would falsely report as a
    // `FrontierIntegrityViolation`. The correct status is `Paused`: the
    // execution is durably suspended awaiting an external signal, not broken.
    //
    // Guards that must BOTH hold:
    //   1. `!non_terminal_signal_waits.is_empty()` — an all-terminal run
    //      must still fall through to Priority-5 `Completed`, never `Paused`.
    //   2. No non-terminal node is `Running`, `Ready`, or `WaitingRetry` —
    //      those states indicate a genuine frontier bug, not a benign park:
    //        - `Running`: the frontier exited while a worker was still live.
    //        - `Ready`: the node was activated and queued for dispatch but the
    //          frontier exited without spawning it — runnable work was
    //          stranded. (A node merely *blocked behind* a signal wait stays
    //          `Pending`: its wait predecessor is non-terminal, so it is never
    //          activated/enqueued and never reaches `Ready`.)
    //        - `WaitingRetry`: the frontier exit condition requires
    //          `retry_heap.is_empty()`; a `WaitingRetry` node present at
    //          exit means its heap entry was lost — an anomaly that must
    //          surface as a `FrontierIntegrityViolation`, not be masked as
    //          `Paused`.
    //        - timer `Waiting{next_attempt_at: Some(_)}`: the loop only exits
    //          once `wait_heap.is_empty()`, and every timer wait is drained to
    //          `Completed` by Phase-0b before that. A timer wait still present
    //          at exit means its `wait_heap` entry was lost — same lost-entry
    //          anomaly as `WaitingRetry`.
    //      Only genuinely-blocked `Pending` nodes are expected alongside a
    //      signal wait (they activate once the wait is satisfied via
    //      `dispatch_resume`).
    {
        let non_terminal_signal_waits: Vec<NodeKey> = exec_state
            .node_states
            .iter()
            .filter(|(_, ns)| !ns.state.is_terminal())
            .filter(|(_, ns)| ns.state == NodeState::Waiting && ns.next_attempt_at.is_none())
            .map(|(id, _)| id.clone())
            .collect();
        let has_non_benign_non_terminal_node = exec_state
            .node_states
            .values()
            .filter(|ns| !ns.state.is_terminal())
            .any(|ns| {
                matches!(
                    ns.state,
                    NodeState::Running | NodeState::Ready | NodeState::WaitingRetry
                ) || (ns.state == NodeState::Waiting && ns.next_attempt_at.is_some())
            });
        if !non_terminal_signal_waits.is_empty() && !has_non_benign_non_terminal_node {
            tracing::info!(
                target = "engine::final_status",
                execution_id = %exec_state.execution_id,
                parked_node_count = non_terminal_signal_waits.len(),
                "final_status_decided (priority 4a: signal-driven waits present, \
                 no in-flight nodes — execution paused awaiting external signal)"
            );
            return FinalStatusDecision {
                status: ExecutionStatus::Paused,
                termination_reason: None,
                integrity_violation: None,
            };
        }
    }

    // Priority 4 — frontier integrity violation (frontier integrity (CAS on version)).
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

/// W-S3e — best-effort cleanup of un-consumed resume tokens after an execution
/// reaches a terminal state.
///
/// Called POST-commit from the engine's terminal sinks (the consolidated final
/// state persist and the no-live-runner cancel-of-parked cleanup). The revoke is
/// deliberately NOT atomic with the terminal transition: mint-on-park rides the
/// `TransitionBatch` so state and token can't diverge on a crash, but the
/// terminal-side cleanup is a separate call. A crash in the window leaves only
/// un-reachable dead token rows, backstopped by the `port_resume_tokens`
/// `ON DELETE CASCADE` FK and the no-op of a resume targeting a terminal
/// execution (see the `nebula_storage_port::store::resume_token` module docs).
///
/// Because the execution is already durably terminal, a revoke failure must
/// never propagate — it is logged at `warn!` (error + `execution_id` only, no
/// token or hash material) and swallowed. A successful revoke logs the count at
/// `debug!` so the cleanup path is observable.
async fn revoke_resume_tokens_best_effort(
    stores: &crate::store_seam::ExecutionStores,
    scope: &Scope,
    execution_id: &str,
) {
    match stores
        .resume_tokens
        .revoke_on_terminal(scope, execution_id)
        .await
    {
        Ok(tokens_revoked) => {
            tracing::debug!(
                target = "engine::wait",
                execution_id,
                tokens_revoked,
                "revoke_on_terminal: purged un-consumed resume tokens on terminal transition"
            );
        },
        Err(error) => {
            tracing::warn!(
                target = "engine::wait",
                execution_id,
                %error,
                "revoke_on_terminal: best-effort resume-token cleanup failed; the execution is \
                 already terminal — the FK ON DELETE CASCADE backstop will reclaim the dead rows"
            );
        },
    }
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
        _ => None,
    }
}

#[cfg(test)]
mod tests;
