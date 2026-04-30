//! Compile-pass probe: `#[derive(Resource)]` accepts a unit struct with all
//! required attributes (key + topology + config). The `create` body emitted
//! by the macro is `todo!()` — the implementor would supply it; the macro
//! itself only needs to expand cleanly without errors.

use nebula_resource::Resource;

#[derive(Clone)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {}

#[derive(Resource)]
#[resource(
    key = "positive.unit",
    topology = "pool",
    config = MyConfig,
)]
struct UnitResource;

fn main() {
    // Confirm the macro emitted a Resource impl that resolves to the trait.
    let _ = <UnitResource as Resource>::key();
}
