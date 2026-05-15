//! Webhook-activation record DTO (ADR-0049).
use crate::Scope;
use serde::{Deserialize, Serialize};

/// One webhook-activation row.
///
/// Maps an incoming `POST /hooks/{slug}` to the owning trigger without
/// scanning trigger configs. Scoped so dispatch resolves only within the
/// request's tenant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebhookActivationRecord {
    /// Owning trigger id (opaque string form).
    pub trigger_id: String,
    /// Tenant scope this activation belongs to.
    pub scope: Scope,
    /// Author-configured webhook path slug.
    pub slug: String,
    /// Whether the activation is currently routable.
    pub active: bool,
}
