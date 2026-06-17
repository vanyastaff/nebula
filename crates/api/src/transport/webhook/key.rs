//! `WebhookKey` — composite key for the webhook routing map.
//!
//! Programmatic `(uuid, nonce)` activations minted by the runtime flow
//! through `POST /{prefix}/{trigger_uuid}/{nonce}`. The slug URL surface
//! (`/api/v1/hooks/{org}/{ws}/{slug}`) was retired in ADR-0096 commit 3;
//! the routing map is now programmatic-only.
//!
//! # Security — nonce redaction
//!
//! The nonce in `WebhookKey` is the bearer token for the capability URL.
//! It is **never** emitted in logs or `Debug` output. [`WebhookKey`] has
//! a manual `Debug` impl that prints only the trigger `uuid` and omits the
//! nonce; `rate_limit_key` returns a token-free bucket label (`rl:{uuid}`)
//! for the same reason.

use uuid::Uuid;

/// Composite routing-map key for a programmatic `(uuid, nonce)` webhook
/// activation minted by the runtime.
///
/// # Security note
///
/// `Debug` is manually implemented to redact the `nonce` field: the nonce
/// is the bearer token embedded in the capability URL and must never appear
/// in log output.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum WebhookKey {
    /// Runtime-minted activation. The transport allocates a fresh
    /// `(uuid, nonce)` pair on each `activate(...)` call and hands
    /// the URL back to the action's `on_activate`.
    Programmatic {
        /// Trigger UUID (path segment 1).
        uuid: Uuid,
        /// 32-char hex nonce (path segment 2). Per-activation random
        /// so stale external hooks pointing at the same UUID cannot
        /// route to a fresh registration.
        nonce: String,
    },
}

/// Redacting `Debug` for [`WebhookKey`].
///
/// The `nonce` is the bearer token embedded in the capability URL — it is
/// excluded from debug output to prevent token leakage into log aggregators
/// and traces.
impl std::fmt::Debug for WebhookKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Programmatic { uuid, .. } => f
                .debug_struct("WebhookKey::Programmatic")
                .field("trigger", uuid)
                .field("nonce", &"<redacted>")
                .finish(),
        }
    }
}

impl WebhookKey {
    /// Convenience: build a programmatic key.
    #[must_use]
    pub fn programmatic(uuid: Uuid, nonce: impl Into<String>) -> Self {
        Self::Programmatic {
            uuid,
            nonce: nonce.into(),
        }
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
            // Intentionally exclude nonce — it is the bearer token.
            Self::Programmatic { uuid, .. } => format!("rl:{uuid}"),
        }
    }

    /// Extract the raw nonce (bearer token) from the key.
    ///
    /// # Security note
    ///
    /// The returned value is the plaintext capability token and must
    /// **never** be emitted in logs or metrics.  This accessor exists
    /// solely for the token-hash computation performed at the API edge
    /// before querying the durable store.
    #[must_use]
    pub(super) fn nonce(&self) -> &str {
        match self {
            Self::Programmatic { nonce, .. } => nonce.as_str(),
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
