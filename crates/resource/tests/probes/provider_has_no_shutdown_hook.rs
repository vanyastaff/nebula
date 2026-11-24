use nebula_resource::{
    Component, Error, Provider, Resident, ResidentProvider, ResourceConfig, ResourceContext, kind,
};

struct LegacyProvider;
nebula_resource::no_credential_slots!(LegacyProvider);

#[derive(Clone, Default, nebula_schema::Schema, serde::Deserialize)]
struct Config;

impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        0
    }
}

impl Component for LegacyProvider {
    type Kind = kind::Resource;
    type Config = Config;

    const KEY: &'static str = "legacy.provider";

    fn metadata() -> nebula_resource::Metadata {
        nebula_resource::Metadata::new(nebula_resource::metadata_name!("LegacyProvider"), "")
    }
}

#[async_trait::async_trait]
impl Provider for LegacyProvider {
    type Instance = ();
    type Topology = Resident<Self>;

    async fn create(&self, _config: &Config, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }

    async fn shutdown(&self, _instance: &()) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for LegacyProvider {}

fn main() {}
