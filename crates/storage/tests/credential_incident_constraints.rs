//! Credential state and provider reconciliation remain closed at the schema seam.

#![cfg(all(feature = "sqlite", feature = "postgres"))]

mod support {
    #[expect(dead_code, reason = "credential incidents need tenancy only")]
    pub(crate) mod execution_parents;
    pub(crate) mod postgres_schema;
}

use support::{execution_parents, postgres_schema};

use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialCreate, CredentialOwner, CredentialPersistence, CredentialSelector, Scope,
    SecretBytes,
};

fn material() -> CredentialCreate {
    CredentialCreate::new(
        "provider.oauth".into(),
        SecretBytes::new(b"fixture".to_vec()),
        "oauth2_state".into(),
        1,
        None,
        None,
        false,
        serde_json::Map::new(),
    )
}

async fn sqlite_credential() -> sqlx::SqlitePool {
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .expect("SQLite deployment");
    let scope = Scope::new("shape-workspace", "shape-org");
    execution_parents::provision_scope(
        &nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone()),
        &scope,
    )
    .await;
    let selector =
        CredentialSelector::new(CredentialOwner::from_scope(&scope), CredentialId::new());
    let store = nebula_storage::credential::SqliteCredentialPersistence::connect_pool(pool.clone())
        .await
        .expect("credential store");
    store
        .create(&selector, material())
        .await
        .expect("credential parent");
    pool
}

async fn postgres_credential() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL required for PostgreSQL evidence");
    let pool = postgres_schema::connect_with_private_schema(&url, "nebula_credential_shape")
        .await
        .expect("PostgreSQL deployment");
    nebula_storage::postgres::init_schema(&pool)
        .await
        .expect("admit schema");
    let scope = Scope::new("shape-workspace", "shape-org");
    execution_parents::provision_scope(
        &nebula_storage::postgres::PgTenantProvisioningStore::new(pool.clone()),
        &scope,
    )
    .await;
    let selector =
        CredentialSelector::new(CredentialOwner::from_scope(&scope), CredentialId::new());
    let store = nebula_storage::credential::PgCredentialPersistence::connect_pool(pool.clone())
        .await
        .expect("credential store");
    store
        .create(&selector, material())
        .await
        .expect("credential parent");
    pool
}

#[tokio::test]
async fn sqlite_retry_gate_requires_its_mode_phase_and_kind() {
    let pool = sqlite_credential().await;
    for (mode, phase, kind) in [
        (Some("never"), None, Some("transient_network")),
        (Some("never"), Some("before_dispatch"), None),
        (None, Some("before_dispatch"), Some("transient_network")),
    ] {
        assert!(sqlx::query("UPDATE credentials SET refresh_retry_mode = ?, refresh_retry_phase = ?, refresh_retry_kind = ?")
            .bind(mode).bind(phase).bind(kind).execute(&pool).await.is_err(), "retry gate cannot have a missing required field");
    }
    sqlx::query("UPDATE credentials SET refresh_retry_mode = 'never', refresh_retry_phase = 'before_dispatch', refresh_retry_kind = 'transient_network'").execute(&pool).await.expect("complete retry gate is valid");
}

#[tokio::test]
async fn postgres_retry_gate_requires_its_mode_phase_and_kind() {
    let pool = postgres_credential().await;
    for (mode, phase, kind) in [
        (Some("never"), None, Some("transient_network")),
        (Some("never"), Some("before_dispatch"), None),
        (None, Some("before_dispatch"), Some("transient_network")),
    ] {
        assert!(sqlx::query("UPDATE credentials SET refresh_retry_mode = $1, refresh_retry_phase = $2, refresh_retry_kind = $3")
            .bind(mode).bind(phase).bind(kind).execute(&pool).await.is_err(), "retry gate cannot have a missing required field");
    }
    sqlx::query("UPDATE credentials SET refresh_retry_mode = 'never', refresh_retry_phase = 'before_dispatch', refresh_retry_kind = 'transient_network'").execute(&pool).await.expect("complete retry gate is valid");
}

#[tokio::test]
async fn sqlite_physical_name_requires_matching_metadata_projection() {
    let pool = sqlite_credential().await;
    assert!(
        sqlx::query("UPDATE credentials SET name = 'Unprojected'")
            .execute(&pool)
            .await
            .is_err(),
        "physical name cannot lack its metadata display projection"
    );
    sqlx::query("UPDATE credentials SET name = 'Projected', metadata = '{\"display\":{\"display_name\":\"Projected\"}}'").execute(&pool).await.expect("matching name projection is valid");
}

#[tokio::test]
async fn postgres_physical_name_requires_matching_metadata_projection() {
    let pool = postgres_credential().await;
    assert!(
        sqlx::query("UPDATE credentials SET name = 'Unprojected'")
            .execute(&pool)
            .await
            .is_err(),
        "physical name cannot lack its metadata display projection"
    );
    sqlx::query("UPDATE credentials SET name = 'Projected', metadata = '{\"display\":{\"display_name\":\"Projected\"}}'::jsonb").execute(&pool).await.expect("matching name projection is valid");
}

#[tokio::test]
async fn sqlite_reconciliation_requires_decision_and_digest_together() {
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .expect("SQLite deployment");
    let scope = Scope::new("incident-workspace", "incident-org");
    execution_parents::provision_scope(
        &nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone()),
        &scope,
    )
    .await;
    let id = CredentialId::new();
    let selector = CredentialSelector::new(CredentialOwner::from_scope(&scope), id);
    let store = nebula_storage::credential::SqliteCredentialPersistence::connect_pool(pool.clone())
        .await
        .expect("credential store");
    store
        .create(&selector, material())
        .await
        .expect("credential parent");
    sqlx::query("INSERT INTO credential_refresh_incidents (org_id,workspace_id,credential_id,claim_id,detected_at,crashed_holder,generation,operation_kind) VALUES (?,?,?,'incident',1,'fixture',1,'refresh')")
        .bind(&scope.org_id).bind(&scope.workspace_id).bind(id.to_string()).execute(&pool).await.expect("unresolved incident is valid");
    assert!(sqlx::query("UPDATE credential_refresh_incidents SET operation_kind = 'revoke' WHERE claim_id = 'incident'").execute(&pool).await.is_err(), "revoke incident must identify the observed material epoch");
    assert!(sqlx::query("INSERT INTO credential_refresh_claims (org_id,workspace_id,credential_id,claim_id,generation,holder_replica_id,acquired_at,expires_at,sentinel,operation_kind) VALUES (?,?,?,'claim',1,'fixture',1,2,0,'revoke')")
        .bind(&scope.org_id).bind(&scope.workspace_id).bind(id.to_string()).execute(&pool).await.is_err(), "revoke claim must identify the observed material epoch");
    for (decision, digest) in [
        (None, Some(vec![0_u8; 32])),
        (Some("provider_applied"), None),
    ] {
        let result = sqlx::query("UPDATE credential_refresh_incidents SET adjudicated_at = 2, adjudication_decision = ?, adjudication_evidence = 'provider audit', adjudication_evidence_digest = ? WHERE claim_id = 'incident'")
            .bind(decision).bind(digest).execute(&pool).await;
        assert!(
            result.is_err(),
            "SQLite must reject partial reconciliation: decision={decision:?}"
        );
    }
    sqlx::query("UPDATE credential_refresh_incidents SET adjudicated_at = 2, adjudication_decision = 'provider_applied', adjudication_evidence = 'provider audit', adjudication_evidence_digest = zeroblob(32) WHERE claim_id = 'incident'").execute(&pool).await.expect("complete reconciliation is valid");
}

#[tokio::test]
async fn postgres_reconciliation_requires_decision_and_digest_together() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL required for PostgreSQL evidence");
    let pool = postgres_schema::connect_with_private_schema(&url, "nebula_incident")
        .await
        .expect("PostgreSQL deployment");
    nebula_storage::postgres::init_schema(&pool)
        .await
        .expect("admit schema");
    let scope = Scope::new("incident-workspace", "incident-org");
    execution_parents::provision_scope(
        &nebula_storage::postgres::PgTenantProvisioningStore::new(pool.clone()),
        &scope,
    )
    .await;
    let id = CredentialId::new();
    let selector = CredentialSelector::new(CredentialOwner::from_scope(&scope), id);
    let store = nebula_storage::credential::PgCredentialPersistence::connect_pool(pool.clone())
        .await
        .expect("credential store");
    store
        .create(&selector, material())
        .await
        .expect("credential parent");
    let incident = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO credential_refresh_incidents (org_id,workspace_id,credential_id,claim_id,detected_at,crashed_holder,generation,operation_kind) VALUES ($1,$2,$3,$4,clock_timestamp(),'fixture',1,'refresh')")
        .bind(&scope.org_id).bind(&scope.workspace_id).bind(id.to_string()).bind(incident).execute(&pool).await.expect("unresolved incident is valid");
    assert!(
        sqlx::query(
            "UPDATE credential_refresh_incidents SET operation_kind = 'revoke' WHERE claim_id = $1"
        )
        .bind(incident)
        .execute(&pool)
        .await
        .is_err(),
        "revoke incident must identify the observed material epoch"
    );
    assert!(sqlx::query("INSERT INTO credential_refresh_claims (org_id,workspace_id,credential_id,claim_id,generation,holder_replica_id,acquired_at,expires_at,sentinel,operation_kind) VALUES ($1,$2,$3,$4,1,'fixture',clock_timestamp(),clock_timestamp() + INTERVAL '1 second',false,'revoke')")
        .bind(&scope.org_id).bind(&scope.workspace_id).bind(id.to_string()).bind(uuid::Uuid::new_v4()).execute(&pool).await.is_err(), "revoke claim must identify the observed material epoch");
    for (decision, digest) in [
        (None, Some(vec![0_u8; 32])),
        (Some("provider_applied"), None),
    ] {
        let result = sqlx::query("UPDATE credential_refresh_incidents SET adjudicated_at = clock_timestamp(), adjudication_decision = $1, adjudication_evidence = 'provider audit', adjudication_evidence_digest = $2 WHERE claim_id = $3")
            .bind(decision).bind(digest).bind(incident).execute(&pool).await;
        assert!(
            result.is_err(),
            "PostgreSQL must reject partial reconciliation: decision={decision:?}"
        );
    }
    sqlx::query("UPDATE credential_refresh_incidents SET adjudicated_at = clock_timestamp(), adjudication_decision = 'provider_applied', adjudication_evidence = 'provider audit', adjudication_evidence_digest = $1 WHERE claim_id = $2")
        .bind(vec![0_u8;32]).bind(incident).execute(&pool).await.expect("complete reconciliation is valid");
}
