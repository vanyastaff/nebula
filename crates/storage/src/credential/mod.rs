//! Credential persistence — durable stores (`SqliteCredentialPersistence`,
//! `PgCredentialPersistence`), `KeyProvider`, and composable layers.
//!
//! Two distinct layer families live here:
//!
//! - `CredentialPersistence` wrappers — `EncryptionLayer`, `CacheLayer`,
//!   `AuditLayer`. They compose around a backing store that persists
//!   `StoredCredential` rows. Owner scoping is mandatory in the port itself;
//!   every adapter predicate receives a complete owner-bound selector.
//! - `ExternalProvider` wrappers — `ProviderCacheLayer`. They compose
//!   around an `Arc<dyn ExternalProvider>` that resolves secrets from a
//!   remote system (Vault, AWS SM, env var, …). The `Provider`-prefixed types
//!   are scoped to that trait and do not interact with the `CredentialPersistence`
//!   cache.
//!
//! The `CredentialPersistence` trait + DTOs live in `nebula-storage-port`; concrete
//! implementations and layers live here. See `crates/storage/README.md` and
//! `docs/INTEGRATION_MODEL.md` (Credential) for integration context.

/// SQLite's clock as `INTEGER` microseconds since the Unix epoch, for
/// statements that compare or author instants inside SQLite. SQLite reads the
/// process clock at millisecond resolution.
#[cfg(feature = "sqlite")]
macro_rules! sqlite_now_us {
    () => {
        "(CAST(strftime('%s', 'now') AS INTEGER) * 1000000 \
         + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER) * 1000)"
    };
}

#[cfg(test)]
mod conformance;
#[cfg(test)]
mod conformance_tests;
pub mod key_provider;
pub mod keyring;
pub mod layer;
pub mod provider_cache;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
mod startup;

#[cfg(any(
    test,
    feature = "credential-in-memory",
    feature = "sqlite",
    feature = "postgres"
))]
pub mod pending;
#[cfg(any(test, feature = "credential-in-memory"))]
mod reference;
#[cfg(any(
    test,
    feature = "credential-in-memory",
    feature = "sqlite",
    feature = "postgres"
))]
mod retry_gate;

/// Cross-replica refresh claim repository (CAS + heartbeat).
pub mod refresh_claim;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(test)]
pub(crate) use conformance::CredentialPersistenceConformance;
pub use key_provider::{EnvKeyProvider, FileKeyProvider, KeyProvider, KeySnapshot, ProviderError};
pub use keyring::{CredentialKeyring, CredentialKeyringError};
pub use layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer,
};
#[cfg(any(test, feature = "credential-in-memory"))]
pub use pending::InMemoryPendingStore;
#[cfg(feature = "postgres")]
pub use pending::PgPendingStateStore;
#[cfg(feature = "sqlite")]
pub use pending::SqlitePendingStateStore;
#[cfg(feature = "postgres")]
pub use postgres::{PgCredentialPersistence, PgCredentialRefreshSchedule};
pub use provider_cache::{ProviderCacheConfig, ProviderCacheLayer, ProviderCacheStats};
#[cfg(test)]
pub(crate) use reference::ReferenceCredentialPersistence;
/// In-process credential persistence for test hosts: the reference adapter,
/// with the lifecycle semantics
/// the SQL adapters are held to (no tenancy, so no live-workspace check).
#[cfg(feature = "credential-in-memory")]
pub use reference::ReferenceCredentialPersistence as InMemoryCredentialPersistence;
#[cfg(feature = "postgres")]
pub use refresh_claim::PgRefreshClaimRepo;
#[cfg(feature = "sqlite")]
pub use refresh_claim::SqliteRefreshClaimRepo;
pub use refresh_claim::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, InMemoryRefreshClaimRepo,
    ReauthEscalation, RefreshClaim, RefreshClaimReclaimer, RefreshClaimRepo, ReplicaId, RepoError,
    SentinelEscalationPolicy, SentinelState,
};
#[cfg(feature = "sqlite")]
pub use sqlite::{SqliteCredentialPersistence, SqliteCredentialRefreshSchedule};
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub use startup::CredentialStoreStartupError;

/// The owners credential unit tests file their credentials under.
///
/// SQL backends file a credential under the workspace its owner partition
/// names and require that workspace to be live, so a test
/// owner is the canonical key of a scope whose tenant the test provisions.
#[cfg(test)]
pub(crate) mod test_owner {
    use nebula_storage_port::{CredentialOwner, Scope};
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    use nebula_storage_port::{
        dto::{
            PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate,
            TenantProvisioningOutcome, TenantProvisioningRequest,
        },
        store::TenantProvisioningStore,
    };

    /// Owners every [`sqlite_store`] provisions.
    #[cfg(feature = "sqlite")]
    pub(crate) const DEFAULT_LABELS: &[&str] = &[
        "test-owner",
        "owner-a",
        "owner-b",
        "revoke-fence-owner",
        "revoke-finalizer-owner",
        "post-commit-fault-owner",
        "precommit-rollback-owner",
        "admission-read-owner",
        "revoke-adjudication",
    ];

    /// The workspace `ws-<label>` in `org-<label>`.
    pub(crate) fn scope(label: &str) -> Scope {
        Scope::new(format!("ws-{label}"), format!("org-{label}"))
    }

    /// The owner partition of workspace `ws-<label>` in `org-<label>`.
    pub(crate) fn owner(label: &str) -> CredentialOwner {
        CredentialOwner::from_scope(&scope(label))
    }

    /// Provision the tenants of `labels` in `tenants`.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    pub(crate) async fn provision(tenants: &dyn TenantProvisioningStore, labels: &[&str]) {
        for label in labels {
            provision_scope(tenants, &scope(label)).await;
        }
    }

    /// A ready in-memory SQLite credential store whose [`DEFAULT_LABELS`]
    /// tenants are provisioned.
    #[cfg(feature = "sqlite")]
    pub(crate) async fn sqlite_store()
    -> Result<super::SqliteCredentialPersistence, super::CredentialStoreStartupError> {
        let store = super::SqliteCredentialPersistence::connect_memory().await?;
        provision(&store.tenant_provisioning_store(), DEFAULT_LABELS).await;
        Ok(store)
    }

    /// Provision `scope`'s org with `scope`'s workspace as its default one.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    pub(crate) async fn provision_scope(tenants: &dyn TenantProvisioningStore, scope: &Scope) {
        let org = TenantOrgCreate::new(
            scope.org_id.clone(),
            scope.org_id.clone(),
            "Fixture".into(),
            "fixture".into(),
            "free".into(),
            None,
            serde_json::json!({}),
        )
        .expect("org values");
        let workspace = TenantDefaultWorkspaceCreate::new(
            scope.workspace_id.clone(),
            "default".into(),
            "Default".into(),
            None,
            "fixture".into(),
            serde_json::json!({}),
        )
        .expect("workspace values");
        let request = TenantProvisioningRequest::new(
            org,
            workspace,
            PrincipalKind::User,
            "fixture-owner".into(),
            None,
        )
        .expect("provisioning request");
        let outcome = tenants
            .provision_tenant(request)
            .await
            .expect("provision the fixture scope");
        assert!(
            matches!(
                outcome,
                TenantProvisioningOutcome::Created | TenantProvisioningOutcome::Replayed
            ),
            "the fixture scope must provision or replay, got {outcome:?}"
        );
    }
}

/// Crate-local helpers for constructing credential lifecycle test commands.
/// Gated on `sqlite` because all callers are `#[cfg(all(test, feature = "sqlite"))]` test modules.
#[cfg(all(test, feature = "sqlite"))]
pub(crate) mod test_support {
    use nebula_storage_port::{
        CredentialCreate, CredentialMaterial, CredentialMaterialTransition, CredentialReplacement,
        CredentialVersion, MaterialUpdate, RefreshRetryTransition, SecretBytes,
    };

    pub(crate) fn make_credential(data: &[u8]) -> CredentialCreate {
        CredentialCreate::new(
            "test_credential".to_owned(),
            SecretBytes::new(data.to_vec()),
            "test".to_owned(),
            1,
            None,
            None,
            false,
            Default::default(),
        )
    }

    /// An `Advance { Replace }` that installs `data` as new material.
    pub(crate) fn make_replacement(
        expected_version: CredentialVersion,
        data: &[u8],
    ) -> CredentialReplacement {
        CredentialReplacement::new(
            expected_version,
            None,
            false,
            Default::default(),
            CredentialMaterialTransition::advance(MaterialUpdate::Replace(
                CredentialMaterial::new(
                    SecretBytes::new(data.to_vec()),
                    "test".to_owned(),
                    1,
                    None,
                ),
            )),
        )
    }

    /// A `Preserve` that applies only `transition` and carries no material.
    pub(crate) fn make_preserve_replacement(
        expected_version: CredentialVersion,
        transition: RefreshRetryTransition,
    ) -> CredentialReplacement {
        CredentialReplacement::new(
            expected_version,
            None,
            false,
            Default::default(),
            CredentialMaterialTransition::preserve(transition),
        )
    }
}
