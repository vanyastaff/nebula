use std::sync::Arc;

use nebula_storage_port::CredentialPersistence;

#[test]
fn credential_persistence_is_directly_object_safe() {
    fn accepts_dyn(_: Option<Arc<dyn CredentialPersistence>>) {}

    accepts_dyn(None);
}
