use nebula_core::ResourceKey;
use nebula_resource::{
    Error, Provider, Resident, ResidentProvider, ResourceConfig, ResourceContext, resource_key,
};

struct LegacyProvider;
nebula_resource::no_credential_slots!(LegacyProvider);

#[derive(Clone, Default, nebula_schema::Schema)]
struct Config;

impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl Provider for LegacyProvider {
    type Config = Config;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("legacy.provider")
    }

    async fn create(&self, _config: &Config, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }

    async fn shutdown(&self, _instance: &()) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> nebula_resource::ResourceMetadataDraft {
        nebula_resource::ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("LegacyProvider"),
            "",
        )
    }
}

impl ResidentProvider for LegacyProvider {}

fn main() {}
