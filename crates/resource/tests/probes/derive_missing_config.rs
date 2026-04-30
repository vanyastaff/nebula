//! Compile-fail probe: `#[derive(Resource)]` requires `config = SomeType`.

use nebula_resource::Resource;

#[derive(Resource)]
#[resource(key = "bad.no_config", topology = "pool")]
struct MissingConfig;

fn main() {}
