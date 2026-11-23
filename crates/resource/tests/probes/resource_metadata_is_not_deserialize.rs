use nebula_resource::ResourceDefinition;

fn requires_deserialize<T: serde::de::DeserializeOwned>() {}

fn main() {
    requires_deserialize::<ResourceDefinition>();
}
