//! Credential baseline references: what the relational schema proves
//! about credentials, their provider-operation claims and their incidents.
//!
//! A credential and a pending interactive flow belong to the live workspace
//! their owner partition names and are purged with it; a claim and its
//! incidents belong to their credential and are purged with it, all through
//! `ON DELETE CASCADE`. An archived credential (`deleted_at`) is invisible to
//! every read and write and can neither be claimed, cross the provider
//! boundary, nor have its incidents adjudicated. These are relational
//! invariants, so they run on the two SQL backends only, against the same
//! shared body.

#![cfg(any(feature = "sqlite", feature = "postgres"))]

#[path = "support/execution_parents.rs"]
#[expect(
    dead_code,
    reason = "credentials need only their tenant, not a workflow"
)]
mod execution_parents;

use std::{sync::Arc, time::Duration};

use nebula_core::CredentialId;
use nebula_credential::{DynPendingStateStore, PendingStoreError};
use nebula_storage::credential::refresh_claim::{
    CredentialIncidentRef, CredentialOperationDecision, CredentialOperationIntent,
    RefreshClaimAdjudicationError, RefreshClaimAdjudicator, RefreshOutcomeDecision,
};
use nebula_storage::credential::{
    ClaimAttempt, EnvKeyProvider, ExpiredClaim, KeyProvider, RefreshClaimReclaimer,
    RefreshClaimRepo, ReplicaId, RepoError, SentinelEscalationPolicy,
};
use nebula_storage_port::store::TenantProvisioningStore;
use nebula_storage_port::{
    CredentialCreate, CredentialMaterial, CredentialMaterialTransition, CredentialOwner,
    CredentialPersistence, CredentialPersistenceError, CredentialRefreshHorizon,
    CredentialRefreshPageSize, CredentialRefreshSchedule, CredentialReplacement,
    CredentialSelector, CredentialTombstone, CredentialVersion, MaterialUpdate, Scope, SecretBytes,
};
use serde_json::{Map, Value};

/// Raw access to the database behind a store, for the two things a case
/// cannot do through the port: archive or purge a credential (issue 1159
/// owns those operations) and observe the rows beneath it.
enum Raw {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlx::SqlitePool),
    #[cfg(feature = "postgres")]
    Postgres(sqlx::PgPool),
}

impl Raw {
    async fn archive(&self, id: CredentialId) {
        let archived = match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                sqlx::query("UPDATE credentials SET deleted_at = 1 WHERE id = ?1")
                    .bind(id.to_string())
                    .execute(pool)
                    .await
                    .map(|done| done.rows_affected())
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                sqlx::query("UPDATE credentials SET deleted_at = now() WHERE id = $1")
                    .bind(id.to_string())
                    .execute(pool)
                    .await
                    .map(|done| done.rows_affected())
            },
        };
        assert_eq!(archived.expect("archive the credential"), 1);
    }

    async fn purge(&self, id: CredentialId) {
        let purged = match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => sqlx::query("DELETE FROM credentials WHERE id = ?1")
                .bind(id.to_string())
                .execute(pool)
                .await
                .map(|done| done.rows_affected()),
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => sqlx::query("DELETE FROM credentials WHERE id = $1")
                .bind(id.to_string())
                .execute(pool)
                .await
                .map(|done| done.rows_affected()),
        };
        assert_eq!(purged.expect("purge the credential"), 1);
    }

    /// Put the credential's claim past its lease deadline.
    async fn expire_claim(&self, id: CredentialId) {
        let expired = match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => sqlx::query(
                "UPDATE credential_refresh_claims SET expires_at = 0 WHERE credential_id = ?1",
            )
            .bind(id.to_string())
            .execute(pool)
            .await
            .map(|done| done.rows_affected()),
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => sqlx::query(
                "UPDATE credential_refresh_claims \
                     SET expires_at = clock_timestamp() - INTERVAL '1 second' \
                     WHERE credential_id = $1",
            )
            .bind(id.to_string())
            .execute(pool)
            .await
            .map(|done| done.rows_affected()),
        };
        assert_eq!(expired.expect("expire the claim"), 1);
    }

    /// `(claims, incidents)` on record for the credential.
    async fn children(&self, id: CredentialId) -> (i64, i64) {
        let counted: Result<(i64, i64), sqlx::Error> = match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                sqlx::query_as(
                    "SELECT \
                       (SELECT COUNT(*) FROM credential_refresh_claims WHERE credential_id = ?1), \
                       (SELECT COUNT(*) FROM credential_refresh_incidents WHERE credential_id = ?1)",
                )
                .bind(id.to_string())
                .fetch_one(pool)
                .await
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                sqlx::query_as(
                    "SELECT \
                       (SELECT COUNT(*) FROM credential_refresh_claims WHERE credential_id = $1), \
                       (SELECT COUNT(*) FROM credential_refresh_incidents WHERE credential_id = $1)",
                )
                .bind(id.to_string())
                .fetch_one(pool)
                .await
            },
        };
        counted.expect("count the credential's claims and incidents")
    }

    /// Archive (`true`) or purge (`false`) the workspace `ws-<label>`.
    async fn retire_workspace(&self, label: &str, archive: bool) {
        let workspace = format!("ws-{label}");
        let retired = match (self, archive) {
            #[cfg(feature = "sqlite")]
            (Self::Sqlite(pool), true) => {
                sqlx::query("UPDATE workspaces SET deleted_at = 1 WHERE id = ?1")
                    .bind(&workspace)
                    .execute(pool)
                    .await
                    .map(|done| done.rows_affected())
            },
            #[cfg(feature = "sqlite")]
            (Self::Sqlite(pool), false) => sqlx::query("DELETE FROM workspaces WHERE id = ?1")
                .bind(&workspace)
                .execute(pool)
                .await
                .map(|done| done.rows_affected()),
            #[cfg(feature = "postgres")]
            (Self::Postgres(pool), true) => {
                sqlx::query("UPDATE workspaces SET deleted_at = now() WHERE id = $1")
                    .bind(&workspace)
                    .execute(pool)
                    .await
                    .map(|done| done.rows_affected())
            },
            #[cfg(feature = "postgres")]
            (Self::Postgres(pool), false) => sqlx::query("DELETE FROM workspaces WHERE id = $1")
                .bind(&workspace)
                .execute(pool)
                .await
                .map(|done| done.rows_affected()),
        };
        assert_eq!(retired.expect("retire the workspace"), 1);
    }

    /// `(credentials, pending states)` filed under the workspace `ws-<label>`.
    async fn workspace_rows(&self, label: &str) -> (i64, i64) {
        let workspace = format!("ws-{label}");
        let counted: Result<(i64, i64), sqlx::Error> = match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                sqlx::query_as(
                    "SELECT \
                       (SELECT COUNT(*) FROM credentials WHERE workspace_id = ?1), \
                       (SELECT COUNT(*) FROM credential_pending_states WHERE workspace_id = ?1)",
                )
                .bind(&workspace)
                .fetch_one(pool)
                .await
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                sqlx::query_as(
                    "SELECT \
                       (SELECT COUNT(*) FROM credentials WHERE workspace_id = $1), \
                       (SELECT COUNT(*) FROM credential_pending_states WHERE workspace_id = $1)",
                )
                .bind(&workspace)
                .fetch_one(pool)
                .await
            },
        };
        counted.expect("count the workspace's credential rows")
    }

    /// The `(org_id, workspace_id)` the credential is filed under.
    async fn tenant(&self, id: CredentialId) -> (String, String) {
        let tenant: Result<(String, String), sqlx::Error> = match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                sqlx::query_as("SELECT org_id, workspace_id FROM credentials WHERE id = ?1")
                    .bind(id.to_string())
                    .fetch_one(pool)
                    .await
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                sqlx::query_as("SELECT org_id, workspace_id FROM credentials WHERE id = $1")
                    .bind(id.to_string())
                    .fetch_one(pool)
                    .await
            },
        };
        tenant.expect("read the credential's tenant")
    }
}

/// Every workspace the cases file credentials under; a backend provisions
/// them before a case runs.
const WORKSPACES: &[&str] = &[
    "filed",
    "claims",
    "cascade",
    "archived",
    "archived-workspace",
    "purged",
    "adjudicated",
];

fn scope(label: &str) -> Scope {
    Scope::new(format!("ws-{label}"), format!("org-{label}"))
}

fn owner(label: &str) -> CredentialOwner {
    CredentialOwner::from_scope(&scope(label))
}

async fn provision(tenants: &dyn TenantProvisioningStore) {
    for label in WORKSPACES {
        execution_parents::provision_scope(tenants, &scope(label)).await;
    }
}

fn pending_key() -> Arc<dyn KeyProvider> {
    Arc::new(
        EnvKeyProvider::from_base64("QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=")
            .expect("valid fixed test key"),
    )
}

/// Take `selector`'s credential across the provider boundary, expire the
/// claim and let the sweep account it: one unresolved incident on record.
async fn poison<C>(claims: &C, raw: &Raw, selector: &CredentialSelector) -> CredentialIncidentRef
where
    C: RefreshClaimRepo + RefreshClaimReclaimer,
{
    let ClaimAttempt::Acquired(claim) = acquire(claims, selector, "poison-holder")
        .await
        .expect("claim the credential")
    else {
        panic!("a fresh credential is claimable");
    };
    claims
        .mark_sentinel(&claim.token)
        .await
        .expect("cross the provider boundary");
    raw.expire_claim(selector.credential_id()).await;
    claims
        .reclaim_stuck(
            SentinelEscalationPolicy::new(u32::MAX, Duration::from_hours(1)).expect("policy"),
        )
        .await
        .expect("account the crashed operation");
    CredentialIncidentRef::from_uuid(claim.token.claim_id)
}

fn create(name: Option<&str>) -> CredentialCreate {
    let mut metadata = Map::new();
    if let Some(name) = name {
        metadata.insert(
            "display".to_owned(),
            serde_json::json!({ "display_name": name }),
        );
    }
    CredentialCreate::new(
        "provider.oauth".to_owned(),
        SecretBytes::new(b"relational-secret".to_vec()),
        "oauth2_state".to_owned(),
        1,
        name.map(str::to_owned),
        Some(chrono::Utc::now() + chrono::Duration::minutes(1)),
        false,
        metadata,
    )
}

fn version(value: i64) -> CredentialVersion {
    CredentialVersion::try_from(value).expect("fixture version")
}

async fn acquire<C: RefreshClaimRepo>(
    claims: &C,
    selector: &CredentialSelector,
    holder: &str,
) -> Result<ClaimAttempt, RepoError> {
    claims
        .try_claim(
            selector,
            &ReplicaId::new(holder),
            Duration::from_secs(30),
            CredentialOperationIntent::Refresh,
        )
        .await
}

/// A credential is filed under the workspace its owner names; a partition
/// that names no workspace owns nothing.
async fn assert_credentials_are_filed_under_their_workspace<S>(store: &S, raw: &Raw)
where
    S: CredentialPersistence,
{
    let id = CredentialId::new();
    store
        .create(&CredentialSelector::new(owner("filed"), id), create(None))
        .await
        .expect("create the credential");
    assert_eq!(
        raw.tenant(id).await,
        ("org-filed".to_owned(), "ws-filed".to_owned())
    );

    let unfiled = CredentialOwner::from_canonical("not-a-workspace");
    assert_eq!(
        store
            .create(
                &CredentialSelector::new(unfiled.clone(), CredentialId::new()),
                create(None)
            )
            .await,
        Err(CredentialPersistenceError::NotFound),
        "a partition that names no workspace cannot own a credential"
    );
    assert!(store.list(&unfiled, None).await.expect("list").is_empty());
}

/// A claim belongs to an existing credential.
async fn assert_claims_require_their_credential<C>(claims: &C)
where
    C: RefreshClaimRepo,
{
    let missing = CredentialSelector::new(owner("claims"), CredentialId::new());
    assert!(
        matches!(
            acquire(claims, &missing, "missing-holder").await,
            Err(RepoError::AggregateUnavailable)
        ),
        "a claim on a credential that does not exist must be refused"
    );
}

/// Purging a credential purges its claim and its incidents.
async fn assert_claims_and_incidents_cascade_with_their_credential<S, C>(
    store: &S,
    claims: &C,
    raw: &Raw,
) where
    S: CredentialPersistence,
    C: RefreshClaimRepo + RefreshClaimReclaimer,
{
    let id = CredentialId::new();
    let selector = CredentialSelector::new(owner("cascade"), id);
    store
        .create(&selector, create(None))
        .await
        .expect("create the credential");
    let ClaimAttempt::Acquired(claim) = acquire(claims, &selector, "cascade-holder")
        .await
        .expect("claim the credential")
    else {
        panic!("a fresh credential is claimable");
    };
    claims
        .mark_sentinel(&claim.token)
        .await
        .expect("cross the provider boundary");
    raw.expire_claim(id).await;
    let outcomes = claims
        .reclaim_stuck(
            SentinelEscalationPolicy::new(u32::MAX, Duration::from_hours(1)).expect("policy"),
        )
        .await
        .expect("account the crashed operation");
    assert!(
        outcomes.iter().any(|outcome| matches!(
            outcome,
            ExpiredClaim::OutcomeUnknownAccounted { selector: accounted, .. }
                if accounted.credential_id() == id
        )),
        "the crashed operation is accounted as an incident"
    );
    assert_eq!(
        raw.children(id).await,
        (1, 1),
        "the poisoned claim and its incident are retained"
    );

    raw.purge(id).await;
    assert_eq!(
        raw.children(id).await,
        (0, 0),
        "the claim and the incident are purged with their credential"
    );
}

/// An archived credential is invisible and unusable: no read finds it, no
/// write reaches it, it is not due for refresh, it cannot be claimed, and an
/// existing claim on it cannot cross the provider boundary. Its name is free.
async fn assert_an_archived_credential_is_not_usable<S, C>(store: &S, claims: &C, raw: &Raw)
where
    S: CredentialPersistence + CredentialRefreshSchedule,
    C: RefreshClaimRepo,
{
    let owner = owner("archived");
    let id = CredentialId::new();
    let selector = CredentialSelector::new(owner.clone(), id);
    store
        .create(&selector, create(Some("Archived")))
        .await
        .expect("create the credential");
    let ClaimAttempt::Acquired(claim) = acquire(claims, &selector, "archived-holder")
        .await
        .expect("claim the live credential")
    else {
        panic!("a live credential is claimable");
    };
    let due = |page: &[nebula_storage_port::DueCredentialRefresh]| {
        page.iter()
            .any(|candidate| candidate.selector().credential_id() == id)
    };
    let page = store
        .scan_due(
            None,
            CredentialRefreshHorizon::default(),
            CredentialRefreshPageSize::default(),
        )
        .await
        .expect("scan for due refreshes");
    assert!(due(&page), "the live credential is due for refresh");

    raw.archive(id).await;

    assert_eq!(
        store.get(&selector).await.map(|_| ()),
        Err(CredentialPersistenceError::NotFound)
    );
    assert_eq!(
        store.get_head(&selector).await.map(|_| ()),
        Err(CredentialPersistenceError::NotFound)
    );
    assert_eq!(
        store.get_with_operation_status(&selector).await.map(|_| ()),
        Err(CredentialPersistenceError::NotFound)
    );
    assert_eq!(
        store.operation_status(&selector).await.map(|_| ()),
        Err(CredentialPersistenceError::NotFound)
    );
    assert_eq!(
        store.refresh_retry_snapshot(&selector).await.map(|_| ()),
        Err(CredentialPersistenceError::NotFound)
    );
    assert_eq!(store.exists(&selector).await, Ok(false));
    assert!(!store.list(&owner, None).await.expect("list").contains(&id));
    assert!(
        store
            .list_operational_heads(&owner, None)
            .await
            .expect("list heads")
            .is_empty()
    );
    assert_eq!(
        store
            .replace(
                &selector,
                CredentialReplacement::new(
                    version(1),
                    None,
                    false,
                    Map::<String, Value>::new(),
                    CredentialMaterialTransition::advance(MaterialUpdate::Replace(
                        CredentialMaterial::new(
                            SecretBytes::new(b"after-archive".to_vec()),
                            "oauth2_state".to_owned(),
                            2,
                            None,
                        )
                    )),
                ),
            )
            .await
            .map(|_| ()),
        Err(CredentialPersistenceError::NotFound)
    );
    assert_eq!(
        store
            .tombstone(&selector, CredentialTombstone::new(version(1)))
            .await
            .map(|_| ()),
        Err(CredentialPersistenceError::NotFound)
    );
    let page = store
        .scan_due(
            None,
            CredentialRefreshHorizon::default(),
            CredentialRefreshPageSize::default(),
        )
        .await
        .expect("scan for due refreshes");
    assert!(
        !due(&page),
        "an archived credential is never due for refresh"
    );

    assert!(
        matches!(
            claims.mark_sentinel(&claim.token).await,
            Err(RepoError::InvalidState)
        ),
        "a claim on an archived credential must not authorize provider egress"
    );
    claims
        .release(claim.token)
        .await
        .expect("an unused claim can still be released");
    assert!(
        matches!(
            acquire(claims, &selector, "archived-holder-2").await,
            Err(RepoError::AggregateUnavailable)
        ),
        "an archived credential cannot be claimed"
    );

    store
        .create(
            &CredentialSelector::new(owner.clone(), CredentialId::new()),
            create(Some("Archived")),
        )
        .await
        .expect("an archived credential's name is free among live ones");
}

/// A credential and a pending flow are created only beneath a live
/// workspace: a missing or archived one refuses both as `NotFound`.
async fn assert_credentials_need_a_live_workspace<S>(
    store: &S,
    pending: &dyn DynPendingStateStore,
    raw: &Raw,
) where
    S: CredentialPersistence,
{
    let unprovisioned = owner("never-provisioned");
    assert_eq!(
        store
            .create(
                &CredentialSelector::new(unprovisioned.clone(), CredentialId::new()),
                create(None)
            )
            .await
            .map(|_| ()),
        Err(CredentialPersistenceError::NotFound),
        "a workspace that does not exist owns no credential"
    );
    assert!(matches!(
        pending
            .put_serialized(
                "oauth2",
                unprovisioned.as_str(),
                "session",
                zeroize::Zeroizing::new(b"pending".to_vec()),
                Duration::from_mins(1),
            )
            .await,
        Err(PendingStoreError::NotFound)
    ));

    let archived = owner("archived-workspace");
    store
        .create(
            &CredentialSelector::new(archived.clone(), CredentialId::new()),
            create(None),
        )
        .await
        .expect("a live workspace owns credentials");
    raw.retire_workspace("archived-workspace", true).await;
    assert_eq!(
        store
            .create(
                &CredentialSelector::new(archived.clone(), CredentialId::new()),
                create(None)
            )
            .await
            .map(|_| ()),
        Err(CredentialPersistenceError::NotFound),
        "an archived workspace refuses new credentials"
    );
    assert!(matches!(
        pending
            .put_serialized(
                "oauth2",
                archived.as_str(),
                "session",
                zeroize::Zeroizing::new(b"pending".to_vec()),
                Duration::from_mins(1),
            )
            .await,
        Err(PendingStoreError::NotFound)
    ));
}

/// Purging a workspace purges its credentials, their claims and incidents,
/// and its pending flows.
async fn assert_purging_a_workspace_purges_its_credentials<S, C>(
    store: &S,
    claims: &C,
    pending: &dyn DynPendingStateStore,
    raw: &Raw,
) where
    S: CredentialPersistence,
    C: RefreshClaimRepo + RefreshClaimReclaimer,
{
    let id = CredentialId::new();
    let selector = CredentialSelector::new(owner("purged"), id);
    store
        .create(&selector, create(None))
        .await
        .expect("create the credential");
    poison(claims, raw, &selector).await;
    pending
        .put_serialized(
            "oauth2",
            owner("purged").as_str(),
            "session",
            zeroize::Zeroizing::new(b"pending".to_vec()),
            Duration::from_mins(1),
        )
        .await
        .expect("put pending state");
    assert_eq!(raw.children(id).await, (1, 1));
    assert_eq!(raw.workspace_rows("purged").await, (1, 1));

    raw.retire_workspace("purged", false).await;
    assert_eq!(
        raw.children(id).await,
        (0, 0),
        "the credential's claim and incident are purged with its workspace"
    );
    assert_eq!(
        raw.workspace_rows("purged").await,
        (0, 0),
        "the workspace's credentials and pending flows are purged with it"
    );
}

/// An archived credential's incidents are not adjudicated: the adjudicator
/// answers `AggregateUnavailable`, like every other archived-credential path.
async fn assert_an_archived_credentials_incident_is_not_adjudicated<S, C>(
    store: &S,
    claims: &C,
    raw: &Raw,
) where
    S: CredentialPersistence,
    C: RefreshClaimRepo + RefreshClaimReclaimer + RefreshClaimAdjudicator,
{
    let id = CredentialId::new();
    let selector = CredentialSelector::new(owner("adjudicated"), id);
    store
        .create(&selector, create(None))
        .await
        .expect("create the credential");
    let incident = poison(claims, raw, &selector).await;
    raw.archive(id).await;
    assert!(
        matches!(
            claims
                .adjudicate(
                    &selector,
                    incident,
                    CredentialOperationDecision::Refresh(RefreshOutcomeDecision::ProviderApplied),
                    "provider audit confirms the refresh landed",
                )
                .await,
            Err(RefreshClaimAdjudicationError::AggregateUnavailable)
        ),
        "an archived credential's incident must not be adjudicated"
    );
}

#[cfg(feature = "sqlite")]
mod sqlite {
    use nebula_storage::credential::SqliteCredentialPersistence;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    use super::*;

    async fn backend() -> (SqliteCredentialPersistence, Raw, tempfile::TempDir) {
        let directory = tempfile::tempdir().expect("temporary database directory");
        let path = directory.path().join("credential-relations.sqlite");
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let store = SqliteCredentialPersistence::connect(&url)
            .await
            .expect("an admitted SQLite credential store");
        let raw = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                url.parse::<SqliteConnectOptions>()
                    .expect("database URL")
                    .create_if_missing(false),
            )
            .await
            .expect("an inspection pool on the same file");
        provision(&nebula_storage::sqlite::SqliteTenantProvisioningStore::new(
            raw.clone(),
        ))
        .await;
        (store, Raw::Sqlite(raw), directory)
    }

    #[tokio::test]
    async fn credentials_need_a_live_workspace() {
        let (store, raw, _directory) = backend().await;
        let pending = store.pending_state_store(pending_key(), Vec::new());
        assert_credentials_need_a_live_workspace(&store, &pending, &raw).await;
    }

    #[tokio::test]
    async fn purging_a_workspace_purges_its_credentials() {
        let (store, raw, _directory) = backend().await;
        let pending = store.pending_state_store(pending_key(), Vec::new());
        assert_purging_a_workspace_purges_its_credentials(
            &store,
            &store.refresh_claim_repo(),
            &pending,
            &raw,
        )
        .await;
    }

    #[tokio::test]
    async fn an_archived_credentials_incident_is_not_adjudicated() {
        let (store, raw, _directory) = backend().await;
        assert_an_archived_credentials_incident_is_not_adjudicated(
            &store,
            &store.refresh_claim_repo(),
            &raw,
        )
        .await;
    }

    #[tokio::test]
    async fn credentials_are_filed_under_their_workspace() {
        let (store, raw, _directory) = backend().await;
        assert_credentials_are_filed_under_their_workspace(&store, &raw).await;
    }

    #[tokio::test]
    async fn claims_require_their_credential() {
        let (store, _raw, _directory) = backend().await;
        assert_claims_require_their_credential(&store.refresh_claim_repo()).await;
    }

    #[tokio::test]
    async fn claims_and_incidents_cascade_with_their_credential() {
        let (store, raw, _directory) = backend().await;
        assert_claims_and_incidents_cascade_with_their_credential(
            &store,
            &store.refresh_claim_repo(),
            &raw,
        )
        .await;
    }

    #[tokio::test]
    async fn an_archived_credential_is_not_usable() {
        let (store, raw, _directory) = backend().await;
        assert_an_archived_credential_is_not_usable(&store, &store.refresh_claim_repo(), &raw)
            .await;
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use std::str::FromStr;

    use nebula_storage::credential::PgCredentialPersistence;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

    use super::*;

    #[path = "../support/postgres_schema.rs"]
    #[expect(
        dead_code,
        reason = "the cases need only the schema-name helper of this shared module"
    )]
    mod postgres_schema;

    /// A store and an inspection pool pinned to one private schema; the
    /// schema is dropped with the database handle.
    struct Database {
        store: PgCredentialPersistence,
        raw: Raw,
        admin: sqlx::PgPool,
        schema: String,
    }

    impl Database {
        async fn connect() -> Self {
            let url = std::env::var("DATABASE_URL")
                .expect("DATABASE_URL must point at a disposable PostgreSQL database");
            let admin = PgPoolOptions::new()
                .max_connections(1)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            let schema = postgres_schema::unique_schema_name("nebula_credential_relations");
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE"
            )))
            .execute(&admin)
            .await
            .expect("drop a stale private schema");
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
                .execute(&admin)
                .await
                .expect("create the private schema");
            let options = PgConnectOptions::from_str(&url)
                .expect("parse DATABASE_URL")
                .options([("search_path", schema.as_str())]);
            let store = PgCredentialPersistence::connect_with(options.clone())
                .await
                .expect("an admitted PostgreSQL credential store");
            let raw = PgPoolOptions::new()
                .max_connections(2)
                .connect_with(options)
                .await
                .expect("an inspection pool on the private schema");
            provision(&nebula_storage::postgres::PgTenantProvisioningStore::new(
                raw.clone(),
            ))
            .await;
            Self {
                store,
                raw: Raw::Postgres(raw),
                admin,
                schema,
            }
        }

        async fn cleanup(self) {
            drop(self.raw);
            drop(self.store);
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA {} CASCADE",
                self.schema
            )))
            .execute(&self.admin)
            .await
            .expect("drop the private schema");
            self.admin.close().await;
        }
    }

    #[tokio::test]
    async fn credentials_are_filed_under_their_workspace() {
        let database = Database::connect().await;
        assert_credentials_are_filed_under_their_workspace(&database.store, &database.raw).await;
        database.cleanup().await;
    }

    #[tokio::test]
    async fn credentials_need_a_live_workspace() {
        let database = Database::connect().await;
        let pending = database
            .store
            .pending_state_store(pending_key(), Vec::new());
        assert_credentials_need_a_live_workspace(&database.store, &pending, &database.raw).await;
        database.cleanup().await;
    }

    #[tokio::test]
    async fn purging_a_workspace_purges_its_credentials() {
        let database = Database::connect().await;
        let pending = database
            .store
            .pending_state_store(pending_key(), Vec::new());
        assert_purging_a_workspace_purges_its_credentials(
            &database.store,
            &database.store.refresh_claim_repo(),
            &pending,
            &database.raw,
        )
        .await;
        database.cleanup().await;
    }

    #[tokio::test]
    async fn an_archived_credentials_incident_is_not_adjudicated() {
        let database = Database::connect().await;
        assert_an_archived_credentials_incident_is_not_adjudicated(
            &database.store,
            &database.store.refresh_claim_repo(),
            &database.raw,
        )
        .await;
        database.cleanup().await;
    }

    #[tokio::test]
    async fn claims_require_their_credential() {
        let database = Database::connect().await;
        assert_claims_require_their_credential(&database.store.refresh_claim_repo()).await;
        database.cleanup().await;
    }

    #[tokio::test]
    async fn claims_and_incidents_cascade_with_their_credential() {
        let database = Database::connect().await;
        assert_claims_and_incidents_cascade_with_their_credential(
            &database.store,
            &database.store.refresh_claim_repo(),
            &database.raw,
        )
        .await;
        database.cleanup().await;
    }

    #[tokio::test]
    async fn an_archived_credential_is_not_usable() {
        let database = Database::connect().await;
        assert_an_archived_credential_is_not_usable(
            &database.store,
            &database.store.refresh_claim_repo(),
            &database.raw,
        )
        .await;
        database.cleanup().await;
    }
}
