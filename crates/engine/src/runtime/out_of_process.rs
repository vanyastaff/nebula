//! Gated out-of-process plugin dispatch through the engine-owned pool.
//!
//! This whole module is behind the `out-of-process-plugins` Cargo feature
//! (default OFF). The feature is the outer gate; the inner gate is
//! [`OutOfProcessConfig::plugin_dirs`] — empty by default. Both must be
//! satisfied (feature on AND a non-empty dir list) before any
//! [`ProcessSandbox`] is constructed. A workflow author cannot reach
//! either gate: they live at the composition root.
//!
//! When the gate is open, [`discover_into_registry`] scans each configured
//! directory via [`nebula_plugin::discovery::discover_directory`],
//! registers the plugins in the supplied [`PluginRegistry`], and registers
//! a [`PooledRemoteActionFactory`] per discovered action into the
//! [`ActionRegistry`]. Dispatch of those actions then flows through an
//! engine-owned [`PluginPool`] keyed by `(binary, ScopeHash)` (ADR-0025
//! §2): the credential scope is derived **engine-side** from the workflow
//! node's slot bindings at dispatch — never on the leaf, never from
//! `ActionMetadata`.
//!
//! There is no broker yet: until the ADR-0025 broker lands there is no
//! egress or credential mediation for these processes. [`discover_into_registry`]
//! emits a single `tracing::warn!` invariant at startup stating exactly
//! that, so an operator who opens the gate sees the honest security
//! posture in the logs.
//!
//! The `runtime.rs` `IsolationLevel` match is deliberately untouched:
//! discovered actions register as ordinary stateless factories, so the
//! live in-process path and the §13 knife are byte-for-byte unaffected
//! when the gate is closed.

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandler, ActionMetadata, ActionResult,
    ErasedAction, ErasedStateless,
};
use nebula_plugin::{PluginRegistry, sandbox_error_to_action_error};
use nebula_sandbox::ProcessSandbox;
use nebula_workflow::NodeDefinition;
use serde_json::Value;

use crate::runtime::{
    ActionRegistry,
    plugin_pool::{Lease, PluginPool, PoolRegistry, pool_key},
};

/// Operator-only runtime configuration for out-of-process plugin dispatch.
///
/// The inner gate. Default-constructed it has **no** plugin directories,
/// so even with the `out-of-process-plugins` feature compiled in the
/// engine constructs no [`ProcessSandbox`] and the registry is identical
/// to today. Populating `plugin_dirs` is a deliberate operator action at
/// the composition root.
#[derive(Debug, Clone)]
pub struct OutOfProcessConfig {
    /// Directories scanned for `nebula-plugin-*` binaries. Empty disables
    /// the path entirely (no discovery, no pool, no behavior change).
    pub plugin_dirs: Vec<PathBuf>,
    /// Per-call envelope round-trip timeout for spawned plugin processes.
    pub default_timeout: Duration,
    /// Maximum concurrent plugin processes per `(binary, scope)` pool key.
    pub max_processes_per_key: usize,
}

impl Default for OutOfProcessConfig {
    fn default() -> Self {
        Self {
            plugin_dirs: Vec::new(),
            default_timeout: Duration::from_secs(30),
            max_processes_per_key: 4,
        }
    }
}

/// `ActionFactory` that dispatches a discovered out-of-process action
/// through an engine-owned [`PluginPool`].
///
/// One factory per discovered action. The credential-scope identity
/// (ADR-0025 §2) is computed in [`instantiate`](ActionFactory::instantiate)
/// from the **workflow node**'s slot bindings via
/// [`pool_key`] — the node is in scope there, the leaf
/// `nebula-sandbox` never sees it, and `ActionMetadata` is never the
/// source.
pub struct PooledRemoteActionFactory {
    metadata: ActionMetadata,
    /// Plugin-local action key (the un-namespaced key the plugin matches
    /// on in its own `PluginHandler::execute`). Sent over the transport
    /// instead of the namespaced `metadata.base.key`.
    local_key: String,
    binary: PathBuf,
    timeout: Duration,
    pools: Arc<PoolRegistry<ProcessSandbox>>,
}

impl std::fmt::Debug for PooledRemoteActionFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledRemoteActionFactory")
            .field("key", &self.metadata.base.key)
            .field("binary", &self.binary)
            .finish_non_exhaustive()
    }
}

impl PooledRemoteActionFactory {
    /// Build a pooled factory for one discovered action.
    fn new(
        metadata: ActionMetadata,
        local_key: String,
        binary: PathBuf,
        timeout: Duration,
        pools: Arc<PoolRegistry<ProcessSandbox>>,
    ) -> Self {
        Self {
            metadata,
            local_key,
            binary,
            timeout,
            pools,
        }
    }
}

impl ActionFactory for PooledRemoteActionFactory {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        _ctx: &'a dyn ActionContext,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<ErasedAction, ActionError>> + Send + 'a>>
    {
        // ADR-0025 §2: the per-process isolation key is derived here, from
        // the workflow node's credential-slot bindings, engine-side. The
        // leaf transport never sees the node.
        let key = pool_key(node, self.binary.clone());
        let pool = self.pools.pool_for(&key);
        let erased: Box<dyn ErasedStateless> = Box::new(PooledErasedStateless {
            metadata: self.metadata.clone(),
            local_key: self.local_key.clone(),
            binary: self.binary.clone(),
            timeout: self.timeout,
            pool,
        });
        Box::pin(async move { Ok(ErasedAction::Stateless(erased)) })
    }
}

/// The erased action produced per dispatch by [`PooledRemoteActionFactory`].
///
/// Holds the resolved `(binary, scope)` pool. On `dispatch` it acquires a
/// [`Lease`], invokes the action through the leased [`ProcessSandbox`],
/// and — on any transport error — poisons the lease before it drops so the
/// desynced connection is destroyed (SIGKILL via `kill_on_drop`) rather
/// than handed to a different caller.
struct PooledErasedStateless {
    metadata: ActionMetadata,
    /// Plugin-local key sent over the transport (see
    /// [`PooledRemoteActionFactory::local_key`]).
    local_key: String,
    binary: PathBuf,
    timeout: Duration,
    pool: Arc<PluginPool<ProcessSandbox>>,
}

#[async_trait]
impl ErasedStateless for PooledErasedStateless {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    async fn dispatch(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let binary = self.binary.clone();
        let timeout = self.timeout;

        // The pool awaits a capacity permit, then either reuses a warm
        // process or runs this spawn closure. A spawn failure surfaces as
        // a per-call `ActionError` and never leaks a permit or wedges the
        // pool (see `PluginPool::acquire`). `ProcessSandbox::new` is
        // infallible — the actual fork/exec/dial happens lazily on the
        // first envelope round-trip — so the closure is `Ok` here; a
        // genuine spawn failure surfaces from `invoke_with_cancel` below.
        let mut lease: Lease<ProcessSandbox> = self
            .pool
            .acquire(|| Ok::<_, ActionError>(ProcessSandbox::new(binary, timeout)))
            .await?;

        let Some(sandbox) = lease.get() else {
            // `Lease::get()` is `Some` for the lease's whole lifetime; a
            // `None` would be an internal pool invariant break. Poison so
            // the (suspect) connection is destroyed, and surface a typed
            // fatal rather than panicking on the dispatch path.
            lease.poison();
            return Err(ActionError::fatal(
                "plugin pool returned an empty lease for an out-of-process action",
            ));
        };

        let result = sandbox
            .invoke_with_cancel(&self.local_key, input, ctx.cancellation())
            .await;

        match result {
            Ok(output) => {
                // Healthy round-trip: the lease drops here and the warm
                // process returns to the idle set for the next acquirer.
                Ok(ActionResult::success(output))
            },
            Err(sandbox_err) => {
                // Any transport-level failure leaves this connection's
                // request/response stream in an undefined position.
                // Poison so `Drop` discards it instead of re-pooling a
                // desynced process for a different execution.
                lease.poison();
                Err(sandbox_error_to_action_error(sandbox_err))
            },
        }
    }
}

/// Composition-root entry: discover out-of-process plugins per the
/// operator config and register each action behind the engine pool.
///
/// No-op when `config.plugin_dirs` is empty (the inner gate). With the
/// `out-of-process-plugins` feature off the whole module — and therefore
/// this function — does not exist, so the live path is byte-identical.
///
/// Emits exactly one `tracing::warn!` invariant when the gate is open,
/// stating the honest pre-broker security posture (no egress / credential
/// mediation), then for every configured directory calls
/// [`nebula_plugin::discovery::discover_directory`] (which registers the
/// plugins in `plugin_registry`) and registers a
/// [`PooledRemoteActionFactory`] per discovered action into
/// `action_registry`.
pub async fn discover_into_registry(
    config: &OutOfProcessConfig,
    plugin_registry: &mut PluginRegistry,
    action_registry: &ActionRegistry,
) {
    if config.plugin_dirs.is_empty() {
        // Inner gate closed: no discovery, no pool, no behavior change.
        return;
    }

    tracing::warn!(
        target = "engine::out_of_process",
        dirs = ?config.plugin_dirs,
        "out-of-process plugins enabled; no broker egress/credential mediation \
         until the ADR-0025 broker lands — untrusted plugins have unmediated \
         network/credential access"
    );

    let pools: Arc<PoolRegistry<ProcessSandbox>> =
        Arc::new(PoolRegistry::new(config.max_processes_per_key));

    for dir in &config.plugin_dirs {
        let discovered = nebula_plugin::discovery::discover_directory(
            dir,
            plugin_registry,
            config.default_timeout,
        )
        .await;

        for action in discovered {
            // Only stateless out-of-process actions are dispatchable today
            // (the discovery path builds `ProcessSandboxHandler` ->
            // stateless). Anything else is skipped with a warn rather than
            // silently dropped.
            if !matches!(action.handler, ActionHandler::Stateless(_)) {
                tracing::warn!(
                    target = "engine::out_of_process",
                    key = %action.metadata.base.key,
                    "skipping non-stateless discovered action (unsupported \
                     out-of-process kind)"
                );
                continue;
            }

            let metadata = action.metadata.clone();
            let factory = PooledRemoteActionFactory::new(
                metadata.clone(),
                action.local_key.clone(),
                action.binary.clone(),
                config.default_timeout,
                Arc::clone(&pools),
            );
            action_registry.register_factory(metadata, Arc::new(factory));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_no_dirs_so_gate_is_closed() {
        let cfg = OutOfProcessConfig::default();
        assert!(
            cfg.plugin_dirs.is_empty(),
            "default config must keep the inner gate closed (no ProcessSandbox \
             ever constructed unless an operator populates plugin_dirs)"
        );
        assert!(
            cfg.max_processes_per_key > 0,
            "zero capacity would deadlock"
        );
    }

    #[tokio::test]
    async fn discover_into_registry_is_a_noop_when_dirs_empty() {
        let cfg = OutOfProcessConfig::default();
        let mut plugin_registry = PluginRegistry::new();
        let action_registry = ActionRegistry::new();

        discover_into_registry(&cfg, &mut plugin_registry, &action_registry).await;

        assert!(
            action_registry.is_empty(),
            "empty plugin_dirs must register nothing (inner gate closed)"
        );
    }
}
