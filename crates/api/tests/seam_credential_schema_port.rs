//! credential-schema validation seam — `CredentialSchemaPort` is api-owned, object-safe, and
//! wires into `AppState` exactly like the `action_registry` precedent
//! (`Option<Arc<dyn …>>`, `None` ⇒ honest 503).

mod common;

use std::sync::Arc;

use nebula_api::ports::credential_schema::{
    CredentialFieldError, CredentialSchemaPort, CredentialTypeDescriptor,
};

struct StubPort;

impl CredentialSchemaPort for StubPort {
    fn validate_data(
        &self,
        _credential_key: &str,
        _data: &serde_json::Value,
    ) -> Result<(), Vec<CredentialFieldError>> {
        Ok(())
    }

    fn list_types(&self) -> Vec<CredentialTypeDescriptor> {
        Vec::new()
    }

    fn get_type(&self, _credential_key: &str) -> Option<CredentialTypeDescriptor> {
        None
    }
}

#[tokio::test]
async fn appstate_credential_schema_defaults_none_and_builder_sets_it() {
    // Object-safety: the trait must be usable as `dyn`.
    fn _assert_object_safe(_: &dyn CredentialSchemaPort) {}

    // The shared harness wires a permissive port by default (credential-schema validation
    // closed the unvalidated-persist fail-open); the *AppState default*
    // (no builder call) is None — assert via the no-port helper.
    let (state, _q) = common::create_state_with_queue_no_credential_port().await;
    assert!(
        state.credential_schema.is_none(),
        "AppState default must be None (honest-503 stub, like action_registry)"
    );

    let state = state.with_credential_schema(Arc::new(StubPort));
    assert!(
        state.credential_schema.is_some(),
        "with_credential_schema must attach the port"
    );
}
