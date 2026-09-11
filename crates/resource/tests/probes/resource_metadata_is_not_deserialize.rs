use nebula_resource::ResourceMetadata;

fn requires_deserialize<T: serde::de::DeserializeOwned>() {}

fn main() {
    requires_deserialize::<ResourceMetadata>();
}
