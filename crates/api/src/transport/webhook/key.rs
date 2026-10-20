//! `WebhookKey` — composite key for the webhook routing map.
//!
//! Programmatic `(uuid, nonce)` activations minted by the runtime flow
//! through `POST /{prefix}/{trigger_uuid}/{nonce}`. The slug URL surface
//! (`/api/v1/hooks/{org}/{ws}/{slug}`) was retired in ADR-0096 commit 3;
//! the routing map is now programmatic-only.
//!
//! # Security — the key holds the token hash, not the nonce
//!
//! The nonce in the URL is the bearer token for the capability URL. The key
//! stores only its SHA-256 hash, the same value `webhook_activations` stores,
//! so an activation restored from the durable row routes exactly like the one
//! minted in this process, and the plaintext token is never kept in the map.
//! [`WebhookKey`] has a manual `Debug` impl that prints only the trigger
//! `uuid`; `rate_limit_key` returns a token-free bucket label (`rl:{uuid}`).

use uuid::Uuid;

use super::token::token_hash;

/// Composite routing-map key for a programmatic `(uuid, nonce)` webhook
/// activation minted by the runtime.
///
/// # Security note
///
/// `Debug` is manually implemented to redact the token hash: it identifies
/// the bearer token embedded in the capability URL and has no place in logs.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum WebhookKey {
    /// Runtime-minted activation. The transport allocates a fresh
    /// `(uuid, nonce)` pair on each `activate(...)` call and hands
    /// the URL back to the action's `on_activate`.
    Programmatic {
        /// Trigger UUID (path segment 1).
        uuid: Uuid,
        /// SHA-256 of the 32-char hex nonce (path segment 2). The nonce is
        /// per-activation random so stale external hooks pointing at the
        /// same UUID cannot route to a fresh registration.
        token_hash: [u8; 32],
    },
}

/// Redacting `Debug` for [`WebhookKey`].
///
/// The token hash identifies the bearer token embedded in the capability
/// URL — it is excluded from debug output so log aggregators and traces never
/// carry it.
impl std::fmt::Debug for WebhookKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Programmatic { uuid, .. } => f
                .debug_struct("WebhookKey::Programmatic")
                .field("trigger", uuid)
                .field("token", &"<redacted>")
                .finish(),
        }
    }
}

impl WebhookKey {
    /// Build the key for a request or activation from the plaintext nonce
    /// in its URL; only the nonce's hash is kept.
    #[must_use]
    pub fn programmatic(uuid: Uuid, nonce: impl AsRef<str>) -> Self {
        Self::from_token_hash(uuid, token_hash(nonce.as_ref()))
    }

    /// Build the key of a persisted activation from its stored token hash.
    #[must_use]
    pub fn from_token_hash(uuid: Uuid, token_hash: [u8; 32]) -> Self {
        Self::Programmatic { uuid, token_hash }
    }

    /// Telemetry label — always `"programmatic"` for the sole remaining
    /// variant. Cardinality budget for the `webhook_key_kind` metric label.
    #[must_use]
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Programmatic { .. } => "programmatic",
        }
    }

    /// Stable string for rate-limit bucket lookup.
    ///
    /// # Security note
    ///
    /// Uses only the trigger `uuid` — the nonce is the bearer token and
    /// must not appear in rate-limit logs or bucket-key strings passed to
    /// limiters.
    #[must_use]
    pub fn rate_limit_key(&self) -> String {
        match self {
            // Intentionally exclude the token — it is the bearer credential.
            Self::Programmatic { uuid, .. } => format!("rl:{uuid}"),
        }
    }

    /// The SHA-256 of the bearer token, as `webhook_activations` stores it.
    ///
    /// # Security note
    ///
    /// The hash identifies the capability token and must **never** be
    /// emitted in logs or metrics; it exists for the durable-store lookup.
    #[must_use]
    pub(super) fn token_hash(&self) -> &[u8; 32] {
        match self {
            Self::Programmatic { token_hash, .. } => token_hash,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Security regression: the nonce (bearer token) must never appear in the
    /// `Debug` representation of `WebhookKey::Programmatic`.  A log aggregator
    /// that captures debug output must not inadvertently record a live bearer
    /// token.
    #[test]
    fn programmatic_debug_does_not_contain_nonce() {
        let uuid = Uuid::new_v4();
        let nonce = "super-secret-nonce-token-do-not-log";
        let key = WebhookKey::programmatic(uuid, nonce);
        let debug_output = format!("{key:?}");
        assert!(
            !debug_output.contains(nonce),
            "Debug output must not contain the nonce (bearer token). Got: {debug_output}"
        );
        assert!(
            debug_output.contains("redacted"),
            "Debug output must contain a redaction marker. Got: {debug_output}"
        );
    }

    /// A request key built from the URL nonce equals the key of the persisted
    /// activation built from the stored hash, so restored routes match.
    #[test]
    fn programmatic_key_matches_the_stored_token_hash() {
        let uuid = Uuid::new_v4();
        let nonce = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            WebhookKey::programmatic(uuid, nonce),
            WebhookKey::from_token_hash(uuid, token_hash(nonce))
        );
        assert_ne!(
            WebhookKey::programmatic(uuid, nonce),
            WebhookKey::programmatic(uuid, "another-nonce")
        );
    }

    /// Security regression: the rate-limit bucket key must not embed the
    /// nonce so that limiter telemetry/logs cannot expose the bearer token.
    #[test]
    fn programmatic_rate_limit_key_does_not_contain_nonce() {
        let uuid = Uuid::new_v4();
        let nonce = "super-secret-nonce-token-do-not-log";
        let key = WebhookKey::programmatic(uuid, nonce);
        let bucket = key.rate_limit_key();
        assert!(
            !bucket.contains(nonce),
            "rate_limit_key must not contain the nonce (bearer token). Got: {bucket}"
        );
        // Must still contain the trigger uuid so different activations get
        // independent buckets.
        assert!(
            bucket.contains(&uuid.to_string()),
            "rate_limit_key must still contain the trigger uuid for per-activation bucketing"
        );
    }
}
