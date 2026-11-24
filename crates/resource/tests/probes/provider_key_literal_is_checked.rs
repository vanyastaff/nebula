use nebula_resource::{
    Error, Metadata, Provider, Resident, ResidentProvider, ResourceContext, metadata_name,
    Component, kind,
};

struct Misnamed;
nebula_resource::no_credential_slots!(Misnamed);

impl Component for Misnamed {
    type Kind = kind::Resource;
    type Config = ();

    const KEY: &'static str = "not a key";

    fn metadata() -> Metadata {
        Metadata::new(metadata_name!("Misnamed"), "")
    }
}

#[async_trait::async_trait]
impl Provider for Misnamed {
    type Instance = ();
    type Topology = Resident<Self>;

    async fn create(&self, _config: &(), _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl ResidentProvider for Misnamed {}

fn main() {}
