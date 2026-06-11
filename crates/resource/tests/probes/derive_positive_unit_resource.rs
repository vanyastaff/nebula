//! Compile-pass probe: `#[derive(Resource)]` + hand-written `impl Provider`
//! accepts a slot-less struct (two-derive pattern). Verifies that:
//!
//! - The derive emits an empty `DeclaresDependencies` impl.
//! - The derive emits `HasCredentialSlots { epoch → 0 }`.
//! - A hand-written `impl Provider` with `create` satisfies the trait.
//! - No `#[resource(...)]` container attribute is required.

use nebula_core::ResourceKey;
use nebula_resource::topology::resident::ResidentProvider;
use nebula_resource::{
    Error, Resident, Resource, ResourceContext, resource::Provider, resource_key,
};

#[derive(Clone, Resource)]
struct UnitResource;

#[derive(Clone, Default)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl Provider for UnitResource {
    type Config = MyConfig;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("positive.unit")
    }

    async fn create(&self, _config: &MyConfig, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for UnitResource {}

fn main() {
    let _ = <UnitResource as Provider>::key();
}
