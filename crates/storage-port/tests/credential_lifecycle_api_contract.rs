use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialAlreadyExistsKey, CredentialCommit, CredentialCreate, CredentialOwner,
    CredentialPersistence, CredentialPersistenceError, CredentialRecordState,
    CredentialReplacement, CredentialSelector, CredentialTombstone, CredentialVersion, SecretBytes,
    StoredCredential, StoredCredentialHead,
};
use serde_json::{Map, Value};

fn instant(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).expect("test timestamp is representable")
}

fn metadata() -> Map<String, Value> {
    Map::from_iter([(
        "display".to_owned(),
        serde_json::json!({"display_name": "Production"}),
    )])
}

fn create_with(secret: Vec<u8>) -> CredentialCreate {
    CredentialCreate::new(
        "github_oauth".to_owned(),
        SecretBytes::new(secret),
        "oauth2_state".to_owned(),
        3,
        Some("Production".to_owned()),
        Some(instant(1_800_000_000)),
        true,
        metadata(),
    )
}

fn replacement_with(secret: Vec<u8>) -> CredentialReplacement {
    CredentialReplacement::new(
        CredentialVersion::try_from(7_i64).expect("valid expected version"),
        SecretBytes::new(secret),
        "oauth2_state".to_owned(),
        4,
        Some("Production".to_owned()),
        Some(instant(1_900_000_000)),
        false,
        metadata(),
    )
}

#[test]
fn selector_owns_a_typed_server_generated_id_and_owner() {
    let owner = CredentialOwner::from_canonical("tenant-a");
    let credential_id = CredentialId::new();
    let selector = CredentialSelector::new(owner.clone(), credential_id);

    assert_eq!(selector.credential_id(), credential_id);
    assert_eq!(selector.owner(), &owner);
}

#[test]
fn command_accessors_expose_only_the_frozen_field_split() {
    let create = create_with(vec![1, 2, 3]);
    assert_eq!(create.credential_key(), "github_oauth");
    assert_eq!(create.data().as_ref(), [1, 2, 3]);
    assert_eq!(create.state_kind(), "oauth2_state");
    assert_eq!(create.state_version(), 3);
    assert_eq!(create.name(), Some("Production"));
    assert_eq!(create.expires_at(), Some(instant(1_800_000_000)));
    assert!(create.reauth_required());
    assert_eq!(create.metadata(), &metadata());

    let expected = CredentialVersion::try_from(7_i64).expect("valid expected version");
    let replacement = replacement_with(vec![4, 5, 6]);
    assert_eq!(replacement.expected_version(), expected);
    assert_eq!(replacement.data().as_ref(), [4, 5, 6]);
    assert_eq!(replacement.state_kind(), "oauth2_state");
    assert_eq!(replacement.state_version(), 4);
    assert_eq!(replacement.name(), Some("Production"));
    assert_eq!(replacement.expires_at(), Some(instant(1_900_000_000)));
    assert!(!replacement.reauth_required());
    assert_eq!(replacement.metadata(), &metadata());

    let tombstone = CredentialTombstone::new(expected);
    assert_eq!(tombstone.expected_version(), expected);
}

#[test]
fn credential_version_reserves_terminal_headroom_for_tombstoning() {
    assert!(CredentialVersion::try_from(-1_i64).is_err());
    assert!(CredentialVersion::try_from(0_i64).is_err());
    assert!(CredentialVersion::try_from(i64::MAX as u64 + 1).is_err());

    let first = CredentialVersion::try_from(1_i64).expect("one is valid");
    let last_live =
        CredentialVersion::try_from(i64::MAX - 1).expect("last live version is representable");
    let terminal =
        CredentialVersion::try_from(i64::MAX).expect("terminal tombstone is representable");

    assert_eq!(first.get(), 1);
    assert_eq!(
        first.next_live().expect("ordinary replace can advance"),
        CredentialVersion::try_from(2_i64).expect("two is valid")
    );
    assert_eq!(
        last_live
            .next_live()
            .expect_err("replace cannot consume the terminal version"),
        CredentialPersistenceError::VersionExhausted
    );
    assert_eq!(
        last_live
            .next_tombstone()
            .expect("tombstone retains terminal headroom"),
        terminal
    );
    assert_eq!(
        terminal
            .next_tombstone()
            .expect_err("version cannot advance above the database range"),
        CredentialPersistenceError::VersionExhausted
    );
}

#[test]
fn secret_bearing_command_debug_is_constant_shape() {
    let empty_create = format!("{:?}", create_with(Vec::new()));
    let short_create = format!("{:?}", create_with(vec![0x41]));
    let long_create = format!("{:?}", create_with(vec![0x5a; 4_096]));
    assert_eq!(empty_create, short_create);
    assert_eq!(short_create, long_create);

    let empty_replace = format!("{:?}", replacement_with(Vec::new()));
    let short_replace = format!("{:?}", replacement_with(vec![0x41]));
    let long_replace = format!("{:?}", replacement_with(vec![0x5a; 4_096]));
    assert_eq!(empty_replace, short_replace);
    assert_eq!(short_replace, long_replace);
}

#[test]
fn commit_is_secret_free_and_lifecycle_consistent() {
    let credential_id = CredentialId::new();
    let first = CredentialVersion::try_from(1_i64).expect("valid version");
    let created_at = instant(1_700_000_000);
    let updated_at = instant(1_700_000_001);

    let live = CredentialCommit::live(credential_id, first, created_at, updated_at)
        .expect("live commit has a live-compatible version");
    assert_eq!(live.credential_id(), credential_id);
    assert_eq!(live.version(), first);
    assert_eq!(live.state(), CredentialRecordState::Live);
    assert_eq!(live.created_at(), created_at);
    assert_eq!(live.updated_at(), updated_at);
    assert_eq!(live.tombstoned_at(), None);

    let tombstoned_at = instant(1_700_000_002);
    let tombstone = CredentialCommit::tombstoned(
        credential_id,
        first,
        created_at,
        tombstoned_at,
        tombstoned_at,
    );
    assert_eq!(tombstone.state(), CredentialRecordState::Tombstoned);
    assert_eq!(tombstone.tombstoned_at(), Some(tombstoned_at));

    let rendered = format!("{tombstone:?}");
    assert!(!rendered.contains("github_oauth"));
    assert!(!rendered.contains("Production"));
}

fn closed_error_code(error: &CredentialPersistenceError) -> &'static str {
    match error {
        CredentialPersistenceError::NotFound => "not_found",
        CredentialPersistenceError::VersionConflict {
            expected: _,
            actual: _,
        } => "version_conflict",
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        } => "already_exists_id",
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        } => "already_exists_name",
        CredentialPersistenceError::VersionExhausted => "version_exhausted",
        CredentialPersistenceError::CorruptRecord => "corrupt_record",
        CredentialPersistenceError::Unavailable => "unavailable",
        CredentialPersistenceError::OutcomeUnknown => "outcome_unknown",
    }
}

fn structural_record_state(record: StoredCredential) -> CredentialRecordState {
    match record {
        StoredCredential::Live(live) => {
            let _: CredentialId = live.credential_id();
            let _: Option<&str> = live.name();
            let _: &str = live.credential_key();
            let _: &SecretBytes = live.data();
            let _: &str = live.state_kind();
            let _: u32 = live.state_version();
            let _: CredentialVersion = live.version();
            let _: DateTime<Utc> = live.created_at();
            let _: DateTime<Utc> = live.updated_at();
            let _: Option<DateTime<Utc>> = live.expires_at();
            let _: bool = live.reauth_required();
            let _: &Map<String, Value> = live.metadata();
            CredentialRecordState::Live
        },
        StoredCredential::Tombstoned(tombstone) => {
            let _: CredentialId = tombstone.credential_id();
            let _: &str = tombstone.credential_key();
            let _: &str = tombstone.state_kind();
            let _: u32 = tombstone.state_version();
            let _: CredentialVersion = tombstone.version();
            let _: DateTime<Utc> = tombstone.created_at();
            let _: DateTime<Utc> = tombstone.updated_at();
            let _: DateTime<Utc> = tombstone.tombstoned_at();
            CredentialRecordState::Tombstoned
        },
    }
}

#[test]
fn stored_state_is_structural_and_tombstones_expose_no_live_only_accessors() {
    let _: fn(StoredCredential) -> CredentialRecordState = structural_record_state;
}

#[test]
fn persistence_errors_are_closed_typed_and_secret_free() {
    let one = CredentialVersion::try_from(1_i64).expect("valid version");
    let two = CredentialVersion::try_from(2_i64).expect("valid version");
    let errors = [
        CredentialPersistenceError::NotFound,
        CredentialPersistenceError::VersionConflict {
            expected: one,
            actual: two,
        },
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        },
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        },
        CredentialPersistenceError::VersionExhausted,
        CredentialPersistenceError::CorruptRecord,
        CredentialPersistenceError::Unavailable,
        CredentialPersistenceError::OutcomeUnknown,
    ];

    assert_eq!(
        errors
            .iter()
            .map(closed_error_code)
            .collect::<Vec<&'static str>>(),
        [
            "not_found",
            "version_conflict",
            "already_exists_id",
            "already_exists_name",
            "version_exhausted",
            "corrupt_record",
            "unavailable",
            "outcome_unknown",
        ]
    );

    for error in &errors {
        let rendered = format!("{error} {error:?}");
        assert!(!rendered.contains("tenant-a"));
        assert!(!rendered.contains("cred_"));
        assert!(!rendered.contains("Production"));
        assert!(!rendered.contains("postgres://"));
        assert!(!rendered.contains("SELECT "));
    }
}

#[derive(Debug)]
struct ContractPersistence;

#[async_trait]
impl CredentialPersistence for ContractPersistence {
    async fn get(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        Err(CredentialPersistenceError::NotFound)
    }

    async fn get_head(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        Err(CredentialPersistenceError::NotFound)
    }

    async fn create(
        &self,
        _selector: &CredentialSelector,
        _create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn replace(
        &self,
        _selector: &CredentialSelector,
        _replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn tombstone(
        &self,
        _selector: &CredentialSelector,
        _tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn list(
        &self,
        _owner: &CredentialOwner,
        _state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn list_heads(
        &self,
        _owner: &CredentialOwner,
        _state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn exists(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }
}

#[test]
fn lifecycle_port_is_directly_object_safe_with_typed_list_ids() {
    let concrete: Arc<ContractPersistence> = Arc::new(ContractPersistence);
    let persistence: Arc<dyn CredentialPersistence> = concrete;

    fn accepts_dyn(_: Arc<dyn CredentialPersistence>) {}
    accepts_dyn(persistence);
}
