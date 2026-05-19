//! Compile-pass probe: `#[derive(Resource)]` accepts `topology =
//! "bounded"` (the folded topology that replaced the legacy
//! `service` / `transport` / `exclusive` strings). The emitted
//! `RESOURCE_TOPOLOGY` const resolves to the matching `TopologyTag`, and
//! the tag enum is exactly the collapsed `Pool` / `Resident` / `Bounded`
//! set.

use nebula_resource::{Resource, TopologyTag};

#[derive(Clone)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {}

#[derive(Resource)]
#[resource(key = "accepted.pool", topology = "pool", config = MyConfig)]
struct PoolRes;

#[derive(Resource)]
#[resource(key = "accepted.resident", topology = "resident", config = MyConfig)]
struct ResidentRes;

#[derive(Resource)]
#[resource(key = "accepted.bounded", topology = "bounded", config = MyConfig)]
struct BoundedRes;

fn main() {
    // Each accepted string maps to its `TopologyTag` via the emitted
    // informational const — the collapsed 3-tag set, no legacy
    // service/transport/exclusive aliases.
    assert_eq!(PoolRes::RESOURCE_TOPOLOGY, TopologyTag::Pool);
    assert_eq!(ResidentRes::RESOURCE_TOPOLOGY, TopologyTag::Resident);
    assert_eq!(BoundedRes::RESOURCE_TOPOLOGY, TopologyTag::Bounded);
    assert_eq!(BoundedRes::RESOURCE_TOPOLOGY.as_str(), "bounded");
    let _ = <BoundedRes as Resource>::key();
}
