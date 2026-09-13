use nebula_resource::ResourceConfig;

#[derive(Clone, ResourceConfig)]
struct NamedConfig {
    endpoint: String,
}

#[derive(Clone, ResourceConfig)]
struct ScalarConfig(String);

#[derive(Clone, ResourceConfig)]
struct TupleConfig(String, u32);

#[derive(Clone, ResourceConfig)]
struct EmptyTupleConfig();

fn main() {}
