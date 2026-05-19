//! Compile-fail probe: an `Exclusive`-cap bounded resource that does not
//! implement `BoundedRelease` (the reset) cannot be driven by
//! `BoundedRuntime`.
//!
//! Same enforcement as the `Capped` probe: the `Exclusive` cap requires a
//! release hook (the reset). The blanket `impl BoundedRelease` covers only
//! `Cap = Unbounded`, so omitting the reset for an exclusive resource fails
//! the `R: BoundedRelease` bound `BoundedRuntime::new` requires —
//! "exclusive resource, no reset" is unrepresentable.

use nebula_core::{ResourceKey, resource_key};
use nebula_resource::{
    BoundedConfig, BoundedRuntime, Resource, ResourceConfig, ResourceContext,
    error::Error,
    resource::ResourceMetadata,
    topology::bounded::{Bounded, Exclusive as ExclusiveCap},
};

#[derive(Clone)]
struct Cfg;
nebula_resource::impl_empty_has_schema!(Cfg);
impl ResourceConfig for Cfg {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[derive(Debug)]
struct E(String);
impl std::fmt::Display for E {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for E {}
impl From<E> for Error {
    fn from(e: E) -> Self {
        Error::transient(e.0)
    }
}

#[derive(Clone)]
struct ExclusiveNoReset;

impl Resource for ExclusiveNoReset {
    type Config = Cfg;
    type Runtime = u32;
    type Lease = u32;
    type Error = E;

    fn key() -> ResourceKey {
        resource_key!("probe.exclusive_no_reset")
    }

    async fn create(&self, _c: &Cfg, _x: &ResourceContext) -> Result<u32, E> {
        Ok(1)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Bounded for ExclusiveNoReset {
    type Cap = ExclusiveCap;

    async fn acquire_one(&self, runtime: &u32, _x: &ResourceContext) -> Result<u32, E> {
        Ok(*runtime)
    }
}

// No `impl BoundedRelease for ExclusiveNoReset` — this must fail to compile
// because `BoundedRuntime::new` requires `R: BoundedRelease`.
fn main() {
    let r = ExclusiveNoReset;
    let _rt: BoundedRuntime<ExclusiveNoReset> =
        BoundedRuntime::new(&r, 1u32, BoundedConfig::default());
}
