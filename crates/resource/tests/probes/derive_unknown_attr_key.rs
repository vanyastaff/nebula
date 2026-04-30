//! Compile-fail probe: `#[derive(Resource)]` rejects unknown keys in `#[resource(...)]`.

use nebula_resource::Resource;

#[derive(Resource)]
#[resource(key = "bad.unknown_attr", topology = "pool", config = MyConfig, mystery = "x")]
struct UnknownAttrKey;

#[derive(Clone)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {}

fn main() {}
