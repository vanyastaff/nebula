use std::{str::FromStr, sync::Arc, time::Duration};

use nebula_storage_port::{
    CredentialMaterialEpoch, CredentialMaterialTransition, CredentialOperationKind,
    CredentialOperationStatus, CredentialOwner, CredentialPersistence, CredentialPersistenceError,
    CredentialReplacement, CredentialSelector, CredentialVersion, RefreshRetryTransition,
    StoredCredential,
    store::{ClaimAttempt, CredentialOperationIntent, RefreshClaimStore, ReplicaId},
};

use crate::credential::test_support::{make_credential, make_replacement};

use super::{
    ReadinessTestGate, SQLITE_MIGRATOR, SqliteCredentialPersistence, TerminalSetupTestGate, schema,
};
use crate::credential::{CredentialSchemaAdmissionReason, CredentialStoreStartupError};

fn version(value: i64) -> CredentialVersion {
    CredentialVersion::try_from(value).expect("test version must be valid")
}

#[tokio::test]
async fn revoke_claim_blocks_authority_replacement_and_is_visible_to_operational_reads() {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("ready store");
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("revoke-fence-owner"),
        nebula_core::CredentialId::new(),
    );
    store
        .create(&selector, make_credential(b"material"))
        .await
        .expect("create");
    let repo = store.refresh_claim_repo();
    assert!(matches!(
        repo.try_claim(
            &selector,
            &ReplicaId::new("revoke-holder"),
            Duration::from_secs(30),
            CredentialOperationIntent::Revoke {
                material_epoch: CredentialMaterialEpoch::MIN,
            },
        )
        .await
        .expect("claim"),
        ClaimAttempt::Acquired(_)
    ));
    assert_eq!(
        store.operation_status(&selector).await.expect("status"),
        CredentialOperationStatus::InFlight {
            operation: CredentialOperationKind::Revoke,
        }
    );
    assert_eq!(
        store
            .get_operational_head(&selector)
            .await
            .expect("operational head")
            .status(),
        CredentialOperationStatus::InFlight {
            operation: CredentialOperationKind::Revoke,
        }
    );
    assert!(matches!(
        store
            .replace(
                &selector,
                make_replacement(version(1), b"new-material", RefreshRetryTransition::Clear),
            )
            .await,
        Err(CredentialPersistenceError::OperationBlocked {
            operation: CredentialOperationKind::Revoke,
        })
    ));
}

#[tokio::test]
async fn revoke_finalizer_accepts_display_version_churn_and_refuses_wrong_epoch() {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("ready store");
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("revoke-finalizer-owner"),
        nebula_core::CredentialId::new(),
    );
    store
        .create(&selector, make_credential(b"material"))
        .await
        .expect("create");
    let repo = store.refresh_claim_repo();
    let claim = match repo
        .try_claim(
            &selector,
            &ReplicaId::new("revoke-holder"),
            Duration::from_secs(30),
            CredentialOperationIntent::Revoke {
                material_epoch: CredentialMaterialEpoch::MIN,
            },
        )
        .await
        .expect("claim")
    {
        ClaimAttempt::Acquired(claim) => claim,
        other => panic!("revoke claim must be acquired: {other:?}"),
    };
    repo.mark_sentinel(&claim.token).await.expect("sentinel");

    let current = match store.get(&selector).await.expect("live") {
        StoredCredential::Live(current) => current,
        StoredCredential::Tombstoned(_) => panic!("fixture must be live"),
    };
    let mut metadata = current.metadata().clone();
    metadata.insert("display_revision".to_owned(), serde_json::json!(2));
    store
        .replace(
            &selector,
            CredentialReplacement::new(
                current.version(),
                current.data().clone(),
                current.state_kind().to_owned(),
                current.state_version(),
                current.name().map(str::to_owned),
                current.expires_at(),
                current.reauth_required(),
                metadata,
                CredentialMaterialTransition::preserve(RefreshRetryTransition::Preserve),
            ),
        )
        .await
        .expect("display-only replacement");

    let wrong_epoch = CredentialMaterialEpoch::MIN.next().expect("epoch two");
    assert!(matches!(
        store
            .tombstone_revoked_material(&selector, wrong_epoch)
            .await,
        Err(CredentialPersistenceError::OperationBlocked {
            operation: CredentialOperationKind::Revoke,
        })
    ));
    let commit = store
        .tombstone_revoked_material(&selector, CredentialMaterialEpoch::MIN)
        .await
        .expect("pinned revoke finalization");
    assert_eq!(commit.version().get(), 3);
    assert!(matches!(
        store.get(&selector).await.expect("terminal row"),
        StoredCredential::Tombstoned(_)
    ));
}

async fn assert_cancelled_terminal_setup_keeps_lock(
    first: tokio::task::JoinHandle<
        Result<SqliteCredentialPersistence, CredentialStoreStartupError>,
    >,
    gate: Arc<TerminalSetupTestGate>,
    assert_lock_held: impl FnOnce(),
    competitor: impl Future<Output = Result<SqliteCredentialPersistence, CredentialStoreStartupError>>,
) {
    gate.entered.notified().await;
    first.abort();
    assert!(
        first
            .await
            .expect_err("caller must be cancelled")
            .is_cancelled()
    );
    assert_lock_held();
    gate.release.notify_one();
    competitor
        .await
        .expect("competing startup succeeds after terminal completion");
    let pool = gate.pool.get().expect("terminal setup published its pool");
    let applied: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(pool)
            .await
            .expect("cancelled startup completed its canonical ledger");
    assert_eq!(
        usize::try_from(applied).expect("positive count"),
        SQLITE_MIGRATOR.iter().count()
    );
}

#[tokio::test]
async fn cancelled_memory_startup_keeps_terminal_setup_owned() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let gate = Arc::new(TerminalSetupTestGate::default());
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("memory options");
        let first = tokio::spawn(SqliteCredentialPersistence::connect_memory_options(
            options,
            Some(Arc::clone(&gate)),
        ));
        assert_cancelled_terminal_setup_keeps_lock(
            first,
            gate,
            crate::migration::assert_sqlite_memory_setup_locked,
            SqliteCredentialPersistence::connect_memory(),
        )
        .await;
    })
    .await
    .expect("memory terminal setup must complete");
}

#[tokio::test]
async fn cancelled_file_startup_keeps_terminal_setup_owned() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let directory = tempfile::tempdir().expect("temporary directory");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(directory.path().join("cancelled-startup.sqlite"))
            .create_if_missing(true);
        let path = options.get_filename().to_owned();
        let gate = Arc::new(TerminalSetupTestGate::default());
        let first = tokio::spawn(SqliteCredentialPersistence::connect_file_options_inner(
            options.clone(),
            None,
            Some(Arc::clone(&gate)),
        ));
        assert_cancelled_terminal_setup_keeps_lock(
            first,
            gate,
            || crate::migration::assert_sqlite_file_setup_locked(&path),
            SqliteCredentialPersistence::connect_file_options(options),
        )
        .await;
    })
    .await
    .expect("file terminal setup must complete");
}

#[test]
fn refresh_retry_snapshot_is_one_backend_clock_statement() {
    let source = include_str!("sqlite.rs");
    let body = source
        .rsplit_once("impl CredentialPersistence for SqliteCredentialPersistence {")
        .expect("credential persistence implementation must exist")
        .1
        .split_once("async fn refresh_retry_snapshot(")
        .expect("snapshot method must exist")
        .1
        .split_once("\n    #[tracing::instrument")
        .expect("the following port method must delimit the snapshot body")
        .0;

    assert_eq!(body.matches("sqlx::query_as(").count(), 1);
    assert!(body.contains("SELECT version, material_epoch, reauth_required, record_state"));
    assert!(body.contains("strftime('%f', 'now')"));
    assert!(!body.contains("self.get("));
}

#[tokio::test]
async fn curated_refresh_claim_repositories_share_the_admitted_private_pool() {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("ready in-memory credential store");
    let first = store.refresh_claim_repo();
    let second = store.refresh_claim_repo();
    let credential_id = nebula_core::CredentialId::new();
    let selector =
        CredentialSelector::new(CredentialOwner::from_canonical("owner-a"), credential_id);

    let acquired = first
        .try_claim(
            &selector,
            &ReplicaId::new("first"),
            Duration::from_secs(30),
            CredentialOperationIntent::Refresh,
        )
        .await
        .expect("first claim attempt");
    assert!(matches!(acquired, ClaimAttempt::Acquired(_)));

    let observed = second
        .try_claim(
            &selector,
            &ReplicaId::new("second"),
            Duration::from_secs(30),
            CredentialOperationIntent::Refresh,
        )
        .await
        .expect("second claim attempt");
    assert!(
        matches!(observed, ClaimAttempt::Contended { .. }),
        "separately-created adapters must observe the same durable claim row"
    );
}

#[tokio::test]
async fn post_commit_fault_is_outcome_unknown_without_automatic_retry()
-> Result<(), CredentialPersistenceError> {
    let store = SqliteCredentialPersistence::connect_memory().await?;
    let owner = CredentialOwner::from_canonical("post-commit-fault-owner");
    let selector = CredentialSelector::new(owner.clone(), nebula_core::CredentialId::new());
    store
        .create(&selector, make_credential(b"version-one"))
        .await?;

    store.arm_post_commit_outcome_unknown();
    let result = store
        .replace(
            &selector,
            make_replacement(version(1), b"version-two", RefreshRetryTransition::Clear),
        )
        .await;
    assert_eq!(result, Err(CredentialPersistenceError::OutcomeUnknown));
    assert_eq!(
        store.injected_post_commit_faults(),
        1,
        "one caller attempt must cross the post-commit fault exactly once"
    );

    let StoredCredential::Live(persisted) = store.get(&selector).await? else {
        panic!("the acknowledged SQL transaction must have persisted a live replacement");
    };
    assert_eq!(persisted.version(), version(2));
    assert_eq!(persisted.data().as_ref(), b"version-two");

    let foreign = CredentialSelector::new(
        CredentialOwner::from_canonical("post-commit-fault-foreign"),
        selector.credential_id(),
    );
    assert_eq!(
        store.get(&foreign).await,
        Err(CredentialPersistenceError::NotFound),
        "the verification read remains owner-qualified"
    );
    Ok(())
}

#[tokio::test]
async fn confirmed_precommit_rollback_is_unavailable_and_preserves_prior_row()
-> Result<(), CredentialPersistenceError> {
    let store = SqliteCredentialPersistence::connect_memory().await?;
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("precommit-rollback-owner"),
        nebula_core::CredentialId::new(),
    );
    store
        .create(&selector, make_credential(b"version-one"))
        .await?;

    store.arm_precommit_failure();
    assert_eq!(
        store
            .replace(
                &selector,
                make_replacement(version(1), b"rolled-back", RefreshRetryTransition::Clear,),
            )
            .await,
        Err(CredentialPersistenceError::Unavailable)
    );
    let StoredCredential::Live(persisted) = store.get(&selector).await? else {
        panic!("confirmed rollback must preserve the prior live row");
    };
    assert_eq!(persisted.version(), version(1));
    assert_eq!(persisted.data().as_ref(), b"version-one");
    assert_eq!(store.injected_post_commit_faults(), 0);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn second_starter_waits_while_first_holds_the_readiness_lock() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let directory = tempfile::tempdir().expect("temporary directory must be created");
            let path = directory.path().join("contended-readiness.sqlite");
            let url = format!("sqlite://{}?mode=rwc", path.display());
            let options = sqlx::sqlite::SqliteConnectOptions::from_str(&url)
                .expect("temporary SQLite URL must parse")
                .create_if_missing(true);
            let gate = Arc::new(ReadinessTestGate::new());

            let first_gate = Arc::clone(&gate);
            let first = tokio::task::spawn_local(async move {
                SqliteCredentialPersistence::connect_file_options_with_gate(options, &first_gate)
                    .await
            });
            gate.lock_acquired.wait().await;

            let contender_started = Arc::new(tokio::sync::Barrier::new(2));
            let contender_signal = Arc::clone(&contender_started);
            let contender_url = url.clone();
            let mut second = tokio::task::spawn_local(async move {
                contender_signal.wait().await;
                SqliteCredentialPersistence::connect(&contender_url).await
            });
            contender_started.wait().await;
            assert!(
                tokio::time::timeout(Duration::from_millis(75), &mut second)
                    .await
                    .is_err(),
                "the second starter must remain blocked while the first owns the file lock"
            );

            gate.release.notify_one();
            let first = first
                .await
                .expect("first starter task must not panic")
                .expect("first starter must establish the ready schema");
            let second = second
                .await
                .expect("second starter task must not panic")
                .expect("second starter must observe the ready schema");

            let (head, successful): (i64, i64) = sqlx::query_as(
                "SELECT MAX(version), COUNT(*) FROM _sqlx_migrations WHERE success = 1",
            )
            .fetch_one(&second.pool)
            .await
            .expect("serialized readiness ledger must be readable");
            assert_eq!(
                head,
                crate::migration::catalog::catalog_head(&SQLITE_MIGRATOR)
            );
            assert_eq!(
                successful,
                i64::try_from(SQLITE_MIGRATOR.iter().count())
                    .expect("migration count must fit in i64")
            );
            drop((first, second));
        })
        .await;
}

#[tokio::test]
async fn rejected_memory_admission_preserves_logical_state() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory rejection fixture must connect");
    sqlx::query("CREATE TABLE unrelated (value TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("unledgered relation must seed");
    sqlx::query("INSERT INTO unrelated (value) VALUES ('preserve')")
        .execute(&pool)
        .await
        .expect("unledgered row must seed");

    let schema_before: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, sql FROM sqlite_schema
         WHERE type = 'table' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("fixture schema must snapshot");
    let rows_before: Vec<String> = sqlx::query_scalar("SELECT value FROM unrelated ORDER BY value")
        .fetch_all(&pool)
        .await
        .expect("fixture rows must snapshot");

    let mut connection = pool.acquire().await.expect("single memory connection");
    let error = schema::admit(&mut connection)
        .await
        .expect_err("unledgered memory database must fail closed");
    assert!(matches!(
        error,
        CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
            if unsupported.reason()
                == &CredentialSchemaAdmissionReason::UnledgeredDatabase
    ));
    drop(connection);

    let schema_after: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, sql FROM sqlite_schema
         WHERE type = 'table' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("rejected schema must remain readable");
    let rows_after: Vec<String> = sqlx::query_scalar("SELECT value FROM unrelated ORDER BY value")
        .fetch_all(&pool)
        .await
        .expect("rejected rows must remain readable");
    assert_eq!(schema_after, schema_before);
    assert_eq!(rows_after, rows_before);
    pool.close().await;
}
