//! Compile-pass probe: `#[derive(Resource)]` + hand-written `impl Provider`
//! accepts a slot-less struct (two-derive pattern). Verifies that:
//!
//! - The derive emits `Component` from `#[resource(key, config)]`, with no
//!   slots.
//! - The derive emits `HasCredentialSlots { epoch → 0 }`.
//! - A hand-written `impl Provider` with `create` satisfies the trait.

use nebula_resource::topology::resident::ResidentProvider;
use nebula_resource::{Component, Error, Resident, Resource, ResourceContext, resource::Provider};

#[derive(Clone, Resource)]
#[resource(key = "positive.unit", config = MyConfig)]
struct UnitResource;

#[derive(Clone, Default, nebula_schema::Schema, serde::Deserialize)]
struct MyConfig;

impl nebula_resource::resource::ResourceConfig for MyConfig {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl Provider for UnitResource {
    type Instance = ();
    type Topology = Resident<Self>;

    async fn create(&self, _config: &MyConfig, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for UnitResource {}

fn main() {
    let _ = nebula_resource::key_of::<UnitResource>();
    assert!(UnitResource::dependencies().is_empty());
    // A slot-less resource pins nothing for the managed call facade.
    let (): <UnitResource as nebula_resource::PinSlots>::Pinned =
        nebula_resource::PinSlots::pin_slots(&UnitResource);
}
