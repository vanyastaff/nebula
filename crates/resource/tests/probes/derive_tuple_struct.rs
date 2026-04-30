//! Compile-fail probe: `#[derive(Resource)]` rejects tuple structs.

use nebula_resource::Resource;

#[derive(Resource)]
#[resource(key = "bad.tuple", topology = "pool", config = MyConfig)]
struct TupleResource(u32);

#[derive(Clone)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {}

fn main() {}
