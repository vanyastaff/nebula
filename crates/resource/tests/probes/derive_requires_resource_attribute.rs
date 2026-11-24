//! Compile-fail probe: `#[derive(Resource)]` needs `#[resource(key, config)]`,
//! since it writes the `Component` impl.

use nebula_resource::Resource;

#[derive(Resource)]
struct Unnamed;

#[derive(Resource)]
#[resource(key = "probe.no-config")]
struct NoConfig;

#[derive(Resource)]
#[resource(key = "probe.unknown", config = (), kind = "pool")]
struct UnknownKey;

fn main() {}
