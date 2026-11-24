use std::any::TypeId;

use nebula_core::ResourceKey;
use nebula_resource::{
    Error, Manager, ResourceAdmissionError, ResourceFactory, ResourceDefinition, SlotIdentity,
    factory::{BoxFut, RegisterRequest},
};

struct ForgedFactory;

impl ResourceFactory for ForgedFactory {
    fn key(&self) -> ResourceKey {
        panic!("compile-only probe")
    }

    fn resource_type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    fn definition(&self) -> Result<&ResourceDefinition, ResourceAdmissionError> {
        panic!("compile-only probe")
    }

    fn validate(&self, _config_json: serde_json::Value) -> Result<(), Error> {
        Ok(())
    }

    fn validate_topology(&self, _settings: Option<&serde_json::Value>) -> Result<(), Error> {
        Ok(())
    }

    fn resilience_policy(&self) -> nebula_resource::rate_limit::ResiliencePolicy {
        nebula_resource::rate_limit::ResiliencePolicy::new()
    }

    fn topology_schema(
        &self,
    ) -> Result<Option<nebula_schema::ValidSchema>, ResourceAdmissionError> {
        Ok(None)
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
