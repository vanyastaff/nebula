# Phase 7.5: Registry Unification + Stateful Execution

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `ActionRegistry` lives ONLY in `nebula-runtime` (single source of truth, SOLID-correct). `nebula-action` is a pure protocol crate — defines `Action`/handlers/adapters but does not own registration or execution. `ActionRuntime::run_handler` dispatches on `ActionHandler` enum, supporting `Stateless` + `Stateful` execution. Trigger/Resource/Agent return typed errors (their lifecycles run elsewhere). `InternalHandler` deleted entirely.

**Architecture:**
- **`nebula-action`** (protocol): keeps `Action`, `ActionMetadata`, all `*Action` traits, all `*Handler` traits, `ActionHandler` enum, all adapters (`StatelessActionAdapter`, `StatefulActionAdapter`, `WebhookTriggerAdapter`, `PollTriggerAdapter`, etc.). **No ActionRegistry, no dashmap dependency.**
- **`nebula-runtime`** (execution): owns `ActionRegistry` (DashMap-backed, `&self` API), all `register_*` convenience methods (stateless/stateful/trigger/webhook/poll/resource), `ActionRuntime`, `run_handler` dispatch logic.
- `ActionRuntime::run_handler` becomes a `match` on `ActionHandler` enum:
  - `Stateless(h)` → call `h.execute(input, ctx)` directly (existing path)
  - `Stateful(h)` → loop: `init_state` → `execute` until `Break`, with state checkpointing in memory
  - `Trigger(_)` → `Err(RuntimeError::TriggerNotExecutable)` permanently (triggers have their own start/stop lifecycle)
  - `Resource(_)` → `Err(RuntimeError::ResourceNotExecutable)` permanently (resources have their own configure/cleanup scoping)
  - `Agent(_)` → `Err(RuntimeError::AgentNotSupportedYet)` (Phase 9)
- `StatefulHandler::execute` signature changes: `input: Value` → `input: &Value`. Avoids per-iteration cloning of large payloads in the stateful loop (input is invariant across iterations). Adapter clones once internally for typed deserialization. User-level `StatefulAction::execute` is unchanged.
- Migrate engine tests from `impl InternalHandler` to `impl StatelessAction` (mechanical rewrite, ~15 sites)
- Sandbox signature: `SandboxRunner::execute` updated to accept the typed handler. Sandboxed Stateful execution is deferred — only Stateless supports non-`None` isolation in Phase 7.5.
- Delete `InternalHandler` trait, `StatelessActionAdapter::InternalHandler` impl, all `#![allow(deprecated)]` workarounds.
- CLI immediately gains all `register_*` methods. Stateful-based DX traits (Paginated/Batch/Transactional) execute end-to-end via the new Stateful loop.

**Tech Stack:** Rust 1.94, `dashmap`, `tokio`, `serde_json`, `parking_lot`, existing nebula-action types

**Prerequisites:** Phase 6 + Phase 7 done (currently on main, merged via #241 #242).

**Why no 7.5a/7.5b split:** The Stateful loop is the critical piece because PaginatedAction/BatchAction/TransactionalAction all desugar to Stateful — landing it unblocks 3 of the 5 DX traits. Trigger/Resource genuinely don't fit `ActionRuntime::execute` semantically (different lifecycles), so returning typed errors is permanent design, not a TODO. Splitting would ship lying registrars where `register_webhook` succeeds and execution panics — strictly worse than today's honest limitation.

**Why ActionRegistry lives in `nebula-runtime`, not `nebula-action`:** SOLID separation of concerns. `nebula-action` defines *what an action is* (protocol). `nebula-runtime` defines *how actions are stored, looked up, and executed* (execution). Registration is an execution concern: the registry holds `Arc`-wrapped handlers for future dispatch, which only makes sense if you're planning to execute them. The protocol crate stays free of `dashmap` and execution-layer dependencies.

---

## Task 1: Add `RuntimeError` variants for non-executable handlers

**Files:**
- Modify: `crates/runtime/src/error.rs`

This is the cheapest task — adds new error variants used by Task 4. Doing it first keeps later steps simple.

**Step 1: Read current error variants**

Look at `crates/runtime/src/error.rs` to understand the existing `RuntimeError` enum shape, derive macros, and naming conventions.

**Step 2: Add new variants**

Add to the `RuntimeError` enum:

```rust
    /// The action key resolves to a trigger, which has its own start/stop
    /// lifecycle and is not executable via `ActionRuntime::execute_action`.
    /// Triggers run via the trigger runtime (separate from action execution).
    #[error("trigger '{key}' is not executable via ActionRuntime — use the trigger runtime")]
    TriggerNotExecutable {
        /// The action key that was looked up.
        key: String,
    },

    /// The action key resolves to a resource, which has its own
    /// configure/cleanup lifecycle scoped to a downstream subtree.
    /// Resources are not executable via `ActionRuntime::execute_action`.
    #[error("resource '{key}' is not executable via ActionRuntime — use the resource graph")]
    ResourceNotExecutable {
        /// The action key that was looked up.
        key: String,
    },

    /// The action key resolves to an agent action (Phase 9), which is not
    /// yet supported by the runtime.
    #[error("agent action '{key}' is not yet supported (Phase 9 work)")]
    AgentNotSupportedYet {
        /// The action key that was looked up.
        key: String,
    },
```

**Step 3: Run check**

Run: `cargo check -p nebula-runtime`
Expected: compiles.

**Step 4: Commit**

```
feat(runtime): add typed errors for non-executable handler variants
```

---

## Task 2: Update `StatefulHandler::execute` to take `input: &Value`

**Files:**
- Modify: `crates/action/src/handler.rs`
- Modify: `crates/action/tests/execution_integration.rs`

The current `StatefulHandler::execute` consumes `input: Value`. In a multi-iteration loop the runtime would clone `input` per iteration, which is expensive for large payloads. The input is invariant across iterations of a single action execution — only `state` mutates.

**Step 1: Update the trait method signature**

In `crates/action/src/handler.rs`, find the `StatefulHandler` trait (around line 612). Change:

```rust
async fn execute(
    &self,
    input: &Value,        // ← was: input: Value
    state: &mut Value,
    ctx: &ActionContext,
) -> Result<ActionResult<Value>, ActionError>;
```

**Step 2: Update `StatefulActionAdapter::execute` impl**

Find the `impl StatefulHandler for StatefulActionAdapter<A>` block (around line 193). Update the method:

```rust
async fn execute(
    &self,
    input: &Value,
    state: &mut Value,
    ctx: &ActionContext,
) -> Result<ActionResult<Value>, ActionError> {
    // Adapter clones input ONCE per iteration to deserialize into typed A::Input.
    // The runtime loop borrows from a single Value instead of cloning a giant
    // serde_json::Value tree per iteration.
    let typed_input: A::Input = serde_json::from_value(input.clone())
        .map_err(|e| ActionError::validation(format!("input deserialization failed: {e}")))?;

    let mut typed_state: A::State = serde_json::from_value(state.clone()).or_else(|e| {
        self.action.migrate_state(state.clone()).ok_or_else(|| {
            ActionError::validation(format!("state deserialization failed: {e}"))
        })
    })?;

    let result = self
        .action
        .execute(typed_input, &mut typed_state, ctx)
        .await?;

    *state = serde_json::to_value(&typed_state)
        .map_err(|e| ActionError::fatal(format!("state serialization failed: {e}")))?;

    result.try_map_output(|output| {
        serde_json::to_value(output)
            .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
    })
}
```

**Step 3: Update test handler in handler.rs**

Find `TestStatefulHandler` (around line 1113):

```rust
async fn execute(
    &self,
    input: &Value,        // ← was: input: Value
    state: &mut Value,
    _ctx: &ActionContext,
) -> Result<ActionResult<Value>, ActionError> {
    let count = state.as_u64().unwrap_or(0);
    *state = serde_json::json!(count + 1);
    Ok(ActionResult::success(input.clone()))   // ← was: input
}
```

**Step 4: Update test call sites in handler.rs**

Find 4 call sites in handler.rs around lines 1457-1490:

```rust
.execute(&serde_json::json!({}), &mut state, &ctx)   // ← add &
```

(Was: `.execute(serde_json::json!({}), &mut state, &ctx)`)

**Step 5: Update test call sites in execution_integration.rs**

Find 2 call sites in `crates/action/tests/execution_integration.rs` around lines 269 and 291:

```rust
.execute(&serde_json::json!({}), &mut state, &ctx)
.execute(&serde_json::Value::Null, &mut state, &ctx)
```

**Step 6: Run check + tests**

Run: `cargo check -p nebula-action && cargo nextest run -p nebula-action`
Expected: compiles, all 276 tests pass.

User-level `StatefulAction::execute` is unchanged — it still receives `Self::Input` by value (owned, after type-erased deserialization). This change is invisible to plugin authors. `StatefulTestHarness::step()` is also unchanged because it works with the typed `StatefulAction`, not the JSON-erased `StatefulHandler`.

**Step 7: Commit**

```
refactor(action): StatefulHandler::execute takes &Value for input
```

---

## Task 3: Move `ActionRegistry` from `nebula-action` to `nebula-runtime`

**Files:**
- Delete: `crates/action/src/registry.rs`
- Modify: `crates/action/src/lib.rs` (remove module + re-exports)
- Modify: `crates/action/src/prelude.rs` (remove re-export)
- Modify: `crates/action/Cargo.toml` (no dashmap added — registry leaves)
- Create/Modify: `crates/runtime/src/registry.rs` (rewrite with new design)
- Modify: `crates/runtime/Cargo.toml` (verify dashmap dependency exists)

**Step 1: Read existing `ActionRegistry` in `nebula-action`**

Read `crates/action/src/registry.rs` to understand what we're moving:
- All 6 `register_*` methods (stateless/stateful/trigger/webhook/poll/resource)
- `register`, `get`, `get_versioned`, `keys`, `len`, `is_empty`
- Test module with `NoopAction` and `make_entry`

**Step 2: Read existing `ActionRegistry` in `nebula-runtime`**

Read `crates/runtime/src/registry.rs` to understand the legacy implementation that will be replaced:
- `DashMap<String, Arc<dyn InternalHandler>>` storage
- Only has `register_stateless`
- Lock-free `&self` API

**Step 3: Replace `crates/runtime/src/registry.rs` with the new unified registry**

Overwrite the file entirely:

```rust
//! Registry of available actions, keyed by `ActionKey`.
//!
//! The `ActionRegistry` is the authoritative source for which action types are
//! available in a running Nebula instance. The runtime consults it during
//! action execution to look up handlers and dispatch on the `ActionHandler`
//! enum variant.
//!
//! # Version-aware lookup
//!
//! Multiple versions of the same action can be registered simultaneously.
//! [`ActionRegistry::get`] returns the **latest** version (highest major,
//! then minor), while [`ActionRegistry::get_versioned`] retrieves a specific
//! `"major.minor"` string.
//!
//! # Thread safety
//!
//! Uses `DashMap` for lock-free concurrent access. Both registration and
//! lookup use `&self` — share via `Arc<ActionRegistry>` without external
//! synchronization.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_runtime::ActionRegistry;
//! use std::sync::Arc;
//!
//! let registry = Arc::new(ActionRegistry::new());
//! registry.register_stateless(my_stateless_action);
//! registry.register_stateful(my_stateful_action);
//! registry.register_webhook(my_webhook_action);
//! ```

use std::sync::Arc;

use dashmap::DashMap;

use nebula_action::{
    Action, ActionDependencies, ActionHandler, ActionMetadata, PollAction, ResourceAction,
    StatefulAction, StatelessAction, TriggerAction, WebhookAction,
};
use nebula_action::{
    PollTriggerAdapter, ResourceActionAdapter, StatefulActionAdapter, StatelessActionAdapter,
    TriggerActionAdapter, WebhookTriggerAdapter,
};
use nebula_core::{ActionKey, InterfaceVersion};

/// A single entry in the registry: metadata paired with its handler.
#[derive(Clone)]
struct ActionEntry {
    metadata: ActionMetadata,
    handler: ActionHandler,
}

/// Type-safe registry for action handlers, keyed by `ActionKey`.
///
/// Single source of truth for action registration in nebula. The runtime owns
/// this type because registration is fundamentally an execution concern —
/// the registry holds `Arc`-wrapped handlers for dispatch.
#[derive(Default)]
pub struct ActionRegistry {
    /// Map from action key to list of entries, each at a distinct version.
    actions: DashMap<ActionKey, Vec<ActionEntry>>,
}

impl ActionRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an action handler.
    ///
    /// If an entry with the same key **and** the same `"major.minor"` version
    /// string already exists it is replaced in-place. Otherwise the new entry
    /// is appended. Entries are kept sorted from lowest to highest version so
    /// that [`get`](Self::get) can return the latest in O(1).
    pub fn register(&self, metadata: ActionMetadata, handler: ActionHandler) {
        let version = metadata.version;
        let mut entries = self.actions.entry(metadata.key.clone()).or_default();

        if let Some(pos) = entries.iter().position(|e| e.metadata.version == version) {
            entries[pos] = ActionEntry { metadata, handler };
        } else {
            entries.push(ActionEntry { metadata, handler });
            entries.sort_by(|a, b| {
                a.metadata
                    .version
                    .major
                    .cmp(&b.metadata.version.major)
                    .then(a.metadata.version.minor.cmp(&b.metadata.version.minor))
            });
        }
    }

    /// Look up an action by key, returning the **latest** registered version.
    ///
    /// Returns owned `(metadata, handler)` — `ActionHandler` is `Arc` inside,
    /// so cloning is a cheap pointer copy. Owned values avoid borrowing
    /// `DashMap` guards across `.await` boundaries.
    pub fn get(&self, key: &ActionKey) -> Option<(ActionMetadata, ActionHandler)> {
        let entries = self.actions.get(key)?;
        let last = entries.last()?;
        Some((last.metadata.clone(), last.handler.clone()))
    }

    /// Look up an action by key string (parses into ActionKey first).
    pub fn get_by_str(&self, key: &str) -> Option<(ActionMetadata, ActionHandler)> {
        ActionKey::new(key).ok().and_then(|k| self.get(&k))
    }

    /// Look up an action by key and exact version.
    pub fn get_versioned(
        &self,
        key: &ActionKey,
        version: &InterfaceVersion,
    ) -> Option<(ActionMetadata, ActionHandler)> {
        let entries = self.actions.get(key)?;
        let entry = entries.iter().find(|e| e.metadata.version == *version)?;
        Some((entry.metadata.clone(), entry.handler.clone()))
    }

    /// Register a stateless action — wraps in `StatelessActionAdapter` automatically.
    pub fn register_stateless<A>(&self, action: A)
    where
        A: Action + StatelessAction + Send + Sync + 'static,
        A::Input: serde::de::DeserializeOwned + Send + Sync,
        A::Output: serde::Serialize + Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Stateless(Arc::new(StatelessActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a stateful action — wraps in `StatefulActionAdapter` automatically.
    pub fn register_stateful<A>(&self, action: A)
    where
        A: Action + StatefulAction + Send + Sync + 'static,
        A::Input: serde::de::DeserializeOwned + Send + Sync,
        A::Output: serde::Serialize + Send + Sync,
        A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Stateful(Arc::new(StatefulActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a trigger action — wraps in `TriggerActionAdapter` automatically.
    pub fn register_trigger<A>(&self, action: A)
    where
        A: Action + TriggerAction + Send + Sync + 'static,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Trigger(Arc::new(TriggerActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a webhook action — wraps in `WebhookTriggerAdapter` automatically.
    pub fn register_webhook<A>(&self, action: A)
    where
        A: WebhookAction + Send + Sync + 'static,
        <A as WebhookAction>::State: Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Trigger(Arc::new(WebhookTriggerAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a poll action — wraps in `PollTriggerAdapter` automatically.
    pub fn register_poll<A>(&self, action: A)
    where
        A: PollAction + Send + Sync + 'static,
        <A as PollAction>::Cursor: Send + Sync,
        <A as PollAction>::Event: Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Trigger(Arc::new(PollTriggerAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a resource action — wraps in `ResourceActionAdapter` automatically.
    pub fn register_resource<A>(&self, action: A)
    where
        A: Action + ResourceAction + Send + Sync + 'static,
        A::Config: Send + Sync + 'static,
        A::Instance: Send + Sync + 'static,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Resource(Arc::new(ResourceActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// All registered action keys.
    #[must_use]
    pub fn keys(&self) -> Vec<ActionKey> {
        self.actions.iter().map(|entry| entry.key().clone()).collect()
    }

    /// Total number of registered action keys (not counting multiple versions of the same key).
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Returns `true` if no actions have been registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

impl std::fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<ActionKey> = self.keys();
        f.debug_struct("ActionRegistry")
            .field("action_count", &self.actions.len())
            .field("keys", &keys)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::{
        ActionDependencies, ActionResult, Context, StatelessAction,
    };
    use nebula_action::error::ActionError;
    use nebula_action::metadata::ActionMetadata;

    struct NoopAction { meta: ActionMetadata }

    impl NoopAction {
        fn new(key: &'static str, major: u32, minor: u32) -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::ActionKey::new(key).unwrap(),
                    "Noop",
                    "Does nothing",
                )
                .with_version(major, minor),
            }
        }
    }

    impl ActionDependencies for NoopAction {}
    impl Action for NoopAction {
        fn metadata(&self) -> &ActionMetadata { &self.meta }
    }
    impl StatelessAction for NoopAction {
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

    #[test]
    fn register_and_get_action() {
        let registry = ActionRegistry::new();
        registry.register_stateless(NoopAction::new("test.noop", 1, 0));
        assert_eq!(registry.len(), 1);
        let key = nebula_core::ActionKey::new("test.noop").unwrap();
        let result = registry.get(&key);
        assert!(result.is_some());
    }

    #[test]
    fn register_replaces_same_version() {
        let registry = ActionRegistry::new();
        registry.register_stateless(NoopAction::new("test.noop", 1, 0));
        registry.register_stateless(NoopAction::new("test.noop", 1, 0));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn versioned_lookup() {
        let registry = ActionRegistry::new();
        registry.register_stateless(NoopAction::new("test.noop", 1, 0));
        registry.register_stateless(NoopAction::new("test.noop", 2, 0));

        let key = nebula_core::ActionKey::new("test.noop").unwrap();
        let v1 = nebula_core::InterfaceVersion::new(1, 0);
        let v2 = nebula_core::InterfaceVersion::new(2, 0);

        assert!(registry.get_versioned(&key, &v1).is_some());
        assert!(registry.get_versioned(&key, &v2).is_some());

        // Latest is v2
        let (meta, _) = registry.get(&key).unwrap();
        assert_eq!(meta.version, v2);
    }
}
```

**Step 4: Delete `crates/action/src/registry.rs`**

```bash
git rm crates/action/src/registry.rs
```

**Step 5: Remove registry from `crates/action/src/lib.rs`**

Remove these lines:
```rust
pub mod registry;
pub use registry::ActionRegistry;
```

**Step 6: Remove from `crates/action/src/prelude.rs`**

Remove `pub use crate::registry::ActionRegistry;` if present.

**Step 7: Verify `dashmap` is in `crates/runtime/Cargo.toml`**

Should already be there from the existing legacy registry. If not, add:
```toml
dashmap = { workspace = true }
```

**Step 8: Run check**

Run: `cargo check -p nebula-action`
Expected: compiles. Action no longer has ActionRegistry.

Run: `cargo check -p nebula-runtime`
Expected: **fails** because `runtime/src/runtime.rs` still uses old `InternalHandler` API. Task 4 fixes this.

**Step 9: Commit**

```
refactor: move ActionRegistry from nebula-action to nebula-runtime

WIP: ActionRuntime::run_handler still uses InternalHandler — fixed next.
```

---

## Task 4: Migrate `ActionRuntime::run_handler` to `ActionHandler` enum dispatch

**Files:**
- Modify: `crates/runtime/src/runtime.rs`

This is the main implementation step. We replace the `InternalHandler::execute` call with a `match` on `ActionHandler`, implementing Stateless and Stateful branches and returning typed errors for the others.

**Step 1: Read current `run_handler`**

Read lines 100-170 of `crates/runtime/src/runtime.rs` to understand the existing flow:
- It takes `Arc<dyn InternalHandler>`
- Calls `handler.execute(input, &context).await` for `IsolationLevel::None`
- Wraps in sandbox for other isolation levels
- Records metrics, enforces data limits, returns result

**Step 2: Add imports**

At the top of `runtime.rs`, add:
```rust
use nebula_action::{ActionError, ActionHandler, ActionMetadata, IsolationLevel, ActionResult};
```

(Some may already be present.)

**Step 3: Rewrite `execute_action` and `run_handler`**

```rust
pub async fn execute_action(
    &self,
    action_key: &str,
    input: serde_json::Value,
    context: ActionContext,
) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
    let (metadata, handler) = self
        .registry
        .get_by_str(action_key)
        .ok_or_else(|| RuntimeError::ActionNotFound { key: action_key.to_owned() })?;

    self.run_handler(action_key, metadata, handler, input, context).await
}

async fn run_handler(
    &self,
    action_key: &str,
    metadata: ActionMetadata,
    handler: ActionHandler,
    input: serde_json::Value,
    context: ActionContext,
) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
    let started = Instant::now();
    let action_counter = self.metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL);
    let error_counter = self.metrics.counter(NEBULA_ACTION_FAILURES_TOTAL);
    let duration_hist = self.metrics.histogram(NEBULA_ACTION_DURATION_SECONDS);

    // Non-executable handler variants: Trigger/Resource/Agent are routed via
    // their own runtimes. Returning typed errors here is by design. We DO
    // count these as failed executions in metrics — they were valid lookups
    // that the caller attempted to execute via the wrong path.
    let result = match handler {
        ActionHandler::Stateless(h) => {
            self.execute_stateless(&metadata, h, input, context).await
        }
        ActionHandler::Stateful(h) => {
            self.execute_stateful(&metadata, h, input, context).await
        }
        ActionHandler::Trigger(_) => {
            action_counter.inc();
            error_counter.inc();
            duration_hist.observe(started.elapsed().as_secs_f64());
            return Err(RuntimeError::TriggerNotExecutable {
                key: action_key.to_owned(),
            });
        }
        ActionHandler::Resource(_) => {
            action_counter.inc();
            error_counter.inc();
            duration_hist.observe(started.elapsed().as_secs_f64());
            return Err(RuntimeError::ResourceNotExecutable {
                key: action_key.to_owned(),
            });
        }
        ActionHandler::Agent(_) => {
            action_counter.inc();
            error_counter.inc();
            duration_hist.observe(started.elapsed().as_secs_f64());
            return Err(RuntimeError::AgentNotSupportedYet {
                key: action_key.to_owned(),
            });
        }
    };

    let elapsed = started.elapsed();
    duration_hist.observe(elapsed.as_secs_f64());
    action_counter.inc();

    match result {
        Ok(mut action_result) => {
            self.enforce_data_limit(action_key, &mut action_result, &error_counter)
                .await?;
            Ok(action_result)
        }
        Err(action_err) => {
            error_counter.inc();
            Err(RuntimeError::ActionError(action_err))
        }
    }
}
```

**Step 4: Implement `execute_stateless`**

```rust
async fn execute_stateless(
    &self,
    metadata: &ActionMetadata,
    handler: Arc<dyn nebula_action::StatelessHandler>,
    input: serde_json::Value,
    context: ActionContext,
) -> Result<ActionResult<serde_json::Value>, ActionError> {
    match metadata.isolation_level {
        IsolationLevel::None => handler.execute(input, &context).await,
        _ => {
            // Sandboxed stateless execution: wrap context, dispatch through sandbox.
            let sandboxed = SandboxedContext::new(context);
            self.sandbox.execute_stateless(sandboxed, metadata, handler, input).await
        }
    }
}
```

**Step 5: Implement `execute_stateful` — the iteration loop**

```rust
async fn execute_stateful(
    &self,
    metadata: &ActionMetadata,
    handler: Arc<dyn nebula_action::StatefulHandler>,
    input: serde_json::Value,
    context: ActionContext,
) -> Result<ActionResult<serde_json::Value>, ActionError> {
    // Sandboxed Stateful execution is not yet supported.
    if !matches!(metadata.isolation_level, IsolationLevel::None) {
        return Err(ActionError::fatal(
            "sandboxed stateful execution is not yet supported (Phase 7.6)",
        ));
    }

    // In-memory state checkpoint — runtime drives the loop, persistence is post-MVP.
    let mut state = handler.init_state()?;

    // Hard cap to prevent runaway loops. Configurable later.
    const MAX_ITERATIONS: u32 = 10_000;

    // Borrow input by reference across all iterations — no cloning per loop pass.
    for _iteration in 0..MAX_ITERATIONS {
        // Cooperative cancellation check BEFORE the next iteration.
        if context.cancellation().is_cancelled() {
            return Err(ActionError::Cancelled);
        }

        let result = handler
            .execute(&input, &mut state, &context)
            .await?;

        match result {
            ActionResult::Continue { delay, .. } => {
                if let Some(d) = delay {
                    tokio::time::sleep(d).await;
                }
                // Loop continues with mutated state.
            }
            other => {
                // Break / Success / Skip / Wait / Retry / Branch / etc.
                return Ok(other);
            }
        }
    }

    Err(ActionError::fatal(format!(
        "stateful action '{}' exceeded max iterations ({MAX_ITERATIONS})",
        metadata.key.as_str()
    )))
}
```

**Step 6: Update sandbox signature**

Read `crates/runtime/src/sandbox.rs`. The current `SandboxRunner::execute(ctx, metadata, input)` needs to know which handler variant to dispatch. For Phase 7.5 simplicity:

- Add `execute_stateless(ctx, metadata, handler: Arc<dyn StatelessHandler>, input)` method
- The old `execute(ctx, metadata, input)` either gets removed or becomes a thin wrapper that errors for non-stateless cases

Pick whichever is less invasive based on actual usage in `sandbox.rs`. If sandbox internally still has its own dispatch logic, leave it for Task 7 cleanup.

**Step 7: Run check**

Run: `cargo check -p nebula-runtime`
Expected: compiles (after fixing imports and any sandbox signature mismatches).

**Step 8: Run runtime tests**

Run: `cargo nextest run -p nebula-runtime`
Expected: runtime tests fail at compile time because they still impl `InternalHandler`. This is expected — fixed in Task 5.

**Step 9: Commit**

```
feat(runtime): dispatch on ActionHandler enum, support stateful execution
```

---

## Task 5: Migrate `runtime` tests off `InternalHandler`

**Files:**
- Modify: `crates/runtime/src/runtime.rs` (test module)
- Modify: `crates/runtime/tests/*.rs` if integration tests exist

**Step 1: Find tests using InternalHandler**

Run: `grep -n "InternalHandler" crates/runtime/`

**Step 2: Mechanical rewrite**

For each `impl InternalHandler for FooHandler` test struct, convert to typed `StatelessAction`:

```rust
// Before:
struct EchoHandler { meta: ActionMetadata }

#[async_trait]
impl InternalHandler for EchoHandler {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
    async fn execute(&self, input: Value, _ctx: &ActionContext) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

// Then registry.register(Arc::new(EchoHandler { ... }));

// After:
struct EchoAction { meta: ActionMetadata }

impl ActionDependencies for EchoAction {}
impl Action for EchoAction {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
}

impl StatelessAction for EchoAction {
    type Input = Value;
    type Output = Value;
    async fn execute(&self, input: Value, _ctx: &impl Context) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

// Then registry.register_stateless(EchoAction { ... });
```

**Step 3: Run runtime tests**

Run: `cargo nextest run -p nebula-runtime`
Expected: all tests pass.

**Step 4: Commit**

```
test(runtime): migrate test handlers from InternalHandler to StatelessAction
```

---

## Task 6: Migrate engine tests off `InternalHandler`

**Files:**
- Modify: `crates/engine/src/engine.rs` (test module, lines ~1976-3375)
- Modify: `crates/engine/tests/integration.rs`
- Modify: `crates/engine/tests/resource_integration.rs`

**Step 1: Catalog test handlers in engine.rs**

Run: `grep -n "impl InternalHandler" crates/engine/src/engine.rs`

Should find ~7-10 test handlers: `EchoHandler`, `FailHandler`, `SlowHandler`, `SkipHandler`, `BranchHandler`, `CountingHandler`, `V1Handler`, `V2Handler`, etc.

**Step 2: Migrate each one**

Same pattern as Task 5. Each `impl InternalHandler for X` becomes `impl StatelessAction for X` with the boilerplate trait stack (`Action`, `ActionDependencies`).

**Important:** Some test handlers might rely on raw `Value` input/output. That maps directly to `type Input = Value; type Output = Value;`.

**Step 3: Update registration calls**

`registry.register(Arc::new(EchoHandler { ... }))` → `registry.register_stateless(EchoAction { ... })`

**Step 4: Migrate `crates/engine/tests/integration.rs`**

Same pattern. About 6 test handlers based on grep.

**Step 5: Migrate `crates/engine/tests/resource_integration.rs`**

Same pattern. About 1-2 test handlers.

**Step 6: Run engine tests**

Run: `cargo nextest run -p nebula-engine`
Expected: all tests pass. May need to remove `#![allow(deprecated)]` from these files.

**Step 7: Commit**

```
test(engine): migrate test handlers from InternalHandler to StatelessAction
```

---

## Task 7: Delete `InternalHandler` and remove `#![allow(deprecated)]`

**Files:**
- Modify: `crates/action/src/handler.rs` (delete `InternalHandler` trait and `impl InternalHandler for StatelessActionAdapter`)
- Modify: `crates/action/src/lib.rs` (remove `pub use handler::InternalHandler` and `#[allow(deprecated)]`)
- Modify: `crates/runtime/src/lib.rs` (remove `#![allow(deprecated)]`)
- Modify: `apps/cli/src/main.rs` (remove `#![allow(deprecated)]`)
- Modify: `crates/engine/src/lib.rs` (remove `#![allow(deprecated)]` if present)

**Step 1: Verify nothing else uses InternalHandler**

Run: `grep -rn "InternalHandler" --include="*.rs" 2>&1 | grep -v ".worktrees"`

Should return only `crates/action/src/handler.rs` (the definition itself). If any other file still references it, go back and migrate.

**Step 2: Delete `InternalHandler` from `handler.rs`**

Find and delete:
- The `#[deprecated(...)]` attribute on the trait
- The `pub trait InternalHandler` definition (~15 lines)
- The `#[allow(deprecated)]` `impl InternalHandler for StatelessActionAdapter` block (~30 lines)
- Test code using InternalHandler in handler.rs `mod tests`

Keep `StatelessActionAdapter` and its `impl StatelessHandler for StatelessActionAdapter` block — that's the new path.

**Step 3: Update `lib.rs` re-exports**

In `crates/action/src/lib.rs`, remove:

```rust
#[allow(deprecated)]
// Reason: InternalHandler re-exported for backward compat during migration
pub use handler::InternalHandler;
```

**Step 4: Remove `#![allow(deprecated)]` from runtime, cli, engine**

```rust
// Remove from:
// crates/runtime/src/lib.rs:3
// apps/cli/src/main.rs:1
// crates/engine/src/lib.rs:3 (if present)
```

**Step 5: Run full workspace check**

Run: `cargo check --workspace && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace`
Expected: all pass, zero warnings.

**Step 6: Commit**

```
chore(action): delete deprecated InternalHandler trait
```

---

## Task 8: Add a CLI smoke test for PaginatedAction

**Files:**
- Modify: `apps/cli/src/actions.rs`

**Step 1: Add a `paginated_demo` action**

This is the validation that Phase 6 + Phase 7.5 work end-to-end from a CLI perspective. A trivial paginator that produces 3 pages of synthetic data.

Add to `apps/cli/src/actions.rs`:

```rust
use nebula_action::stateful::{PageResult, PaginatedAction};

// ── paginated_demo — yields 3 pages of fake data, demonstrates DX ──────────

struct PaginatedDemoAction { meta: ActionMetadata }

impl PaginatedDemoAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("paginated_demo"),
                "Paginated Demo",
                "Demo: yields 3 pages of fake data via PaginatedAction DX trait",
            ),
        }
    }
}

impl ActionDependencies for PaginatedDemoAction {}
impl Action for PaginatedDemoAction {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
}

impl PaginatedAction for PaginatedDemoAction {
    type Input = serde_json::Value;
    type Output = Vec<i32>;
    type Cursor = u32;

    fn max_pages(&self) -> u32 { 5 }

    async fn fetch_page(
        &self,
        _input: &Self::Input,
        cursor: Option<&u32>,
        _ctx: &impl Context,
    ) -> Result<PageResult<Vec<i32>, u32>, ActionError> {
        let page = cursor.copied().unwrap_or(0);
        let data: Vec<i32> = ((page * 10)..((page + 1) * 10)).map(|i| i as i32).collect();
        let next = if page + 1 < 3 { Some(page + 1) } else { None };
        Ok(PageResult { data, next_cursor: next })
    }
}

nebula_action::impl_paginated_action!(PaginatedDemoAction);
```

**Step 2: Register in `register_builtins`**

```rust
pub fn register_builtins(registry: &ActionRegistry) {
    // ... existing registrations ...
    registry.register_stateful(PaginatedDemoAction::new());
}
```

(Note: `register_builtins` already takes `&ActionRegistry`. Now uses `register_stateful` from runtime's registry.)

**Step 3: Run check**

Run: `cargo check -p nebula-cli`
Expected: compiles.

**Step 4: Commit**

```
feat(cli): add paginated_demo action validating PaginatedAction DX in CLI
```

---

## Task 9: Workspace validation + context docs

**Files:**
- Modify: `.claude/crates/action.md`
- Modify: `.claude/crates/runtime.md`

**Step 1: Run full workspace check**

```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace
```

Expected: all pass. The known sandbox failures (pre-existing, unrelated to this work) are not part of the exit criteria.

**Step 2: Update `.claude/crates/action.md`**

Add to "Key Decisions":

```
- `ActionRegistry` lives in `nebula-runtime`, NOT `nebula-action`. Protocol crate stays free of execution concerns and dashmap dependency. Phase 7.5 moved it.
- `StatefulHandler::execute` takes `input: &Value` (not owned `Value`) — runtime loop borrows input across iterations without per-iteration cloning. Adapter clones once internally for typed deserialization. User-level `StatefulAction::execute` is unchanged.
- `InternalHandler` deleted entirely. All execution goes through `ActionHandler` enum dispatch in `ActionRuntime::run_handler`.
```

Add to "Traps":

```
- `ActionRegistry` is in `nebula-runtime`, not `nebula-action`. `use nebula_runtime::ActionRegistry`.
- `StatefulHandler::execute` borrows input — pass `&value` not `value` when calling at the JSON-handler level.
```

**Step 3: Update `.claude/crates/runtime.md`** (or create if missing)

Add or update:

```
## Phase 7.5 changes (2026-04-09)

### ActionRegistry
- Now lives in `nebula-runtime` (moved from `nebula-action`). Single source of truth for action registration.
- DashMap-backed, `&self` API — share via `Arc<ActionRegistry>` without external locking.
- Convenience methods: `register_stateless`, `register_stateful`, `register_trigger`, `register_webhook`, `register_poll`, `register_resource`.
- Lookup returns owned `(ActionMetadata, ActionHandler)` — `ActionHandler` is `Arc` inside, cloning is cheap.

### ActionRuntime::run_handler
- Dispatches on `ActionHandler` enum:
  - `Stateless` → direct execution (or sandbox for non-None isolation)
  - `Stateful` → iteration loop with in-memory state checkpoint, hard cap MAX_ITERATIONS=10_000
  - `Trigger` → `Err(RuntimeError::TriggerNotExecutable)` permanently — triggers run via dedicated trigger runtime (post-v1)
  - `Resource` → `Err(RuntimeError::ResourceNotExecutable)` permanently — resources scoped via resource graph (post-v1)
  - `Agent` → `Err(RuntimeError::AgentNotSupportedYet)` (Phase 9)
- Sandboxed stateful execution returns Fatal — only Stateless supports non-None isolation in Phase 7.5.

## Traps
- `ActionRuntime::execute_action` only runs Stateless and Stateful actions. Triggers/Resources/Agents return typed errors — they have separate lifecycles that don't fit the one-shot execute model.
- Stateful state is in-memory only — does not survive process restart. Persistence requires nebula-storage integration.
- Sandboxed Stateful execution is not yet implemented. Returns `Fatal` for non-None isolation.
- Cooperative cancellation in stateful loop checks between iterations only. A poorly-written `execute()` that hangs forever inside one iteration cannot be cancelled.
```

**Step 4: Commit**

```
docs: update context for Phase 7.5 registry unification
```

---

## Exit Criteria Verification

| Criterion                                                  | Test                                                       |
|------------------------------------------------------------|------------------------------------------------------------|
| `ActionRegistry` lives only in `nebula-runtime`           | `grep ActionRegistry crates/action/src/` returns nothing    |
| `nebula-action` does not depend on `dashmap`              | `grep dashmap crates/action/Cargo.toml` returns nothing     |
| `InternalHandler` deleted                                  | `grep InternalHandler` returns nothing outside worktree    |
| `#![allow(deprecated)]` removed from runtime/cli/engine    | grep returns nothing                                        |
| Stateless execution still works                            | All existing runtime + engine tests pass                   |
| Stateful execution works                                   | Loop test in runtime, paginated_demo registers in CLI      |
| Trigger returns typed error                                | New runtime test asserts `TriggerNotExecutable`            |
| Resource returns typed error                               | New runtime test asserts `ResourceNotExecutable`           |
| CLI compiles with `register_stateful` for paginated demo  | `cargo check -p nebula-cli`                                |
| Workspace clippy clean                                     | `cargo clippy --workspace -- -D warnings`                  |
| All workspace tests pass                                   | `cargo nextest run --workspace`                            |

## Risk Notes

**Registry move** (`nebula-action` → `nebula-runtime`): mechanical but touches re-exports. The risk is missing a `use nebula_action::ActionRegistry` in some downstream crate. Mitigation: `cargo check --workspace` after Task 3 catches all compile errors.

**`StatefulHandler::execute` signature change** (`Value` → `&Value`): breaking change to a public trait, but the user-facing `StatefulAction::execute` is unaffected because it operates on typed `A::Input`, not the JSON-erased value. ~6 test call sites need updating with `&` prefix.

**Engine test rewrite**: ~15 test handlers across 3 files. Mechanical, but tedious. The risk is missing one and leaving a stale `impl InternalHandler` that won't compile after Task 7. Run `grep` between tasks.

**Sandbox signature**: `SandboxRunner::execute` currently takes `(ctx, metadata, input)`. Adding typed handler dispatch means a new method or signature change. Defer Stateful sandboxing — Phase 7.5 only supports sandboxed Stateless.

**Cancellation in Stateful loop**: cooperative — checked between iterations. A hung `execute()` cannot be cancelled. Phase 7.6 concern.

**Stateful state persistence**: in-memory only. Process restart loses state mid-iteration. Acceptable for alpha.

**`MAX_ITERATIONS = 10_000`**: arbitrary safety cap. Hardcoded for now.

**Metrics for non-executable variants**: Trigger/Resource/Agent paths increment both `executions_total` and `failures_total`. Documented as a deliberate choice — they were valid lookups attempted via the wrong path, so they count as failed executions.
