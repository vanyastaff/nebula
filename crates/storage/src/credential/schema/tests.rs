use std::collections::BTreeSet;

use nebula_core::CredentialId;

use super::{
    AdmissionReason, BackendKind, BackendMigrationPolicy, LegacyCredentialRecord, MigrationLedger,
    MigrationLedgerRow, MigrationSpec, SchemaAdmission, SchemaObservation,
    UnsupportedSchemaVersion, classify_schema,
};
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use super::{CredentialStoreStartupError, observation_fetch_error};

const SUPPORTED_FLOOR: i64 = 30;
const CURRENT_VERSION: i64 = 39;

fn backend_versions(backend: BackendKind) -> Vec<i64> {
    match backend {
        BackendKind::Sqlite => (1..=28).chain(30..=35).chain([39]).collect(),
        BackendKind::Postgres => (1..=39).collect(),
    }
}

fn migration_spec(version: i64) -> MigrationSpec {
    MigrationSpec {
        version,
        description: format!("canonical migration {version:04}"),
        checksum: (version as u64).to_be_bytes().repeat(6),
    }
}

fn policy(backend: BackendKind) -> BackendMigrationPolicy {
    let canonical = backend_versions(backend)
        .into_iter()
        .map(migration_spec)
        .collect();
    let reserved_other_backend_versions = match backend {
        BackendKind::Sqlite => BTreeSet::from([29, 36, 37, 38]),
        BackendKind::Postgres => BTreeSet::new(),
    };

    BackendMigrationPolicy {
        supported_floor: SUPPORTED_FLOOR,
        current_version: CURRENT_VERSION,
        canonical,
        reserved_other_backend_versions,
    }
}

fn canonical_ledger(policy: &BackendMigrationPolicy, latest: i64) -> Vec<MigrationLedgerRow> {
    policy
        .canonical
        .iter()
        .filter(|migration| migration.version <= latest)
        .map(|migration| MigrationLedgerRow {
            version: migration.version,
            description: migration.description.clone(),
            checksum: migration.checksum.clone(),
            success: true,
        })
        .collect()
}

fn credential(metadata: &str) -> LegacyCredentialRecord {
    LegacyCredentialRecord {
        id: CredentialId::new().to_string(),
        owner_id: Some("owner-a".to_owned()),
        name: None,
        state_version: 0,
        data_len: 0,
        version: 1,
        metadata: metadata.to_owned(),
        record_state: None,
        tombstoned_at_present: false,
        expires_at_present: false,
        reauth_required: false,
    }
}

fn fresh_observation() -> SchemaObservation {
    SchemaObservation {
        migration_ledger: MigrationLedger::Absent,
        has_user_relations: false,
        has_credentials_relation: false,
        credentials: Vec::new(),
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
#[test]
fn observation_fetch_errors_distinguish_schema_evidence_from_unavailability() {
    for error in [
        sqlx::Error::ColumnNotFound("version".to_owned()),
        sqlx::Error::RowNotFound,
        sqlx::Error::TypeNotFound {
            type_name: "canonical_type".to_owned(),
        },
    ] {
        assert!(matches!(
            observation_fetch_error(error, AdmissionReason::InvalidMigrationLedger),
            CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                if unsupported.reason() == &AdmissionReason::InvalidMigrationLedger
        ));
    }

    for error in [
        sqlx::Error::Protocol("connection stream ended".to_owned()),
        sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset",
        )),
        sqlx::Error::PoolTimedOut,
    ] {
        assert_eq!(
            observation_fetch_error(error, AdmissionReason::InvalidMigrationLedger),
            CredentialStoreStartupError::Unavailable
        );
    }
}

fn migrated_observation(
    policy: &BackendMigrationPolicy,
    latest: i64,
    credentials: Vec<LegacyCredentialRecord>,
) -> SchemaObservation {
    SchemaObservation {
        migration_ledger: MigrationLedger::Present(canonical_ledger(policy, latest)),
        has_user_relations: true,
        has_credentials_relation: true,
        credentials,
    }
}

fn rejected_reason(
    policy: &BackendMigrationPolicy,
    observation: &SchemaObservation,
) -> AdmissionReason {
    classify_schema(policy, observation)
        .expect_err("observation must be rejected")
        .reason()
        .clone()
}

#[test]
fn accepts_only_a_genuine_fresh_database_without_a_ledger_or_user_relations() {
    let policy = policy(BackendKind::Sqlite);

    assert_eq!(
        classify_schema(&policy, &fresh_observation()),
        Ok(SchemaAdmission::Fresh)
    );
}

#[test]
fn rejects_empty_ledger_and_unledgered_nonempty_database() {
    let policy = policy(BackendKind::Sqlite);
    let empty_ledger = SchemaObservation {
        migration_ledger: MigrationLedger::Present(Vec::new()),
        has_user_relations: false,
        has_credentials_relation: false,
        credentials: Vec::new(),
    };
    let unledgered_nonempty = SchemaObservation {
        migration_ledger: MigrationLedger::Absent,
        has_user_relations: true,
        has_credentials_relation: false,
        credentials: Vec::new(),
    };

    assert_eq!(
        rejected_reason(&policy, &empty_ledger),
        AdmissionReason::EmptyLedger
    );
    assert_eq!(
        rejected_reason(&policy, &unledgered_nonempty),
        AdmissionReason::UnledgeredDatabase
    );
}

#[test]
fn rejects_a_supported_ledger_without_the_credentials_relation() {
    let policy = policy(BackendKind::Sqlite);
    let observation = SchemaObservation {
        migration_ledger: MigrationLedger::Present(canonical_ledger(&policy, 35)),
        has_user_relations: true,
        has_credentials_relation: false,
        credentials: Vec::new(),
    };

    assert_eq!(
        rejected_reason(&policy, &observation),
        AdmissionReason::MissingCredentialsRelation
    );
}

#[test]
fn accepts_exact_successful_backend_prefixes_at_or_above_the_floor() {
    for (backend, accepted_heads) in [
        (BackendKind::Sqlite, &[30, 35, 39][..]),
        (BackendKind::Postgres, &[30, 38, 39][..]),
    ] {
        let policy = policy(backend);
        for latest in accepted_heads {
            let observation = migrated_observation(&policy, *latest, Vec::new());
            assert_eq!(
                classify_schema(&policy, &observation),
                Ok(SchemaAdmission::CanonicalPrefix { latest: *latest }),
                "{backend:?} rejected canonical head {latest:04}"
            );
        }
    }
}

#[test]
fn sqlite_prefix_is_logical_and_does_not_require_postgres_only_versions() {
    let policy = policy(BackendKind::Sqlite);
    let observation = migrated_observation(&policy, 39, Vec::new());
    let present_versions = match &observation.migration_ledger {
        MigrationLedger::Absent => panic!("fixture must contain a ledger"),
        MigrationLedger::Present(rows) => {
            rows.iter().map(|row| row.version).collect::<BTreeSet<_>>()
        },
    };

    assert!(!present_versions.contains(&29));
    assert!(!present_versions.contains(&36));
    assert!(!present_versions.contains(&37));
    assert!(!present_versions.contains(&38));
    assert_eq!(
        classify_schema(&policy, &observation),
        Ok(SchemaAdmission::CanonicalPrefix { latest: 39 })
    );
}

#[test]
fn rejects_prefixes_below_the_supported_floor() {
    let sqlite_policy = policy(BackendKind::Sqlite);
    let postgres_policy = policy(BackendKind::Postgres);

    assert_eq!(
        rejected_reason(
            &sqlite_policy,
            &migrated_observation(&sqlite_policy, 28, Vec::new())
        ),
        AdmissionReason::BelowSupportedFloor { latest: 28 }
    );
    assert_eq!(
        rejected_reason(
            &postgres_policy,
            &migrated_observation(&postgres_policy, 29, Vec::new())
        ),
        AdmissionReason::BelowSupportedFloor { latest: 29 }
    );
}

#[test]
fn rejects_failed_duplicate_gapped_and_out_of_order_ledgers() {
    let policy = policy(BackendKind::Sqlite);

    let mut failed = canonical_ledger(&policy, 35);
    failed
        .iter_mut()
        .find(|row| row.version == 30)
        .expect("canonical fixture contains migration 0030")
        .success = false;
    let failed_observation = SchemaObservation {
        migration_ledger: MigrationLedger::Present(failed),
        has_user_relations: true,
        has_credentials_relation: true,
        credentials: Vec::new(),
    };
    assert_eq!(
        rejected_reason(&policy, &failed_observation),
        AdmissionReason::FailedMigration { migration: 30 }
    );

    let mut duplicate = canonical_ledger(&policy, 35);
    let duplicate_row = duplicate
        .iter()
        .find(|row| row.version == 30)
        .expect("canonical fixture contains migration 0030")
        .clone();
    duplicate.insert(29, duplicate_row);
    let duplicate_observation = SchemaObservation {
        migration_ledger: MigrationLedger::Present(duplicate),
        has_user_relations: true,
        has_credentials_relation: true,
        credentials: Vec::new(),
    };
    assert_eq!(
        rejected_reason(&policy, &duplicate_observation),
        AdmissionReason::DuplicateMigration { migration: 30 }
    );

    let mut gapped = canonical_ledger(&policy, 35);
    gapped.retain(|row| row.version != 17);
    let gapped_observation = SchemaObservation {
        migration_ledger: MigrationLedger::Present(gapped),
        has_user_relations: true,
        has_credentials_relation: true,
        credentials: Vec::new(),
    };
    assert!(matches!(
        rejected_reason(&policy, &gapped_observation),
        AdmissionReason::NonCanonicalOrder { .. }
    ));

    let mut out_of_order = canonical_ledger(&policy, 35);
    out_of_order.swap(10, 11);
    let out_of_order_observation = SchemaObservation {
        migration_ledger: MigrationLedger::Present(out_of_order),
        has_user_relations: true,
        has_credentials_relation: true,
        credentials: Vec::new(),
    };
    assert!(matches!(
        rejected_reason(&policy, &out_of_order_observation),
        AdmissionReason::NonCanonicalOrder { .. }
    ));
}

#[test]
fn rejects_other_backend_reserved_unknown_and_future_versions() {
    let sqlite_policy = policy(BackendKind::Sqlite);

    for migration in [29, 36, 37, 38] {
        let mut rows = canonical_ledger(&sqlite_policy, 35);
        rows.push(MigrationLedgerRow {
            version: migration,
            description: "postgres-only migration".to_owned(),
            checksum: vec![0; 48],
            success: true,
        });
        rows.sort_by_key(|row| row.version);
        let observation = SchemaObservation {
            migration_ledger: MigrationLedger::Present(rows),
            has_user_relations: true,
            has_credentials_relation: true,
            credentials: Vec::new(),
        };
        assert_eq!(
            rejected_reason(&sqlite_policy, &observation),
            AdmissionReason::ReservedForOtherBackend { migration }
        );
    }

    for migration in [40, 777] {
        let mut rows = canonical_ledger(&sqlite_policy, 39);
        rows.push(MigrationLedgerRow {
            version: migration,
            description: "not canonical".to_owned(),
            checksum: vec![0; 48],
            success: true,
        });
        let observation = SchemaObservation {
            migration_ledger: MigrationLedger::Present(rows),
            has_user_relations: true,
            has_credentials_relation: true,
            credentials: Vec::new(),
        };
        assert_eq!(
            rejected_reason(&sqlite_policy, &observation),
            AdmissionReason::UnknownMigration { migration }
        );
    }
}

#[test]
fn rejects_description_and_checksum_drift() {
    let policy = policy(BackendKind::Sqlite);

    let mut wrong_description = canonical_ledger(&policy, 35);
    wrong_description
        .iter_mut()
        .find(|row| row.version == 30)
        .expect("canonical fixture contains migration 0030")
        .description = "tampered description".to_owned();
    let observation = SchemaObservation {
        migration_ledger: MigrationLedger::Present(wrong_description),
        has_user_relations: true,
        has_credentials_relation: true,
        credentials: Vec::new(),
    };
    assert_eq!(
        rejected_reason(&policy, &observation),
        AdmissionReason::DescriptionMismatch { migration: 30 }
    );

    let mut wrong_checksum = canonical_ledger(&policy, 35);
    wrong_checksum
        .iter_mut()
        .find(|row| row.version == 30)
        .expect("canonical fixture contains migration 0030")
        .checksum = vec![0xA5; 48];
    let observation = SchemaObservation {
        migration_ledger: MigrationLedger::Present(wrong_checksum),
        has_user_relations: true,
        has_credentials_relation: true,
        credentials: Vec::new(),
    };
    assert_eq!(
        rejected_reason(&policy, &observation),
        AdmissionReason::ChecksumMismatch { migration: 30 }
    );
}

#[test]
fn rejects_ownerless_invalid_id_and_out_of_range_legacy_rows() {
    let policy = policy(BackendKind::Sqlite);

    let mut ownerless = credential("{}");
    ownerless.owner_id = None;
    assert_eq!(
        rejected_reason(&policy, &migrated_observation(&policy, 35, vec![ownerless])),
        AdmissionReason::OwnerlessCredential
    );

    let mut invalid_id = credential("{}");
    invalid_id.id = "raw-credential-id".to_owned();
    assert_eq!(
        rejected_reason(
            &policy,
            &migrated_observation(&policy, 35, vec![invalid_id])
        ),
        AdmissionReason::InvalidCredentialId
    );

    for invalid in [-1, i64::from(u32::MAX) + 1] {
        let mut row = credential("{}");
        row.state_version = invalid;
        assert_eq!(
            rejected_reason(&policy, &migrated_observation(&policy, 35, vec![row])),
            AdmissionReason::InvalidStateVersion
        );
    }

    for invalid in [-1, 0] {
        let mut row = credential("{}");
        row.version = invalid;
        assert_eq!(
            rejected_reason(&policy, &migrated_observation(&policy, 35, vec![row])),
            AdmissionReason::InvalidCredentialVersion
        );
    }

    let mut exhausted_live = credential("{}");
    exhausted_live.version = i64::MAX;
    assert_eq!(
        rejected_reason(
            &policy,
            &migrated_observation(&policy, 35, vec![exhausted_live])
        ),
        AdmissionReason::LiveVersionExhausted
    );
}

#[test]
fn rejects_malformed_non_object_and_recursively_duplicate_metadata() {
    let policy = policy(BackendKind::Sqlite);

    for metadata in ["{", r#""string""#, "null", "[]", "42"] {
        let reason = rejected_reason(
            &policy,
            &migrated_observation(&policy, 35, vec![credential(metadata)]),
        );
        if metadata == "{" {
            assert_eq!(reason, AdmissionReason::MalformedMetadata);
        } else {
            assert_eq!(reason, AdmissionReason::MetadataNotObject);
        }
    }

    for metadata in [
        r#"{"same":1,"same":2}"#,
        r#"{"nested":{"same":1,"same":2}}"#,
        r#"{"array":[{"same":1,"same":2}]}"#,
    ] {
        assert_eq!(
            rejected_reason(
                &policy,
                &migrated_observation(&policy, 35, vec![credential(metadata)])
            ),
            AdmissionReason::DuplicateMetadataKey
        );
    }
}

#[test]
fn validates_display_shape_and_physical_name_projection() {
    let policy = policy(BackendKind::Sqlite);

    for metadata in [
        r#"{"display":null}"#,
        r#"{"display":[]}"#,
        r#"{"display":{"display_name":7}}"#,
        r#"{"display":{"description":false}}"#,
        r#"{"display":{"tags":[]}}"#,
        r#"{"display":{"tags":{"env":7}}}"#,
    ] {
        assert_eq!(
            rejected_reason(
                &policy,
                &migrated_observation(&policy, 35, vec![credential(metadata)])
            ),
            AdmissionReason::InvalidDisplay
        );
    }

    let mut mismatch = credential(r#"{"display":{"display_name":"projected"}}"#);
    mismatch.name = Some("physical".to_owned());
    assert_eq!(
        rejected_reason(&policy, &migrated_observation(&policy, 35, vec![mismatch])),
        AdmissionReason::DisplayNameMismatch
    );

    let mut orphan = credential(r#"{"display":{"description":"still unnamed"}}"#);
    orphan.name = Some("orphan".to_owned());
    assert_eq!(
        rejected_reason(&policy, &migrated_observation(&policy, 35, vec![orphan])),
        AdmissionReason::OrphanPhysicalName
    );
}

#[test]
fn accepts_unnamed_zero_byte_and_projectable_live_rows() {
    let policy = policy(BackendKind::Sqlite);

    let unnamed_absent_display = credential(r#"{"future":{"preserved":true}}"#);
    let unnamed_null_name = credential(r#"{"display":{"display_name":null}}"#);
    let projected = credential(r#"{"display":{"display_name":"projected","future":"preserved"}}"#);
    let mut matching = credential(r#"{"display":{"display_name":"matching"},"unknown":[1,2,3]}"#);
    matching.name = Some("matching".to_owned());

    let observation = migrated_observation(
        &policy,
        35,
        vec![
            unnamed_absent_display,
            unnamed_null_name,
            projected,
            matching,
        ],
    );
    assert_eq!(
        classify_schema(&policy, &observation),
        Ok(SchemaAdmission::CanonicalPrefix { latest: 35 })
    );
}

#[test]
fn projected_names_are_unique_per_owner_and_ignore_terminal_rows() {
    let policy = policy(BackendKind::Sqlite);

    let first = credential(r#"{"display":{"display_name":"shared"}}"#);
    let mut second = credential(r#"{"display":{"display_name":"shared"}}"#);
    second.owner_id = Some("owner-a".to_owned());
    assert_eq!(
        rejected_reason(
            &policy,
            &migrated_observation(&policy, 35, vec![first.clone(), second.clone()])
        ),
        AdmissionReason::DuplicateProjectedName
    );

    second.owner_id = Some("owner-b".to_owned());
    assert_eq!(
        classify_schema(
            &policy,
            &migrated_observation(&policy, 35, vec![first.clone(), second])
        ),
        Ok(SchemaAdmission::CanonicalPrefix { latest: 35 })
    );

    let mut terminal = credential(
        r#"{"revoked_at":null,"display":{"display_name":"shared"},"future":"discarded"}"#,
    );
    terminal.version = i64::MAX;
    assert_eq!(
        classify_schema(
            &policy,
            &migrated_observation(&policy, 35, vec![first, terminal])
        ),
        Ok(SchemaAdmission::CanonicalPrefix { latest: 35 })
    );
}

#[test]
fn revoked_at_key_presence_is_terminal_and_its_value_is_not_parsed() {
    let policy = policy(BackendKind::Sqlite);

    for metadata in [
        r#"{"revoked_at":null,"display":false}"#,
        r#"{"revoked_at":"not-a-timestamp","display":42}"#,
        r#"{"revoked_at":{"opaque":["never","parsed"]},"display":[]}"#,
    ] {
        let mut terminal = credential(metadata);
        terminal.version = i64::MAX;
        terminal.name = Some("released-by-tombstone".to_owned());
        terminal.data_len = 24;

        assert_eq!(
            classify_schema(&policy, &migrated_observation(&policy, 35, vec![terminal])),
            Ok(SchemaAdmission::CanonicalPrefix { latest: 35 })
        );
    }

    let duplicate_inside_terminal =
        credential(r#"{"revoked_at":null,"nested":{"same":1,"same":2}}"#);
    assert_eq!(
        rejected_reason(
            &policy,
            &migrated_observation(&policy, 35, vec![duplicate_inside_terminal])
        ),
        AdmissionReason::DuplicateMetadataKey
    );
}

#[test]
fn validates_structural_live_and_tombstoned_rows_at_current_head() {
    let policy = policy(BackendKind::Sqlite);

    let mut live = credential(r#"{"display":{"display_name":"live"}}"#);
    live.name = Some("live".to_owned());
    live.version = i64::MAX - 1;
    live.record_state = Some("live".to_owned());

    let mut tombstone = credential("{}");
    tombstone.version = i64::MAX;
    tombstone.record_state = Some("tombstoned".to_owned());
    tombstone.tombstoned_at_present = true;

    assert_eq!(
        classify_schema(
            &policy,
            &migrated_observation(&policy, 39, vec![live, tombstone])
        ),
        Ok(SchemaAdmission::CanonicalPrefix { latest: 39 })
    );
}

#[test]
fn current_live_revoked_at_key_is_opaque_metadata() {
    let policy = policy(BackendKind::Sqlite);
    let mut live =
        credential(r#"{"revoked_at":{"opaque":["never","parsed"]},"future":{"nested":true}}"#);
    live.record_state = Some("live".to_owned());

    assert_eq!(
        classify_schema(&policy, &migrated_observation(&policy, 39, vec![live])),
        Ok(SchemaAdmission::CanonicalPrefix { latest: 39 })
    );
}

#[test]
fn rejects_forged_or_malformed_current_record_shapes() {
    let policy = policy(BackendKind::Sqlite);

    let missing_state = credential("{}");
    assert_eq!(
        rejected_reason(
            &policy,
            &migrated_observation(&policy, 39, vec![missing_state])
        ),
        AdmissionReason::InvalidRecordState
    );

    let mut unknown_state = credential("{}");
    unknown_state.record_state = Some("unknown".to_owned());
    assert_eq!(
        rejected_reason(
            &policy,
            &migrated_observation(&policy, 39, vec![unknown_state])
        ),
        AdmissionReason::InvalidRecordState
    );

    let mut malformed_tombstone = credential("{}");
    malformed_tombstone.record_state = Some("tombstoned".to_owned());
    malformed_tombstone.tombstoned_at_present = true;
    malformed_tombstone.data_len = 1;
    assert_eq!(
        rejected_reason(
            &policy,
            &migrated_observation(&policy, 39, vec![malformed_tombstone])
        ),
        AdmissionReason::InvalidTombstoneShape
    );

    let mut noncanonical_tombstone = credential("{ }");
    noncanonical_tombstone.record_state = Some("tombstoned".to_owned());
    noncanonical_tombstone.tombstoned_at_present = true;
    assert_eq!(
        rejected_reason(
            &policy,
            &migrated_observation(&policy, 39, vec![noncanonical_tombstone])
        ),
        AdmissionReason::InvalidTombstoneShape
    );
}

#[test]
fn startup_schema_errors_are_closed_and_secret_free() {
    fn assert_error_contract<E: std::error::Error + Send + Sync + 'static>() {}
    assert_error_contract::<UnsupportedSchemaVersion>();

    fn exhaustively_match_closed_reason(reason: &AdmissionReason) {
        match reason {
            AdmissionReason::EmptyLedger
            | AdmissionReason::UnledgeredDatabase
            | AdmissionReason::InvalidMigrationLedger
            | AdmissionReason::MissingCredentialsRelation
            | AdmissionReason::InvalidCredentialsRelation
            | AdmissionReason::InvalidSentinelEventsRelation
            | AdmissionReason::BelowSupportedFloor { .. }
            | AdmissionReason::FailedMigration { .. }
            | AdmissionReason::DuplicateMigration { .. }
            | AdmissionReason::NonCanonicalOrder { .. }
            | AdmissionReason::ReservedForOtherBackend { .. }
            | AdmissionReason::UnknownMigration { .. }
            | AdmissionReason::DescriptionMismatch { .. }
            | AdmissionReason::ChecksumMismatch { .. }
            | AdmissionReason::OwnerlessCredential
            | AdmissionReason::InvalidCredentialId
            | AdmissionReason::InvalidStateVersion
            | AdmissionReason::InvalidCredentialVersion
            | AdmissionReason::LiveVersionExhausted
            | AdmissionReason::InvalidRecordState
            | AdmissionReason::InvalidTombstoneShape
            | AdmissionReason::MalformedMetadata
            | AdmissionReason::MetadataNotObject
            | AdmissionReason::DuplicateMetadataKey
            | AdmissionReason::InvalidDisplay
            | AdmissionReason::DisplayNameMismatch
            | AdmissionReason::OrphanPhysicalName
            | AdmissionReason::DuplicateProjectedName => {},
        }
    }

    const NAME_CANARY: &str = "customer-name-NEVER-RENDER";
    const ID_CANARY: &str = "credential-id-NEVER-RENDER";
    const METADATA_CANARY: &str = "metadata-secret-NEVER-RENDER";
    const MIGRATION_CANARY: &str = "migration-text-NEVER-RENDER";
    let policy = policy(BackendKind::Sqlite);
    let mut row = credential(&format!(
        r#"{{"display":{{"display_name":"{METADATA_CANARY}"}}}}"#
    ));
    row.name = Some(NAME_CANARY.to_owned());
    row.data_len = usize::MAX;
    let error = classify_schema(&policy, &migrated_observation(&policy, 35, vec![row]))
        .expect_err("mismatched projection must reject");

    let mut invalid_id = credential("{}");
    invalid_id.id = ID_CANARY.to_owned();
    let invalid_id_error = classify_schema(
        &policy,
        &migrated_observation(&policy, 35, vec![invalid_id]),
    )
    .expect_err("invalid typed credential id must reject");

    let mut drifted_ledger = canonical_ledger(&policy, 35);
    let drifted = drifted_ledger
        .iter_mut()
        .find(|row| row.version == 30)
        .expect("canonical fixture contains migration 0030");
    drifted.description = MIGRATION_CANARY.to_owned();
    drifted.checksum = MIGRATION_CANARY.as_bytes().to_vec();
    let migration_error = classify_schema(
        &policy,
        &SchemaObservation {
            migration_ledger: MigrationLedger::Present(drifted_ledger),
            has_user_relations: true,
            has_credentials_relation: true,
            credentials: Vec::new(),
        },
    )
    .expect_err("migration drift must reject");

    for error in [&error, &invalid_id_error, &migration_error] {
        exhaustively_match_closed_reason(error.reason());
        for rendered in [format!("{error}"), format!("{error:?}")] {
            assert!(!rendered.contains(NAME_CANARY));
            assert!(!rendered.contains(ID_CANARY));
            assert!(!rendered.contains(METADATA_CANARY));
            assert!(!rendered.contains(MIGRATION_CANARY));
            assert!(!rendered.contains("SELECT"));
            assert!(!rendered.contains("sqlite://"));
            assert!(!rendered.contains("postgres://"));
        }
    }
}
