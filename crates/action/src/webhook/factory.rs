//! Provider factory contract.
//!
//! Operator-configured webhook activations carry an `action_kind`
//! (`"slack"`, `"stripe"`, `"generic"`, …) plus provider-specific
//! parameters. The runtime registry looks up a
//! [`WebhookActionFactory`] for that kind and calls
//! [`WebhookActionFactory::build`] to produce the dyn-erased
//! [`TriggerHandler`] the transport ultimately registers under a
//! `WebhookKey::Slug`-equivalent key (the API-side enum lives in
//! `nebula-api` and is intentionally not imported here).
//!
//! This trait lives in `nebula-action` (not in the engine) so each
//! provider in [`crate::webhook::providers`] can implement it
//! natively. The engine simply maintains a string-keyed map of
//! factories.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{trigger::TriggerHandler, webhook::WebhookConfig};

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
    /// raw bytes inline; see for the secret-handling
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
    /// Override for the timestamp encoding. Slack and Stripe ignore
    /// this field (they hardcode Unix seconds); Generic respects it
    /// when its `RequiredPolicy` carries a timestamp header.
    pub timestamp_format: Option<crate::webhook::TimestampFormat>,
    /// Provider-specific JSON blob. Generic looks for
    /// `{"challenge_token": "..."}`; other providers ignore.
    pub provider_config: Option<serde_json::Value>,
    /// Optional per-key rate-limit override (requests per minute).
    pub rate_limit_per_minute: Option<u64>,
}

impl WebhookActivationSpec {
    /// Construct a spec with the required fields populated. Optional
    /// knobs default to `None`; chain `with_*` to set them.
    #[must_use]
    pub fn new(action_kind: impl Into<String>, secret: impl Into<Vec<u8>>) -> Self {
        Self {
            action_kind: action_kind.into(),
            secret: secret.into(),
            replay_window_secs: None,
            timestamp_header: None,
            timestamp_format: None,
            provider_config: None,
            rate_limit_per_minute: None,
        }
    }

    /// Override the timestamp encoding (Unix seconds / Unix
    /// milliseconds / RFC 3339).
    #[must_use]
    pub fn with_timestamp_format(mut self, format: crate::webhook::TimestampFormat) -> Self {
        self.timestamp_format = Some(format);
        self
    }

    /// Override the replay window (seconds).
    #[must_use]
    pub fn with_replay_window_secs(mut self, secs: u64) -> Self {
        self.replay_window_secs = Some(secs);
        self
    }

    /// Override the timestamp header name.
    #[must_use]
    pub fn with_timestamp_header(mut self, header: impl Into<String>) -> Self {
        self.timestamp_header = Some(header.into());
        self
    }

    /// Attach a provider-specific JSON config blob.
    #[must_use]
    pub fn with_provider_config(mut self, config: serde_json::Value) -> Self {
        self.provider_config = Some(config);
        self
    }

    /// Override the per-key rate-limit budget (requests per minute).
    #[must_use]
    pub fn with_rate_limit_per_minute(mut self, rpm: u64) -> Self {
        self.rate_limit_per_minute = Some(rpm);
        self
    }
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

/// Output of [`WebhookActionFactory::build`] — the dyn-erased
/// handler plus the cached [`WebhookConfig`] read from the action.
///
/// The transport needs both to register a slug activation: the
/// handler (for dispatch) and the config (for signature/replay
/// enforcement). The factory cannot expose only the dyn handler
/// because [`crate::WebhookConfig`] does not flow through
/// [`TriggerHandler`].
pub struct BuiltWebhookHandler {
    /// Dyn-erased handler the transport stores in its routing map.
    /// The concrete underlying type is
    /// <code>[crate::WebhookTriggerAdapter]&lt;A&gt;</code> for some provider `A`.
    pub handler: Arc<dyn TriggerHandler>,
    /// `WebhookConfig` read from the wrapped [`crate::WebhookAction`]
    /// at construction time. Identical to what
    /// [`crate::WebhookTriggerAdapter::config`] would return if the
    /// transport had a typed reference.
    pub config: WebhookConfig,
}

/// Provider factory: takes a [`WebhookActivationSpec`] and produces
/// a [`BuiltWebhookHandler`] ready to register with the transport.
///
/// Implementors live in [`crate::webhook::providers`]. The engine
/// runtime registers one instance per provider kind at startup.
pub trait WebhookActionFactory: Send + Sync + 'static {
    /// Stable string key used by the registry. Must match the
    /// `action_kind` field in [`WebhookActivationSpec`].
    fn kind(&self) -> &'static str;

    /// Build a handler + config bundle from a stored activation
    /// spec.
    ///
    /// # Errors
    ///
    /// Returns [`FactoryError::InvalidSpec`] if provider-specific
    /// fields are missing or malformed; [`FactoryError::SecretResolution`]
    /// if the secret material cannot be resolved.
    fn build(&self, spec: &WebhookActivationSpec) -> Result<BuiltWebhookHandler, FactoryError>;
}
