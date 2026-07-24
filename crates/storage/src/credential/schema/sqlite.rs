use std::collections::BTreeSet;

use sqlx::{Row, SqliteConnection};

use super::{
    AdmissionReason, CredentialStoreStartupError, LegacyCredentialRecord, MigrationLedger,
    MigrationLedgerRow, SchemaAdmission, SchemaObservation, UnsupportedSchemaVersion,
    classify_schema, observation_fetch_error, sqlite_policy,
};

#[derive(Debug, PartialEq, Eq)]
struct ColumnShape {
    name: String,
    declared_type: String,
    not_null: bool,
    default: Option<String>,
    primary_key_position: i64,
}

#[derive(Debug)]
struct ExpectedColumnShape {
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    default: Option<&'static str>,
    primary_key_position: i64,
}

const fn column(
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    default: Option<&'static str>,
    primary_key_position: i64,
) -> ExpectedColumnShape {
    ExpectedColumnShape {
        name,
        declared_type,
        not_null,
        default,
        primary_key_position,
    }
}

const LEDGER_SHAPE: [ExpectedColumnShape; 6] = [
    column("version", "BIGINT", false, None, 1),
    column("description", "TEXT", true, None, 0),
    column(
        "installed_on",
        "TIMESTAMP",
        true,
        Some("CURRENT_TIMESTAMP"),
        0,
    ),
    column("success", "BOOLEAN", true, None, 0),
    column("checksum", "BLOB", true, None, 0),
    column("execution_time", "BIGINT", true, None, 0),
];

const LEGACY_SHAPE: [ExpectedColumnShape; 13] = [
    column("id", "TEXT", true, None, 1),
    column("name", "TEXT", false, None, 0),
    column("owner_id", "TEXT", false, None, 0),
    column("credential_key", "TEXT", true, None, 0),
    column("state_kind", "TEXT", true, None, 0),
    column("state_version", "INTEGER", true, None, 0),
    column("data", "BLOB", true, None, 0),
    column("version", "INTEGER", true, None, 0),
    column("created_at", "INTEGER", true, None, 0),
    column("updated_at", "INTEGER", true, None, 0),
    column("expires_at", "INTEGER", false, None, 0),
    column("reauth_required", "INTEGER", true, Some("0"), 0),
    column("metadata", "TEXT", true, Some("'{}'"), 0),
];

const LIFECYCLE_SHAPE: [ExpectedColumnShape; 15] = [
    column("id", "TEXT", true, None, 1),
    column("name", "TEXT", false, None, 0),
    column("owner_id", "TEXT", true, None, 0),
    column("credential_key", "TEXT", true, None, 0),
    column("state_kind", "TEXT", true, None, 0),
    column("state_version", "INTEGER", true, None, 0),
    column("data", "BLOB", true, None, 0),
    column("version", "INTEGER", true, None, 0),
    column("created_at", "INTEGER", true, None, 0),
    column("updated_at", "INTEGER", true, None, 0),
    column("expires_at", "INTEGER", false, None, 0),
    column("reauth_required", "INTEGER", true, None, 0),
    column("metadata", "TEXT", true, None, 0),
    column("record_state", "TEXT", true, None, 0),
    column("tombstoned_at", "INTEGER", false, None, 0),
];

const CURRENT_SHAPE: [ExpectedColumnShape; 21] = [
    column("id", "TEXT", true, None, 1),
    column("name", "TEXT", false, None, 0),
    column("owner_id", "TEXT", true, None, 0),
    column("credential_key", "TEXT", true, None, 0),
    column("state_kind", "TEXT", true, None, 0),
    column("state_version", "INTEGER", true, None, 0),
    column("data", "BLOB", true, None, 0),
    column("version", "INTEGER", true, None, 0),
    column("material_epoch", "INTEGER", true, None, 0),
    column("created_at", "INTEGER", true, None, 0),
    column("updated_at", "INTEGER", true, None, 0),
    column("expires_at", "INTEGER", false, None, 0),
    column("reauth_required", "INTEGER", true, None, 0),
    column("metadata", "TEXT", true, None, 0),
    column("record_state", "TEXT", true, None, 0),
    column("tombstoned_at", "INTEGER", false, None, 0),
    column("refresh_retry_mode", "TEXT", false, None, 0),
    column("refresh_retry_not_before", "INTEGER", false, None, 0),
    column("refresh_retry_phase", "TEXT", false, None, 0),
    column("refresh_retry_kind", "TEXT", false, None, 0),
    column("refresh_retry_diagnostic_code", "TEXT", false, None, 0),
];

const LEGACY_SENTINEL_EVENT_SHAPE: [ExpectedColumnShape; 5] = [
    column("id", "INTEGER", false, None, 1),
    column("credential_id", "TEXT", true, None, 0),
    column("detected_at", "INTEGER", true, None, 0),
    column("crashed_holder", "TEXT", true, None, 0),
    column("generation", "INTEGER", true, None, 0),
];

const CURRENT_SENTINEL_EVENT_SHAPE: [ExpectedColumnShape; 6] = [
    column("id", "INTEGER", false, None, 1),
    column("credential_id", "TEXT", true, None, 0),
    column("detected_at", "INTEGER", true, None, 0),
    column("crashed_holder", "TEXT", true, None, 0),
    column("generation", "INTEGER", true, None, 0),
    column("claim_id", "TEXT", false, None, 0),
];

pub(crate) async fn admit(
    connection: &mut SqliteConnection,
) -> Result<SchemaAdmission, CredentialStoreStartupError> {
    let policy = sqlite_policy();
    let observation = observe(connection, policy.current_version).await?;
    classify_schema(&policy, &observation).map_err(Into::into)
}

async fn observe(
    connection: &mut SqliteConnection,
    current_version: i64,
) -> Result<SchemaObservation, CredentialStoreStartupError> {
    let ledger_exists = relation_exists(connection, "_sqlx_migrations").await?;
    let credentials_exists = relation_exists(connection, "credentials").await?;
    let has_user_relations: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM sqlite_schema
             WHERE type IN ('table', 'view')
               AND name NOT LIKE 'sqlite_%'
               AND name <> '_sqlx_migrations'
         )",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| CredentialStoreStartupError::Unavailable)?;

    if !ledger_exists {
        return Ok(SchemaObservation {
            migration_ledger: MigrationLedger::Absent,
            has_user_relations,
            has_credentials_relation: credentials_exists,
            credentials: Vec::new(),
        });
    }

    let ledger_columns = table_shape(connection, "_sqlx_migrations").await?;
    if !matches_shape(&ledger_columns, &LEDGER_SHAPE) {
        return unsupported(AdmissionReason::InvalidMigrationLedger);
    }
    let ledger_rows = sqlx::query_as::<_, (i64, String, bool, Vec<u8>)>(
        "SELECT version, description, success, checksum
         FROM _sqlx_migrations
         ORDER BY version",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| observation_fetch_error(error, AdmissionReason::InvalidMigrationLedger))?
    .into_iter()
    .map(
        |(version, description, success, checksum)| MigrationLedgerRow {
            version,
            description,
            checksum,
            success,
        },
    )
    .collect::<Vec<_>>();

    let latest = ledger_rows.last().map(|row| row.version);
    let latest_is_supported = latest.is_some_and(|version| version <= current_version);
    if latest_is_supported && latest.is_some_and(|version| version >= 30) {
        if !relation_exists(connection, "credential_sentinel_events").await? {
            return unsupported(AdmissionReason::InvalidSentinelEventsRelation);
        }
        validate_sentinel_events_relation(connection, latest.is_some_and(|version| version >= 39))
            .await?;
    }
    let credentials =
        if credentials_exists && latest_is_supported && latest.is_some_and(|version| version >= 30)
        {
            let latest = latest.ok_or(CredentialStoreStartupError::Unavailable)?;
            validate_credentials_relation(connection, latest).await?;
            credential_rows(connection, latest).await?
        } else {
            Vec::new()
        };

    Ok(SchemaObservation {
        migration_ledger: MigrationLedger::Present(ledger_rows),
        has_user_relations,
        has_credentials_relation: credentials_exists,
        credentials,
    })
}

async fn relation_exists(
    connection: &mut SqliteConnection,
    relation: &'static str,
) -> Result<bool, CredentialStoreStartupError> {
    sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM sqlite_schema
             WHERE type = 'table' AND name = ?
         )",
    )
    .bind(relation)
    .fetch_one(connection)
    .await
    .map_err(|_| CredentialStoreStartupError::Unavailable)
}

async fn table_shape(
    connection: &mut SqliteConnection,
    table: &'static str,
) -> Result<Vec<ColumnShape>, CredentialStoreStartupError> {
    let statement = match table {
        "_sqlx_migrations" => "PRAGMA table_info('_sqlx_migrations')",
        "credentials" => "PRAGMA table_info('credentials')",
        "credential_sentinel_events" => "PRAGMA table_info('credential_sentinel_events')",
        _ => return unsupported(AdmissionReason::InvalidCredentialsRelation),
    };
    let rows = sqlx::query(statement)
        .fetch_all(connection)
        .await
        .map_err(|_| CredentialStoreStartupError::Unavailable)?;
    rows.into_iter()
        .map(|row| {
            Ok(ColumnShape {
                name: row
                    .try_get("name")
                    .map_err(|_| CredentialStoreStartupError::Unavailable)?,
                declared_type: row
                    .try_get("type")
                    .map_err(|_| CredentialStoreStartupError::Unavailable)?,
                not_null: row
                    .try_get::<i64, _>("notnull")
                    .map_err(|_| CredentialStoreStartupError::Unavailable)?
                    == 1,
                default: row
                    .try_get("dflt_value")
                    .map_err(|_| CredentialStoreStartupError::Unavailable)?,
                primary_key_position: row
                    .try_get("pk")
                    .map_err(|_| CredentialStoreStartupError::Unavailable)?,
            })
        })
        .collect()
}

fn matches_shape(actual: &[ColumnShape], expected: &[ExpectedColumnShape]) -> bool {
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(actual, expected)| {
            actual.name == expected.name
                && actual.declared_type == expected.declared_type
                && actual.not_null == expected.not_null
                && actual.default.as_deref() == expected.default
                && actual.primary_key_position == expected.primary_key_position
        })
}

async fn validate_credentials_relation(
    connection: &mut SqliteConnection,
    latest: i64,
) -> Result<(), CredentialStoreStartupError> {
    let columns = table_shape(connection, "credentials").await?;
    let expected = if latest >= 40 {
        CURRENT_SHAPE.as_slice()
    } else if latest >= 39 {
        LIFECYCLE_SHAPE.as_slice()
    } else {
        LEGACY_SHAPE.as_slice()
    };
    if !matches_shape(&columns, expected) {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    }

    let table_sql: Option<String> = sqlx::query_scalar(
        "SELECT sql
         FROM sqlite_schema
         WHERE type = 'table' AND name = 'credentials'",
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| CredentialStoreStartupError::Unavailable)?
    .flatten();
    let Some(table_sql) = table_sql else {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    };
    if latest >= 40 {
        validate_current_checks(&table_sql)?;
    } else if latest >= 39 {
        validate_lifecycle_checks(&table_sql)?;
    } else if normalize_schema_sql(&table_sql).matches("check(").count() != 0 {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    }

    validate_indexes(connection, latest >= 39).await
}

async fn validate_sentinel_events_relation(
    connection: &mut SqliteConnection,
    current: bool,
) -> Result<(), CredentialStoreStartupError> {
    let columns = table_shape(connection, "credential_sentinel_events").await?;
    let expected = if current {
        CURRENT_SENTINEL_EVENT_SHAPE.as_slice()
    } else {
        LEGACY_SENTINEL_EVENT_SHAPE.as_slice()
    };
    if !matches_shape(&columns, expected) {
        return unsupported(AdmissionReason::InvalidSentinelEventsRelation);
    }

    let indexes = sqlx::query("PRAGMA index_list('credential_sentinel_events')")
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| CredentialStoreStartupError::Unavailable)?;
    let expected_names = if current {
        BTreeSet::from([
            "idx_credential_sentinel_events_claim_id".to_owned(),
            "idx_sentinel_events_cred_time".to_owned(),
        ])
    } else {
        BTreeSet::from(["idx_sentinel_events_cred_time".to_owned()])
    };
    let names = indexes
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<BTreeSet<_>>();
    if names != expected_names {
        return unsupported(AdmissionReason::InvalidSentinelEventsRelation);
    }

    let attributes = |name: &str| {
        indexes
            .iter()
            .find(|row| row.get::<String, _>("name") == name)
            .map(|row| {
                (
                    row.get::<i64, _>("unique"),
                    row.get::<i64, _>("partial"),
                    row.get::<String, _>("origin"),
                )
            })
    };
    if attributes("idx_sentinel_events_cred_time") != Some((0, 0, "c".to_owned()))
        || (current
            && attributes("idx_credential_sentinel_events_claim_id")
                != Some((1, 1, "c".to_owned())))
    {
        return unsupported(AdmissionReason::InvalidSentinelEventsRelation);
    }

    let time_columns: Vec<String> = sqlx::query_scalar(
        "SELECT name
         FROM pragma_index_info('idx_sentinel_events_cred_time')
         ORDER BY seqno",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| CredentialStoreStartupError::Unavailable)?;
    if time_columns != ["credential_id", "detected_at"] {
        return unsupported(AdmissionReason::InvalidSentinelEventsRelation);
    }

    if current {
        let identity_columns: Vec<String> = sqlx::query_scalar(
            "SELECT name
             FROM pragma_index_info('idx_credential_sentinel_events_claim_id')
             ORDER BY seqno",
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        let identity_sql: Option<String> = sqlx::query_scalar(
            "SELECT sql
             FROM sqlite_schema
             WHERE type = 'index'
               AND name = 'idx_credential_sentinel_events_claim_id'",
        )
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| CredentialStoreStartupError::Unavailable)?
        .flatten();
        if identity_columns != ["claim_id"]
            || identity_sql.as_deref().is_none_or(|actual| {
                normalize_schema_sql(actual)
                    != normalize_schema_sql(
                        "CREATE UNIQUE INDEX idx_credential_sentinel_events_claim_id
                         ON credential_sentinel_events(claim_id)
                         WHERE claim_id IS NOT NULL",
                    )
            })
        {
            return unsupported(AdmissionReason::InvalidSentinelEventsRelation);
        }
    }

    Ok(())
}

const LIFECYCLE_CHECKS: [(&str, &str); 7] = [
    (
        "credentials_state_version_range",
        "CONSTRAINT credentials_state_version_range
         CHECK (
             typeof(state_version) = 'integer'
             AND state_version BETWEEN 0 AND 4294967295
         )",
    ),
    (
        "credentials_version_range",
        "CONSTRAINT credentials_version_range
         CHECK (
             typeof(version) = 'integer'
             AND version BETWEEN 1 AND 9223372036854775807
         )",
    ),
    (
        "credentials_reauth_boolean",
        "CONSTRAINT credentials_reauth_boolean
         CHECK (
             typeof(reauth_required) = 'integer'
             AND reauth_required IN (0, 1)
         )",
    ),
    (
        "credentials_data_blob",
        "CONSTRAINT credentials_data_blob
         CHECK (typeof(data) = 'blob')",
    ),
    (
        "credentials_metadata_object",
        "CONSTRAINT credentials_metadata_object
         CHECK (
             typeof(metadata) = 'text'
             AND json_valid(metadata)
             AND json_type(metadata) = 'object'
         )",
    ),
    (
        "credentials_live_name_projection",
        "CONSTRAINT credentials_live_name_projection
         CHECK (
             record_state = 'tombstoned'
             OR (
                 record_state = 'live'
                 AND (
                     (
                         name IS NULL
                         AND (
                             json_type(metadata, '$.display.display_name') IS NULL
                             OR json_type(metadata, '$.display.display_name') = 'null'
                         )
                     )
                     OR (
                         name IS NOT NULL
                         AND json_type(metadata, '$.display.display_name') = 'text'
                         AND name = json_extract(metadata, '$.display.display_name')
                     )
                 )
             )
         )",
    ),
    (
        "credentials_record_shape",
        "CONSTRAINT credentials_record_shape
         CHECK (
             (
                 record_state = 'live'
                 AND tombstoned_at IS NULL
                 AND version <= 9223372036854775806
             )
             OR
             (
                 record_state = 'tombstoned'
                 AND tombstoned_at IS NOT NULL
                 AND length(data) = 0
                 AND name IS NULL
                 AND expires_at IS NULL
                 AND reauth_required = 0
                 AND metadata = '{}'
             )
         )",
    ),
];

const CURRENT_CHECKS: [(&str, &str); 9] = [
    LIFECYCLE_CHECKS[0],
    LIFECYCLE_CHECKS[1],
    LIFECYCLE_CHECKS[2],
    LIFECYCLE_CHECKS[3],
    LIFECYCLE_CHECKS[4],
    LIFECYCLE_CHECKS[5],
    (
        "credentials_material_epoch_range",
        "CONSTRAINT credentials_material_epoch_range
         CHECK (
             typeof(material_epoch) = 'integer'
             AND material_epoch BETWEEN 1 AND 9223372036854775807
         )",
    ),
    (
        "credentials_refresh_retry_gate_shape",
        "CONSTRAINT credentials_refresh_retry_gate_shape
         CHECK (
             (
                 refresh_retry_mode IS NULL
                 AND refresh_retry_not_before IS NULL
                 AND refresh_retry_phase IS NULL
                 AND refresh_retry_kind IS NULL
                 AND refresh_retry_diagnostic_code IS NULL
             )
             OR
             (
                 record_state = 'live'
                 AND refresh_retry_mode IS NOT NULL
                 AND refresh_retry_phase IS NOT NULL
                 AND refresh_retry_phase IN (
                     'before_dispatch',
                     'provider_confirmed_not_applied'
                 )
                 AND refresh_retry_kind IS NOT NULL
                 AND refresh_retry_kind IN (
                     'transient_network',
                     'provider_unavailable',
                     'protocol_error'
                 )
                 AND (
                     refresh_retry_diagnostic_code IS NULL
                     OR (
                         typeof(refresh_retry_diagnostic_code) = 'text'
                         AND length(refresh_retry_diagnostic_code) BETWEEN 1 AND 64
                         AND refresh_retry_diagnostic_code
                             NOT GLOB '*[^A-Za-z0-9_.:-]*'
                     )
                 )
                 AND (
                     (
                         refresh_retry_mode = 'never'
                         AND refresh_retry_not_before IS NULL
                     )
                     OR
                     (
                         refresh_retry_mode = 'not_before'
                         AND refresh_retry_not_before IS NOT NULL
                         AND typeof(refresh_retry_not_before) = 'integer'
                     )
                 )
             )
         )",
    ),
    (
        "credentials_record_shape",
        "CONSTRAINT credentials_record_shape
         CHECK (
             (
                 record_state = 'live'
                 AND tombstoned_at IS NULL
                 AND version <= 9223372036854775806
             )
             OR
             (
                 record_state = 'tombstoned'
                 AND tombstoned_at IS NOT NULL
                 AND length(data) = 0
                 AND name IS NULL
                 AND expires_at IS NULL
                 AND reauth_required = 0
                 AND metadata = '{}'
                 AND refresh_retry_mode IS NULL
                 AND refresh_retry_not_before IS NULL
                 AND refresh_retry_phase IS NULL
                 AND refresh_retry_kind IS NULL
                 AND refresh_retry_diagnostic_code IS NULL
             )
         )",
    ),
];

fn validate_lifecycle_checks(table_sql: &str) -> Result<(), CredentialStoreStartupError> {
    validate_checks(table_sql, &LIFECYCLE_CHECKS)
}

fn validate_current_checks(table_sql: &str) -> Result<(), CredentialStoreStartupError> {
    validate_checks(table_sql, &CURRENT_CHECKS)
}

fn validate_checks(
    table_sql: &str,
    checks: &[(&str, &str)],
) -> Result<(), CredentialStoreStartupError> {
    let normalized = normalize_schema_sql(table_sql);
    if normalized.matches("check(").count() != checks.len() {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    }
    for (name, required) in checks {
        let constraint_token = format!("constraint{name}check(");
        let required = normalize_schema_sql(required);
        if normalized.matches(&constraint_token).count() != 1
            || normalized.matches(&required).count() != 1
        {
            return unsupported(AdmissionReason::InvalidCredentialsRelation);
        }
    }
    Ok(())
}

fn normalize_schema_sql(sql: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        SingleQuoted,
        DoubleQuoted,
        LineComment,
        BlockComment,
    }

    let mut normalized = String::with_capacity(sql.len());
    let mut characters = sql.chars().peekable();
    let mut state = State::Code;
    while let Some(character) = characters.next() {
        match state {
            State::Code => match (character, characters.peek().copied()) {
                ('-', Some('-')) => {
                    characters.next();
                    state = State::LineComment;
                },
                ('/', Some('*')) => {
                    characters.next();
                    state = State::BlockComment;
                },
                ('\'', _) => {
                    normalized.push(character);
                    state = State::SingleQuoted;
                },
                ('"', _) => {
                    normalized.push(character);
                    state = State::DoubleQuoted;
                },
                _ if character.is_ascii_whitespace() => {},
                _ => normalized.push(character.to_ascii_lowercase()),
            },
            State::SingleQuoted => {
                normalized.push(character);
                if character == '\'' {
                    if characters.peek() == Some(&'\'') {
                        normalized.push(characters.next().unwrap_or('\''));
                    } else {
                        state = State::Code;
                    }
                }
            },
            State::DoubleQuoted => {
                normalized.push(character);
                if character == '"' {
                    if characters.peek() == Some(&'"') {
                        normalized.push(characters.next().unwrap_or('"'));
                    } else {
                        state = State::Code;
                    }
                }
            },
            State::LineComment => {
                if character == '\n' {
                    state = State::Code;
                }
            },
            State::BlockComment => {
                if character == '*' && characters.peek() == Some(&'/') {
                    characters.next();
                    state = State::Code;
                }
            },
        }
    }
    normalized
}

async fn validate_indexes(
    connection: &mut SqliteConnection,
    current: bool,
) -> Result<(), CredentialStoreStartupError> {
    let indexes = sqlx::query("PRAGMA index_list('credentials')")
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| CredentialStoreStartupError::Unavailable)?;
    let names = indexes
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<BTreeSet<_>>();
    let expected_names = BTreeSet::from([
        "idx_credentials_expiring".to_owned(),
        "idx_credentials_owner_name".to_owned(),
        "idx_credentials_state_kind".to_owned(),
        "sqlite_autoindex_credentials_1".to_owned(),
    ]);
    if names != expected_names {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    }

    let index_attributes = |name: &str| {
        indexes
            .iter()
            .find(|row| row.get::<String, _>("name") == name)
            .map(|row| {
                (
                    row.get::<i64, _>("unique"),
                    row.get::<i64, _>("partial"),
                    row.get::<String, _>("origin"),
                )
            })
    };
    let owner_name_attributes = if current {
        (1, 0, "c".to_owned())
    } else {
        (1, 1, "c".to_owned())
    };
    if index_attributes("idx_credentials_owner_name") != Some(owner_name_attributes)
        || index_attributes("idx_credentials_state_kind") != Some((0, 0, "c".to_owned()))
        || index_attributes("idx_credentials_expiring") != Some((0, 1, "c".to_owned()))
        || index_attributes("sqlite_autoindex_credentials_1") != Some((1, 0, "pk".to_owned()))
    {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    }

    let owner_name = index_columns(connection, "idx_credentials_owner_name").await?;
    let state = index_columns(connection, "idx_credentials_state_kind").await?;
    let expiry = index_columns(connection, "idx_credentials_expiring").await?;
    let primary_key = index_columns(connection, "sqlite_autoindex_credentials_1").await?;
    if owner_name != ["owner_id", "name"]
        || state != ["state_kind"]
        || expiry != ["expires_at"]
        || primary_key != ["id"]
    {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    }

    let owner_name_sql = if current {
        "CREATE UNIQUE INDEX idx_credentials_owner_name
         ON credentials(owner_id, name)"
    } else {
        "CREATE UNIQUE INDEX idx_credentials_owner_name
         ON credentials(owner_id, name)
         WHERE name IS NOT NULL"
    };
    for (name, expected_sql) in [
        ("idx_credentials_owner_name", owner_name_sql),
        (
            "idx_credentials_state_kind",
            "CREATE INDEX idx_credentials_state_kind
             ON credentials(state_kind)",
        ),
        (
            "idx_credentials_expiring",
            "CREATE INDEX idx_credentials_expiring
             ON credentials(expires_at)
             WHERE expires_at IS NOT NULL",
        ),
    ] {
        let actual_sql: Option<String> =
            sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?")
                .bind(name)
                .fetch_optional(&mut *connection)
                .await
                .map_err(|_| CredentialStoreStartupError::Unavailable)?
                .flatten();
        if actual_sql
            .as_deref()
            .is_none_or(|actual| normalize_schema_sql(actual) != normalize_schema_sql(expected_sql))
        {
            return unsupported(AdmissionReason::InvalidCredentialsRelation);
        }
    }

    Ok(())
}

async fn index_columns(
    connection: &mut SqliteConnection,
    index: &'static str,
) -> Result<Vec<String>, CredentialStoreStartupError> {
    let statement = match index {
        "idx_credentials_owner_name" => {
            "SELECT name FROM pragma_index_info('idx_credentials_owner_name') ORDER BY seqno"
        },
        "idx_credentials_state_kind" => {
            "SELECT name FROM pragma_index_info('idx_credentials_state_kind') ORDER BY seqno"
        },
        "idx_credentials_expiring" => {
            "SELECT name FROM pragma_index_info('idx_credentials_expiring') ORDER BY seqno"
        },
        "sqlite_autoindex_credentials_1" => {
            "SELECT name FROM pragma_index_info('sqlite_autoindex_credentials_1') ORDER BY seqno"
        },
        _ => return unsupported(AdmissionReason::InvalidCredentialsRelation),
    };
    sqlx::query_scalar(statement)
        .fetch_all(connection)
        .await
        .map_err(|_| CredentialStoreStartupError::Unavailable)
}

async fn credential_rows(
    connection: &mut SqliteConnection,
    latest: i64,
) -> Result<Vec<LegacyCredentialRecord>, CredentialStoreStartupError> {
    let rows = if latest >= 40 {
        sqlx::query(
            "SELECT id, owner_id, name, state_version, length(data) AS data_len,
                    version, material_epoch, metadata, record_state,
                    tombstoned_at IS NOT NULL AS tombstoned_at_present,
                    expires_at IS NOT NULL AS expires_at_present,
                    reauth_required,
                    refresh_retry_mode,
                    refresh_retry_not_before IS NOT NULL
                        AS refresh_retry_not_before_present,
                    refresh_retry_phase,
                    refresh_retry_kind,
                    refresh_retry_diagnostic_code,
                    typeof(id) AS id_type,
                    typeof(owner_id) AS owner_id_type,
                    typeof(name) AS name_type,
                    typeof(credential_key) AS credential_key_type,
                    typeof(state_kind) AS state_kind_type,
                    typeof(state_version) AS state_version_type,
                    typeof(data) AS data_type,
                    typeof(version) AS version_type,
                    typeof(material_epoch) AS material_epoch_type,
                    typeof(created_at) AS created_at_type,
                    typeof(updated_at) AS updated_at_type,
                    typeof(expires_at) AS expires_at_type,
                    typeof(reauth_required) AS reauth_required_type,
                    typeof(metadata) AS metadata_type,
                    typeof(record_state) AS record_state_type,
                    typeof(tombstoned_at) AS tombstoned_at_type,
                    typeof(refresh_retry_mode) AS refresh_retry_mode_type,
                    typeof(refresh_retry_not_before) AS refresh_retry_not_before_type,
                    typeof(refresh_retry_phase) AS refresh_retry_phase_type,
                    typeof(refresh_retry_kind) AS refresh_retry_kind_type,
                    typeof(refresh_retry_diagnostic_code)
                        AS refresh_retry_diagnostic_code_type
             FROM credentials",
        )
        .fetch_all(connection)
        .await
    } else if latest >= 39 {
        sqlx::query(
            "SELECT id, owner_id, name, state_version, length(data) AS data_len,
                    version, NULL AS material_epoch, metadata, record_state,
                    tombstoned_at IS NOT NULL AS tombstoned_at_present,
                    expires_at IS NOT NULL AS expires_at_present,
                    reauth_required,
                    NULL AS refresh_retry_mode,
                    0 AS refresh_retry_not_before_present,
                    NULL AS refresh_retry_phase,
                    NULL AS refresh_retry_kind,
                    NULL AS refresh_retry_diagnostic_code,
                    typeof(id) AS id_type,
                    typeof(owner_id) AS owner_id_type,
                    typeof(name) AS name_type,
                    typeof(credential_key) AS credential_key_type,
                    typeof(state_kind) AS state_kind_type,
                    typeof(state_version) AS state_version_type,
                    typeof(data) AS data_type,
                    typeof(version) AS version_type,
                    'null' AS material_epoch_type,
                    typeof(created_at) AS created_at_type,
                    typeof(updated_at) AS updated_at_type,
                    typeof(expires_at) AS expires_at_type,
                    typeof(reauth_required) AS reauth_required_type,
                    typeof(metadata) AS metadata_type,
                    typeof(record_state) AS record_state_type,
                    typeof(tombstoned_at) AS tombstoned_at_type,
                    'null' AS refresh_retry_mode_type,
                    'null' AS refresh_retry_not_before_type,
                    'null' AS refresh_retry_phase_type,
                    'null' AS refresh_retry_kind_type,
                    'null' AS refresh_retry_diagnostic_code_type
             FROM credentials",
        )
        .fetch_all(connection)
        .await
    } else {
        sqlx::query(
            "SELECT id, owner_id, name, state_version, length(data) AS data_len,
                    version, NULL AS material_epoch, metadata, NULL AS record_state,
                    0 AS tombstoned_at_present,
                    expires_at IS NOT NULL AS expires_at_present,
                    reauth_required,
                    NULL AS refresh_retry_mode,
                    0 AS refresh_retry_not_before_present,
                    NULL AS refresh_retry_phase,
                    NULL AS refresh_retry_kind,
                    NULL AS refresh_retry_diagnostic_code,
                    typeof(id) AS id_type,
                    typeof(owner_id) AS owner_id_type,
                    typeof(name) AS name_type,
                    typeof(credential_key) AS credential_key_type,
                    typeof(state_kind) AS state_kind_type,
                    typeof(state_version) AS state_version_type,
                    typeof(data) AS data_type,
                    typeof(version) AS version_type,
                    'null' AS material_epoch_type,
                    typeof(created_at) AS created_at_type,
                    typeof(updated_at) AS updated_at_type,
                    typeof(expires_at) AS expires_at_type,
                    typeof(reauth_required) AS reauth_required_type,
                    typeof(metadata) AS metadata_type,
                    'null' AS record_state_type,
                    'null' AS tombstoned_at_type,
                    'null' AS refresh_retry_mode_type,
                    'null' AS refresh_retry_not_before_type,
                    'null' AS refresh_retry_phase_type,
                    'null' AS refresh_retry_kind_type,
                    'null' AS refresh_retry_diagnostic_code_type
             FROM credentials",
        )
        .fetch_all(connection)
        .await
    }
    .map_err(|error| observation_fetch_error(error, AdmissionReason::InvalidCredentialsRelation))?;

    rows.into_iter()
        .map(|row| {
            let storage_type = |column| {
                row.try_get::<String, _>(column)
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))
            };
            let id_type = storage_type("id_type")?;
            let owner_id_type = storage_type("owner_id_type")?;
            let name_type = storage_type("name_type")?;
            let credential_key_type = storage_type("credential_key_type")?;
            let state_kind_type = storage_type("state_kind_type")?;
            let state_version_type = storage_type("state_version_type")?;
            let data_type = storage_type("data_type")?;
            let version_type = storage_type("version_type")?;
            let material_epoch_type = storage_type("material_epoch_type")?;
            let created_at_type = storage_type("created_at_type")?;
            let updated_at_type = storage_type("updated_at_type")?;
            let expires_at_type = storage_type("expires_at_type")?;
            let reauth_required_type = storage_type("reauth_required_type")?;
            let metadata_type = storage_type("metadata_type")?;
            let record_state_type = storage_type("record_state_type")?;
            let tombstoned_at_type = storage_type("tombstoned_at_type")?;
            let refresh_retry_mode_type = storage_type("refresh_retry_mode_type")?;
            let refresh_retry_not_before_type = storage_type("refresh_retry_not_before_type")?;
            let refresh_retry_phase_type = storage_type("refresh_retry_phase_type")?;
            let refresh_retry_kind_type = storage_type("refresh_retry_kind_type")?;
            let refresh_retry_diagnostic_code_type =
                storage_type("refresh_retry_diagnostic_code_type")?;
            let nullable_text = |value: &str| matches!(value, "text" | "null");
            let nullable_integer = |value: &str| matches!(value, "integer" | "null");
            let has_lifecycle = latest >= 39;
            let has_retry_gate = latest >= 40;
            if id_type != "text"
                || !nullable_text(&owner_id_type)
                || !nullable_text(&name_type)
                || credential_key_type != "text"
                || state_kind_type != "text"
                || state_version_type != "integer"
                || data_type != "blob"
                || version_type != "integer"
                || (has_retry_gate && material_epoch_type != "integer")
                || (!has_retry_gate && material_epoch_type != "null")
                || created_at_type != "integer"
                || updated_at_type != "integer"
                || !nullable_integer(&expires_at_type)
                || reauth_required_type != "integer"
                || metadata_type != "text"
                || (has_lifecycle && record_state_type != "text")
                || (!has_lifecycle && record_state_type != "null")
                || (has_lifecycle && !nullable_integer(&tombstoned_at_type))
                || (!has_lifecycle && tombstoned_at_type != "null")
                || (has_retry_gate && !nullable_text(&refresh_retry_mode_type))
                || (!has_retry_gate && refresh_retry_mode_type != "null")
                || (has_retry_gate && !nullable_integer(&refresh_retry_not_before_type))
                || (!has_retry_gate && refresh_retry_not_before_type != "null")
                || (has_retry_gate && !nullable_text(&refresh_retry_phase_type))
                || (!has_retry_gate && refresh_retry_phase_type != "null")
                || (has_retry_gate && !nullable_text(&refresh_retry_kind_type))
                || (!has_retry_gate && refresh_retry_kind_type != "null")
                || (has_retry_gate && !nullable_text(&refresh_retry_diagnostic_code_type))
                || (!has_retry_gate && refresh_retry_diagnostic_code_type != "null")
            {
                return Err(unsupported_error(
                    AdmissionReason::InvalidCredentialsRelation,
                ));
            }
            let reauth_required = row
                .try_get::<i64, _>("reauth_required")
                .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?;
            if !matches!(reauth_required, 0 | 1) {
                return Err(unsupported_error(
                    AdmissionReason::InvalidCredentialsRelation,
                ));
            }
            let data_len = row
                .try_get::<i64, _>("data_len")
                .ok()
                .and_then(|length| usize::try_from(length).ok())
                .ok_or_else(|| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?;
            Ok(LegacyCredentialRecord {
                id: row
                    .try_get("id")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                owner_id: row
                    .try_get("owner_id")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                name: row
                    .try_get("name")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                state_version: row
                    .try_get("state_version")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                data_len,
                version: row
                    .try_get("version")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                material_epoch: row
                    .try_get("material_epoch")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                metadata: row
                    .try_get("metadata")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                record_state: row
                    .try_get("record_state")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                tombstoned_at_present: row
                    .try_get("tombstoned_at_present")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                expires_at_present: row
                    .try_get("expires_at_present")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                reauth_required: reauth_required != 0,
                refresh_retry_mode: row
                    .try_get("refresh_retry_mode")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                refresh_retry_not_before_present: row
                    .try_get("refresh_retry_not_before_present")
                    .map_err(|_| {
                    unsupported_error(AdmissionReason::InvalidCredentialsRelation)
                })?,
                refresh_retry_phase: row
                    .try_get("refresh_retry_phase")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                refresh_retry_kind: row
                    .try_get("refresh_retry_kind")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
                refresh_retry_diagnostic_code: row
                    .try_get("refresh_retry_diagnostic_code")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
            })
        })
        .collect()
}

fn unsupported<T>(reason: AdmissionReason) -> Result<T, CredentialStoreStartupError> {
    Err(unsupported_error(reason))
}

fn unsupported_error(reason: AdmissionReason) -> CredentialStoreStartupError {
    CredentialStoreStartupError::UnsupportedSchemaVersion(UnsupportedSchemaVersion::new(reason))
}
