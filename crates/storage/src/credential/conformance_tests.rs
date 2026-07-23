//! One backend-neutral semantic oracle for the credential lifecycle.
//!
//! The reference adapter, SQLite, and PostgreSQL all execute the same body.
//! SQL-specific migration, restart, race, and transport-fault evidence lives in
//! the dedicated backend tests; this file proves behavioural parity.

use std::error::Error;

use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialAlreadyExistsKey, CredentialCreate, CredentialOwner, CredentialPersistenceError,
    CredentialRecordState, CredentialReplacement, CredentialSelector, CredentialTombstone,
    CredentialVersion, SecretBytes, StoredCredential,
};
use serde_json::{Map, Value};

#[cfg(feature = "postgres")]
use super::PgCredentialPersistence;
#[cfg(feature = "sqlite")]
use super::SqliteCredentialPersistence;
use super::{CredentialPersistenceConformance, ReferenceCredentialPersistence};

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn owner(value: &str) -> CredentialOwner {
    CredentialOwner::from_canonical(value)
}

fn selector(owner: &CredentialOwner, credential_id: CredentialId) -> CredentialSelector {
    CredentialSelector::new(owner.clone(), credential_id)
}

fn metadata(name: Option<&str>, marker: &str) -> Map<String, Value> {
    let mut metadata = Map::from_iter([("opaque".to_owned(), Value::String(marker.to_owned()))]);
    if let Some(name) = name {
        metadata.insert(
            "display".to_owned(),
            serde_json::json!({"display_name": name}),
        );
    }
    metadata
}

fn create(name: Option<&str>, secret: &[u8], marker: &str) -> CredentialCreate {
    CredentialCreate::new(
        "provider.api-token".to_owned(),
        SecretBytes::new(secret.to_vec()),
        "active".to_owned(),
        7,
        name.map(str::to_owned),
        None,
        false,
        metadata(name, marker),
    )
}

fn replacement(
    expected: CredentialVersion,
    name: Option<&str>,
    secret: &[u8],
    marker: &str,
) -> CredentialReplacement {
    CredentialReplacement::new(
        expected,
        SecretBytes::new(secret.to_vec()),
        "active".to_owned(),
        8,
        name.map(str::to_owned),
        None,
        true,
        metadata(name, marker),
    )
}

async fn run_semantic_oracle<B>(store: &B) -> TestResult<()>
where
    B: CredentialPersistenceConformance + ?Sized,
{
    let owner_a = owner("oracle-owner-a");
    let owner_b = owner("oracle-owner-b");
    let credential_id = CredentialId::new();
    let key = selector(&owner_a, credential_id);

    assert!(store.list(&owner_a, None).await?.is_empty());
    assert!(store.list_heads(&owner_a, None).await?.is_empty());
    assert!(!store.exists(&key).await?);

    let created = store
        .create(&key, create(Some("Production"), b"\x00\xff-v1", "create"))
        .await?;
    assert_eq!(created.credential_id(), credential_id);
    assert_eq!(created.version(), CredentialVersion::MIN);
    assert_eq!(created.state(), CredentialRecordState::Live);
    assert_eq!(created.tombstoned_at(), None);

    let live = match store.get(&key).await? {
        StoredCredential::Live(live) => live,
        StoredCredential::Tombstoned(_) => panic!("a create must produce a live record"),
    };
    assert_eq!(live.credential_id(), credential_id);
    assert_eq!(live.name(), Some("Production"));
    assert_eq!(live.credential_key(), "provider.api-token");
    assert_eq!(live.data().as_ref(), b"\x00\xff-v1");
    assert_eq!(live.version(), created.version());
    assert_eq!(live.created_at(), created.created_at());
    assert_eq!(live.updated_at(), created.updated_at());
    assert!(store.exists(&key).await?);

    let head = store.get_head(&key).await?;
    assert_eq!(head.credential_id(), credential_id);
    assert_eq!(head.version(), CredentialVersion::MIN);
    assert_eq!(head.name(), Some("Production"));
    assert_eq!(store.list(&owner_a, Some("active")).await?, [credential_id]);
    assert_eq!(store.list_heads(&owner_a, None).await?.len(), 1);
    assert!(store.list(&owner_b, None).await?.is_empty());

    assert_eq!(
        store
            .create(&key, create(Some("Other"), b"same-id", "same-id"))
            .await
            .expect_err("same-owner id must remain reserved"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        }
    );
    assert_eq!(
        store
            .create(
                &selector(&owner_b, credential_id),
                create(Some("Foreign"), b"foreign", "foreign"),
            )
            .await
            .expect_err("foreign-owner global id collision is existence-hidden"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .create(
                &selector(&owner_a, CredentialId::new()),
                create(Some("Production"), b"same-name", "same-name"),
            )
            .await
            .expect_err("a live owner-local name is unique"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        }
    );

    let replaced = store
        .replace(
            &key,
            replacement(
                CredentialVersion::MIN,
                Some("Renamed"),
                b"\x00\xff-v2",
                "replace",
            ),
        )
        .await?;
    let version_two = CredentialVersion::MIN.next_live()?;
    assert_eq!(replaced.version(), version_two);
    assert_eq!(replaced.created_at(), created.created_at());
    let live = match store.get(&key).await? {
        StoredCredential::Live(live) => live,
        StoredCredential::Tombstoned(_) => panic!("replace must preserve live state"),
    };
    assert_eq!(live.name(), Some("Renamed"));
    assert_eq!(live.data().as_ref(), b"\x00\xff-v2");
    assert_eq!(live.state_version(), 8);
    assert!(live.reauth_required());
    assert_eq!(
        store
            .replace(
                &key,
                replacement(CredentialVersion::MIN, Some("Stale"), b"stale", "stale",),
            )
            .await
            .expect_err("stale replace must report both typed versions"),
        CredentialPersistenceError::VersionConflict {
            expected: CredentialVersion::MIN,
            actual: version_two,
        }
    );
    assert_eq!(
        store
            .replace(
                &selector(&owner_b, credential_id),
                replacement(
                    CredentialVersion::MAX,
                    Some("Foreign"),
                    b"foreign",
                    "foreign",
                ),
            )
            .await
            .expect_err("foreign owner precedes version and exhaustion"),
        CredentialPersistenceError::NotFound
    );

    let occupied_id = CredentialId::new();
    let occupied_key = selector(&owner_a, occupied_id);
    store
        .create(
            &occupied_key,
            create(Some("Occupied"), b"occupied", "occupied"),
        )
        .await?;
    assert_eq!(
        store
            .tombstone(
                &selector(&owner_b, credential_id),
                CredentialTombstone::new(CredentialVersion::MAX),
            )
            .await
            .expect_err("foreign owner precedes tombstone version and exhaustion"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .tombstone(&key, CredentialTombstone::new(CredentialVersion::MIN))
            .await
            .expect_err("stale tombstone must report both typed versions"),
        CredentialPersistenceError::VersionConflict {
            expected: CredentialVersion::MIN,
            actual: version_two,
        }
    );
    assert_eq!(
        store
            .replace(
                &key,
                replacement(
                    version_two,
                    Some("Occupied"),
                    b"name-collision",
                    "name-collision",
                ),
            )
            .await
            .expect_err("replace cannot claim another live owner-local name"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        }
    );
    let unchanged_head = store.get_head(&key).await?;
    assert_eq!(unchanged_head.version(), version_two);
    assert_eq!(unchanged_head.name(), Some("Renamed"));

    let tombstoned = store
        .tombstone(&key, CredentialTombstone::new(version_two))
        .await?;
    assert_eq!(
        tombstoned.version(),
        version_two
            .next_tombstone()
            .expect("version two has headroom")
    );
    assert_eq!(tombstoned.state(), CredentialRecordState::Tombstoned);
    assert!(tombstoned.tombstoned_at().is_some());
    match store.get(&key).await? {
        StoredCredential::Tombstoned(record) => {
            assert_eq!(record.credential_id(), credential_id);
            assert_eq!(record.version(), tombstoned.version());
            assert_eq!(record.credential_key(), "provider.api-token");
            assert_eq!(record.state_kind(), "active");
        },
        StoredCredential::Live(_) => panic!("tombstone must be structural"),
    }
    assert_eq!(
        store
            .get_head(&key)
            .await
            .expect_err("management projection is live-only"),
        CredentialPersistenceError::NotFound
    );
    assert!(!store.exists(&key).await?);
    assert!(!store.list(&owner_a, None).await?.contains(&credential_id));
    assert_eq!(
        store
            .replace(
                &key,
                replacement(
                    CredentialVersion::MAX,
                    Some("Resurrect"),
                    b"resurrect",
                    "resurrect",
                ),
            )
            .await
            .expect_err("terminal state precedes stale version and exhaustion"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .tombstone(&key, CredentialTombstone::new(CredentialVersion::MAX))
            .await
            .expect_err("terminal tombstone is not repeatable as a mutation"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .create(&key, create(Some("Resurrect"), b"resurrect", "resurrect"),)
            .await
            .expect_err("a tombstone reserves the id forever"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        }
    );

    let reused_id = CredentialId::new();
    let reused_key = selector(&owner_a, reused_id);
    let reused = store
        .create(
            &reused_key,
            create(Some("Renamed"), b"reused-name", "reused-name"),
        )
        .await?;
    assert_eq!(reused.credential_id(), reused_id);
    assert!(store.exists(&reused_key).await?);

    let missing = selector(&owner_a, CredentialId::new());
    assert_eq!(
        store
            .replace(
                &missing,
                replacement(CredentialVersion::MAX, None, b"missing", "missing",),
            )
            .await
            .expect_err("missing row precedes version exhaustion"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .tombstone(&missing, CredentialTombstone::new(CredentialVersion::MAX))
            .await
            .expect_err("missing row precedes tombstone exhaustion"),
        CredentialPersistenceError::NotFound
    );

    let malformed = CredentialCreate::new(
        "provider.api-token".to_owned(),
        SecretBytes::new(Vec::new()),
        "active".to_owned(),
        0,
        Some("Physical".to_owned()),
        None,
        false,
        metadata(Some("Different"), "mismatch"),
    );
    assert_eq!(
        store
            .create(&selector(&owner_a, CredentialId::new()), malformed)
            .await
            .expect_err("physical/display name mismatch must fail before storage"),
        CredentialPersistenceError::CorruptRecord
    );

    let unnamed_id = CredentialId::new();
    let unnamed_key = selector(&owner_b, unnamed_id);
    store
        .create(&unnamed_key, create(None, b"", "unnamed-empty"))
        .await?;
    let unnamed = match store.get(&unnamed_key).await? {
        StoredCredential::Live(live) => live,
        StoredCredential::Tombstoned(_) => panic!("unnamed empty credential must be live"),
    };
    assert_eq!(unnamed.name(), None);
    assert!(unnamed.data().is_empty());

    let dual_id = CredentialId::new();
    let dual_key = selector(&owner_b, dual_id);
    let (dual_a, dual_b) = tokio::join!(
        store.create(
            &dual_key,
            create(Some("Dual Collision"), b"dual-a", "dual-a")
        ),
        store.create(
            &dual_key,
            create(Some("Dual Collision"), b"dual-b", "dual-b")
        )
    );
    let dual_outcomes: [_; 2] = (dual_a, dual_b).into();
    assert_eq!(
        dual_outcomes
            .iter()
            .filter(|outcome| outcome.is_ok())
            .count(),
        1
    );
    assert_eq!(
        dual_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    Err(CredentialPersistenceError::AlreadyExists {
                        key: CredentialAlreadyExistsKey::Id,
                    })
                )
            })
            .count(),
        1,
        "simultaneous id+name collision must classify as id"
    );

    let headroom_id = CredentialId::new();
    let headroom_key = selector(&owner_b, headroom_id);
    store
        .create(&headroom_key, create(None, b"last-live", "headroom"))
        .await?;
    store
        .force_live_version_for_conformance(&headroom_key, CredentialVersion::MAX_LIVE)
        .await?;
    assert_eq!(
        store
            .replace(
                &headroom_key,
                replacement(CredentialVersion::MAX_LIVE, None, b"overflow", "overflow",),
            )
            .await
            .expect_err("replace must preserve terminal version headroom"),
        CredentialPersistenceError::VersionExhausted
    );
    let terminal = store
        .tombstone(
            &headroom_key,
            CredentialTombstone::new(CredentialVersion::MAX_LIVE),
        )
        .await?;
    assert_eq!(terminal.version(), CredentialVersion::MAX);

    let corrupt_id = CredentialId::new();
    let corrupt_key = selector(&owner_b, corrupt_id);
    store
        .create(&corrupt_key, create(None, b"opaque", "corrupt"))
        .await?;
    store
        .corrupt_live_projection_for_conformance(&corrupt_key)
        .await?;
    assert_eq!(
        store
            .get(&corrupt_key)
            .await
            .expect_err("persisted malformed projection must fail closed"),
        CredentialPersistenceError::CorruptRecord
    );

    Ok(())
}

#[tokio::test]
async fn reference_semantic_oracle() -> TestResult<()> {
    let store = ReferenceCredentialPersistence::new();
    run_semantic_oracle(&store).await
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_semantic_oracle() -> TestResult<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("credential-oracle.sqlite3");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let store = SqliteCredentialPersistence::connect(&url).await?;
    run_semantic_oracle(&store).await
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_memory_semantic_oracle() -> TestResult<()> {
    let store = SqliteCredentialPersistence::connect_memory().await?;
    run_semantic_oracle(&store).await
}

#[cfg(feature = "postgres")]
mod postgres {
    use std::{
        str::FromStr,
        time::{SystemTime, UNIX_EPOCH},
    };

    use sqlx::{
        PgPool,
        postgres::{PgConnectOptions, PgPoolOptions},
    };

    use super::*;

    struct IsolatedSchema {
        admin: PgPool,
        options: PgConnectOptions,
        schema: String,
    }

    impl IsolatedSchema {
        async fn connect() -> Option<Self> {
            let url = match std::env::var("DATABASE_URL") {
                Ok(url) => url,
                Err(std::env::VarError::NotPresent) => {
                    assert!(
                        std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                        "NEBULA_REQUIRE_POSTGRES=1 but DATABASE_URL is absent"
                    );
                    return None;
                },
                Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
            };
            let admin = PgPoolOptions::new()
                .max_connections(2)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            let schema = format!("nebula_credential_oracle_{}_{nanos}", std::process::id());
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
                .execute(&admin)
                .await
                .expect("create isolated semantic-oracle schema");
            let options = PgConnectOptions::from_str(&url)
                .expect("parse DATABASE_URL")
                .options([("search_path", schema.as_str())]);
            Some(Self {
                admin,
                options,
                schema,
            })
        }

        async fn cleanup(self) {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA {} CASCADE",
                self.schema
            )))
            .execute(&self.admin)
            .await
            .expect("drop isolated semantic-oracle schema");
            self.admin.close().await;
        }
    }

    #[tokio::test]
    async fn postgres_semantic_oracle() -> TestResult<()> {
        let Some(database) = IsolatedSchema::connect().await else {
            return Ok(());
        };
        let store = PgCredentialPersistence::connect_with(database.options.clone()).await?;
        run_semantic_oracle(&store).await?;
        drop(store);
        database.cleanup().await;
        Ok(())
    }
}
