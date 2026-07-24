//! SQLite credential lifecycle conformance for the K2 persistence contract.

#![cfg(feature = "sqlite")]

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use nebula_storage::credential::SqliteCredentialPersistence;
use nebula_storage_port::{
    CredentialAlreadyExistsKey, CredentialCreate, CredentialOwner, CredentialPersistence,
    CredentialPersistenceError, CredentialRecordState, CredentialReplacement, CredentialSelector,
    CredentialTombstone, CredentialVersion, SecretBytes, StoredCredential,
};
use serde_json::{Map, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

fn instant(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).expect("test timestamp must be representable")
}

fn metadata(label: &str) -> Map<String, Value> {
    Map::from_iter([
        (
            "display".to_owned(),
            serde_json::json!({"display_name": label}),
        ),
        ("preserved".to_owned(), serde_json::json!({"nested": true})),
    ])
}

fn unnamed_metadata() -> Map<String, Value> {
    Map::from_iter([("preserved".to_owned(), serde_json::json!({"nested": true}))])
}

fn create(name: Option<&str>, secret: &[u8]) -> CredentialCreate {
    CredentialCreate::new(
        "provider.oauth".to_owned(),
        SecretBytes::new(secret.to_vec()),
        "oauth2_state".to_owned(),
        3,
        name.map(str::to_owned),
        Some(instant(1_900_000_000)),
        true,
        name.map(metadata).unwrap_or_else(unnamed_metadata),
    )
}

fn replacement(
    expected_version: CredentialVersion,
    name: Option<&str>,
    secret: &[u8],
) -> CredentialReplacement {
    CredentialReplacement::new(
        expected_version,
        SecretBytes::new(secret.to_vec()),
        "oauth2_refreshed".to_owned(),
        4,
        name.map(str::to_owned),
        Some(instant(2_000_000_000)),
        false,
        name.map(metadata).unwrap_or_else(unnamed_metadata),
        nebula_storage_port::CredentialMaterialTransition::advance(),
    )
}

fn version(value: i64) -> CredentialVersion {
    CredentialVersion::try_from(value).expect("fixture version must satisfy the port invariant")
}

fn selector(owner: &CredentialOwner, credential_id: CredentialId) -> CredentialSelector {
    CredentialSelector::new(owner.clone(), credential_id)
}

fn file_url(path: &std::path::Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

#[tokio::test]
async fn lifecycle_is_structural_live_only_and_restart_durable() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("credential-lifecycle.sqlite");
    let url = file_url(&path);
    let owner = CredentialOwner::from_canonical("tenant-lifecycle");
    let credential_id = CredentialId::new();
    let selector = selector(&owner, credential_id);
    let store = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("fresh credential database must become ready");

    let created = store
        .create(&selector, create(Some("Primary"), b"secret-v1"))
        .await
        .expect("create must commit");
    assert_eq!(created.credential_id(), credential_id);
    assert_eq!(created.version(), version(1));
    assert_eq!(created.state(), CredentialRecordState::Live);
    assert_eq!(created.tombstoned_at(), None);

    let stored = store
        .get(&selector)
        .await
        .expect("live row must be readable");
    let StoredCredential::Live(live) = stored else {
        panic!("freshly created credential must be structurally live");
    };
    assert_eq!(live.credential_id(), credential_id);
    assert_eq!(live.name(), Some("Primary"));
    assert_eq!(live.credential_key(), "provider.oauth");
    assert_eq!(live.data().as_ref(), b"secret-v1");
    assert_eq!(live.state_kind(), "oauth2_state");
    assert_eq!(live.state_version(), 3);
    assert_eq!(live.version(), version(1));
    assert_eq!(live.expires_at(), Some(instant(1_900_000_000)));
    assert!(live.reauth_required());
    assert_eq!(live.metadata(), &metadata("Primary"));

    let head = store
        .get_head(&selector)
        .await
        .expect("live row must have a management head");
    assert_eq!(head.credential_id(), credential_id);
    assert_eq!(head.version(), version(1));
    assert_eq!(
        store.list(&owner, None).await.expect("live ids must list"),
        vec![credential_id]
    );
    assert_eq!(
        store
            .list_heads(&owner, None)
            .await
            .expect("live heads must list")
            .len(),
        1
    );
    assert!(
        store
            .exists(&selector)
            .await
            .expect("live existence must query")
    );

    drop(store);
    let store = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("acknowledged create must survive a fully recreated pool");
    let StoredCredential::Live(restarted_create) = store
        .get(&selector)
        .await
        .expect("created row must survive restart")
    else {
        panic!("restarted created credential must remain live");
    };
    assert_eq!(restarted_create.version(), version(1));
    assert_eq!(restarted_create.data().as_ref(), b"secret-v1");

    let replaced = store
        .replace(
            &selector,
            replacement(version(1), Some("Renamed"), b"secret-v2"),
        )
        .await
        .expect("matching CAS replacement must commit");
    assert_eq!(replaced.version(), version(2));
    assert_eq!(replaced.state(), CredentialRecordState::Live);

    let StoredCredential::Live(live) = store.get(&selector).await.expect("replacement must read")
    else {
        panic!("replacement must remain structurally live");
    };
    assert_eq!(live.name(), Some("Renamed"));
    assert_eq!(live.data().as_ref(), b"secret-v2");
    assert_eq!(live.state_kind(), "oauth2_refreshed");
    assert_eq!(live.state_version(), 4);
    assert_eq!(live.version(), version(2));
    assert_eq!(live.expires_at(), Some(instant(2_000_000_000)));
    assert!(!live.reauth_required());
    assert_eq!(live.metadata(), &metadata("Renamed"));

    drop(store);
    let store = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("acknowledged replacement must survive a fully recreated pool");
    let StoredCredential::Live(restarted_replacement) = store
        .get(&selector)
        .await
        .expect("replaced row must survive restart")
    else {
        panic!("restarted replacement must remain live");
    };
    assert_eq!(restarted_replacement.version(), version(2));
    assert_eq!(restarted_replacement.data().as_ref(), b"secret-v2");

    let tombstoned = store
        .tombstone(&selector, CredentialTombstone::new(version(2)))
        .await
        .expect("matching tombstone CAS must commit");
    assert_eq!(tombstoned.version(), version(3));
    assert_eq!(tombstoned.state(), CredentialRecordState::Tombstoned);
    assert!(tombstoned.tombstoned_at().is_some());

    let StoredCredential::Tombstoned(tombstone) = store
        .get(&selector)
        .await
        .expect("physical lookup must retain a tombstone")
    else {
        panic!("deleted credential must be structurally tombstoned");
    };
    assert_eq!(tombstone.credential_id(), credential_id);
    assert_eq!(tombstone.credential_key(), "provider.oauth");
    assert_eq!(tombstone.state_kind(), "oauth2_refreshed");
    assert_eq!(tombstone.state_version(), 4);
    assert_eq!(tombstone.version(), version(3));

    let raw_options = url
        .parse::<SqliteConnectOptions>()
        .expect("temporary SQLite URL must parse")
        .create_if_missing(false);
    let raw_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(raw_options)
        .await
        .expect("raw verification pool must open");
    let durable_shape: (
        i64,
        Option<String>,
        Option<i64>,
        i64,
        String,
        String,
        Option<i64>,
    ) = sqlx::query_as(
        "SELECT length(data), name, expires_at, reauth_required, metadata, \
         record_state, tombstoned_at \
         FROM credentials WHERE id = ?1 AND owner_id = ?2",
    )
    .bind(credential_id.to_string())
    .bind(owner.as_str())
    .fetch_one(&raw_pool)
    .await
    .expect("terminal durable row must be inspectable");
    assert_eq!(
        durable_shape,
        (
            0,
            None,
            None,
            0,
            "{}".to_owned(),
            "tombstoned".to_owned(),
            Some(tombstone.tombstoned_at().timestamp_millis()),
        )
    );
    raw_pool.close().await;

    assert_eq!(
        store
            .get_head(&selector)
            .await
            .expect_err("tombstones have no live management head"),
        CredentialPersistenceError::NotFound
    );
    assert!(
        store
            .list(&owner, None)
            .await
            .expect("live list must query")
            .is_empty()
    );
    assert!(
        store
            .list_heads(&owner, None)
            .await
            .expect("live head list must query")
            .is_empty()
    );
    assert!(
        !store
            .exists(&selector)
            .await
            .expect("tombstone existence must query")
    );
    assert_eq!(
        store
            .replace(
                &selector,
                replacement(version(3), Some("Forbidden"), b"secret-v3"),
            )
            .await
            .expect_err("terminal rows cannot be replaced"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .tombstone(&selector, CredentialTombstone::new(version(3)))
            .await
            .expect_err("terminal rows cannot be tombstoned twice"),
        CredentialPersistenceError::NotFound
    );

    drop(store);
    let reopened = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("canonical current database must reopen");
    assert!(matches!(
        reopened
            .get(&selector)
            .await
            .expect("tombstone must survive restart"),
        StoredCredential::Tombstoned(_)
    ));
    assert!(
        reopened
            .list(&owner, None)
            .await
            .expect("live list after restart must query")
            .is_empty()
    );
}

#[tokio::test]
async fn unnamed_zero_length_live_credential_is_valid() {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("memory credential database must become ready");
    let owner = CredentialOwner::from_canonical("tenant-unnamed");
    let credential_id = CredentialId::new();
    let primary_selector = selector(&owner, credential_id);

    store
        .create(&primary_selector, create(None, b""))
        .await
        .expect("unnamed zero-length live data is valid");
    let StoredCredential::Live(live) = store
        .get(&primary_selector)
        .await
        .expect("unnamed live credential must read")
    else {
        panic!("unnamed credential must remain structurally live");
    };
    assert_eq!(live.name(), None);
    assert!(live.data().is_empty());
    assert_eq!(live.metadata(), &unnamed_metadata());

    let null_projection_id = CredentialId::new();
    let null_projection_selector = selector(&owner, null_projection_id);
    let null_projection_metadata = Map::from_iter([(
        "display".to_owned(),
        serde_json::json!({"display_name": null}),
    )]);
    store
        .create(
            &null_projection_selector,
            CredentialCreate::new(
                "provider.oauth".to_owned(),
                SecretBytes::new(Vec::new()),
                "oauth2_state".to_owned(),
                3,
                None,
                None,
                false,
                null_projection_metadata,
            ),
        )
        .await
        .expect("null display-name projection is an unnamed credential");

    let null_display_selector = selector(&owner, CredentialId::new());
    assert_eq!(
        store
            .create(
                &null_display_selector,
                CredentialCreate::new(
                    "provider.oauth".to_owned(),
                    SecretBytes::new(Vec::new()),
                    "oauth2_state".to_owned(),
                    3,
                    None,
                    None,
                    false,
                    Map::from_iter([("display".to_owned(), Value::Null)]),
                ),
            )
            .await
            .expect_err("present display metadata must be an object"),
        CredentialPersistenceError::CorruptRecord
    );
    assert!(
        !store
            .exists(&null_display_selector)
            .await
            .expect("rejected null display must leave no row")
    );
}

#[tokio::test]
async fn malformed_or_mismatched_display_projection_is_rejected_before_write() {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("memory credential database must become ready");
    let owner = CredentialOwner::from_canonical("tenant-display");

    let mismatched_id = CredentialId::new();
    let mismatched_selector = selector(&owner, mismatched_id);
    let mismatched_create = CredentialCreate::new(
        "provider.oauth".to_owned(),
        SecretBytes::new(b"mismatch".to_vec()),
        "oauth2_state".to_owned(),
        1,
        Some("Command Name".to_owned()),
        None,
        false,
        metadata("Metadata Name"),
    );
    assert_eq!(
        store
            .create(&mismatched_selector, mismatched_create)
            .await
            .expect_err("name and metadata projection must agree"),
        CredentialPersistenceError::CorruptRecord
    );
    assert!(
        !store
            .exists(&mismatched_selector)
            .await
            .expect("rejected create must leave no row")
    );

    let malformed_id = CredentialId::new();
    let malformed_selector = selector(&owner, malformed_id);
    let malformed_metadata = Map::from_iter([(
        "display".to_owned(),
        Value::String("not-an-object".to_owned()),
    )]);
    let malformed_create = CredentialCreate::new(
        "provider.oauth".to_owned(),
        SecretBytes::new(b"malformed".to_vec()),
        "oauth2_state".to_owned(),
        1,
        None,
        None,
        false,
        malformed_metadata,
    );
    assert_eq!(
        store
            .create(&malformed_selector, malformed_create)
            .await
            .expect_err("malformed display metadata must fail closed"),
        CredentialPersistenceError::CorruptRecord
    );
    assert!(
        !store
            .exists(&malformed_selector)
            .await
            .expect("malformed create must leave no row")
    );

    for (label, malformed_display) in [
        (
            "description must be nullable text",
            serde_json::json!({"description": 7}),
        ),
        (
            "tags must be an object",
            serde_json::json!({"tags": ["production"]}),
        ),
        (
            "tag values must be text",
            serde_json::json!({"tags": {"environment": 7}}),
        ),
        ("tags cannot be null", serde_json::json!({"tags": null})),
    ] {
        let malformed_shape_selector = selector(&owner, CredentialId::new());
        let malformed_shape = CredentialCreate::new(
            "provider.oauth".to_owned(),
            SecretBytes::new(label.as_bytes().to_vec()),
            "oauth2_state".to_owned(),
            1,
            None,
            None,
            false,
            Map::from_iter([("display".to_owned(), malformed_display)]),
        );
        assert_eq!(
            store
                .create(&malformed_shape_selector, malformed_shape)
                .await
                .expect_err(label),
            CredentialPersistenceError::CorruptRecord,
            "{label}"
        );
        assert!(
            !store
                .exists(&malformed_shape_selector)
                .await
                .expect("rejected display shape must leave no row"),
            "{label}"
        );
    }

    let live_id = CredentialId::new();
    let live_selector = selector(&owner, live_id);
    store
        .create(&live_selector, create(Some("Stable"), b"stable"))
        .await
        .expect("valid fixture create must commit");
    let mismatched_replacement = CredentialReplacement::new(
        version(1),
        SecretBytes::new(b"forbidden".to_vec()),
        "oauth2_state".to_owned(),
        2,
        Some("Changed".to_owned()),
        None,
        false,
        metadata("Different"),
        nebula_storage_port::CredentialMaterialTransition::advance(),
    );
    assert_eq!(
        store
            .replace(&live_selector, mismatched_replacement)
            .await
            .expect_err("mismatched replacement must fail before SQL"),
        CredentialPersistenceError::CorruptRecord
    );
    let StoredCredential::Live(unchanged) = store
        .get(&live_selector)
        .await
        .expect("rejected replacement must preserve the row")
    else {
        panic!("rejected replacement cannot change lifecycle state");
    };
    assert_eq!(unchanged.version(), version(1));
    assert_eq!(unchanged.name(), Some("Stable"));
    assert_eq!(unchanged.data().as_ref(), b"stable");
}

#[tokio::test]
async fn current_live_revoked_at_metadata_is_opaque_across_restart() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("opaque-revoked-at.sqlite");
    let url = file_url(&path);
    let owner = CredentialOwner::from_canonical("tenant-opaque-revoked-at");
    let credential_id = CredentialId::new();
    let selector = selector(&owner, credential_id);
    let mut opaque_metadata = metadata("Opaque");
    opaque_metadata.insert(
        "revoked_at".to_owned(),
        serde_json::json!({"opaque": ["not", "a", "legacy-marker"]}),
    );

    let store = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("fresh credential database must become ready");
    store
        .create(
            &selector,
            CredentialCreate::new(
                "provider.oauth".to_owned(),
                SecretBytes::new(b"still-live".to_vec()),
                "oauth2_state".to_owned(),
                3,
                Some("Opaque".to_owned()),
                None,
                false,
                opaque_metadata.clone(),
            ),
        )
        .await
        .expect("current writes must preserve opaque metadata keys");
    drop(store);

    let reopened = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("current live opaque metadata must pass restart admission");
    let StoredCredential::Live(live) = reopened
        .get(&selector)
        .await
        .expect("reopened current credential must remain readable")
    else {
        panic!("a current metadata key must not imply legacy tombstone state");
    };
    assert_eq!(live.credential_id(), credential_id);
    assert_eq!(live.version(), version(1));
    assert_eq!(live.metadata(), &opaque_metadata);
}

#[tokio::test]
async fn create_conflicts_follow_the_frozen_information_safe_precedence() {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("memory credential database must become ready");
    let owner = CredentialOwner::from_canonical("tenant-create");
    let foreign_owner = CredentialOwner::from_canonical("tenant-foreign");
    let first_id = CredentialId::new();
    let second_id = CredentialId::new();
    let first = selector(&owner, first_id);
    let second = selector(&owner, second_id);

    store
        .create(&first, create(Some("First"), b"one"))
        .await
        .expect("first create must commit");
    store
        .create(&second, create(Some("Second"), b"two"))
        .await
        .expect("second create must commit");

    assert_eq!(
        store
            .create(&first, create(Some("Second"), b"both-conflict"))
            .await
            .expect_err("same-owner id takes precedence over name collision"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        }
    );
    assert_eq!(
        store
            .create(
                &selector(&owner, CredentialId::new()),
                create(Some("First"), b"name-conflict"),
            )
            .await
            .expect_err("same-owner live name must be unique"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        }
    );
    assert_eq!(
        store
            .create(
                &selector(&foreign_owner, first_id),
                create(Some("Foreign"), b"hidden-id"),
            )
            .await
            .expect_err("foreign-owner global id collision must not disclose existence"),
        CredentialPersistenceError::NotFound
    );

    store
        .tombstone(&first, CredentialTombstone::new(version(1)))
        .await
        .expect("fixture tombstone must commit");
    assert_eq!(
        store
            .create(&first, create(Some("Recreated"), b"forbidden"))
            .await
            .expect_err("same-owner tombstone id remains reserved"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        }
    );
}

#[tokio::test]
async fn mutation_precedence_and_concurrent_cas_are_deterministic() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("credential-cas.sqlite");
    let url = file_url(&path);
    let store = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("file credential database must become ready");
    let owner = CredentialOwner::from_canonical("tenant-cas");
    let foreign_owner = CredentialOwner::from_canonical("tenant-cas-foreign");
    let credential_id = CredentialId::new();
    let primary_selector = selector(&owner, credential_id);
    let foreign_selector = selector(&foreign_owner, credential_id);
    let missing = selector(&owner, CredentialId::new());

    assert_eq!(
        store
            .replace(
                &missing,
                replacement(version(99), Some("Missing"), b"missing"),
            )
            .await
            .expect_err("missing wins before version comparison"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .tombstone(&missing, CredentialTombstone::new(version(99)))
            .await
            .expect_err("missing wins before tombstone version comparison"),
        CredentialPersistenceError::NotFound
    );

    store
        .create(&primary_selector, create(Some("CAS"), b"seed"))
        .await
        .expect("fixture create must commit");
    assert_eq!(
        store
            .replace(
                &foreign_selector,
                replacement(version(1), Some("Foreign"), b"foreign"),
            )
            .await
            .expect_err("wrong owner is indistinguishable from missing"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .replace(
                &primary_selector,
                replacement(version(7), Some("Conflict"), b"conflict"),
            )
            .await
            .expect_err("live mismatched version must be typed"),
        CredentialPersistenceError::VersionConflict {
            expected: version(7),
            actual: version(1),
        }
    );
    assert_eq!(
        store
            .tombstone(&primary_selector, CredentialTombstone::new(version(7)))
            .await
            .expect_err("live mismatched tombstone version must be typed"),
        CredentialPersistenceError::VersionConflict {
            expected: version(7),
            actual: version(1),
        }
    );

    let occupied_name_selector = selector(&owner, CredentialId::new());
    store
        .create(
            &occupied_name_selector,
            create(Some("Occupied"), b"occupied"),
        )
        .await
        .expect("name-collision fixture must commit");
    assert_eq!(
        store
            .replace(
                &primary_selector,
                replacement(version(1), Some("Occupied"), b"name-conflict"),
            )
            .await
            .expect_err("replacement cannot steal another live name"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        }
    );

    let left = store.clone();
    let right = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("independent contender must open its own pool");
    let left_selector = primary_selector.clone();
    let right_selector = primary_selector.clone();
    let (left_result, right_result) = tokio::join!(
        left.replace(
            &left_selector,
            replacement(version(1), Some("Left"), b"left"),
        ),
        right.replace(
            &right_selector,
            replacement(version(1), Some("Right"), b"right"),
        ),
    );
    let results: [_; 2] = (left_result, right_result).into();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(CredentialPersistenceError::VersionConflict {
                        expected,
                        actual,
                    }) if *expected == version(1) && *actual == version(2)
                )
            })
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn barrier_synchronized_two_connection_lifecycle_races_have_one_winner() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("credential-lifecycle-races.sqlite");
    let url = file_url(&path);
    let left = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("first independent store must connect");
    let right = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("second independent store must connect");
    let owner = CredentialOwner::from_canonical("tenant-races");
    let credential_id = CredentialId::new();
    let selector = selector(&owner, credential_id);

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let left_barrier = std::sync::Arc::clone(&barrier);
    let left_selector = selector.clone();
    let left_create = left.clone();
    let left_task = tokio::spawn(async move {
        left_barrier.wait().await;
        left_create
            .create(&left_selector, create(Some("Raced"), b"left-create"))
            .await
    });
    let right_barrier = std::sync::Arc::clone(&barrier);
    let right_selector = selector.clone();
    let right_create = right.clone();
    let right_task = tokio::spawn(async move {
        right_barrier.wait().await;
        right_create
            .create(&right_selector, create(Some("Raced"), b"right-create"))
            .await
    });
    barrier.wait().await;
    let create_results: [_; 2] = (
        left_task.await.expect("left create task must not panic"),
        right_task.await.expect("right create task must not panic"),
    )
        .into();
    assert_eq!(
        create_results
            .iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        create_results
            .iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(CredentialPersistenceError::AlreadyExists {
                        key: CredentialAlreadyExistsKey::Id,
                    })
                )
            })
            .count(),
        1
    );

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let left_barrier = std::sync::Arc::clone(&barrier);
    let left_selector = selector.clone();
    let left_replace = left.clone();
    let left_task = tokio::spawn(async move {
        left_barrier.wait().await;
        left_replace
            .replace(
                &left_selector,
                replacement(version(1), Some("Left"), b"left-replace"),
            )
            .await
    });
    let right_barrier = std::sync::Arc::clone(&barrier);
    let right_selector = selector.clone();
    let right_replace = right.clone();
    let right_task = tokio::spawn(async move {
        right_barrier.wait().await;
        right_replace
            .replace(
                &right_selector,
                replacement(version(1), Some("Right"), b"right-replace"),
            )
            .await
    });
    barrier.wait().await;
    let replace_results: [_; 2] = (
        left_task.await.expect("left replace task must not panic"),
        right_task.await.expect("right replace task must not panic"),
    )
        .into();
    assert_eq!(
        replace_results
            .iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        replace_results
            .iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(CredentialPersistenceError::VersionConflict {
                        expected,
                        actual,
                    }) if *expected == version(1) && *actual == version(2)
                )
            })
            .count(),
        1
    );

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let left_barrier = std::sync::Arc::clone(&barrier);
    let left_selector = selector.clone();
    let left_tombstone = left;
    let left_task = tokio::spawn(async move {
        left_barrier.wait().await;
        left_tombstone
            .tombstone(&left_selector, CredentialTombstone::new(version(2)))
            .await
    });
    let right_barrier = std::sync::Arc::clone(&barrier);
    let right_selector = selector.clone();
    let right_tombstone = right;
    let right_task = tokio::spawn(async move {
        right_barrier.wait().await;
        right_tombstone
            .tombstone(&right_selector, CredentialTombstone::new(version(2)))
            .await
    });
    barrier.wait().await;
    let tombstone_results: [_; 2] = (
        left_task.await.expect("left tombstone task must not panic"),
        right_task
            .await
            .expect("right tombstone task must not panic"),
    )
        .into();
    assert_eq!(
        tombstone_results
            .iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        tombstone_results
            .iter()
            .filter(|result| matches!(result, Err(CredentialPersistenceError::NotFound)))
            .count(),
        1
    );

    let verification = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("verification store must use a fully independent pool");
    let StoredCredential::Tombstoned(tombstone) = verification
        .get(&selector)
        .await
        .expect("winning tombstone must persist")
    else {
        panic!("terminal race winner must leave a structural tombstone");
    };
    assert_eq!(tombstone.version(), version(3));
}

#[tokio::test]
async fn interrupted_uncommitted_transaction_preserves_the_prior_row() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory
        .path()
        .join("credential-interrupted-transaction.sqlite");
    let url = file_url(&path);
    let store = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("credential store must become ready");
    let owner = CredentialOwner::from_canonical("tenant-interrupted");
    let credential_id = CredentialId::new();
    let selector = selector(&owner, credential_id);
    store
        .create(&selector, create(Some("Stable"), b"version-one"))
        .await
        .expect("fixture create must commit");

    let raw_options = url
        .parse::<SqliteConnectOptions>()
        .expect("temporary SQLite URL must parse")
        .create_if_missing(false);
    let raw_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(raw_options)
        .await
        .expect("raw interruption pool must connect");
    let mut transaction = raw_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("raw write transaction must begin");
    sqlx::query(
        "UPDATE credentials
         SET data = ?1, version = 2
         WHERE id = ?2 AND owner_id = ?3 AND record_state = 'live' AND version = 1",
    )
    .bind(b"uncommitted-version-two".as_slice())
    .bind(credential_id.to_string())
    .bind(owner.as_str())
    .execute(&mut *transaction)
    .await
    .expect("uncommitted fixture update must execute");
    let transactional_version: i64 =
        sqlx::query_scalar("SELECT version FROM credentials WHERE id = ?1 AND owner_id = ?2")
            .bind(credential_id.to_string())
            .bind(owner.as_str())
            .fetch_one(&mut *transaction)
            .await
            .expect("transaction must observe its own uncommitted write");
    assert_eq!(transactional_version, 2);

    drop(transaction);
    let rollback_barrier = raw_pool
        .acquire()
        .await
        .expect("pool must finish the drop-triggered rollback before reuse");
    drop(rollback_barrier);
    raw_pool.close().await;

    let StoredCredential::Live(persisted) = store
        .get(&selector)
        .await
        .expect("prior committed row must remain readable")
    else {
        panic!("interrupted uncommitted update cannot alter lifecycle state");
    };
    assert_eq!(persisted.version(), version(1));
    assert_eq!(persisted.data().as_ref(), b"version-one");
}

#[tokio::test]
async fn live_version_headroom_is_reserved_for_terminal_tombstone() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("credential-version-headroom.sqlite");
    let url = file_url(&path);
    let owner = CredentialOwner::from_canonical("tenant-headroom");
    let credential_id = CredentialId::new();
    let selector = selector(&owner, credential_id);
    let store = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("fresh credential database must become ready");
    store
        .create(&selector, create(Some("Headroom"), b"seed"))
        .await
        .expect("fixture create must commit");

    let options = url
        .parse::<SqliteConnectOptions>()
        .expect("temporary SQLite URL must parse")
        .create_if_missing(false);
    let raw_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("raw fixture pool must open");
    sqlx::query(
        "UPDATE credentials SET version = ?1 WHERE id = ?2 AND owner_id = ?3 AND record_state = 'live'",
    )
    .bind(i64::MAX - 1)
    .bind(credential_id.to_string())
    .bind(owner.as_str())
    .execute(&raw_pool)
    .await
    .expect("fixture must move the live row to the last live version");
    raw_pool.close().await;

    let last_live = version(i64::MAX - 1);
    assert_eq!(
        store
            .replace(
                &selector,
                replacement(last_live, Some("Overflow"), b"overflow"),
            )
            .await
            .expect_err("replace cannot consume terminal version headroom"),
        CredentialPersistenceError::VersionExhausted
    );
    let tombstoned = store
        .tombstone(&selector, CredentialTombstone::new(last_live))
        .await
        .expect("tombstone may consume the terminal version");
    assert_eq!(tombstoned.version(), version(i64::MAX));
    assert_eq!(tombstoned.state(), CredentialRecordState::Tombstoned);
}

#[tokio::test]
async fn persisted_malformed_projection_is_rejected_as_corrupt() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let path = directory.path().join("credential-corrupt-row.sqlite");
    let url = file_url(&path);
    let owner = CredentialOwner::from_canonical("tenant-corrupt");
    let credential_id = CredentialId::new();
    let selector = selector(&owner, credential_id);
    let store = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("fresh credential database must become ready");
    store
        .create(&selector, create(None, b"opaque"))
        .await
        .expect("valid fixture create must commit");

    let options = url
        .parse::<SqliteConnectOptions>()
        .expect("temporary SQLite URL must parse")
        .create_if_missing(false);
    let raw_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("raw fixture pool must open");
    sqlx::query(
        "UPDATE credentials
         SET metadata = '{\"display\":\"not-an-object\"}'
         WHERE id = ?1 AND owner_id = ?2",
    )
    .bind(credential_id.to_string())
    .bind(owner.as_str())
    .execute(&raw_pool)
    .await
    .expect("fixture corruption must satisfy the database-level JSON check");
    raw_pool.close().await;

    assert_eq!(
        store
            .get(&selector)
            .await
            .expect_err("persisted malformed projection must fail closed"),
        CredentialPersistenceError::CorruptRecord
    );
}
