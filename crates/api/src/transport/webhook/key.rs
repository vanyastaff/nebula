//! `WebhookKey` — composite key for the converged routing map.
//!
//! Both URL shapes funnel into the same map:
//!
//! - `POST /webhooks/{trigger_uuid}/{nonce}` → [`WebhookKey::Programmatic`]
//! - `POST /api/v1/hooks/{org}/{ws}/{slug}` → [`WebhookKey::Slug`]
//!
//! The transport's `dispatch_inner` looks up by [`WebhookKey`]; rate
//! limiting and metric labels are also keyed on the variant.
//!
//! # Security — nonce redaction
//!
//! The nonce in [`WebhookKey::Programmatic`] is the bearer token for the
//! capability URL.  It is **never** emitted in logs or `Debug` output.
//! [`WebhookKey`] has a manual `Debug` impl that prints only the trigger
//! `uuid` and omits the nonce; `rate_limit_key` returns a token-free bucket
//! label (`rl:{uuid}`) for the same reason.

use uuid::Uuid;

/// Slug tuple identifying a webhook trigger configured via storage
/// rather than minted by the runtime.
///
/// Carried in [`WebhookKey::Slug`] and surfaced in observability so
/// logs and `application/problem+json` responses can echo the failing
/// path without re-parsing the URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TriggerCoordinates {
    /// Org slug from the URL path.
    pub org: String,
    /// Workspace slug from the URL path.
    pub workspace: String,
    /// Trigger slug from the URL path.
    pub trigger: String,
}

impl TriggerCoordinates {
    /// Build a coordinate triple from owned strings.
    #[must_use]
    pub fn new(
        org: impl Into<String>,
        workspace: impl Into<String>,
        trigger: impl Into<String>,
    ) -> Self {
        Self {
            org: org.into(),
            workspace: workspace.into(),
            trigger: trigger.into(),
        }
    }

    /// Render the triple as the human-facing slash-separated path
    /// suffix. Used as the rate-limit bucket key for the slug surface
    /// and as a span field on dispatch.
    #[must_use]
    pub fn path_suffix(&self) -> String {
        format!("{}/{}/{}", self.org, self.workspace, self.trigger)
    }
}

/// Composite routing-map key — programmatic `(uuid, nonce)` minted by
/// the runtime, or slug `(org, workspace, trigger)` configured in
/// storage. Both flow through the same transport `dispatch_inner` and
/// receive the same signature, replay, rate-limit, and metric
/// treatment.
///
/// # Security note
///
/// `Debug` is manually implemented to redact the `nonce` field on the
/// `Programmatic` variant: the nonce is the bearer token embedded in
/// the capability URL and must never appear in log output.
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
    /// Operator-configured activation. The transport loads slug
    /// registrations from the storage layer at startup and on
    /// lifecycle events.
    Slug(TriggerCoordinates),
}

/// Redacting `Debug` for [`WebhookKey`].
///
/// The `nonce` in `Programmatic` is the bearer token embedded in the
/// capability URL — it is excluded from debug output to prevent token
/// leakage into log aggregators and traces.
impl std::fmt::Debug for WebhookKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Programmatic { uuid, .. } => f
                .debug_struct("WebhookKey::Programmatic")
                .field("trigger", uuid)
                .field("nonce", &"<redacted>")
                .finish(),
            Self::Slug(coords) => f.debug_tuple("WebhookKey::Slug").field(coords).finish(),
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

    /// Convenience: build a slug key.
    #[must_use]
    pub fn slug(coords: TriggerCoordinates) -> Self {
        Self::Slug(coords)
    }

    /// Telemetry label — `"programmatic"` or `"slug"`. Cardinality
    /// budget for the `webhook_key_kind` metric label.
    #[must_use]
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Programmatic { .. } => "programmatic",
            Self::Slug(_) => "slug",
        }
    }

    /// Stable string for rate-limit bucket lookup. Includes the kind
    /// prefix so a slug `acme/main/x` and a programmatic uuid that
    /// stringifies similarly cannot collide.
    ///
    /// # Security note
    ///
    /// The `Programmatic` bucket uses only the trigger `uuid` — the nonce
    /// is the bearer token and must not appear in rate-limit logs or
    /// bucket-key strings passed to limiters.
    #[must_use]
    pub fn rate_limit_key(&self) -> String {
        match self {
            // Intentionally exclude nonce — it is the bearer token.
            Self::Programmatic { uuid, .. } => format!("rl:{uuid}"),
            Self::Slug(c) => format!("s:{}", c.path_suffix()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programmatic_and_slug_keys_do_not_collide_in_rate_limit_buckets() {
        let uuid = Uuid::new_v4();
        let prog = WebhookKey::programmatic(uuid, "deadbeef");
        let slug = WebhookKey::slug(TriggerCoordinates::new(uuid.to_string(), "deadbeef", "x"));
        assert_ne!(prog.rate_limit_key(), slug.rate_limit_key());
        assert_eq!(prog.kind_label(), "programmatic");
        assert_eq!(slug.kind_label(), "slug");
    }

    #[test]
    fn coordinates_path_suffix_round_trips() {
        let c = TriggerCoordinates::new("acme", "main", "github");
        assert_eq!(c.path_suffix(), "acme/main/github");
    }

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
