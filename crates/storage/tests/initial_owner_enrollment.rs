//! Durable initial ownership through the production SQLite and PostgreSQL adapters.

#![cfg(any(feature = "sqlite", feature = "postgres"))]

use chrono::Utc;
use nebula_core::{OrgId, UserId, WorkspaceId};
use nebula_storage::{
    StorageError,
    auth::{InitialOwnerBegin, InitialOwnerRegistration, InitialOwnerStatus, UserRepo, UserRow},
};
use nebula_storage_port::{
    dto::{
        OrgRow, PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate,
        TenantProvisioningRequest,
    },
    store::OrgStore,
};

mod support {
    #[cfg(feature = "postgres")]
    pub(crate) mod postgres_schema;
}

#[derive(Clone, Copy)]
enum BackendKind {
    #[cfg(feature = "sqlite")]
    Sqlite,
    #[cfg(feature = "postgres")]
    Postgres,
}

#[derive(Clone)]
enum Database {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlx::SqlitePool),
    #[cfg(feature = "postgres")]
    Postgres(sqlx::PgPool),
}

impl Database {
    async fn migrate_to_previous_head(&self) {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => sqlx::migrate!("./migrations/sqlite")
                .run_to(9, pool)
                .await
                .unwrap(),
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => sqlx::migrate!("./migrations/postgres")
                .run_to(9, pool)
                .await
                .unwrap(),
        }
    }

    async fn migrate(&self) {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => nebula_storage::sqlite::init_schema(pool).await.unwrap(),
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => nebula_storage::postgres::init_schema(pool).await.unwrap(),
        }
    }

    async fn begin(
        &self,
        owner: UserId,
        request: &TenantProvisioningRequest,
    ) -> Result<InitialOwnerBegin, StorageError> {
        let email = format!("{owner}@example.test");
        let registration = InitialOwnerRegistration {
            user_id: owner,
            email: &email,
            display_name: "Initial owner",
            password_hash: "prepared-password-hash",
            tenant_request: request,
        };
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                nebula_storage::auth::sqlite::SqliteAccountLifecycle::new(pool.clone())
                    .begin_initial_owner(&registration)
                    .await
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                nebula_storage::auth::postgres::PgAccountLifecycle::new(pool.clone())
                    .begin_initial_owner(&registration)
                    .await
            },
        }
    }

    async fn status(&self) -> Result<InitialOwnerStatus, StorageError> {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone())
                    .initial_owner_status()
                    .await
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                nebula_storage::postgres::PgTenantProvisioningStore::new(pool.clone())
                    .initial_owner_status()
                    .await
            },
        }
    }

    async fn accept(&self) -> Result<InitialOwnerStatus, StorageError> {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone())
                    .accept_initial_owner()
                    .await
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                nebula_storage::postgres::PgTenantProvisioningStore::new(pool.clone())
                    .accept_initial_owner()
                    .await
            },
        }
    }

    fn users(&self) -> Box<dyn UserRepo> {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => Box::new(nebula_storage::auth::sqlite::SqliteUserRepo::new(
                pool.clone(),
            )),
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => Box::new(nebula_storage::auth::postgres::PgUserRepo::new(
                pool.clone(),
            )),
        }
    }

    fn orgs(&self) -> Box<dyn OrgStore> {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                Box::new(nebula_storage::sqlite::SqliteOrgStore::new(pool.clone()))
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                Box::new(nebula_storage::postgres::PgOrgStore::new(pool.clone()))
            },
        }
    }

    async fn count(&self, table: &'static str) -> i64 {
        let sql = sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}"));
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => sqlx::query_scalar(sql).fetch_one(pool).await.unwrap(),
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => sqlx::query_scalar(sql).fetch_one(pool).await.unwrap(),
        }
    }

    async fn execute(&self, statement: &'static str) {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                sqlx::raw_sql(statement).execute(pool).await.unwrap();
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                sqlx::raw_sql(statement).execute(pool).await.unwrap();
            },
        }
    }

    async fn replace_request(&self, value: serde_json::Value) {
        let sql = "UPDATE initial_owner_enrollment SET tenant_request = $1 WHERE singleton = 1";
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => {
                sqlx::query(sql).bind(value).execute(pool).await.unwrap();
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                sqlx::query(sql).bind(value).execute(pool).await.unwrap();
            },
        }
    }

    async fn stored_request(&self) -> serde_json::Value {
        let sql = "SELECT tenant_request FROM initial_owner_enrollment WHERE singleton = 1";
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => sqlx::query_scalar(sql).fetch_one(pool).await.unwrap(),
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => sqlx::query_scalar(sql).fetch_one(pool).await.unwrap(),
        }
    }

    async fn reject_account_insert(&self) {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(_) => self.execute("CREATE TRIGGER reject_enrollment_user BEFORE INSERT ON users BEGIN SELECT RAISE(ABORT, 'secret-insert-canary'); END;").await,
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => self.execute("CREATE FUNCTION reject_enrollment_user() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'secret-insert-canary'; END $$; CREATE TRIGGER reject_enrollment_user BEFORE INSERT ON users FOR EACH ROW EXECUTE FUNCTION reject_enrollment_user();").await,
        }
    }

    async fn permit_account_insert(&self) {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(_) => self.execute("DROP TRIGGER reject_enrollment_user").await,
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => self.execute("DROP TRIGGER reject_enrollment_user ON users; DROP FUNCTION reject_enrollment_user();").await,
        }
    }

    async fn cleanup(&self) {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => pool.close().await,
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                let schema: String = sqlx::query_scalar("SELECT current_schema()")
                    .fetch_one(pool)
                    .await
                    .unwrap();
                assert!(schema.bytes().all(|byte| byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || byte == b'_'));
                sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
                    .execute(pool)
                    .await
                    .unwrap();
                pool.close().await;
            },
        }
    }
}

fn tenant_request(owner: UserId) -> TenantProvisioningRequest {
    let owner = owner.to_string();
    TenantProvisioningRequest::new(
        TenantOrgCreate::new(
            OrgId::new().to_string(),
            "personal".into(),
            "Personal".into(),
            owner.clone(),
            "free".into(),
            None,
            serde_json::json!({}),
        )
        .unwrap(),
        TenantDefaultWorkspaceCreate::new(
            WorkspaceId::new().to_string(),
            "default".into(),
            "Default".into(),
            None,
            owner.clone(),
            serde_json::json!({}),
        )
        .unwrap(),
        PrincipalKind::User,
        owner,
        Some("operator-enrollment".into()),
    )
    .unwrap()
}

fn ordinary_user(id: UserId) -> UserRow {
    UserRow {
        id: id.as_bytes().to_vec(),
        email: format!("{id}@example.test"),
        email_verified_at: None,
        display_name: "Ordinary account".into(),
        avatar_url: None,
        password_hash: Some("ordinary-password-hash".into()),
        created_at: Utc::now(),
        last_login_at: None,
        locked_until: None,
        failed_login_count: 0,
        mfa_enabled: false,
        mfa_secret_envelope: None,
        version: 0,
        deleted_at: None,
    }
}

fn existing_org(owner: UserId) -> OrgRow {
    OrgRow {
        id: OrgId::new().to_string(),
        slug: "personal".into(),
        display_name: "Existing organization".into(),
        created_at: Utc::now(),
        created_by: owner.to_string(),
        plan: "free".into(),
        billing_email: None,
        settings: serde_json::json!({}),
        version: 0,
        deleted_at: None,
    }
}

#[derive(Clone, Copy, Debug)]
enum Case {
    PendingAndAccepted,
    ConcurrentBegin,
    ArchivedBeforeAcceptance,
    PurgedBeforeAcceptance,
    HistoricalAcceptance,
    BeginRollback,
    OrdinaryAccountSeals,
    MigrationSealsUser,
    MigrationSealsOrg,
    CorruptRequest,
    OccupiedSlug,
}

async fn run(kind: BackendKind, case: Case) {
    let directory = tempfile::tempdir().unwrap();
    let database = match kind {
        #[cfg(feature = "sqlite")]
        BackendKind::Sqlite => {
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(4)
                .connect_with(
                    sqlx::sqlite::SqliteConnectOptions::new()
                        .filename(directory.path().join("enrollment.db"))
                        .create_if_missing(true)
                        .foreign_keys(true)
                        .busy_timeout(std::time::Duration::from_secs(5)),
                )
                .await
                .unwrap();
            Database::Sqlite(pool)
        },
        #[cfg(feature = "postgres")]
        BackendKind::Postgres => {
            let url = std::env::var("DATABASE_URL")
                .expect("DATABASE_URL required for PostgreSQL enrollment proof");
            let pool =
                support::postgres_schema::connect_with_private_schema(&url, "nebula_initial_owner")
                    .await
                    .unwrap();
            Database::Postgres(pool)
        },
    };
    // A task boundary preserves the assertion failure while cleanup runs even
    // when the shared contract panics. Every PostgreSQL case drops its schema.
    let tested = database.clone();
    let result = tokio::spawn(async move { exercise(&tested, case).await }).await;
    database.cleanup().await;
    drop(directory);
    if let Err(error) = result {
        std::panic::resume_unwind(error.into_panic());
    }
}

async fn exercise(database: &Database, case: Case) {
    database.migrate_to_previous_head().await;
    let owner = UserId::new();
    let request = tenant_request(owner);
    match case {
        Case::MigrationSealsUser => {
            database
                .users()
                .create(&ordinary_user(owner))
                .await
                .unwrap();
            database
                .users()
                .soft_delete(&owner.as_bytes())
                .await
                .unwrap();
        },
        Case::MigrationSealsOrg => {
            database.orgs().create(existing_org(owner)).await.unwrap();
        },
        _ => {},
    }
    database.migrate().await;
    match case {
        Case::MigrationSealsUser | Case::MigrationSealsOrg => {
            assert_eq!(database.status().await.unwrap(), InitialOwnerStatus::Sealed);
            database.execute("DELETE FROM users").await;
            database.execute("DELETE FROM orgs").await;
            assert_eq!(
                database.begin(owner, &request).await.unwrap(),
                InitialOwnerBegin::Unavailable
            );
            assert_eq!(database.accept().await.unwrap(), InitialOwnerStatus::Sealed);
            assert_eq!(database.count("users").await, 0);
            assert_eq!(database.count("orgs").await, 0);
        },
        Case::OrdinaryAccountSeals => {
            database
                .users()
                .create(&ordinary_user(owner))
                .await
                .unwrap();
            assert_eq!(database.status().await.unwrap(), InitialOwnerStatus::Sealed);
            database.execute("DELETE FROM users").await;
            assert_eq!(
                database.begin(owner, &request).await.unwrap(),
                InitialOwnerBegin::Unavailable
            );
            assert_eq!(database.count("users").await, 0);
        },
        Case::ConcurrentBegin => {
            let other = UserId::new();
            let other_request = tenant_request(other);
            let (first, second) = tokio::join!(
                database.begin(owner, &request),
                database.begin(other, &other_request)
            );
            let outcomes = [first.unwrap(), second.unwrap()];
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|outcome| **outcome == InitialOwnerBegin::Begun)
                    .count(),
                1
            );
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|outcome| **outcome == InitialOwnerBegin::AlreadyStarted)
                    .count(),
                1
            );
            assert_eq!(database.count("users").await, 1);
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::Pending
            );
            assert_eq!(
                database.accept().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            let winning_request = if outcomes[0] == InitialOwnerBegin::Begun {
                &request
            } else {
                &other_request
            };
            assert!(
                database
                    .orgs()
                    .get(winning_request.org().id())
                    .await
                    .unwrap()
                    .is_some()
            );
            assert_eq!(database.count("orgs").await, 1);
        },
        Case::BeginRollback => {
            database.reject_account_insert().await;
            let error = database.begin(owner, &request).await.unwrap_err();
            assert!(!format!("{error:?} {error}").contains("secret-insert-canary"));
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::Available
            );
            assert_eq!(database.count("users").await, 0);
            assert_eq!(database.count("orgs").await, 0);
            database.permit_account_insert().await;
            assert_eq!(
                database.begin(owner, &request).await.unwrap(),
                InitialOwnerBegin::Begun
            );
        },
        _ => {
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::Available
            );
            assert_eq!(
                database.begin(owner, &request).await.unwrap(),
                InitialOwnerBegin::Begun
            );
            exercise_started(database, case, owner, &request).await;
        },
    }
}

async fn exercise_started(
    database: &Database,
    case: Case,
    owner: UserId,
    request: &TenantProvisioningRequest,
) {
    match case {
        Case::OccupiedSlug => {
            let existing = existing_org(owner);
            let existing_id = existing.id.clone();
            database.orgs().create(existing).await.unwrap();
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::Conflict
            );
            assert_eq!(
                database.accept().await.unwrap(),
                InitialOwnerStatus::Conflict
            );
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::Conflict
            );
            assert!(
                database
                    .orgs()
                    .get(request.org().id())
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(database.orgs().get(&existing_id).await.unwrap().is_some());
            assert_eq!(database.count("orgs").await, 1);
            assert_eq!(database.count("org_memberships").await, 0);
        },
        Case::PendingAndAccepted => {
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::Pending
            );
            let user = database
                .users()
                .get(&owner.as_bytes())
                .await
                .unwrap()
                .unwrap();
            assert!(user.email_verified_at.is_none());
            assert_eq!(
                user.password_hash.as_deref(),
                Some("prepared-password-hash")
            );
            assert_eq!(database.count("verification_tokens").await, 0);
            assert_eq!(database.count("sessions").await, 0);
            assert_eq!(database.count("orgs").await, 0);
            assert_eq!(
                database.accept().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            assert_eq!(
                database.accept().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            assert_eq!(database.count("org_memberships").await, 1);
            assert_eq!(database.count("tenant_provisioning_receipts").await, 1);
            let other = UserId::new();
            assert_eq!(
                database.begin(other, &tenant_request(other)).await.unwrap(),
                InitialOwnerBegin::AlreadyStarted
            );
            assert!(
                database
                    .users()
                    .get(&other.as_bytes())
                    .await
                    .unwrap()
                    .is_none()
            );
        },
        Case::ArchivedBeforeAcceptance | Case::PurgedBeforeAcceptance => {
            if matches!(case, Case::ArchivedBeforeAcceptance) {
                database
                    .users()
                    .soft_delete(&owner.as_bytes())
                    .await
                    .unwrap();
            } else {
                database.execute("DELETE FROM users").await;
            }
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::OwnerUnavailable
            );
            assert_eq!(
                database.accept().await.unwrap(),
                InitialOwnerStatus::OwnerUnavailable
            );
            assert_eq!(database.count("orgs").await, 0);
            assert_eq!(database.count("org_memberships").await, 0);
            assert_eq!(database.count("tenant_provisioning_receipts").await, 0);
            assert_eq!(
                database.begin(owner, request).await.unwrap(),
                InitialOwnerBegin::AlreadyStarted
            );
        },
        Case::HistoricalAcceptance => {
            assert_eq!(
                database.accept().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            database.execute("DELETE FROM org_memberships").await;
            assert_eq!(
                database.accept().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            assert_eq!(database.count("org_memberships").await, 0);
            database
                .users()
                .soft_delete(&owner.as_bytes())
                .await
                .unwrap();
            database
                .orgs()
                .soft_delete(request.org().id())
                .await
                .unwrap();
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            assert_eq!(
                database.accept().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            database.execute("DELETE FROM users").await;
            database.execute("DELETE FROM orgs").await;
            assert_eq!(
                database.status().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            assert_eq!(
                database.accept().await.unwrap(),
                InitialOwnerStatus::Accepted
            );
            assert_eq!(
                database.begin(owner, request).await.unwrap(),
                InitialOwnerBegin::AlreadyStarted
            );
            for table in ["users", "orgs", "workspaces", "org_memberships"] {
                assert_eq!(database.count(table).await, 0, "replay recreated {table}");
            }
            assert_eq!(database.count("tenant_provisioning_receipts").await, 1);
        },
        Case::CorruptRequest => {
            let original = database.stored_request().await;
            let mut unknown_version = original.clone();
            unknown_version["version"] = serde_json::json!(999);
            let mut wrong_owner = original;
            wrong_owner["owner"]["id"] = serde_json::json!("secret-corrupt-owner-canary");
            for value in [
                unknown_version,
                wrong_owner,
                serde_json::json!({"private": "secret-corrupt-owner-canary"}),
            ] {
                database.replace_request(value).await;
                for result in [database.status().await, database.accept().await] {
                    let error = result.unwrap_err();
                    assert!(matches!(error, StorageError::Corrupt(_)));
                    assert!(!format!("{error:?} {error}").contains("secret-corrupt-owner-canary"));
                }
                assert!(matches!(
                    database.begin(owner, request).await,
                    Err(StorageError::Corrupt(_))
                ));
                assert_eq!(database.count("orgs").await, 0);
                assert_eq!(database.count("tenant_provisioning_receipts").await, 0);
            }
        },
        _ => unreachable!("case does not start enrollment"),
    }
}

macro_rules! enrollment_cases {
    ($name:ident, $backend:expr, $(#[$attribute:meta])*) => {
        #[rstest::rstest]
        #[case::pending_and_accepted(Case::PendingAndAccepted)]
        #[case::concurrent_begin(Case::ConcurrentBegin)]
        #[case::archived_before_acceptance(Case::ArchivedBeforeAcceptance)]
        #[case::purged_before_acceptance(Case::PurgedBeforeAcceptance)]
        #[case::historical_acceptance(Case::HistoricalAcceptance)]
        #[case::begin_rollback(Case::BeginRollback)]
        #[case::ordinary_account_seals(Case::OrdinaryAccountSeals)]
        #[case::migration_seals_user(Case::MigrationSealsUser)]
        #[case::migration_seals_org(Case::MigrationSealsOrg)]
        #[case::corrupt_request(Case::CorruptRequest)]
        #[case::occupied_slug(Case::OccupiedSlug)]
        $(#[$attribute])*
        #[tokio::test]
        async fn $name(#[case] case: Case) {
            run($backend, case).await;
        }
    };
}

#[cfg(feature = "sqlite")]
enrollment_cases!(sqlite, BackendKind::Sqlite,);

#[cfg(feature = "postgres")]
enrollment_cases!(postgres, BackendKind::Postgres, #[ignore = "requires live DATABASE_URL"]);

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "requires live DATABASE_URL"]
async fn postgres_acceptance_waits_for_archive_then_rejects() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
    let pool = support::postgres_schema::connect_with_private_schema(&url, "nebula_owner_archive")
        .await
        .unwrap();
    let database = Database::Postgres(pool.clone());
    let tested = database.clone();
    let result = tokio::spawn(async move {
        tested.migrate().await;
        let owner = UserId::new();
        let request = tenant_request(owner);
        assert_eq!(tested.begin(owner, &request).await.unwrap(), InitialOwnerBegin::Begun);
        let mut archive = pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *archive).await.unwrap();
        sqlx::query("UPDATE users SET deleted_at = clock_timestamp() WHERE id = $1")
            .bind(owner.as_bytes().as_slice()).execute(&mut *archive).await.unwrap();
        let accepting = tested.clone();
        let acceptance = tokio::spawn(async move { accepting.accept().await });
        let observed_wait = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let blocked: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
                ).bind(blocker_pid).fetch_one(&pool).await.unwrap();
                if blocked { break; }
                tokio::task::yield_now().await;
            }
        }).await;
        if observed_wait.is_err() {
            acceptance.abort();
            let _ = acceptance.await;
            archive.rollback().await.unwrap();
            panic!("acceptance never waited on the held account archive lock");
        }
        assert!(!acceptance.is_finished());
        archive.commit().await.unwrap();
        assert_eq!(acceptance.await.unwrap().unwrap(), InitialOwnerStatus::OwnerUnavailable);
        assert_eq!(tested.status().await.unwrap(), InitialOwnerStatus::OwnerUnavailable);
        assert_eq!(tested.count("orgs").await, 0);
        assert_eq!(tested.count("tenant_provisioning_receipts").await, 0);
    }).await;
    database.cleanup().await;
    if let Err(error) = result {
        std::panic::resume_unwind(error.into_panic());
    }
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "requires live DATABASE_URL"]
async fn postgres_begin_waits_for_ordinary_signup_then_stays_sealed() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
    let pool = support::postgres_schema::connect_with_private_schema(&url, "nebula_owner_signup")
        .await
        .unwrap();
    let database = Database::Postgres(pool.clone());
    let tested = database.clone();
    let result = tokio::spawn(async move {
        tested.migrate().await;
        let ordinary = UserId::new();
        let mut signup = pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *signup).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, display_name, password_hash, created_at)
             VALUES ($1, 'ordinary@example.test', 'Ordinary account', 'prepared-hash', clock_timestamp())",
        ).bind(ordinary.as_bytes().as_slice()).execute(&mut *signup).await.unwrap();
        let state: String = sqlx::query_scalar(
            "SELECT state FROM initial_owner_enrollment WHERE singleton = 1",
        ).fetch_one(&mut *signup).await.unwrap();
        assert_eq!(state, "sealed", "the user trigger sealed eligibility before commit");

        let owner = UserId::new();
        let request = tenant_request(owner);
        let enrolling = tested.clone();
        let enrollment = tokio::spawn(async move { enrolling.begin(owner, &request).await });
        let observed_wait = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let blocked: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
                ).bind(blocker_pid).fetch_one(&pool).await.unwrap();
                if blocked { break; }
                tokio::task::yield_now().await;
            }
        }).await;
        if observed_wait.is_err() {
            enrollment.abort();
            let _ = enrollment.await;
            signup.rollback().await.unwrap();
            panic!("initial enrollment never waited on the singleton held by ordinary signup");
        }
        assert!(!enrollment.is_finished());
        signup.commit().await.unwrap();
        assert_eq!(enrollment.await.unwrap().unwrap(), InitialOwnerBegin::Unavailable);
        assert_eq!(tested.status().await.unwrap(), InitialOwnerStatus::Sealed);
        assert_eq!(tested.accept().await.unwrap(), InitialOwnerStatus::Sealed);
        assert!(tested.users().get(&ordinary.as_bytes()).await.unwrap().is_some());
        assert!(tested.users().get(&owner.as_bytes()).await.unwrap().is_none());
        assert_eq!(tested.count("users").await, 1);
        assert_eq!(tested.count("orgs").await, 0);
        assert_eq!(tested.count("tenant_provisioning_receipts").await, 0);
    }).await;
    database.cleanup().await;
    if let Err(error) = result {
        std::panic::resume_unwind(error.into_panic());
    }
}
