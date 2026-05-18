//! Webhook-activation store trait (ADR-0049).
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
}
