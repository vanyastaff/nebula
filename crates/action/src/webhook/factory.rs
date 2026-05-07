//! Provider factory contract.
//!
//! Operator-configured webhook activations carry an `action_kind`
//! (`"slack"`, `"stripe"`, `"generic"`, …) plus provider-specific
//! parameters. The runtime registry looks up a
//! [`WebhookActionFactory`] for that kind and calls
//! [`WebhookActionFactory::build`] to produce the dyn-erased
//! [`TriggerHandler`] the transport ultimately registers under a
//! [`crate::WebhookKeySlug`]-equivalent key.
//!
//! This trait lives in `nebula-action` (not in the engine) so each
//! provider in [`crate::webhook::providers`] can implement it
//! natively. The engine simply maintains a string-keyed map of
//! factories.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::trigger::TriggerHandler;

/// Operator-supplied parameters for an activation.
///
/// Source of truth: storage row decoded from the kind-namespaced
/// `triggers.config` JSONB. The factory consumes a borrowed view so
/// implementors can clone selectively.
///
/// `provider_config` is an opaque JSON blob that the factory
/// interprets — Slack and Stripe use it for nothing today; Generic
/// uses it for `challenge_token`. Future providers can extend without
/// adding fields here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WebhookActivationSpec {
    /// `"slack" | "stripe" | "generic"` — keys the factory registry.
    pub action_kind: String,
    /// HMAC secret material as raw bytes. Storage layers should
    /// resolve this from a credential reference rather than store
    /// raw bytes inline; see ADR-0049 for the secret-handling
    /// rationale.
    pub secret: Vec<u8>,
    /// Replay window in seconds. `None` defers to the provider's
    /// published default (5 min for Slack, Stripe; opt-in for
    /// Generic).
    pub replay_window_secs: Option<u64>,
    /// Override for the timestamp header (Generic uses
    /// `X-Nebula-Timestamp` by convention; Slack and Stripe ignore
    /// this field — their headers are fixed).
    pub timestamp_header: Option<String>,
    /// Provider-specific JSON blob. Generic looks for
    /// `{"challenge_token": "..."}`; other providers ignore.
    pub provider_config: Option<serde_json::Value>,
    /// Optional per-key rate-limit override (requests per minute).
    pub rate_limit_per_minute: Option<u64>,
}

/// Failures from [`WebhookActionFactory::build`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FactoryError {
    /// No factory registered for the given `action_kind`.
    #[error("unknown webhook provider kind: {0}")]
    UnknownKind(String),
    /// Spec was structurally valid but provider-specific fields were
    /// missing or malformed (e.g. Generic without
    /// `challenge_token`).
    #[error("invalid spec for {kind}: {reason}")]
    InvalidSpec {
        /// The factory's `kind()` so callers know which provider
        /// rejected the spec.
        kind: &'static str,
        /// One-line cause; safe for surfacing in observability.
        reason: String,
    },
    /// Secret resolution failed (credential store unavailable, etc).
    #[error("secret resolution failed: {0}")]
    SecretResolution(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Provider factory: takes a [`WebhookActivationSpec`] and produces
/// a dyn-erased [`TriggerHandler`] ready to register with the
/// transport.
///
/// Implementors live in [`crate::webhook::providers`]. The engine
/// runtime registers one instance per provider kind at startup.
pub trait WebhookActionFactory: Send + Sync + 'static {
    /// Stable string key used by the registry. Must match the
    /// `action_kind` field in [`WebhookActivationSpec`].
    fn kind(&self) -> &'static str;

    /// Build a handler from a stored activation spec.
    ///
    /// # Errors
    ///
    /// Returns [`FactoryError::InvalidSpec`] if provider-specific
    /// fields are missing or malformed; [`FactoryError::SecretResolution`]
    /// if the secret material cannot be resolved.
    fn build(&self, spec: &WebhookActivationSpec) -> Result<Arc<dyn TriggerHandler>, FactoryError>;
}
