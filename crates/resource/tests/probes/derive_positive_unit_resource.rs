//! Compile-pass probe: `#[derive(ResourceSlots)]` + hand-written `impl Resource`
//! accepts a slot-less struct (two-derive pattern). Verifies that:
//!
//! - The derive emits an empty `DeclaresDependencies` impl.
//! - The derive emits `HasCredentialSlots { epoch → 0 }`.
//! - A hand-written `impl Resource` with `create` satisfies the trait.
//! - No `#[resource(...)]` container attribute is required.

use nebula_core::ResourceKey;
use nebula_resource::{Error, Resource, ResourceContext, ResourceSlots, resource_key};

#[derive(Clone, ResourceSlots)]
struct UnitResource;

#[derive(Clone, Default)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {
    fn fingerprint(&self) -> u64 {
        0
    }
}

impl Resource for UnitResource {
    type Config = MyConfig;
    type Runtime = ();

    fn key() -> ResourceKey {
        resource_key!("positive.unit")
    }

    async fn create(&self, _config: &MyConfig, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

fn main() {
    let _ = <UnitResource as Resource>::key();
}
