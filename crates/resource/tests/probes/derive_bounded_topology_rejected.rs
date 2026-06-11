//! Compile-fail probe: `#[derive(Resource)]` rejects `topology = "bounded"`
//! now that the Bounded topology has been removed. The accepted set is
//! `pool` / `resident` only.

use nebula_resource::Resource;

#[derive(Clone)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {}

#[derive(Resource)]
#[resource(key = "probe.bounded_rejected", topology = "bounded", config = MyConfig)]
struct BoundedRes;

fn main() {}
