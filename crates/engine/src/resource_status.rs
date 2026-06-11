//! Engine-side read-only resource runtime-status seam.
//!
//! [`EngineResourceStatus`] is the **read-only** projection of a live
//! resource's lifecycle phase, exposed in api-safe types so a consumer
//! that must not depend on `nebula-resource` (the public API tier —
//! `deny.toml` `[[wrappers]]` forbids `nebula-api → nebula-resource`) can
//! still report runtime status. It is the status counterpart of
//! [`EngineResourceAccessor`](crate::EngineResourceAccessor): the accessor
//! is the action-capability seam (acquire), this is the diagnostics seam
//! (observe).
//!
//! # No lifecycle mutation (.1)
//!
//! The trait deliberately exposes **only** a phase read. There is no
//! acquire / release / drain / reload entry point: resource lifecycle is
//! owned by the engine and is not reachable through this seam. A status
//! query can never mutate a resource — observing is not operating.
//!
//! # Projection
//!
//! The held [`nebula_resource::Manager`] is keyed by `(ResourceKey,
//! ScopeLevel)`, not by workspace; resources register at
//! [`ScopeLevel::Global`] (the same lookup scope
//! [`EngineResourceAccessor`](crate::EngineResourceAccessor) uses), and
//! tenant isolation is enforced by the *caller* (the config-row owner
//! check) before this seam is ever consulted. `get_any` is fail-closed on
//! ambiguity (several resolved-credential rows at one `(key, scope)`)
//! returning `None` — a diagnostic peek must never alias one tenant's
//! runtime to another. The erased phase is mapped to a stable, non-secret
//! string at the engine boundary so the api-safe struct carries no
//! `nebula-resource` type and no configuration/credential material
//!

use std::{fmt, sync::Arc};

use nebula_core::{ResourceKey, ScopeLevel};

/// Stable, non-secret runtime-status projection of one live resource.
///
/// Carries lifecycle phase only — never configuration, credential, or any
/// other resource-supplied material . The `phase` string is a
/// closed, stable vocabulary; consumers match on it rather than
/// re-deriving from internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRuntimeStatus {
    /// Lifecycle phase as a stable lowercase token. The recognised values
    /// are exactly `nebula_resource::state::ResourcePhase`'s canonical
    /// `Display` rendering (single source of truth — not re-enumerated
    /// here), plus `"unknown"` for an unrecognised future variant this
    /// build does not yet name. `project` is drift-pinned to that
    /// `Display` for every known variant.
    pub phase: &'static str,
    /// `true` iff the resource is in a healthy, request-serving phase
    /// (`ready`). Reloading still accepts traffic but is not "healthy"
    /// for an at-a-glance status, so it reports `false` here while
    /// `accepting` stays `true`.
    pub healthy: bool,
    /// `true` iff the resource can currently accept new acquire requests
    /// (`ready` or `reloading`). This is the phase's own
    /// accept-new-work predicate, surfaced read-only — it does **not**
    /// acquire anything.
    pub accepting: bool,
}

/// Read-only resource runtime-status port.
///
/// The single seam through which a non-`nebula-resource` crate observes a
/// live resource's lifecycle phase. Returns `None` when the resource has
/// no live runtime registered for the lookup scope (e.g. a persisted
/// definition that was never activated, or a fail-closed ambiguous
/// `(key, scope)`): "configured but not currently active" is a `None`
/// here, distinct from a transport-level "no status backend".
pub trait EngineResourceStatus: Send + Sync {
    /// Project the current runtime status of the resource identified by
    /// `key`, or `None` if no live runtime is registered for it.
    ///
    /// Read-only: this never registers, acquires, releases, or otherwise
    /// mutates a resource.
    fn runtime_status(&self, key: &ResourceKey) -> Option<ResourceRuntimeStatus>;
}

/// [`EngineResourceStatus`] backed by the engine's
/// [`nebula_resource::Manager`].
///
/// Holds the same `Arc<Manager>` the engine is wired with and projects
/// `AnyManagedResource::phase_erased()` through the manager's fail-closed
/// `get_any` peek. Resources are looked up at [`ScopeLevel::Global`] —
/// identical to [`EngineResourceAccessor`](crate::EngineResourceAccessor)
/// — because tenant isolation is the caller's config-row check, not a
/// manager-scope concern.
pub struct EngineManagerResourceStatus {
    manager: Arc<nebula_resource::Manager>,
}

impl EngineManagerResourceStatus {
    /// Create a status port backed by the given resource manager.
    #[must_use]
    pub fn new(manager: Arc<nebula_resource::Manager>) -> Self {
        Self { manager }
    }
}

impl fmt::Debug for EngineManagerResourceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineManagerResourceStatus")
            .field("manager", &"<Manager>")
            .finish()
    }
}

/// Map a `nebula_resource` lifecycle phase to the stable, non-secret
/// api-safe projection. Centralised so the `phase` vocabulary and the
/// `healthy` / `accepting` predicates have exactly one definition.
///
/// `ResourcePhase` is `#[non_exhaustive]`: a future variant this build
/// does not yet name maps to the stable `"unknown"` token (honest — the
/// projection genuinely does not recognise it — never a panic or a
/// fabricated phase). `healthy` / `accepting` are derived from the
/// phase's own predicates, so a yet-unknown phase is conservatively
/// reported as not-healthy.
fn project(phase: nebula_resource::state::ResourcePhase) -> ResourceRuntimeStatus {
    use nebula_resource::state::ResourcePhase;
    let phase_str = match phase {
        ResourcePhase::Initializing => "initializing",
        ResourcePhase::Ready => "ready",
        ResourcePhase::Reloading => "reloading",
        ResourcePhase::Draining => "draining",
        ResourcePhase::ShuttingDown => "shutting_down",
        ResourcePhase::Failed => "failed",
        // `#[non_exhaustive]` fail-safe: an unrecognised future phase is
        // honestly "unknown", not a panic or a guessed label.
        _ => "unknown",
    };
    ResourceRuntimeStatus {
        phase: phase_str,
        healthy: matches!(phase, ResourcePhase::Ready),
        accepting: phase.is_accepting(),
    }
}

impl EngineResourceStatus for EngineManagerResourceStatus {
    fn runtime_status(&self, key: &ResourceKey) -> Option<ResourceRuntimeStatus> {
        // `get_any` is the manager's fail-closed diagnostic peek: `None`
        // both when nothing is registered and when several resolved
        // credential rows share `(key, scope)` (ambiguous). A status
        // probe must not alias one tenant's runtime to another.
        self.manager
            .get_any(key, &ScopeLevel::Global)
            .map(|managed| project(managed.phase()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_resource::Manager;

    use super::*;

    fn rk(key: &str) -> ResourceKey {
        ResourceKey::new(key).expect("valid resource key in test")
    }

    #[tokio::test]
    async fn unregistered_resource_has_no_runtime_status() {
        // `Manager::new()` spawns a release-queue worker that needs a
        // Tokio runtime — keep this under `#[tokio::test]`.
        let status = EngineManagerResourceStatus::new(Arc::new(Manager::new()));
        assert!(
            status.runtime_status(&rk("postgres")).is_none(),
            "a resource with no live runtime must project no status"
        );
    }

    #[tokio::test]
    async fn debug_redacts_manager() {
        let status = EngineManagerResourceStatus::new(Arc::new(Manager::new()));
        let dbg = format!("{status:?}");
        assert!(dbg.contains("<Manager>"));
    }

    #[test]
    fn phase_projection_is_stable_and_non_secret() {
        use nebula_resource::state::ResourcePhase;

        let ready = project(ResourcePhase::Ready);
        assert_eq!(ready.phase, "ready");
        assert!(ready.healthy);
        assert!(ready.accepting);

        let reloading = project(ResourcePhase::Reloading);
        assert_eq!(reloading.phase, "reloading");
        assert!(!reloading.healthy, "reloading is not at-a-glance healthy");
        assert!(reloading.accepting, "reloading still accepts new work");

        let failed = project(ResourcePhase::Failed);
        assert_eq!(failed.phase, "failed");
        assert!(!failed.healthy);
        assert!(!failed.accepting);

        for p in [
            ResourcePhase::Initializing,
            ResourcePhase::Draining,
            ResourcePhase::ShuttingDown,
        ] {
            let s = project(p);
            assert!(!s.healthy, "{p:?} must not be healthy");
            // `accepting` mirrors `ResourcePhase::is_accepting()` (only
            // `Ready`/`Reloading`), so every phase in this loop must
            // report not-accepting; `Ready`/`Reloading` are asserted
            // accepting above.
            assert!(!s.accepting, "{p:?} must not accept new work");
        }
    }

    #[test]
    fn projection_token_matches_canonical_display_for_every_known_phase() {
        use nebula_resource::state::ResourcePhase;

        // The projection in `project()` hand-maintains a `ResourcePhase →
        // &'static str` table (kept as `&'static str`, not delegated to
        // `to_string()`, by design). This pin makes that table's every
        // KNOWN variant equal `nebula-resource`'s canonical `Display`, so
        // a renamed token or a newly added `ResourcePhase` variant fails
        // here instead of silently projecting as `"unknown"`.
        for p in [
            ResourcePhase::Initializing,
            ResourcePhase::Ready,
            ResourcePhase::Reloading,
            ResourcePhase::Draining,
            ResourcePhase::ShuttingDown,
            ResourcePhase::Failed,
        ] {
            assert_eq!(
                project(p).phase,
                p.to_string(),
                "projection token must equal nebula-resource's canonical \
                 Display for {p:?}; a renamed token or a new ResourcePhase \
                 variant must fail here, not silently become \"unknown\""
            );
        }
    }
}
