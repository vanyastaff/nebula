//! Provider factory contract.
//!
//! Operator-configured webhook activations carry an `action_kind`
//! (`"slack"`, `"stripe"`, `"generic"`, …) plus provider-specific
//! parameters. The runtime registry looks up a
//! [`WebhookActionFactory`] for that kind and calls
//! [`WebhookActionFactory::build`] to produce the dyn-erased
//! [`TriggerHandler`] the transport ultimately registers under its
//! capability-token routing key (the API-side `WebhookKey` lives in
//! `nebula-api` and is intentionally not imported here).
//!
//! This trait lives in `nebula-action` (not in the engine) so each
//! provider in [`crate::webhook::providers`] can implement it
//! natively. The engine simply maintains a string-keyed map of
//! factories.

use std::{fmt, sync::Arc};

use zeroize::Zeroize;

use crate::{trigger::TriggerHandler, webhook::WebhookConfig};

// ── HmacSecret ──────────────────────────────────────────────────────────────

/// Redacting wrapper for an HMAC verification secret carried in
/// [`WebhookActivationSpec`].
///
/// # Security guarantees
///
/// - `Debug` prints `HmacSecret([REDACTED; N bytes])` — the raw bytes never
///   appear in logs, tracing output, or structured serialization.
/// - `Serialize` / `Deserialize` are intentionally **not** derived: the secret
///   is resolved at runtime from a credential store (not stored inline in the
///   spec row) and therefore has no place in any serialized envelope. Call
///   sites that truly need to persist a secret must do so through the
///   credential subsystem, not through this type.
/// - The inner `Vec<u8>` is zeroed on drop via [`Zeroize`] so the bytes are
///   not left dangling in freed heap memory.
///
/// # Accessing the bytes
///
/// Use [`HmacSecret::reveal`] at the one call site that needs the raw bytes
/// for HMAC computation — the factory `build` implementations.
pub struct HmacSecret(Vec<u8>);

impl HmacSecret {
    /// Wrap raw HMAC secret bytes.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// Expose the raw bytes for HMAC computation.
    ///
    /// Keep the returned slice as short-lived as possible — pass directly to
    /// the HMAC primitive without binding to a named `let` that lingers in
    /// scope.
    #[must_use]
    pub fn reveal(&self) -> &[u8] {
        &self.0
    }

    /// Number of bytes in the secret (safe to log — reveals length, not content).
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the secret is empty (empty secrets are the fail-closed default).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for HmacSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HmacSecret([REDACTED; {} bytes])", self.0.len())
    }
}

impl Drop for HmacSecret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

// ── WebhookActivationSpec ────────────────────────────────────────────────────

/// Operator-supplied parameters for an activation.
///
/// Source of truth: storage row decoded from the kind-namespaced
/// `triggers.config` JSONB. The factory consumes a borrowed view so
/// implementors can clone selectively.
///
/// `provider_config` is an opaque, non-secret JSON blob that the factory
/// interprets. Authority belongs in the credential subsystem and must reach a
/// factory through an explicit secret-bearing type, never as a JSON string.
/// Future providers can extend configuration without adding fields here.
///
/// # Security: `secret` is redacted
///
/// The [`HmacSecret`] field does not implement `Serialize` / `Deserialize` and
/// its `Debug` output is redacted. Call [`WebhookActivationSpec::secret`] to
/// access a reference, or [`WebhookActivationSpec::into_secret`] to take
/// ownership (e.g. when passing into a factory builder). The secret bytes are
/// zeroed on drop.
#[non_exhaustive]
pub struct WebhookActivationSpec {
    /// `"slack" | "stripe" | "generic"` — keys the factory registry.
    pub action_kind: String,
    /// HMAC secret material. Resolved at runtime from the credential store;
    /// never serialized to disk or logs. Access via [`Self::secret`].
    secret: HmacSecret,
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
    /// Provider-specific, non-secret JSON blob. Implementations must reject
    /// inline credentials rather than interpreting them as authority.
    pub provider_config: Option<serde_json::Value>,
    /// Optional per-key rate-limit override (requests per minute).
    pub rate_limit_per_minute: Option<u64>,
}

impl fmt::Debug for WebhookActivationSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebhookActivationSpec")
            .field("action_kind", &self.action_kind)
            .field("secret", &self.secret)
            .field("replay_window_secs", &self.replay_window_secs)
            .field("timestamp_header", &self.timestamp_header)
            .field("timestamp_format", &self.timestamp_format)
            .field(
                "provider_config",
                &self.provider_config.as_ref().map(|_| "[REDACTED]"),
            )
            .field("rate_limit_per_minute", &self.rate_limit_per_minute)
            .finish()
    }
}

impl WebhookActivationSpec {
    /// Construct a spec with the required fields populated. Optional
    /// knobs default to `None`; chain `with_*` to set them.
    #[must_use]
    pub fn new(action_kind: impl Into<String>, secret: impl Into<Vec<u8>>) -> Self {
        Self {
            action_kind: action_kind.into(),
            secret: HmacSecret::new(secret),
            replay_window_secs: None,
            timestamp_header: None,
            timestamp_format: None,
            provider_config: None,
            rate_limit_per_minute: None,
        }
    }

    /// Borrow the HMAC secret bytes.
    ///
    /// Use only at the HMAC computation call site; do not bind to a
    /// long-lived variable.
    #[must_use]
    pub fn secret(&self) -> &HmacSecret {
        &self.secret
    }

    /// Consume the spec, returning the HMAC secret.
    ///
    /// Use when the factory needs to take ownership of the secret bytes
    /// to pass into a provider action constructor (`impl Into<Arc<[u8]>>`).
    #[must_use]
    pub fn into_secret(self) -> HmacSecret {
        self.secret
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

/// Reject a provider configuration for a built-in provider that does not
/// define one. Empty objects are treated as an omitted optional field so JSON
/// clients can use one stable serializer without changing semantics.
fn reject_unsupported_provider_config(
    config: Option<&serde_json::Value>,
    kind: &'static str,
) -> Result<(), FactoryError> {
    match config {
        None => Ok(()),
        Some(serde_json::Value::Object(map)) if map.is_empty() => Ok(()),
        Some(_) => Err(FactoryError::InvalidSpec {
            kind,
            reason: "provider_config is not supported by this built-in provider",
        }),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET_BYTES: &[u8] = b"super-secret-hmac-key";

    static_assertions::assert_not_impl_any!(HmacSecret: Clone);
    static_assertions::assert_not_impl_any!(WebhookActivationSpec: Clone);

    /// `HmacSecret`'s `Debug` output must never contain the raw secret bytes.
    /// This is a security invariant: if the type is formatted in a log or span
    /// field, the key material must not leak (FIX 1).
    #[test]
    fn hmac_secret_debug_is_redacted() {
        let secret = HmacSecret::new(SECRET_BYTES);
        let debug_output = format!("{secret:?}");

        // The raw secret string must not appear — this is the load-bearing assertion.
        assert!(
            !debug_output.contains("super-secret-hmac-key"),
            "raw secret must not appear in Debug output; got: {debug_output}",
        );
        // The redaction marker must be present so consumers know the field was intentionally hidden.
        assert!(
            debug_output.contains("REDACTED"),
            "Debug output must include REDACTED marker; got: {debug_output}",
        );
        // Length is safe to reveal (does not expose key material).
        assert!(
            debug_output.contains(&SECRET_BYTES.len().to_string()),
            "Debug should report byte length for observability; got: {debug_output}",
        );
    }

    /// `WebhookActivationSpec` formatted via `Debug` must not expose the secret.
    /// The spec is the envelope carried through factory dispatch — it may appear
    /// in tracing spans if a caller derives span fields from it.
    #[test]
    fn webhook_activation_spec_debug_redacts_secret() {
        let spec = WebhookActivationSpec::new("slack", SECRET_BYTES);
        let debug_output = format!("{spec:?}");

        assert!(
            !debug_output.contains("super-secret-hmac-key"),
            "raw secret must not appear in spec Debug output; got: {debug_output}",
        );
    }

    #[test]
    fn webhook_activation_spec_debug_redacts_provider_config() {
        const CANARY: &str = "WEBHOOK_ACTION_CONFIG_AUTHORITY_CANARY-24fa";
        let spec = WebhookActivationSpec::new("generic", SECRET_BYTES)
            .with_provider_config(serde_json::json!({ "challenge_token": CANARY }));
        let debug_output = format!("{spec:?}");

        assert!(!debug_output.contains(CANARY));
        assert!(debug_output.contains("[REDACTED]"));
    }

    /// `HmacSecret` must grant access to the raw bytes only via `reveal()`,
    /// and `reveal()` must return the original bytes unchanged.
    #[test]
    fn hmac_secret_reveal_returns_original_bytes() {
        let secret = HmacSecret::new(SECRET_BYTES);
        assert_eq!(secret.reveal(), SECRET_BYTES);
    }

    /// `HmacSecret::len` returns the byte count, not the string length of the
    /// redacted representation — a caller must not use this to infer key material.
    #[test]
    fn hmac_secret_len_matches_byte_count() {
        let secret = HmacSecret::new(SECRET_BYTES);
        assert_eq!(secret.len(), SECRET_BYTES.len());
        assert!(!secret.is_empty());
    }

    /// An empty `HmacSecret` is considered empty — callers can gate on this.
    #[test]
    fn hmac_secret_empty_is_detected() {
        let secret = HmacSecret::new(Vec::<u8>::new());
        assert!(secret.is_empty());
        assert_eq!(secret.len(), 0);
    }

    /// `WebhookActivationSpec::into_secret` transfers ownership so the caller
    /// can pass the secret directly to the HMAC primitive without cloning.
    #[test]
    fn webhook_activation_spec_into_secret_transfers_bytes() {
        let spec = WebhookActivationSpec::new("generic", SECRET_BYTES);
        let secret = spec.into_secret();
        assert_eq!(secret.reveal(), SECRET_BYTES);
    }
}

/// Failures from [`WebhookActionFactory::build`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FactoryError {
    /// No factory registered for the given `action_kind`.
    #[error("unknown webhook provider kind: {0}")]
    UnknownKind(&'static str),
    /// Spec was structurally valid but provider-specific fields were missing,
    /// malformed, or violated an authority boundary.
    #[error("invalid spec for {kind}: {reason}")]
    InvalidSpec {
        /// The factory's `kind()` so callers know which provider
        /// rejected the spec.
        kind: &'static str,
        /// Static one-line cause. Requiring a static string makes it impossible
        /// to echo caller configuration or secret material into diagnostics.
        reason: &'static str,
    },
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

    /// Validate provider-specific configuration before any credential or
    /// activation is persisted.
    ///
    /// The default is deliberately closed: built-in and downstream factories
    /// that do not define a complete non-secret schema accept only an omitted
    /// or empty object. A trusted custom factory may override this method, but
    /// then owns exhaustive validation and MUST reject inline credentials or
    /// bearer authority. Registration calls this seam before its first write;
    /// [`Self::build`] should call it again as defense in depth for bootstrap
    /// and non-HTTP composition paths.
    fn validate_provider_config(
        &self,
        config: Option<&serde_json::Value>,
    ) -> Result<(), FactoryError> {
        reject_unsupported_provider_config(config, self.kind())
    }

    /// Build a handler + config bundle from a stored activation
    /// spec.
    ///
    /// # Errors
    ///
    /// Returns [`FactoryError::InvalidSpec`] if provider-specific fields are
    /// missing or malformed. Secret resolution is deliberately outside the
    /// factory contract: callers resolve authority before constructing the
    /// activation spec.
    fn build(&self, spec: &WebhookActivationSpec) -> Result<BuiltWebhookHandler, FactoryError>;
}
