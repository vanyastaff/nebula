use nebula_resource::{
    Error, MetadataDraft, Provider, Resident, ResidentProvider, ResourceContext, metadata_name,
};

struct Misnamed;
nebula_resource::no_credential_slots!(Misnamed);

#[async_trait::async_trait]
impl Provider for Misnamed {
    type Config = ();
    type Instance = ();
    type Topology = Resident<Self>;

    const KEY: &'static str = "not a key";

    fn metadata() -> MetadataDraft {
        MetadataDraft::new(metadata_name!("Misnamed"), "")
    }

    async fn create(&self, _config: &(), _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for Misnamed {}

fn main() {}
