//! ResourceAction Output marker — `ResourceProduces<R>`.
//!
//! Per ADR-0043 §4 (Variant A trait shape), the base [`Action`](crate::Action)
//! trait has `type Output` for *what this action produces*. For
//! `ResourceAction`, the produced thing is a scoped resource binding that
//! downstream nodes consume via `ctx.resource::<R>()`. There is no flowing
//! payload, so the Output is a type-marker — `ResourceProduces<R>` carries
//! the resource type identity, the topology tag, and a static schema marker
//! that catalog / UI code consumes to draw scoped-binding edges in workflow
//! graphs.
//!
//! ## Trait constraint usage
//!
//! Phase 3 of `m6-resource-finalization-integration-audit.md` adds the
//! constraint:
//!
//! ```ignore
//! pub trait ResourceAction:
//!     Action<Output = ResourceProduces<<Self as ResourceAction>::Resource>>
//! { ... }
//! ```
//!
//! so the compiler enforces that a ResourceAction's Output is the marker
//! tied to its own `type Resource`. Mismatches surface at impl time, not
//! at runtime.

use std::{fmt, marker::PhantomData};

use nebula_core::ResourceKey;
use nebula_resource::{Resource, topology_tag::TopologyTag};

/// Marker type returned by a [`ResourceAction`](crate::resource::ResourceAction)
/// in its `Action::Output` slot.
///
/// `ResourceProduces<R>` carries the resource type identity (`R::KEY`) and
/// the topology tag describing how downstream nodes will bind to the scoped
/// resource. The marker is zero-sized at runtime apart from a `PhantomData<R>`
/// — it produces no payload, only metadata for catalog / UI consumption.
///
/// # Examples
///
/// ```ignore
/// use nebula_action::ResourceProduces;
/// use nebula_resource::topology_tag::TopologyTag;
///
/// // For a Postgres-pool ResourceAction:
/// // `type Output = ResourceProduces<Postgres>;`
///
/// let marker: ResourceProduces<Postgres> = ResourceProduces::new(TopologyTag::Pool);
/// assert_eq!(marker.topology(), TopologyTag::Pool);
/// ```
pub struct ResourceProduces<R: ?Sized> {
    /// The topology the scoped resource is provided through (Pool, Resident,
    /// Service, Transport, Exclusive). Mirrors the resource's primary topology
    /// trait impl.
    topology: TopologyTag,
    /// Type-level marker for the concrete `Resource` impl. `fn() -> R` so
    /// `ResourceProduces<R>` is `Send + Sync` regardless of `R`.
    _phantom: PhantomData<fn() -> R>,
}

impl<R: ?Sized> ResourceProduces<R> {
    /// Constructs a marker tagged with the given topology.
    ///
    /// Typically invoked by the macro-emitted `ResourceAction` impl —
    /// authors do not write this directly. The macro reads
    /// `R::topology()` (or the topology tag declared on the resource impl)
    /// and emits a `ResourceProduces::new(...)` call.
    #[must_use]
    pub fn new(topology: TopologyTag) -> Self {
        Self {
            topology,
            _phantom: PhantomData,
        }
    }

    /// Returns the topology tag tied to this produced resource.
    #[must_use]
    pub fn topology(&self) -> TopologyTag {
        self.topology
    }
}

impl<R: Resource> ResourceProduces<R> {
    /// Returns the registered resource key for the produced resource type.
    ///
    /// Read at catalog-construction time to label workflow-graph edges with
    /// the resource that downstream nodes will bind via `ctx.resource::<R>()`.
    #[must_use]
    pub fn resource_key() -> ResourceKey {
        R::key()
    }
}

impl<R: ?Sized> Clone for ResourceProduces<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: ?Sized> Copy for ResourceProduces<R> {}

impl<R: ?Sized> fmt::Debug for ResourceProduces<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceProduces")
            .field("topology", &self.topology)
            .field("type", &std::any::type_name::<R>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Phantom resource type for tests — ResourceProduces is a marker,
    // so we don't need a Resource trait impl on FakeRes for the basic
    // construction / accessor tests.
    struct FakeRes;

    #[test]
    fn new_carries_topology() {
        let m: ResourceProduces<FakeRes> = ResourceProduces::new(TopologyTag::Pool);
        assert_eq!(m.topology(), TopologyTag::Pool);
    }

    #[test]
    fn copy_clone_preserves_topology() {
        let m: ResourceProduces<FakeRes> = ResourceProduces::new(TopologyTag::Resident);
        let c = m;
        // Copy semantics — `m` is still usable
        assert_eq!(m.topology(), TopologyTag::Resident);
        assert_eq!(c.topology(), TopologyTag::Resident);
    }

    #[test]
    fn debug_includes_topology_and_type() {
        let m: ResourceProduces<FakeRes> = ResourceProduces::new(TopologyTag::Service);
        let s = format!("{m:?}");
        assert!(s.contains("Service"));
        assert!(s.contains("FakeRes"));
    }

    #[test]
    fn each_topology_variant_constructs() {
        // Smoke test: every TopologyTag variant should be storable in the marker.
        for tag in [
            TopologyTag::Pool,
            TopologyTag::Resident,
            TopologyTag::Service,
            TopologyTag::Transport,
            TopologyTag::Exclusive,
        ] {
            let m: ResourceProduces<FakeRes> = ResourceProduces::new(tag);
            assert_eq!(m.topology(), tag);
        }
    }
}
