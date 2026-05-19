//! Compile-fail probe: a `Capped<N>` bounded resource that does not
//! implement `BoundedRelease` cannot be driven by `BoundedRuntime`.
//!
//! The release hook is type-required for release-bearing caps: the blanket
//! `impl BoundedRelease` covers only `Cap = Unbounded`, so a `Capped`
//! resource that omits its own `impl BoundedRelease` fails the
//! `R: BoundedRelease` bound `BoundedRuntime::new` requires. This makes
//! "tracked lease, no release hook" unrepresentable (it was a silent
//! run-time no-op under the old `TOKEN_MODE ==` discriminant).

use nebula_core::{ResourceKey, resource_key};
use nebula_resource::{
    BoundedConfig, BoundedRuntime, Resource, ResourceConfig, ResourceContext,
    error::Error,
    resource::ResourceMetadata,
    topology::bounded::{Bounded, Capped},
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
struct CappedNoRelease;

impl Resource for CappedNoRelease {
    type Config = Cfg;
    type Runtime = u32;
    type Lease = u32;
    type Error = E;

    fn key() -> ResourceKey {
        resource_key!("probe.capped_no_release")
    }

    async fn create(&self, _c: &Cfg, _x: &ResourceContext) -> Result<u32, E> {
        Ok(1)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Bounded for CappedNoRelease {
    type Cap = Capped<4>;

    async fn acquire_one(&self, runtime: &u32, _x: &ResourceContext) -> Result<u32, E> {
        Ok(*runtime)
    }
}

// No `impl BoundedRelease for CappedNoRelease` — this must fail to compile
// because `BoundedRuntime::new` requires `R: BoundedRelease`.
fn main() {
    let r = CappedNoRelease;
    let _rt: BoundedRuntime<CappedNoRelease> =
        BoundedRuntime::new(&r, 1u32, BoundedConfig::default());
}
