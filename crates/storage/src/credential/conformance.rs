//! Test-only physical fault hooks for the credential semantic oracle.
//!
//! Product code must reach persistence only through lifecycle commands. This
//! feature-gated trait exists solely so one backend-neutral integration suite
//! can place each concrete adapter at otherwise unreachable version/corruption
//! boundaries and then verify the public port's fail-closed behaviour.

use async_trait::async_trait;
use nebula_storage_port::{
    CredentialMaterialEpoch, CredentialPersistence, CredentialPersistenceError, CredentialSelector,
    CredentialVersion,
};

/// Physical fixture controls used only by credential backend conformance.
#[async_trait]
pub(crate) trait CredentialPersistenceConformance: CredentialPersistence {
    /// Move an existing live fixture to an exact valid live version.
    async fn force_live_version_for_conformance(
        &self,
        selector: &CredentialSelector,
        version: CredentialVersion,
    ) -> Result<(), CredentialPersistenceError>;

    /// Move an existing live fixture to an exact valid material epoch.
    async fn force_live_material_epoch_for_conformance(
        &self,
        selector: &CredentialSelector,
        material_epoch: CredentialMaterialEpoch,
    ) -> Result<(), CredentialPersistenceError>;

    /// Corrupt an existing live row in a way the database can represent but
    /// the closed persistence DTO must reject.
    async fn corrupt_live_projection_for_conformance(
        &self,
        selector: &CredentialSelector,
    ) -> Result<(), CredentialPersistenceError>;
}
