//! Scoped resource resolution + per-branch lifecycle (M6.1 Phase 6 wiring + M6.2 Phase 7 storage).
//!
//! ## Architecture (Phase 7 — M6.2)
//!
//! Phase 6 landed the **resolution precedence** layer (`scoped → global`)
//! used by `nebula_action::ActionContextExt::acquire_resource_by_id`
//! and `nebula_resource::HasResourcesExt::resource`.
//!
//! Phase 7 supplies the concrete `DashMap`-backed [`DashScopedResourceMap`]
//! that stores per-branch entries plus parent-pointer ancestry, the
//! [`BranchId`] newtype that scopes those entries, the
//! [`ScopedResourceGuard`] RAII wrapper that drives cleanup on branch
//! exit, and the `ScopedResourceCleanupTimeout` event variant on
//! `crate::ExecutionEvent` emitted when a `Resource::destroy` overruns
//! the configured budget.
//!
//! ## Closest-ancestor walk
//!
//! Every branch is registered with an optional parent.
//! [`DashScopedResourceMap::lookup_in_ancestors`] walks `branch_id → parent → grandparent → …`
//! until either a hit is found or the root is reached. The walk is bounded by
//! [`MAX_ANCESTOR_DEPTH`] to surface cycles as `CoreError::DependencyCycle`
//! rather than spinning forever.
//!
//! ## Cleanup ordering (Task 7.4)
//!
//! 1. **Inner-to-outer.** When nested branches end, the deepest branch cleans up first, then walks
//!    up. The engine drives this by calling [`DashScopedResourceMap::pop`] on the leaf branch, then
//!    on its ancestors.
//! 2. **LIFO within a branch.** If multiple resources are registered at the same branch level,
//!    [`DashScopedResourceMap::pop`] returns them in reverse registration order so cleanup runs in
//!    the inverse order of `push`.
//! 3. **Cancel-safe.** The [`ScopedResourceGuard`] wraps the popped entry in a
//!    `scopeguard::ScopeGuard` so cleanup fires even if the engine task panics or the cancellation
//!    token trips mid-execution.
//! 4. **Per-resource timeout.** Each `cleanup` future is wrapped in `tokio::time::timeout` at
//!    [`DEFAULT_CLEANUP_TIMEOUT`] (30s). On overrun the future is dropped and a
//!    `ScopedResourceCleanupTimeout` event is emitted via the engine's event bus.
//!
//! ## Invariants
//!
//! - A branch can hold zero or more entries; `push` is total (never errors).
//! - `pop` returns `None` for a never-registered branch and `Some(Vec::new())` for a branch whose
//!   only effect was registering its parent pointer.
//! - `lookup_in_ancestors` only sees entries currently held by the map; if `pop` removed them they
//!   are no longer visible.
//! - `register_branch(child, Some(parent))` is idempotent — subsequent calls overwrite the parent
//!   pointer (engines that re-bind a branch must call `pop` first, otherwise the previously
//!   registered entries become reparented under the new ancestor and would be returned by
//!   subsequent `lookup_in_ancestors` from siblings of the new parent).
//!
//! ## Why a trait + a concrete impl
//!
//! Phase 6 ships [`EmptyScopedResourceMap`] (always misses); Phase 7 ships
//! [`DashScopedResourceMap`]. Both implement [`ScopedResourceMap`] so the
//! `LayeredResourceAccessor` wiring in `engine.rs` does not need to know
//! which one is in use — Phase 7 simply swaps the constructor without
//! touching action call sites.

use std::{
    any::Any,
    collections::HashSet,
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use nebula_core::{CoreError, NodeKey, ResourceKey, accessor::ResourceAccessor};

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Default cleanup timeout per resource (Task 7.4).
///
/// The engine wraps every `Resource::destroy` future in
/// `tokio::time::timeout` at this duration. On overrun the future is
/// dropped and a `ScopedResourceCleanupTimeout` event variant on
/// `crate::ExecutionEvent` fires; the runtime is *not* awaited further.
pub const DEFAULT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Hard cap on ancestor walk depth.
///
/// 1024 is well above any plausible workflow nesting; it exists to surface
/// accidental cycles in the parent-pointer tree as
/// `CoreError::DependencyCycle` instead of an unbounded loop.
pub const MAX_ANCESTOR_DEPTH: usize = 1024;

/// Type-erased lookup payload returned by [`ScopedResourceMap::lookup_in_ancestors`].
pub type ScopedLookup = Box<dyn Any + Send + Sync>;

/// Identifier for a single scope frame in the engine.
///
/// A branch is the unit of scope ownership: when a
/// [`ResourceAction`](nebula_action::resource::ResourceAction) node enters, the engine calls
/// [`DashScopedResourceMap::push`] with the node's [`BranchId`]; when the branch exits, the engine
/// calls [`DashScopedResourceMap::pop`] with the same id and drives cleanup.
///
/// # Mapping to engine concepts
///
/// Phase 7 carries the branch id as a thin newtype around [`NodeKey`] —
/// every node in the DAG is its own potential branch frame. The frontier
/// runner does not yet drive `push`/`pop` per-node (the open architecture
/// decision is tracked in the maintainers' private design vault); the API surface is in place so
/// consumers can wire branch lifecycle when scope semantics are formalized.
///
/// # Why a newtype rather than a raw `NodeKey`
///
/// The branch id is purely about scope ownership; it is not the same thing
/// as "this is the executing node." Future work may introduce sub-frames
/// (loop iterations, retry attempts) that share a NodeKey but live in
/// distinct scopes — the newtype reserves headroom for that without an
/// invasive rename.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BranchId(NodeKey);

impl BranchId {
    /// Build a branch id from the owning node key.
    #[must_use]
    pub const fn from_node_key(node_key: NodeKey) -> Self {
        Self(node_key)
    }

    /// Inner [`NodeKey`].
    #[must_use]
    pub const fn node_key(&self) -> &NodeKey {
        &self.0
    }
}

impl From<NodeKey> for BranchId {
    fn from(value: NodeKey) -> Self {
        Self(value)
    }
}

impl fmt::Display for BranchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// One entry in a branch's scope frame.
///
/// Stored in registration order; [`DashScopedResourceMap::pop`] returns
/// the entries in reverse so cleanup runs LIFO.
struct ScopedEntry {
    /// The resource's lookup key (e.g., `postgres`).
    key: ResourceKey,
    /// Type-erased payload registered by `push`.
    payload: Arc<dyn Any + Send + Sync>,
}

impl fmt::Debug for ScopedEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedEntry")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// Closest-ancestor lookup over a workflow's scope chain.
///
/// Implementations walk the ancestor chain (current branch → parent → … →
/// root) and return the first registered resource matching `key`. Phase 7
/// ships [`DashScopedResourceMap`] as the per-branch storage; Phase 6
/// shipped [`EmptyScopedResourceMap`] as the no-op default that drops
/// every lookup through to the global accessor.
///
/// # Contract
///
/// - `lookup_in_ancestors` returns `Ok(Some(_))` when the key is registered at any ancestor.
/// - `Ok(None)` means *no scope owns the key* — the caller should fall through to the global
///   accessor.
/// - `Err(_)` is reserved for genuine lookup faults (cycles, poisoned storage).
///
/// `has_in_ancestors` mirrors the lookup but returns a plain `bool`.
pub trait ScopedResourceMap: Send + Sync + fmt::Debug {
    /// Walk ancestor scopes for `key`; closest-ancestor wins.
    fn lookup_in_ancestors<'a>(
        &'a self,
        key: &'a ResourceKey,
    ) -> BoxFut<'a, Result<Option<ScopedLookup>, CoreError>>;

    /// Existence check across ancestor scopes.
    fn has_in_ancestors(&self, key: &ResourceKey) -> bool;
}

/// No-op [`ScopedResourceMap`] — every lookup misses.
///
/// Phase 6 default. The layered accessor wired with this stub behaves
/// identically to a global-only accessor; Phase 7's
/// [`DashScopedResourceMap`] is the production storage.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyScopedResourceMap;

impl ScopedResourceMap for EmptyScopedResourceMap {
    fn lookup_in_ancestors<'a>(
        &'a self,
        _key: &'a ResourceKey,
    ) -> BoxFut<'a, Result<Option<ScopedLookup>, CoreError>> {
        Box::pin(async { Ok(None) })
    }

    fn has_in_ancestors(&self, _key: &ResourceKey) -> bool {
        false
    }
}

/// Concrete [`ScopedResourceMap`] backed by a `DashMap` of branch entries
/// and a parent-pointer tree (Phase 7 / Task 7.1).
///
/// # Threading
///
/// `DashMap` provides lock-free per-shard concurrency; sibling branches
/// pushing concurrently never block each other. The active-branch map
/// holds a single per-branch `Vec<ScopedEntry>` so within-branch ordering
/// is preserved.
///
/// # API
///
/// - [`Self::register_branch`] — declare a branch with an optional parent. Must be called once per
///   branch before [`Self::push`] / [`Self::pop`] / [`Self::lookup_in_ancestors_from`].
/// - [`Self::push`] — append a `(key, payload)` to a branch's entries.
/// - [`Self::pop`] — drain a branch's entries (LIFO order) and detach the parent pointer.
/// - [`Self::lookup_in_ancestors_from`] — walk from a starting branch up the ancestor chain.
///
/// The trait-level [`Self::lookup_in_ancestors`] / [`Self::has_in_ancestors`]
/// reflect lookups from the **current branch** set via
/// [`Self::set_current_branch`] — used by the
/// [`LayeredResourceAccessor`] when actions call `ctx.resource::<R>()`
/// without an explicit branch parameter. Engine wiring is responsible for
/// stamping `current_branch` per-task.
///
/// # Invariants
///
/// - A `pop`-ed branch is removed from `entries` and `parents`; subsequent `lookup` calls treat it
///   as nonexistent.
/// - The parent-pointer tree is acyclic by construction (there is no API to install a cycle); a
///   misuse defense exists via [`MAX_ANCESTOR_DEPTH`].
pub struct DashScopedResourceMap {
    /// `branch_id → entries` in registration order. Empty `Vec` is a
    /// branch declared but holding no payloads.
    entries: DashMap<BranchId, Vec<ScopedEntry>>,
    /// `branch_id → parent_branch_id` (None for root branches).
    parents: DashMap<BranchId, Option<BranchId>>,
    /// Current branch used by the trait-level
    /// [`ScopedResourceMap::lookup_in_ancestors`] entry point. Engine code
    /// can stamp this per-task; if unset, the trait-level lookup behaves
    /// as a no-op (returns `Ok(None)`).
    current_branch: parking_lot::RwLock<Option<BranchId>>,
}

impl Default for DashScopedResourceMap {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for DashScopedResourceMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DashScopedResourceMap")
            .field("branches", &self.entries.len())
            .field("current", &self.current_branch.read().clone())
            .finish_non_exhaustive()
    }
}

impl DashScopedResourceMap {
    /// Create an empty scoped map.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            parents: DashMap::new(),
            current_branch: parking_lot::RwLock::new(None),
        }
    }

    /// Declare a branch with an optional parent.
    ///
    /// Idempotent: re-registering the same branch overwrites the parent
    /// pointer in place. Engines that re-purpose a branch id should call
    /// [`Self::pop`] first to drain the previous payloads.
    #[tracing::instrument(level = "debug", skip(self), fields(branch = %branch, parent = ?parent))]
    pub fn register_branch(&self, branch: BranchId, parent: Option<BranchId>) {
        self.entries.entry(branch.clone()).or_default();
        self.parents.insert(branch, parent);
    }

    /// Append a payload to the branch's scope frame.
    ///
    /// Returns `false` if the branch has not been registered via
    /// [`Self::register_branch`]; in that case the payload is dropped and
    /// no scope frame is created. Returning `false` is an engine-level
    /// invariant breach (calling code must register before push), and is
    /// surfaced as a `tracing::warn!` event for observability.
    #[tracing::instrument(level = "debug", skip(self, payload), fields(branch = %branch, key = %key))]
    pub fn push(
        &self,
        branch: BranchId,
        key: ResourceKey,
        payload: Arc<dyn Any + Send + Sync>,
    ) -> bool {
        let Some(mut entries) = self.entries.get_mut(&branch) else {
            tracing::warn!(
                branch = %branch,
                key = %key,
                "DashScopedResourceMap::push called for unregistered branch (invariant breach)",
            );
            return false;
        };
        entries.push(ScopedEntry { key, payload });
        true
    }

    /// Drain a branch's entries (LIFO) and detach its parent pointer.
    ///
    /// Returns `None` if the branch was never registered. Returns
    /// `Some(Vec::new())` for a registered-but-empty branch. Entries are
    /// returned in **reverse registration order** so the engine can call
    /// `Resource::destroy` LIFO without an extra reverse step.
    #[tracing::instrument(level = "debug", skip(self), fields(branch = %branch))]
    pub fn pop(&self, branch: &BranchId) -> Option<Vec<PoppedEntry>> {
        let entries = self.entries.remove(branch).map(|(_, v)| v)?;
        // Best-effort cleanup of the parent pointer; if absent it's already
        // been pruned and we don't fail the pop on that.
        let _ = self.parents.remove(branch);
        let popped = entries
            .into_iter()
            .rev()
            .map(|e| PoppedEntry {
                branch: branch.clone(),
                key: e.key,
                payload: e.payload,
            })
            .collect();
        Some(popped)
    }

    /// Set the current branch used by the trait-level
    /// [`ScopedResourceMap::lookup_in_ancestors`] entry point.
    ///
    /// Engine wiring should stamp the executing node's branch here so
    /// action call sites resolve scoped resources transparently.
    pub fn set_current_branch(&self, branch: Option<BranchId>) {
        *self.current_branch.write() = branch;
    }

    /// Walk from `start` up the ancestor chain looking for `key`.
    ///
    /// Returns the first match's payload (closest-ancestor wins), or
    /// `Ok(None)` if no ancestor owns the key.
    ///
    /// # Errors
    ///
    /// Returns `CoreError::DependencyCycle` if the parent chain exceeds
    /// [`MAX_ANCESTOR_DEPTH`] — surfacing accidental cycles as a typed
    /// error rather than spinning indefinitely.
    pub fn lookup_in_ancestors_from(
        &self,
        start: &BranchId,
        key: &ResourceKey,
    ) -> Result<Option<ScopedLookup>, CoreError> {
        let mut visited: HashSet<BranchId> = HashSet::new();
        let mut cursor = Some(start.clone());
        let mut depth = 0usize;

        while let Some(current) = cursor {
            if depth >= MAX_ANCESTOR_DEPTH || !visited.insert(current.clone()) {
                tracing::error!(
                    branch = %current,
                    depth,
                    "scoped-resource ancestor walk exceeded {MAX_ANCESTOR_DEPTH} or revisited a branch (cycle)"
                );
                return Err(CoreError::DependencyCycle {
                    path: vec!["scoped-resource ancestor walk"],
                });
            }
            depth += 1;

            if let Some(entries) = self.entries.get(&current) {
                // Closest-ancestor wins, LIFO within branch (last push at this
                // level shadows earlier pushes of the same key).
                if let Some(entry) = entries.iter().rev().find(|e| e.key == *key) {
                    return Ok(Some(arc_payload_clone(&entry.payload)));
                }
            }

            cursor = self.parents.get(&current).and_then(|p| p.value().clone());
        }

        Ok(None)
    }

    /// Existence check from `start` up the ancestor chain.
    pub fn has_in_ancestors_from(&self, start: &BranchId, key: &ResourceKey) -> bool {
        let mut visited: HashSet<BranchId> = HashSet::new();
        let mut cursor = Some(start.clone());
        let mut depth = 0usize;
        while let Some(current) = cursor {
            if depth >= MAX_ANCESTOR_DEPTH || !visited.insert(current.clone()) {
                return false;
            }
            depth += 1;
            if let Some(entries) = self.entries.get(&current)
                && entries.iter().any(|e| e.key == *key)
            {
                return true;
            }
            cursor = self.parents.get(&current).and_then(|p| p.value().clone());
        }
        false
    }
}

/// One drained entry produced by [`DashScopedResourceMap::pop`].
///
/// The engine takes ownership and drives cleanup (typically via
/// [`ScopedResourceGuard`]) for each popped entry in order.
#[derive(Clone)]
pub struct PoppedEntry {
    /// The branch this entry belonged to.
    pub branch: BranchId,
    /// The resource key (lookup key for ancestor walks).
    pub key: ResourceKey,
    /// Type-erased payload registered by `push`.
    pub payload: Arc<dyn Any + Send + Sync>,
}

impl fmt::Debug for PoppedEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoppedEntry")
            .field("branch", &self.branch)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// Helper: clone an `Arc<dyn Any + Send + Sync>` into a `Box<dyn Any + …>`.
///
/// Lookup callers receive a `Box<dyn Any>` (ScopedLookup), but the storage
/// holds `Arc<dyn Any>` so multiple ancestors can hand out the same
/// resource without a deep copy. This wraps the `Arc` in a fresh box; the
/// inner Arc is preserved through the box and downstream `downcast` works
/// against `Arc<T>` payloads.
fn arc_payload_clone(payload: &Arc<dyn Any + Send + Sync>) -> ScopedLookup {
    Box::new(Arc::clone(payload))
}

impl ScopedResourceMap for DashScopedResourceMap {
    fn lookup_in_ancestors<'a>(
        &'a self,
        key: &'a ResourceKey,
    ) -> BoxFut<'a, Result<Option<ScopedLookup>, CoreError>> {
        let current = self.current_branch.read().clone();
        Box::pin(async move {
            match current {
                Some(start) => self.lookup_in_ancestors_from(&start, key),
                None => Ok(None),
            }
        })
    }

    fn has_in_ancestors(&self, key: &ResourceKey) -> bool {
        let Some(start) = self.current_branch.read().clone() else {
            return false;
        };
        self.has_in_ancestors_from(&start, key)
    }
}

// ── Cleanup driver (Task 7.4) ──────────────────────────────────────────────

/// Outcome of running a single resource cleanup future.
#[derive(Debug)]
pub enum CleanupOutcome {
    /// Cleanup completed within the budget.
    Completed {
        /// How long the cleanup took.
        elapsed: Duration,
    },
    /// Cleanup exceeded the budget; the runtime was dropped.
    TimedOut {
        /// The configured budget.
        budget: Duration,
        /// Elapsed time at the point we dropped the future.
        elapsed: Duration,
    },
    /// Cleanup body returned an error.
    Failed {
        /// How long before the failure surfaced.
        elapsed: Duration,
        /// Stringified error (the engine bus does not require a typed
        /// payload for observability).
        error: String,
    },
}

/// Run `cleanup_fut` with [`DEFAULT_CLEANUP_TIMEOUT`].
///
/// Equivalent to [`run_cleanup_with_timeout`] passing
/// [`DEFAULT_CLEANUP_TIMEOUT`].
pub async fn run_cleanup<F, E>(cleanup_fut: F) -> CleanupOutcome
where
    F: Future<Output = Result<(), E>>,
    E: fmt::Display,
{
    run_cleanup_with_timeout(cleanup_fut, DEFAULT_CLEANUP_TIMEOUT).await
}

/// Wrap a cleanup future in a timeout and classify the outcome.
///
/// On timeout the future is dropped (Tokio's `timeout` cancels the inner
/// future's poll). The runtime should treat a `TimedOut` outcome as a hard
/// release: any in-flight `Resource::destroy` is abandoned and the engine
/// emits the `ScopedResourceCleanupTimeout` variant of `crate::ExecutionEvent`
/// for observability.
#[tracing::instrument(level = "debug", skip(cleanup_fut), fields(budget_ms = budget.as_millis() as u64))]
pub async fn run_cleanup_with_timeout<F, E>(cleanup_fut: F, budget: Duration) -> CleanupOutcome
where
    F: Future<Output = Result<(), E>>,
    E: fmt::Display,
{
    let started = Instant::now();
    match tokio::time::timeout(budget, cleanup_fut).await {
        Ok(Ok(())) => CleanupOutcome::Completed {
            elapsed: started.elapsed(),
        },
        Ok(Err(err)) => CleanupOutcome::Failed {
            elapsed: started.elapsed(),
            error: err.to_string(),
        },
        Err(_) => CleanupOutcome::TimedOut {
            budget,
            elapsed: started.elapsed(),
        },
    }
}

// ── RAII guard (Task 7.2 cancel-safety) ─────────────────────────────────────

/// RAII wrapper that ensures [`DashScopedResourceMap::pop`] is invoked even
/// if the holding task panics or the cancellation token trips.
///
/// Construction takes the branch id; on `Drop`, [`Self::dismiss`] not
/// having been called causes `pop` to fire. The drained entries are
/// surfaced to a caller-provided sink so engines can run the actual
/// `Resource::destroy` cleanup outside `Drop` (which is sync and
/// non-async).
///
/// # Async cleanup pattern
///
/// `Drop` runs on the executing task; it cannot await
/// `Resource::destroy`. The engine pattern is:
///
/// 1. Build a `ScopedResourceGuard` for the entering branch.
/// 2. Run the branch body.
/// 3. On normal exit, call `guard.into_drained()` to recover the entries explicitly and run async
///    cleanup with full timeout/event semantics.
/// 4. On panic / cancel, the `Drop` impl calls `pop` and the entries are routed to the configured
///    sink — typically a `mpsc::Sender` to a background cleanup task that drains it asynchronously.
///
/// The point of the guard is that the **map state** is corrected even on
/// panic; async cleanup of Resource runtimes still requires the engine to
/// own the cleanup driver (since we cannot block in Drop).
pub struct ScopedResourceGuard<'a> {
    map: &'a DashScopedResourceMap,
    branch: BranchId,
    drained: Option<Vec<PoppedEntry>>,
    on_panic_sink: Option<Box<dyn FnOnce(Vec<PoppedEntry>) + Send + 'a>>,
    dismissed: bool,
}

impl<'a> ScopedResourceGuard<'a> {
    /// Build a new guard for `branch` against `map`.
    ///
    /// `on_panic_sink` is invoked from `Drop` if the guard was not
    /// `dismiss`-ed; it receives the popped entries so a background task
    /// can run async cleanup. If `None`, drained entries are dropped on
    /// panic and a `tracing::warn!` event fires (resource runtimes leak
    /// their `Drop` impl is the only cleanup).
    #[must_use]
    pub fn new(
        map: &'a DashScopedResourceMap,
        branch: BranchId,
        on_panic_sink: Option<Box<dyn FnOnce(Vec<PoppedEntry>) + Send + 'a>>,
    ) -> Self {
        Self {
            map,
            branch,
            drained: None,
            on_panic_sink,
            dismissed: false,
        }
    }

    /// The branch this guard owns.
    #[must_use]
    pub fn branch(&self) -> &BranchId {
        &self.branch
    }

    /// Drain the branch's entries explicitly (normal exit path).
    ///
    /// Marks the guard `dismissed` so `Drop` does not double-pop. The
    /// engine then drives async cleanup against the returned entries with
    /// full event/timeout semantics.
    #[must_use]
    pub fn into_drained(mut self) -> Vec<PoppedEntry> {
        self.dismissed = true;
        self.map.pop(&self.branch).unwrap_or_default()
    }

    /// Mark the guard as dismissed without draining.
    ///
    /// Used when the engine externally drove `pop` and wants the guard to
    /// become inert. Subsequent `Drop` is a no-op.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }
}

impl Drop for ScopedResourceGuard<'_> {
    fn drop(&mut self) {
        if self.dismissed {
            return;
        }
        // We're in a Drop impl on a possibly-panicking task. Best-effort:
        // pop and route to the panic sink if available.
        let popped = self.map.pop(&self.branch).unwrap_or_default();
        if popped.is_empty() {
            return;
        }
        match self.on_panic_sink.take() {
            Some(sink) => {
                tracing::warn!(
                    branch = %self.branch,
                    entries = popped.len(),
                    "ScopedResourceGuard dropped without dismiss; routing entries to panic sink",
                );
                sink(popped);
            },
            None => {
                tracing::error!(
                    branch = %self.branch,
                    entries = popped.len(),
                    "ScopedResourceGuard dropped without dismiss and no panic sink; resources leaked their async cleanup",
                );
            },
        }
        // Stash so Debug printing doesn't double-warn after Drop ran.
        self.drained = Some(Vec::new());
    }
}

impl fmt::Debug for ScopedResourceGuard<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedResourceGuard")
            .field("branch", &self.branch)
            .field("dismissed", &self.dismissed)
            .finish_non_exhaustive()
    }
}

// ── Layered accessor (Phase 6 wiring) ───────────────────────────────────────

/// `ResourceAccessor` impl that consults a [`ScopedResourceMap`] before
/// falling through to a global accessor.
///
/// # Lookup order
///
/// 1. `scoped.lookup_in_ancestors(key)` — closest-ancestor walk.
/// 2. On miss (`Ok(None)`), `global.acquire_any(key)`.
///
/// This is the accessor injected into
/// `nebula_action::ActionRuntimeContext` from Phase 6 onwards. Action
/// authors do not see the layering — they call `ctx.resource::<R>()` /
/// `ctx.acquire_resource_by_id::<R>(id)` and the precedence is applied
/// transparently.
///
/// # Examples
///
/// ```rust
/// use std::sync::Arc;
///
/// use nebula_core::{ResourceKey, accessor::ResourceAccessor, scope::Scope};
/// use nebula_engine::{
///     EngineResourceAccessor,
///     scoped_resources::{DashScopedResourceMap, LayeredResourceAccessor},
/// };
/// use nebula_resource::Manager;
/// use tokio_util::sync::CancellationToken;
///
/// # let rt = tokio::runtime::Runtime::new().unwrap();
/// # let _guard = rt.enter();
/// // `Manager` registers a background release queue, so build it inside a
/// // Tokio runtime context.
/// let manager = Arc::new(Manager::new());
/// let global = Arc::new(EngineResourceAccessor::new(
///     manager,
///     Scope::default(),
///     CancellationToken::new(),
/// ));
/// let scoped = Arc::new(DashScopedResourceMap::new());
/// // `layered` is the `ResourceAccessor` injected into the action context.
/// let layered = LayeredResourceAccessor::new(scoped, global);
///
/// // Neither layer has anything registered, so no key resolves yet.
/// assert!(!layered.has(&ResourceKey::new("postgres").unwrap()));
/// ```
pub struct LayeredResourceAccessor {
    scoped: Arc<dyn ScopedResourceMap>,
    global: Arc<dyn ResourceAccessor>,
}

impl LayeredResourceAccessor {
    /// Build a layered accessor from the scoped map and global fallthrough.
    #[must_use]
    pub fn new(scoped: Arc<dyn ScopedResourceMap>, global: Arc<dyn ResourceAccessor>) -> Self {
        Self { scoped, global }
    }

    /// Convenience constructor for the no-scope-state path.
    ///
    /// Equivalent to `LayeredResourceAccessor::new(Arc::new(EmptyScopedResourceMap), global)`.
    #[must_use]
    pub fn global_only(global: Arc<dyn ResourceAccessor>) -> Self {
        Self::new(Arc::new(EmptyScopedResourceMap), global)
    }
}

impl fmt::Debug for LayeredResourceAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayeredResourceAccessor")
            .field("scoped", &self.scoped)
            .field("global", &"<dyn ResourceAccessor>")
            .finish()
    }
}

impl ResourceAccessor for LayeredResourceAccessor {
    fn has(&self, key: &ResourceKey) -> bool {
        self.scoped.has_in_ancestors(key) || self.global.has(key)
    }

    fn acquire_any(&self, key: &ResourceKey) -> BoxFut<'_, Result<ScopedLookup, CoreError>> {
        let key_owned = key.clone();
        Box::pin(async move {
            match self.scoped.lookup_in_ancestors(&key_owned).await? {
                Some(payload) => Ok(payload),
                None => self.global.acquire_any(&key_owned).await,
            }
        })
    }

    fn try_acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Option<ScopedLookup>, CoreError>> {
        let key_owned = key.clone();
        Box::pin(async move {
            match self.scoped.lookup_in_ancestors(&key_owned).await? {
                Some(payload) => Ok(Some(payload)),
                None => self.global.try_acquire_any(&key_owned).await,
            }
        })
    }
}

#[cfg(test)]
#[path = "scoped_resources_tests.rs"]
mod tests;
