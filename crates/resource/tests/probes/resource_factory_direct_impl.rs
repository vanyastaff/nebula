use std::any::TypeId;

use nebula_core::{Dependencies, ResourceKey};
use nebula_resource::{
    Error, Manager, MetadataBuildError, ResourceFactory, ResourceMetadata, SlotIdentity,
    factory::{BoxFut, RegisterRequest},
};

struct ForgedFactory;

impl ResourceFactory for ForgedFactory {
    fn key(&self) -> ResourceKey {
        panic!("compile-only probe")
    }

    fn dependencies(&self) -> &Dependencies {
        panic!("compile-only probe")
    }

    fn resource_type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    fn metadata(&self) -> Result<&ResourceMetadata, MetadataBuildError> {
        panic!("compile-only probe")
    }

    fn validate(&self, _config_json: serde_json::Value) -> Result<(), Error> {
        Ok(())
    }

    fn register<'a>(
        &'a self,
        _manager: &'a Manager,
        _request: RegisterRequest<'a>,
        _expected_slot_identity: &'a SlotIdentity,
    ) -> BoxFut<'a, Result<SlotIdentity, Error>> {
        Box::pin(async { Ok(SlotIdentity::Unbound) })
    }
}

fn main() {}
