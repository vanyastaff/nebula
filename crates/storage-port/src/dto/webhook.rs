//! Webhook-activation record DTO.
use crate::Scope;
use serde::{Deserialize, Serialize};

/// Routing mode for a webhook activation.
///
/// Controls whether the activation routes to the live durable workflow engine
/// (`Prod`) or only to the lightweight test-mode path (`Test`).  The default
/// is `Test` so that an activation row inserted without an explicit mode never
/// accidentally triggers production side-effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WebhookMode {
    /// Test mode: routes only to the lightweight, non-durable test path.
    ///
    /// This is the safe default.  An activation that has not been explicitly
    /// promoted to `Prod` will never trigger a live workflow run.
    #[default]
    Test,
    /// Production mode: routes to the durable workflow engine.
    ///
    /// Only activations with this mode participate in the ADR-0095 D1
    /// U-D1.4b capability-token dispatch path.
    Prod,
}

/// One webhook-activation row.
///
/// Maps an incoming capability-token request to the owning trigger without
/// scanning trigger configs. Scoped so dispatch resolves only within the
/// request's tenant.
///
/// The struct is `#[non_exhaustive]`; use [`WebhookActivationRecord::new`]
/// to construct it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebhookActivationRecord {
    /// The workflow trigger-binding NodeKey — the dispatch routing key
    /// (`do_emit_prod` parses it as a NodeKey to resolve the binding).
    /// NOT the `port_triggers` PK (that is `spec_trigger_id`).
    pub trigger_id: String,
    /// Tenant scope this activation belongs to.
    pub scope: Scope,
    /// Author-configured webhook path slug.
    pub slug: String,
    /// Whether the activation is currently routable.
    pub active: bool,
    /// The workflow this activation dispatches into, if known.
    ///
    /// `None` until an activation is wired to a specific workflow by the
    /// API layer (ADR-0096 commit 2).
    pub workflow_id: Option<String>,
    /// Routing mode for this activation.
    ///
    /// Defaults to [`WebhookMode::Test`].  Must be explicitly set to
    /// [`WebhookMode::Prod`] before the activation participates in the
    /// ADR-0095 D1 U-D1.4b capability-token dispatch path.
    pub mode: WebhookMode,
    /// SHA-256 hash of the capability token for this activation.
    ///
    /// Stored as a fixed 32-byte array.  The all-zeros sentinel
    /// (`[0u8; 32]`) means "no token assigned yet" — the partial unique
    /// index on the `port_webhook_activations` table excludes the sentinel
    /// so existing rows do not collide.
    pub token_hash: [u8; 32],
    /// The `port_triggers` spec-row PK (`TriggerId`, `trg_` prefix) this
    /// activation was built from — the ADR-0101 L1 spec link, used by
    /// bootstrap reconstruct to re-resolve the webhook spec via the API
    /// crate's `TriggerSpecLookup` (the trait lives in `nebula-api`, so this
    /// is a plain reference, not an intra-doc link).
    ///
    /// `None` on legacy rows written before this field existed.
    pub spec_trigger_id: Option<String>,
}

impl WebhookActivationRecord {
    /// Construct a new record with safe defaults for the extended fields.
    ///
    /// `workflow_id` is `None`, `mode` is [`WebhookMode::default()`]
    /// (currently [`WebhookMode::Test`]), `token_hash` is the all-zeros
    /// sentinel (no token assigned yet), and `spec_trigger_id` is `None`.
    #[must_use]
    pub fn new(
        trigger_id: impl Into<String>,
        scope: Scope,
        slug: impl Into<String>,
        active: bool,
    ) -> Self {
        Self {
            trigger_id: trigger_id.into(),
            scope,
            slug: slug.into(),
            active,
            workflow_id: None,
            mode: WebhookMode::default(),
            token_hash: [0u8; 32],
            spec_trigger_id: None,
        }
    }
}
