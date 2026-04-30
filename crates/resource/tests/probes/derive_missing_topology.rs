//! Compile-fail probe: `#[derive(Resource)]` requires `topology = "..."`.

use nebula_resource::Resource;

#[derive(Resource)]
#[resource(key = "bad.no_topology", config = MyConfig)]
struct MissingTopology;

#[derive(Clone)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {}

fn main() {}
