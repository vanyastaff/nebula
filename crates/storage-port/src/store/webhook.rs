//! Webhook-activation store trait (durable activation rows for outbound webhooks).
use crate::dto::WebhookActivationRecord;
use crate::error::StorageError;
use crate::scope::Scope;

/// Maps an incoming `POST /hooks/{slug}` to its owning trigger without
/// scanning trigger configs. Scoped so dispatch resolves only within the
/// request's tenant.
#[async_trait::async_trait]
pub trait WebhookActivationStore: Send + Sync + std::fmt::Debug {
    /// Register (or replace) a webhook activation.
    async fn upsert(
        &self,
        scope: &Scope,
        record: WebhookActivationRecord,
    ) -> Result<(), StorageError>;

    /// Resolve an active activation by slug within `scope`.
    async fn resolve(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WebhookActivationRecord>, StorageError>;

    /// Deactivate an activation for a trigger.
    async fn deactivate(&self, scope: &Scope, trigger_id: &str) -> Result<(), StorageError>;

    /// Resolve the single activation row whose stored token hash matches.
    ///
    /// **SYSTEM-SURFACE** — the tenant scope comes *out of* the returned row
    /// (scope discovery), so this method is NOT scope-keyed.  Callers pass
    /// the SHA-256 hash of the capability token, **never** the plaintext.
    ///
    /// # Sentinel rejection
    ///
    /// If `token_hash` is the all-zeros sentinel (`[0u8; 32]`) this method
    /// returns `Ok(None)` **without querying the backend**.  The sentinel
    /// means "no token assigned yet" and is excluded from the partial unique
    /// index on `port_webhook_activations`; querying for it would either
    /// return garbage or match many rows.
    ///
    /// # Fail-closed parsing
    ///
    /// Mode and token-hash fields are parsed with the same fail-closed
    /// defaults as [`WebhookActivationStore::resolve`]: an unrecognised mode
    /// text falls back to `Test`; a malformed or short blob falls back to
    /// the zero sentinel.
    async fn resolve_by_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<WebhookActivationRecord>, StorageError>;

    /// All **active** activation rows across **all** tenants.
    ///
    /// **SYSTEM-SURFACE** — cross-tenant enumeration, not scope-keyed.
    /// Intended for bootstrap map population: the caller reads the full set
    /// of active rows once on startup and populates an in-process routing
    /// map; individual requests then resolve in memory without touching
    /// storage.
    ///
    /// The returned order is unspecified and may differ between backends.
    async fn list_all_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError>;
}
