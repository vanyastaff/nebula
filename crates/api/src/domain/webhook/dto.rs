//! DTOs for the webhook registration endpoint.
//!
//! # Security invariants
//!
//! - The **request** DTO carries NO `scope`, `mode`, `token_hash`, `nonce`,
//!   `secret`, or `secret_id`.  Every security-sensitive field is either
//!   server-derived (scope, mode) or minted by the handler (secret, secret_id,
//!   token_hash, nonce).
//! - The **response** DTO exposes `signing_secret` exactly once (201 body).
//!   No read-back path returns it.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ── Request ───────────────────────────────────────────────────────────────────

/// Request body for `POST /orgs/{org}/workspaces/{ws}/webhooks`.
///
/// The scope is server-derived from the authenticated principal — it MUST NOT
/// be supplied here.  `mode` is always `Prod` for this endpoint; there is no
/// caller override.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterWebhookRequest {
    /// The workflow this trigger belongs to.
    pub workflow_id: String,
    /// The stable `NodeKey` id of the trigger binding within the workflow
    /// definition's `trigger_bindings` array.  Must match a binding entry;
    /// cross-scope or absent ids return 404.
    pub trigger_id: String,
    /// Factory provider tag — selects which `WebhookActionFactory` builds the
    /// handler.  Examples: `"generic"`, `"slack"`, `"stripe"`.
    pub provider: String,
    /// Optional replay-window override in seconds.  `None` keeps the
    /// action's compiled-in default.
    #[serde(default)]
    pub replay_window_secs: Option<u64>,
    /// Optional timestamp header override (lower-cased canonical form).
    #[serde(default)]
    pub timestamp_header: Option<String>,
    /// Optional provider-specific configuration blob.  Decoded by the factory.
    #[serde(default)]
    pub provider_config: Option<serde_json::Value>,
    /// Optional per-key rate-limit budget, in requests per minute.
    #[serde(default)]
    pub rate_limit_per_minute: Option<u64>,
}

// ── Response ──────────────────────────────────────────────────────────────────

/// Response body for a successful webhook registration (HTTP 201).
///
/// `signing_secret` is returned **exactly once** in this response.
/// No subsequent GET returns it — store it immediately or re-register to rotate.
#[derive(Debug, Serialize, ToSchema)]
pub struct RegisterWebhookResponse {
    /// The fully-qualified HTTPS URL the external provider should POST to.
    pub webhook_url: String,
    /// The `whsec_<base64>` HMAC signing secret.  Returned **once only**;
    /// not persisted in plaintext anywhere in Nebula.
    pub signing_secret: String,
    /// Opaque activation identity (the trigger UUID as a string).  Use this
    /// to correlate log entries and to identify this activation in future
    /// management operations.
    pub activation_id: String,
}
