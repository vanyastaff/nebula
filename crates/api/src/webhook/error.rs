//! Typed errors for slug-routed webhook dispatch.
//!
//! The dispatcher splits failure into three disjoint categories so the
//! HTTP handler can map each to the right status code without inferring
//! intent from a string:
//!
//! - [`WebhookAuthError`] — authentication / signature problems (401).
//! - [`WebhookEnqueueError`] — sink rejected the event (500/503).
//! - [`WebhookDispatchError`] — top-level error returned by the dispatcher; composes the two above
//!   plus a routing-miss variant.
//!
//! Per `feedback_observability_as_completion.md`: every variant is a
//! typed `thiserror` carrying enough context for a recovery decision.

use thiserror::Error;

/// Reasons a webhook request fails per-trigger authentication.
///
/// Mirrors [`nebula_action::SignatureOutcome`] semantics for HMAC paths
/// and adds policy-misconfiguration cases that are specific to the
/// slug-routed surface (where the auth config is read from a registered
/// trigger row rather than from a typed `WebhookAction`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WebhookAuthError {
    /// Required signature header was not present on the request.
    #[error("webhook signature header missing")]
    SignatureMissing,

    /// Signature header was present but did not match the computed HMAC
    /// over the request body.
    #[error("webhook signature invalid")]
    SignatureInvalid,

    /// Trigger declared a signed policy but no secret is configured for
    /// the registration. Treated as operator misconfiguration (5xx) by
    /// the handler — the caller did nothing wrong.
    #[error("webhook signature secret not configured for trigger")]
    SecretNotConfigured,

    /// Trigger requires an authentication token (e.g. shared bearer)
    /// that was not provided.
    #[error("webhook authentication token missing")]
    TokenMissing,

    /// Authentication token was provided but did not match the
    /// configured value.
    #[error("webhook authentication token invalid")]
    TokenInvalid,
}

/// Reasons the dispatcher could not hand the event to the engine sink.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WebhookEnqueueError {
    /// The configured sink rejected the event because its receiver has
    /// closed or its bounded queue is saturated. Permanent for this
    /// registration — the engine side has likely shut down.
    #[error("trigger event sink unavailable for trigger {trigger:?}")]
    SinkUnavailable {
        /// Slug-tuple identifying the failed trigger, for diagnostics.
        trigger: TriggerCoordinates,
    },

    /// The sink is reachable but is back-pressuring on a transient
    /// timeout / capacity bound. Caller should treat as 503.
    #[error("trigger event sink temporarily unavailable for trigger {trigger:?}")]
    SinkBackpressure {
        /// Slug-tuple identifying the failed trigger.
        trigger: TriggerCoordinates,
    },
}

/// Top-level error returned by [`crate::webhook::WebhookDispatcher::dispatch`].
///
/// Wraps the auth / enqueue variants and adds a routing-miss case for
/// requests that landed on the slug surface but no active registration
/// exists. The HTTP handler maps each variant to its status code.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WebhookDispatchError {
    /// No active webhook trigger is registered for the given slug
    /// tuple.
    #[error("no webhook registered for trigger {trigger:?}")]
    NotFound {
        /// Slug-tuple from the request path.
        trigger: TriggerCoordinates,
    },

    /// Per-trigger authentication failed.
    #[error(transparent)]
    Auth(#[from] WebhookAuthError),

    /// Engine sink failed.
    #[error(transparent)]
    Enqueue(#[from] WebhookEnqueueError),
}

/// Slug tuple identifying a webhook trigger for the slug-routed
/// dispatcher. Carried in errors so logs and problem-details can
/// surface the failing path without parsing the URL again.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_errors_are_distinguishable_via_pattern_match() {
        let missing = WebhookAuthError::SignatureMissing;
        let invalid = WebhookAuthError::SignatureInvalid;
        assert_ne!(missing, invalid);
        assert!(matches!(missing, WebhookAuthError::SignatureMissing));
        assert!(matches!(invalid, WebhookAuthError::SignatureInvalid));
    }

    #[test]
    fn dispatch_error_composes_auth_via_from() {
        let err: WebhookDispatchError = WebhookAuthError::SignatureInvalid.into();
        assert!(matches!(
            err,
            WebhookDispatchError::Auth(WebhookAuthError::SignatureInvalid)
        ));
    }

    #[test]
    fn dispatch_error_composes_enqueue_via_from() {
        let err: WebhookDispatchError = WebhookEnqueueError::SinkUnavailable {
            trigger: TriggerCoordinates::new("acme", "main", "github"),
        }
        .into();
        assert!(matches!(err, WebhookDispatchError::Enqueue(_)));
    }

    #[test]
    fn coordinates_round_trip_through_into_from_str_and_owned() {
        let owned = TriggerCoordinates::new("acme", "main", "gh");
        let from_str =
            TriggerCoordinates::new("acme".to_string(), "main".to_string(), "gh".to_string());
        assert_eq!(owned, from_str);
    }
}
