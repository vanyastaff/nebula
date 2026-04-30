//! Compile-fail probe: `#[derive(Resource)]` rejects unknown topology values.

use nebula_resource::Resource;

#[derive(Resource)]
#[resource(key = "bad.bad_topology", topology = "wat", config = MyConfig)]
struct BadTopology;

#[derive(Clone)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {}

fn main() {}
