//! `WebhookKey` — composite key for the converged routing map.
//!
//! Both URL shapes funnel into the same map:
//!
//! - `POST /webhooks/{trigger_uuid}/{nonce}` → [`WebhookKey::Programmatic`]
//! - `POST /api/v1/hooks/{org}/{ws}/{slug}` → [`WebhookKey::Slug`]
//!
//! The transport's `dispatch_inner` looks up by [`WebhookKey`]; rate
//! limiting and metric labels are also keyed on the variant.

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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    #[must_use]
    pub fn rate_limit_key(&self) -> String {
        match self {
            Self::Programmatic { uuid, nonce } => format!("p:{uuid}/{nonce}"),
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
}
