//! Trigger-dispatch routing seam.
//!
//! [`RoutingResolver`] is a port (DIP seam) the [`DurableExecutionEmitter`]
//! calls to determine which `required_plugin_key` (and flavour SHA) should be
//! attached to each enqueued [`JobDispatchMsg`].  The engine's orchestrator
//! pull-loop claims only rows whose `required_plugin_key` is in the worker's
//! advertised capability set, so the resolver is the single place that maps a
//! `(workflow_id, trigger_id)` pair onto a dispatch route.
//!
//! ## Slice scope
//!
//! This slice ships [`StaticRoutingResolver`] — a single hardcoded mapping
//! used in the integration test harness.  Dynamic flavour-SHA derivation and
//! per-workflow plugin-key lookup are D1 work (follow-up ADR-0095 deliverable).
//!
//! [`DurableExecutionEmitter`]: super::durable_emitter::DurableExecutionEmitter
//! [`JobDispatchMsg`]: nebula_storage_port::dto::JobDispatchMsg

use nebula_core::{NodeKey, id::WorkflowId};
use nebula_storage_port::dto::CapabilityTag;

// ── types ─────────────────────────────────────────────────────────────────────

/// A resolved dispatch route for one `(workflow_id, trigger_id)` pair.
///
/// Returned by [`RoutingResolver::resolve`].  The `required_plugin_key` is
/// written into the `JobDispatchMsg::required_plugin_key` field; the
/// orchestrator claims only rows whose key is in the worker's advertised
/// capability set.
/// `target_flavor_sha` is a version-pin guard written into the message but
/// never used for routing.
#[derive(Debug, Clone)]
pub struct DispatchRoute {
    /// The advertised `PluginKey` string this job requires a worker to support.
    pub required_plugin_key: String,
    /// Full set of capability tags accepted by this job (superset of
    /// `required_plugin_key`).
    pub capability_tags: Vec<CapabilityTag>,
    /// SHA of the plugin flavor this message targets (version-pin guard; not
    /// used for routing).
    pub target_flavor_sha: String,
}

/// Errors returned by [`RoutingResolver::resolve`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RoutingError {
    /// No route could be determined for the given `(workflow_id, trigger_id)`.
    #[error("no route found for workflow `{workflow_id}` trigger `{trigger_id}`")]
    NotFound {
        /// The workflow id that could not be routed.
        workflow_id: String,
        /// The trigger node key that could not be routed.
        trigger_id: String,
    },
}

// ── trait ─────────────────────────────────────────────────────────────────────

/// Port for mapping `(workflow_id, trigger_id)` → [`DispatchRoute`].
///
/// The [`DurableExecutionEmitter`] holds an `Arc<dyn RoutingResolver>` so the
/// routing strategy can be swapped without touching the emitter.  In the slice
/// harness a [`StaticRoutingResolver`] with a single hardcoded route is used;
/// the production implementation (D1) reads the plugin registry.
///
/// [`DurableExecutionEmitter`]: super::durable_emitter::DurableExecutionEmitter
pub trait RoutingResolver: Send + Sync + std::fmt::Debug {
    /// Resolve the dispatch route for a trigger.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::NotFound`] when no route can be determined.
    fn resolve(
        &self,
        workflow_id: &WorkflowId,
        trigger_id: &NodeKey,
    ) -> Result<DispatchRoute, RoutingError>;
}

// ── StaticRoutingResolver ─────────────────────────────────────────────────────

/// Pinned flavor SHA used by the slice harness.
///
/// A single constant avoids a freshly minted random value per call (which
/// would defeat version-pin guards in integration tests).
pub const SLICE_FLAVOR_SHA: &str = "slice-flavor-0000000000000000000000000000000000000001";

/// A single-mapping routing resolver for the trigger-dispatch slice.
///
/// Returns one [`DispatchRoute`] for every `(workflow_id, trigger_id)` pair —
/// the caller supplies the `required_plugin_key` at construction time.  Used
/// by the integration test harness; production wiring is D1.
///
/// # Invariant
///
/// `required_plugin_key` must be non-empty — an empty key matches no worker's
/// advertised capability set, so every dispatch would be permanently
/// un-claimable. Enforced by a `debug_assert!` in [`StaticRoutingResolver::new`]
/// (fast-fail on obvious misuse) **and** a fail-closed runtime check in
/// [`RoutingResolver::resolve`] so the invariant also holds in release builds.
#[derive(Debug, Clone)]
pub struct StaticRoutingResolver {
    required_plugin_key: String,
}

impl StaticRoutingResolver {
    /// Build a resolver that always returns a route with the given plugin key.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when `required_plugin_key` is empty (broken
    /// invariant: an empty routing key would never match a worker's capability
    /// set, making every dispatch permanently un-claimable).
    #[must_use]
    pub fn new(required_plugin_key: impl Into<String>) -> Self {
        let key = required_plugin_key.into();
        debug_assert!(!key.is_empty(), "required_plugin_key must not be empty");
        Self {
            required_plugin_key: key,
        }
    }
}

impl RoutingResolver for StaticRoutingResolver {
    fn resolve(
        &self,
        workflow_id: &WorkflowId,
        trigger_id: &NodeKey,
    ) -> Result<DispatchRoute, RoutingError> {
        // Fail closed in release too: an empty routing key matches no worker's
        // advertised capability set, so the job would be permanently
        // un-claimable. `new`'s debug_assert catches obvious misuse early; this
        // guard upholds the invariant in release builds as well.
        if self.required_plugin_key.is_empty() {
            return Err(RoutingError::NotFound {
                workflow_id: workflow_id.to_string(),
                trigger_id: trigger_id.to_string(),
            });
        }
        Ok(DispatchRoute {
            required_plugin_key: self.required_plugin_key.clone(),
            capability_tags: vec![CapabilityTag::from(self.required_plugin_key.as_str())],
            target_flavor_sha: SLICE_FLAVOR_SHA.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_returns_route_for_nonempty_key() {
        let resolver = StaticRoutingResolver::new("plugin.demo");
        let route = resolver
            .resolve(&WorkflowId::new(), &NodeKey::new("trigger").unwrap())
            .expect("non-empty key resolves to a route");
        assert_eq!(route.required_plugin_key, "plugin.demo");
        assert_eq!(
            route.capability_tags,
            vec![CapabilityTag::from("plugin.demo")]
        );
        assert_eq!(route.target_flavor_sha, SLICE_FLAVOR_SHA);
    }

    #[test]
    fn resolve_fails_closed_on_empty_key() {
        // Build directly to bypass `new`'s debug_assert and exercise the
        // release-path runtime guard in `resolve`.
        let resolver = StaticRoutingResolver {
            required_plugin_key: String::new(),
        };
        let err = resolver
            .resolve(&WorkflowId::new(), &NodeKey::new("trigger").unwrap())
            .expect_err("empty routing key must fail closed");
        assert!(matches!(err, RoutingError::NotFound { .. }));
    }
}
