//! In-memory claim/adjudication reference checks.
//!
//! Atomic threshold reclaim is intentionally absent: the in-memory claim repo
//! has no credential aggregate. Deployment conformance lives in the SQLite and
//! PostgreSQL suites.

use std::time::Duration;

use nebula_core::CredentialId;
use nebula_storage::credential::{
    ClaimAttempt, InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId,
};
use nebula_storage_port::{CredentialOwner, CredentialSelector};

#[tokio::test]
async fn owner_partitions_do_not_contend_for_the_same_credential_id() {
    let repo = InMemoryRefreshClaimRepo::new();
    let credential_id = CredentialId::new();
    let owner_a =
        CredentialSelector::new(CredentialOwner::from_canonical("owner-a"), credential_id);
    let owner_b =
        CredentialSelector::new(CredentialOwner::from_canonical("owner-b"), credential_id);

    for (selector, holder) in [(&owner_a, "holder-a"), (&owner_b, "holder-b")] {
        let result = repo
            .try_claim(selector, &ReplicaId::new(holder), Duration::from_secs(30))
            .await
            .expect("owner-qualified claim");
        assert!(matches!(result, ClaimAttempt::Acquired(_)));
    }
}
