//! Topology runtime implementations.
//!
//! Each topology trait ([`Pooled`], [`Resident`], [`Bounded`]) has a
//! corresponding runtime struct that manages instance lifecycle, and a
//! dispatch enum ([`TopologyRuntime`]) that erases the topology at the
//! registration level.
//!
//! [`Bounded`] is the single parameterized runtime for capped short-lived
//! leases: its [`Cap`](crate::topology::bounded::Bounded::Cap) typestate
//! (`Unbounded` / `Capped<N>` / `Exclusive`) selects the semaphore arity and
//! release shape, so one runtime covers the long-lived-runtime /
//! multiplexed-session / one-caller-at-a-time access patterns.
//!
//! [`Pooled`]: crate::topology::pooled::Pooled
//! [`Resident`]: crate::topology::resident::Resident
//! [`Bounded`]: crate::topology::bounded::Bounded

pub mod bounded;
pub mod managed;
pub mod pool;
pub mod resident;

use crate::{resource::Resource, topology_tag::TopologyTag};

/// Dispatch enum for all topology runtimes.
///
/// Each variant holds the runtime state for a specific topology. The
/// engine stores one `TopologyRuntime<R>` per registered resource,
/// inside [`ManagedResource`](managed::ManagedResource).
pub enum TopologyRuntime<R: Resource> {
    /// Pool of N interchangeable instances with checkout/recycle.
    Pool(pool::PoolRuntime<R>),
    /// Single shared instance, clone on acquire.
    Resident(resident::ResidentRuntime<R>),
    /// One long-lived runtime handing out capped short-lived leases.
    ///
    /// The [`Cap`](crate::topology::bounded::Bounded::Cap) typestate
    /// (`Unbounded` / `Capped<N>` / `Exclusive`) selects the semaphore
    /// arity and release shape, so this one variant covers the
    /// long-lived-runtime, multiplexed-session, and one-caller-at-a-time
    /// access patterns.
    Bounded(bounded::BoundedRuntime<R>),
}

impl<R: Resource> TopologyRuntime<R> {
    /// Returns the topology tag for this runtime variant.
    pub fn tag(&self) -> TopologyTag {
        match self {
            Self::Pool(_) => TopologyTag::Pool,
            Self::Resident(_) => TopologyTag::Resident,
            Self::Bounded(_) => TopologyTag::Bounded,
        }
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{ResourceKey, resource_key};

    use super::*;
    use crate::{
        context::ResourceContext,
        error::Error,
        resource::{ResourceConfig, ResourceMetadata},
        topology::bounded::{Bounded, Unbounded, config::Config as BoundedConfig},
    };

    #[derive(Clone)]
    struct TagFixture;

    /// Dedicated config/error newtypes — sibling test modules in this
    /// crate already own `String`-based `ResourceConfig` / `From<_> for
    /// Error` impls, and lib-test builds compile every `#[cfg(test)]`
    /// module together, so this fixture must not reuse those types.
    #[derive(Clone)]
    struct TagCfg;

    nebula_schema::impl_empty_has_schema!(TagCfg);

    impl ResourceConfig for TagCfg {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TagErr(String);

    impl std::fmt::Display for TagErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for TagErr {}

    impl From<TagErr> for Error {
        fn from(e: TagErr) -> Self {
            Error::transient(e.0)
        }
    }

    impl Resource for TagFixture {
        type Config = TagCfg;
        type Runtime = String;
        type Lease = String;
        type Error = TagErr;

        fn key() -> ResourceKey {
            resource_key!("runtime-tag-fixture")
        }

        async fn create(&self, _config: &TagCfg, _ctx: &ResourceContext) -> Result<String, TagErr> {
            Ok("runtime".into())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    // `Cap = Unbounded` ⇒ the blanket `BoundedRelease` no-op applies, so
    // no release boilerplate is needed for this fixture.
    impl Bounded for TagFixture {
        type Cap = Unbounded;

        async fn acquire_one(
            &self,
            runtime: &String,
            _ctx: &ResourceContext,
        ) -> Result<String, TagErr> {
            Ok(format!("{runtime}-lease"))
        }
    }

    /// `tag()` is total over every variant and the new `Bounded` variant
    /// reports `TopologyTag::Bounded`. Constructing the variant also
    /// proves `TopologyRuntime::Bounded(BoundedRuntime<R>)` type-checks
    /// (the 3-arg `BoundedRuntime::new` used by the migration units).
    #[test]
    fn bounded_variant_tags_as_bounded() {
        let rt = bounded::BoundedRuntime::<TagFixture>::new(
            &TagFixture,
            "runtime".to_owned(),
            BoundedConfig::default(),
        );
        let topo: TopologyRuntime<TagFixture> = TopologyRuntime::Bounded(rt);
        assert_eq!(topo.tag(), TopologyTag::Bounded);
        // Round-trips through the tag string set kept in lockstep with
        // the enum.
        assert_eq!(topo.tag().as_str(), "bounded");
    }
}
