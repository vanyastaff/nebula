//! Production [`TriggerSpecLookup`] backed by the `port_triggers` store.
//!
//! [`TriggerStoreSpecLookup`] holds the **undecorated** `Arc<dyn TriggerStore>`
//! and re-binds it to the caller-supplied [`Scope`] on every call via
//! `nebula_tenancy::ScopedTriggerStore`. This means the raw store never
//! serves a cross-tenant row — the scope binding is structural, not discipline.
//!
//! # Scope enforcement
//!
//! `ScopedTriggerStore::get` replaces the caller-supplied `scope` argument with
//! the scope the decorator was bound to at construction. A `trigger_id` from
//! tenant A passed alongside tenant B's `scope` correctly misses (returns
//! `Ok(None)`) because the underlying store partitions by
//! `(workspace_id, org_id)` (see `nebula_tenancy::ScopedTriggerStore` docs).

use std::future::Future;
use std::sync::Arc;

use nebula_storage::rows::WebhookActivationSpec;
use nebula_storage_port::{Scope, store::TriggerStore};
use nebula_tenancy::ScopedTriggerStore;

use super::bootstrap::TriggerSpecLookup;

/// [`TriggerSpecLookup`] backed by a real `port_triggers` store (ADR-0096).
///
/// Constructed at the composition root from the same `Arc<dyn TriggerStore>`
/// as the trigger-CRUD handlers. Wrap this in `Arc` and pass to
/// [`crate::AppState::with_webhook_spec_lookup`].
///
/// # Tenant isolation
///
/// Each call to [`lookup`](TriggerStoreSpecLookup::lookup) constructs a fresh
/// `ScopedTriggerStore` bound to the supplied `scope`, ensuring the underlying
/// store partitions the `get` by `(workspace_id, org_id)`. A row that exists
/// under `scope_a` is invisible to a call with `scope_b` — the scoped
/// decorator closes the BOLA/IDOR surface by construction.
#[derive(Clone)]
pub struct TriggerStoreSpecLookup {
    store: Arc<dyn TriggerStore>,
}

impl std::fmt::Debug for TriggerStoreSpecLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerStoreSpecLookup")
            .finish_non_exhaustive()
    }
}

impl TriggerStoreSpecLookup {
    /// Wrap `store` in a spec-lookup adapter.
    ///
    /// `store` must be the **undecorated** base `TriggerStore`; the adapter
    /// applies `ScopedTriggerStore` per call.
    #[must_use]
    pub fn new(store: Arc<dyn TriggerStore>) -> Self {
        Self { store }
    }
}

impl TriggerSpecLookup for TriggerStoreSpecLookup {
    fn lookup<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        scope: &'life1 Scope,
        trigger_id: &'life2 str,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Option<WebhookActivationSpec>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
    {
        let scoped = ScopedTriggerStore::new(Arc::clone(&self.store), scope.clone());
        let trigger_id = trigger_id.to_owned();
        Box::pin(async move {
            // The `scope` arg passed to `ScopedTriggerStore::get` is ignored by
            // the decorator — it always partitions by `self.bound` (the scope
            // captured in `ScopedTriggerStore::new` above). Passing it here
            // matches the workspace convention used by every other scoped-store
            // call site (e.g. `ScopedWorkflowVersionStore::get_published`); the
            // isolation guarantee is structural via the bound scope, not the arg.
            let row = scoped
                .get(scope, &trigger_id)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            let Some(row) = row else {
                return Ok(None);
            };
            // `from_trigger_config` returns `Ok(None)` when the
            // `webhook_activation` key is absent (e.g. a cron trigger).
            WebhookActivationSpec::from_trigger_config(&row.config)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
        })
    }
}
