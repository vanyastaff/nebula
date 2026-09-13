//! Opaque resource contributions used by SDK-only derive expansions.

use std::{fmt, marker::PhantomData, sync::Arc};

mod private {
    pub(crate) trait Sealed {}
}

/// A resource contribution created from typed authoring contracts.
///
/// This deliberately exposes no registration or manager operations. The SDK
/// keeps the erased runtime factory behind the sealed contribution boundary.
#[expect(
    private_bounds,
    reason = "the private supertrait keeps runtime factory extraction inside the SDK"
)]
pub trait ResourceContribution: private::Sealed + Send + Sync + 'static {
    /// Returns the authored resource key carried by this contribution.
    fn key(&self) -> nebula_core::ResourceKey;
}

/// Typed bridge used by the SDK-only `Resource` derive expansion.
///
/// Authors receive the erased [`ResourceContribution`] token, not this bridge
/// or the runtime factory it owns.
#[must_use = "a resource contribution must be passed to an SDK composition surface"]
pub struct ResourceContributionBridge<R, FResource, FTopology> {
    factory: Arc<dyn nebula_resource::ResourceFactory>,
    resource_marker: PhantomData<fn() -> R>,
    resource_factory_marker: PhantomData<fn() -> FResource>,
    topology_factory_marker: PhantomData<fn() -> FTopology>,
}

impl<R, FResource, FTopology> ResourceContributionBridge<R, FResource, FTopology>
where
    R: nebula_resource::Provider + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    R::Topology: nebula_resource::Topology<R>,
    FResource: Fn() -> R + Send + Sync + 'static,
    FTopology: Fn() -> R::Topology + Send + Sync + 'static,
{
    /// Creates an opaque contribution from typed resource and topology factories.
    pub fn new(resource_factory: FResource, topology_factory: FTopology) -> Self {
        let factory = nebula_resource::KindActivator::<R, FResource, FTopology>::new(
            resource_factory,
            topology_factory,
        );
        Self {
            factory: Arc::new(factory),
            resource_marker: PhantomData,
            resource_factory_marker: PhantomData,
            topology_factory_marker: PhantomData,
        }
    }
}

impl<R, FResource, FTopology> fmt::Debug for ResourceContributionBridge<R, FResource, FTopology> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceContributionBridge")
            .field("key", &self.factory.key())
            .finish_non_exhaustive()
    }
}

impl<R, FResource, FTopology> private::Sealed
    for ResourceContributionBridge<R, FResource, FTopology>
{
}

impl<R, FResource, FTopology> ResourceContribution
    for ResourceContributionBridge<R, FResource, FTopology>
where
    R: 'static,
    FResource: 'static,
    FTopology: 'static,
{
    fn key(&self) -> nebula_core::ResourceKey {
        self.factory.key()
    }
}
