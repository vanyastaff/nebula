//! Compile-pass probe: `#[derive(Resource)]` accepts `topology =
//! "bounded"` (the folded topology) and still accepts the legacy
//! `service` / `transport` / `exclusive` strings while consumers migrate
//! onto `bounded`. The emitted `RESOURCE_TOPOLOGY` const resolves to the
//! matching `TopologyTag` for each.

use nebula_resource::{Resource, TopologyTag};

#[derive(Clone)]
struct MyConfig;
nebula_schema::impl_empty_has_schema!(MyConfig);
impl nebula_resource::resource::ResourceConfig for MyConfig {}

#[derive(Resource)]
#[resource(key = "accepted.bounded", topology = "bounded", config = MyConfig)]
struct BoundedRes;

#[derive(Resource)]
#[resource(key = "accepted.service", topology = "service", config = MyConfig)]
struct ServiceRes;

#[derive(Resource)]
#[resource(key = "accepted.transport", topology = "transport", config = MyConfig)]
struct TransportRes;

#[derive(Resource)]
#[resource(key = "accepted.exclusive", topology = "exclusive", config = MyConfig)]
struct ExclusiveRes;

fn main() {
    // Each accepted string maps to its `TopologyTag` via the emitted
    // informational const.
    assert_eq!(BoundedRes::RESOURCE_TOPOLOGY, TopologyTag::Bounded);
    assert_eq!(ServiceRes::RESOURCE_TOPOLOGY, TopologyTag::Service);
    assert_eq!(TransportRes::RESOURCE_TOPOLOGY, TopologyTag::Transport);
    assert_eq!(ExclusiveRes::RESOURCE_TOPOLOGY, TopologyTag::Exclusive);
    let _ = <BoundedRes as Resource>::key();
}
