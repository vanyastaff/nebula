use sqlx::{PgConnection, Row};

use super::{
    AdmissionReason, CredentialStoreStartupError, LegacyCredentialRecord, MigrationLedger,
    MigrationLedgerRow, SchemaAdmission, SchemaObservation, UnsupportedSchemaVersion,
    classify_schema, observation_fetch_error, postgres_policy,
};

#[derive(Debug, Clone, Copy)]
struct ExpectedColumn {
    name: &'static str,
    data_type: &'static str,
    nullable: bool,
    default: Option<&'static str>,
}

const fn column(
    name: &'static str,
    data_type: &'static str,
    nullable: bool,
    default: Option<&'static str>,
) -> ExpectedColumn {
    ExpectedColumn {
        name,
        data_type,
        nullable,
        default,
    }
}

const LEDGER_COLUMNS: [ExpectedColumn; 6] = [
    column("version", "int8", false, None),
    column("description", "text", false, None),
    column("installed_on", "timestamptz", false, Some("now()")),
    column("success", "bool", false, None),
    column("checksum", "bytea", false, None),
    column("execution_time", "int8", false, None),
];

const LEGACY_COLUMNS: [ExpectedColumn; 13] = [
    column("id", "text", false, None),
    column("name", "text", true, None),
    column("owner_id", "text", true, None),
    column("credential_key", "text", false, None),
    column("state_kind", "text", false, None),
    column("state_version", "int8", false, None),
    column("data", "bytea", false, None),
    column("version", "int8", false, None),
    column("created_at", "timestamptz", false, None),
    column("updated_at", "timestamptz", false, None),
    column("expires_at", "timestamptz", true, None),
    column("reauth_required", "bool", false, Some("false")),
    column("metadata", "text", false, Some("'{}'::text")),
];

const LIFECYCLE_COLUMNS: [ExpectedColumn; 15] = [
    column("id", "text", false, None),
    column("name", "text", true, None),
    column("owner_id", "text", false, None),
    column("credential_key", "text", false, None),
    column("state_kind", "text", false, None),
    column("state_version", "int8", false, None),
    column("data", "bytea", false, None),
    column("version", "int8", false, None),
    column("created_at", "timestamptz", false, None),
    column("updated_at", "timestamptz", false, None),
    column("expires_at", "timestamptz", true, None),
    column("reauth_required", "bool", false, Some("false")),
    column("metadata", "text", false, Some("'{}'::text")),
    column("record_state", "text", false, None),
    column("tombstoned_at", "timestamptz", true, None),
];

const CURRENT_COLUMNS: [ExpectedColumn; 21] = [
    column("id", "text", false, None),
    column("name", "text", true, None),
    column("owner_id", "text", false, None),
    column("credential_key", "text", false, None),
    column("state_kind", "text", false, None),
    column("state_version", "int8", false, None),
    column("data", "bytea", false, None),
    column("version", "int8", false, None),
    column("material_epoch", "int8", false, None),
    column("created_at", "timestamptz", false, None),
    column("updated_at", "timestamptz", false, None),
    column("expires_at", "timestamptz", true, None),
    column("reauth_required", "bool", false, Some("false")),
    column("metadata", "text", false, Some("'{}'::text")),
    column("record_state", "text", false, None),
    column("tombstoned_at", "timestamptz", true, None),
    column("refresh_retry_mode", "text", true, None),
    column("refresh_retry_not_before", "timestamptz", true, None),
    column("refresh_retry_phase", "text", true, None),
    column("refresh_retry_kind", "text", true, None),
    column("refresh_retry_diagnostic_code", "text", true, None),
];

const LEGACY_SENTINEL_EVENT_COLUMNS: [ExpectedColumn; 5] = [
    column(
        "id",
        "int8",
        false,
        Some("nextval('credential_sentinel_events_id_seq'::regclass)"),
    ),
    column("credential_id", "text", false, None),
    column("detected_at", "timestamptz", false, None),
    column("crashed_holder", "text", false, None),
    column("generation", "int8", false, None),
];

const CURRENT_SENTINEL_EVENT_COLUMNS: [ExpectedColumn; 6] = [
    column(
        "id",
        "int8",
        false,
        Some("nextval('credential_sentinel_events_id_seq'::regclass)"),
    ),
    column("credential_id", "text", false, None),
    column("detected_at", "timestamptz", false, None),
    column("crashed_holder", "text", false, None),
    column("generation", "int8", false, None),
    column("claim_id", "uuid", true, None),
];

#[derive(sqlx::FromRow)]
struct ColumnShape {
    name: String,
    data_type: String,
    nullable: bool,
    default_value: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ConstraintShape {
    name: String,
    kind: String,
    definition: String,
}

#[derive(sqlx::FromRow)]
struct IndexShape {
    name: String,
    unique_index: bool,
    valid: bool,
    key_count: i16,
    has_expressions: bool,
    access_method: String,
    predicate: Option<String>,
    columns: Vec<String>,
    collations: Vec<String>,
    operator_classes: Vec<String>,
    index_options: String,
}

#[derive(Clone, Copy)]
struct ExpectedIndex {
    name: &'static str,
    unique_index: bool,
    predicate: Option<&'static str>,
    columns: &'static [&'static str],
    collations: &'static [&'static str],
    operator_classes: &'static [&'static str],
    options: &'static str,
}

const fn index(
    name: &'static str,
    unique: bool,
    predicate: Option<&'static str>,
    columns: &'static [&'static str],
    collations: &'static [&'static str],
    operator_classes: &'static [&'static str],
    options: &'static str,
) -> ExpectedIndex {
    ExpectedIndex {
        name,
        unique_index: unique,
        predicate,
        columns,
        collations,
        operator_classes,
        options,
    }
}

pub(crate) async fn admit(
    connection: &mut PgConnection,
) -> Result<SchemaAdmission, CredentialStoreStartupError> {
    let policy = postgres_policy();
    let observation = observe(connection, policy.current_version).await?;
    classify_schema(&policy, &observation).map_err(Into::into)
}

async fn observe(
    connection: &mut PgConnection,
    current_version: i64,
) -> Result<SchemaObservation, CredentialStoreStartupError> {
    let ledger_exists = relation_exists(connection, "_sqlx_migrations").await?;
    let credentials_exists = relation_exists(connection, "credentials").await?;
    let has_user_relations: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM pg_class AS relation
             JOIN pg_namespace AS namespace ON namespace.oid = relation.relnamespace
             WHERE namespace.nspname = current_schema()
               AND relation.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
               AND relation.relname <> '_sqlx_migrations'
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

    if !columns_match(connection, "_sqlx_migrations", &LEDGER_COLUMNS).await? {
        return unsupported(AdmissionReason::InvalidMigrationLedger);
    }
    let ledger_constraints = constraint_shapes(connection, "_sqlx_migrations").await?;
    if !constraints_match(
        &ledger_constraints,
        &[("_sqlx_migrations_pkey", "p", "PRIMARY KEY (version)")],
    ) {
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
    connection: &mut PgConnection,
    relation: &'static str,
) -> Result<bool, CredentialStoreStartupError> {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(relation)
        .fetch_one(connection)
        .await
        .map_err(|_| CredentialStoreStartupError::Unavailable)
}

async fn column_shapes(
    connection: &mut PgConnection,
    table: &'static str,
) -> Result<Vec<ColumnShape>, CredentialStoreStartupError> {
    sqlx::query_as(
        "SELECT column_name AS name,
                udt_name AS data_type,
                is_nullable = 'YES' AS nullable,
                column_default AS default_value
         FROM information_schema.columns
         WHERE table_schema = current_schema() AND table_name = $1
         ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(connection)
    .await
    .map_err(|_| CredentialStoreStartupError::Unavailable)
}

async fn columns_match(
    connection: &mut PgConnection,
    table: &'static str,
    expected: &[ExpectedColumn],
) -> Result<bool, CredentialStoreStartupError> {
    let actual = column_shapes(connection, table).await?;
    Ok(actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(actual, expected)| {
            actual.name == expected.name
                && actual.data_type == expected.data_type
                && actual.nullable == expected.nullable
                && actual.default_value.as_deref() == expected.default
        }))
}

async fn constraint_shapes(
    connection: &mut PgConnection,
    table: &'static str,
) -> Result<Vec<ConstraintShape>, CredentialStoreStartupError> {
    sqlx::query_as(
        "SELECT schema_constraint.conname AS name,
                schema_constraint.contype::text AS kind,
                pg_get_constraintdef(schema_constraint.oid, true) AS definition
         FROM pg_constraint AS schema_constraint
         JOIN pg_class AS relation ON relation.oid = schema_constraint.conrelid
         JOIN pg_namespace AS namespace ON namespace.oid = relation.relnamespace
         WHERE namespace.nspname = current_schema()
           AND relation.relname = $1
         ORDER BY schema_constraint.conname",
    )
    .bind(table)
    .fetch_all(connection)
    .await
    .map_err(|_| CredentialStoreStartupError::Unavailable)
}

fn constraints_match(actual: &[ConstraintShape], expected: &[(&str, &str, &str)]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(actual, (name, kind, definition))| {
                actual.name == *name && actual.kind == *kind && actual.definition == *definition
            })
}

async fn validate_credentials_relation(
    connection: &mut PgConnection,
    latest: i64,
) -> Result<(), CredentialStoreStartupError> {
    let expected = if latest >= 40 {
        &CURRENT_COLUMNS[..]
    } else if latest >= 39 {
        &LIFECYCLE_COLUMNS[..]
    } else {
        &LEGACY_COLUMNS[..]
    };
    if !columns_match(connection, "credentials", expected).await? {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    }

    let constraints = constraint_shapes(connection, "credentials").await?;
    let constraint_contract: &[(&str, &str, &str)] = if latest >= 40 {
        &[
            (
                "credentials_live_name_projection",
                "c",
                "CHECK (record_state = 'tombstoned'::text OR record_state = 'live'::text AND (name IS NULL AND ((metadata::jsonb #> '{display,display_name}'::text[]) IS NULL OR jsonb_typeof(metadata::jsonb #> '{display,display_name}'::text[]) = 'null'::text) OR name IS NOT NULL AND jsonb_typeof(metadata::jsonb #> '{display,display_name}'::text[]) = 'string'::text AND name = (metadata::jsonb #>> '{display,display_name}'::text[])))",
            ),
            (
                "credentials_material_epoch_range",
                "c",
                "CHECK (material_epoch >= 1 AND material_epoch <= '9223372036854775807'::bigint)",
            ),
            (
                "credentials_metadata_object",
                "c",
                "CHECK (metadata IS JSON OBJECT WITH UNIQUE KEYS)",
            ),
            ("credentials_pkey", "p", "PRIMARY KEY (id)"),
            (
                "credentials_record_shape",
                "c",
                "CHECK (record_state = 'live'::text AND tombstoned_at IS NULL AND version <= '9223372036854775806'::bigint OR record_state = 'tombstoned'::text AND tombstoned_at IS NOT NULL AND octet_length(data) = 0 AND name IS NULL AND expires_at IS NULL AND reauth_required = false AND metadata = '{}'::text AND refresh_retry_mode IS NULL AND refresh_retry_not_before IS NULL AND refresh_retry_phase IS NULL AND refresh_retry_kind IS NULL AND refresh_retry_diagnostic_code IS NULL)",
            ),
            (
                "credentials_refresh_retry_gate_shape",
                "c",
                "CHECK (refresh_retry_mode IS NULL AND refresh_retry_not_before IS NULL AND refresh_retry_phase IS NULL AND refresh_retry_kind IS NULL AND refresh_retry_diagnostic_code IS NULL OR record_state = 'live'::text AND refresh_retry_mode IS NOT NULL AND refresh_retry_phase IS NOT NULL AND (refresh_retry_phase = ANY (ARRAY['before_dispatch'::text, 'provider_confirmed_not_applied'::text])) AND refresh_retry_kind IS NOT NULL AND (refresh_retry_kind = ANY (ARRAY['transient_network'::text, 'provider_unavailable'::text, 'protocol_error'::text])) AND (refresh_retry_diagnostic_code IS NULL OR (refresh_retry_diagnostic_code COLLATE \"C\") ~ '^[A-Za-z0-9_.:-]{1,64}$'::text) AND (refresh_retry_mode = 'never'::text AND refresh_retry_not_before IS NULL OR refresh_retry_mode = 'not_before'::text AND refresh_retry_not_before IS NOT NULL))",
            ),
            (
                "credentials_state_version_range",
                "c",
                "CHECK (state_version >= 0 AND state_version <= '4294967295'::bigint)",
            ),
            (
                "credentials_version_range",
                "c",
                "CHECK (version >= 1 AND version <= '9223372036854775807'::bigint)",
            ),
        ]
    } else if latest >= 39 {
        &[
            (
                "credentials_live_name_projection",
                "c",
                "CHECK (record_state = 'tombstoned'::text OR record_state = 'live'::text AND (name IS NULL AND ((metadata::jsonb #> '{display,display_name}'::text[]) IS NULL OR jsonb_typeof(metadata::jsonb #> '{display,display_name}'::text[]) = 'null'::text) OR name IS NOT NULL AND jsonb_typeof(metadata::jsonb #> '{display,display_name}'::text[]) = 'string'::text AND name = (metadata::jsonb #>> '{display,display_name}'::text[])))",
            ),
            (
                "credentials_metadata_object",
                "c",
                "CHECK (metadata IS JSON OBJECT WITH UNIQUE KEYS)",
            ),
            ("credentials_pkey", "p", "PRIMARY KEY (id)"),
            (
                "credentials_record_shape",
                "c",
                "CHECK (record_state = 'live'::text AND tombstoned_at IS NULL AND version <= '9223372036854775806'::bigint OR record_state = 'tombstoned'::text AND tombstoned_at IS NOT NULL AND octet_length(data) = 0 AND name IS NULL AND expires_at IS NULL AND reauth_required = false AND metadata = '{}'::text)",
            ),
            (
                "credentials_state_version_range",
                "c",
                "CHECK (state_version >= 0 AND state_version <= '4294967295'::bigint)",
            ),
            (
                "credentials_version_range",
                "c",
                "CHECK (version >= 1 AND version <= '9223372036854775807'::bigint)",
            ),
        ]
    } else {
        &[("credentials_pkey", "p", "PRIMARY KEY (id)")]
    };
    if !constraints_match(&constraints, constraint_contract) {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    }

    validate_indexes(connection, latest >= 39).await
}

async fn validate_sentinel_events_relation(
    connection: &mut PgConnection,
    current: bool,
) -> Result<(), CredentialStoreStartupError> {
    let expected = if current {
        &CURRENT_SENTINEL_EVENT_COLUMNS[..]
    } else {
        &LEGACY_SENTINEL_EVENT_COLUMNS[..]
    };
    if !columns_match(connection, "credential_sentinel_events", expected).await? {
        return unsupported(AdmissionReason::InvalidSentinelEventsRelation);
    }

    let constraints = constraint_shapes(connection, "credential_sentinel_events").await?;
    if !constraints_match(
        &constraints,
        &[("credential_sentinel_events_pkey", "p", "PRIMARY KEY (id)")],
    ) {
        return unsupported(AdmissionReason::InvalidSentinelEventsRelation);
    }

    let indexes = index_shapes(connection, "credential_sentinel_events").await?;
    let expected = [
        index(
            "credential_sentinel_events_pkey",
            true,
            None,
            &["id"],
            &[""],
            &["int8_ops"],
            "0",
        ),
        index(
            "idx_sentinel_events_cred_time",
            false,
            None,
            &["credential_id", "detected_at"],
            &["default", ""],
            &["text_ops", "timestamptz_ops"],
            "0 0",
        ),
    ];
    let current_identity = index(
        "idx_credential_sentinel_events_claim_id",
        true,
        Some("claim_id IS NOT NULL"),
        &["claim_id"],
        &[""],
        &["uuid_ops"],
        "0",
    );
    let expected = if current {
        vec![expected[0], current_identity, expected[1]]
    } else {
        expected.to_vec()
    };
    if !indexes_match(&indexes, &expected) {
        return unsupported(AdmissionReason::InvalidSentinelEventsRelation);
    }
    Ok(())
}

async fn validate_indexes(
    connection: &mut PgConnection,
    current: bool,
) -> Result<(), CredentialStoreStartupError> {
    let indexes = index_shapes(connection, "credentials").await?;
    let owner_name_predicate = if current {
        None
    } else {
        Some("name IS NOT NULL")
    };
    let expected = [
        index(
            "credentials_pkey",
            true,
            None,
            &["id"],
            &["default"],
            &["text_ops"],
            "0",
        ),
        index(
            "idx_credentials_expiring",
            false,
            Some("expires_at IS NOT NULL"),
            &["expires_at"],
            &[""],
            &["timestamptz_ops"],
            "0",
        ),
        index(
            "idx_credentials_owner_name",
            true,
            owner_name_predicate,
            &["owner_id", "name"],
            &["default", "default"],
            &["text_ops", "text_ops"],
            "0 0",
        ),
        index(
            "idx_credentials_state_kind",
            false,
            None,
            &["state_kind"],
            &["default"],
            &["text_ops"],
            "0",
        ),
    ];
    if !indexes_match(&indexes, &expected) {
        return unsupported(AdmissionReason::InvalidCredentialsRelation);
    }
    Ok(())
}

async fn index_shapes(
    connection: &mut PgConnection,
    table: &'static str,
) -> Result<Vec<IndexShape>, CredentialStoreStartupError> {
    sqlx::query_as::<_, IndexShape>(
        "SELECT
             index_relation.relname AS name,
             schema_index.indisunique AS unique_index,
             schema_index.indisvalid AS valid,
             schema_index.indnkeyatts AS key_count,
             schema_index.indexprs IS NOT NULL AS has_expressions,
             access_method.amname AS access_method,
             pg_get_expr(schema_index.indpred, schema_index.indrelid, true) AS predicate,
             ARRAY(
                 SELECT attribute.attname::text
                 FROM unnest(schema_index.indkey) WITH ORDINALITY AS key(attnum, ordinal)
                 JOIN pg_attribute AS attribute
                   ON attribute.attrelid = schema_index.indrelid
                  AND attribute.attnum = key.attnum
                 WHERE key.attnum > 0
                 ORDER BY key.ordinal
             ) AS columns,
             ARRAY(
                 SELECT COALESCE(schema_collation.collname::text, '')
                 FROM unnest(schema_index.indcollation)
                      WITH ORDINALITY AS item(collation_oid, ordinal)
                 LEFT JOIN pg_collation AS schema_collation
                   ON schema_collation.oid = item.collation_oid
                 ORDER BY item.ordinal
             ) AS collations,
             ARRAY(
                 SELECT operator_class.opcname::text
                 FROM unnest(schema_index.indclass)
                      WITH ORDINALITY AS item(operator_class_oid, ordinal)
                 JOIN pg_opclass AS operator_class
                   ON operator_class.oid = item.operator_class_oid
                 ORDER BY item.ordinal
             ) AS operator_classes,
             schema_index.indoption::text AS index_options
         FROM pg_index AS schema_index
         JOIN pg_class AS table_relation ON table_relation.oid = schema_index.indrelid
         JOIN pg_namespace AS namespace ON namespace.oid = table_relation.relnamespace
         JOIN pg_class AS index_relation ON index_relation.oid = schema_index.indexrelid
         JOIN pg_am AS access_method ON access_method.oid = index_relation.relam
         WHERE namespace.nspname = current_schema()
           AND table_relation.relname = $1
         ORDER BY index_relation.relname",
    )
    .bind(table)
    .fetch_all(connection)
    .await
    .map_err(|_| CredentialStoreStartupError::Unavailable)
}

fn indexes_match(indexes: &[IndexShape], expected: &[ExpectedIndex]) -> bool {
    if indexes.len() != expected.len() {
        return false;
    }
    for (index, expected) in indexes.iter().zip(expected) {
        let matches = index.name == expected.name
            && index.unique_index == expected.unique_index
            && index.valid
            && !index.has_expressions
            && usize::try_from(index.key_count).ok() == Some(expected.columns.len())
            && index.access_method == "btree"
            && index.predicate.as_deref() == expected.predicate
            && index
                .columns
                .iter()
                .map(String::as_str)
                .eq(expected.columns.iter().copied())
            && index
                .collations
                .iter()
                .map(String::as_str)
                .eq(expected.collations.iter().copied())
            && index
                .operator_classes
                .iter()
                .map(String::as_str)
                .eq(expected.operator_classes.iter().copied())
            && index.index_options == expected.options;
        if !matches {
            return false;
        }
    }
    true
}

async fn credential_rows(
    connection: &mut PgConnection,
    latest: i64,
) -> Result<Vec<LegacyCredentialRecord>, CredentialStoreStartupError> {
    let rows = if latest >= 40 {
        sqlx::query(
            "SELECT id, owner_id, name, state_version, octet_length(data)::bigint AS data_len,
                    version, material_epoch, metadata, record_state,
                    tombstoned_at IS NOT NULL AS tombstoned_at_present,
                    expires_at IS NOT NULL AS expires_at_present,
                    reauth_required, refresh_retry_mode,
                    refresh_retry_not_before IS NOT NULL
                        AS refresh_retry_not_before_present,
                    refresh_retry_phase, refresh_retry_kind,
                    refresh_retry_diagnostic_code
             FROM credentials",
        )
        .fetch_all(connection)
        .await
    } else if latest >= 39 {
        sqlx::query(
            "SELECT id, owner_id, name, state_version, octet_length(data)::bigint AS data_len,
                    version, NULL::bigint AS material_epoch, metadata, record_state,
                    tombstoned_at IS NOT NULL AS tombstoned_at_present,
                    expires_at IS NOT NULL AS expires_at_present,
                    reauth_required, NULL::text AS refresh_retry_mode,
                    FALSE AS refresh_retry_not_before_present,
                    NULL::text AS refresh_retry_phase,
                    NULL::text AS refresh_retry_kind,
                    NULL::text AS refresh_retry_diagnostic_code
             FROM credentials",
        )
        .fetch_all(connection)
        .await
    } else {
        sqlx::query(
            "SELECT id, owner_id, name, state_version, octet_length(data)::bigint AS data_len,
                    version, NULL::bigint AS material_epoch, metadata, NULL::text AS record_state,
                    FALSE AS tombstoned_at_present,
                    expires_at IS NOT NULL AS expires_at_present,
                    reauth_required, NULL::text AS refresh_retry_mode,
                    FALSE AS refresh_retry_not_before_present,
                    NULL::text AS refresh_retry_phase,
                    NULL::text AS refresh_retry_kind,
                    NULL::text AS refresh_retry_diagnostic_code
             FROM credentials",
        )
        .fetch_all(connection)
        .await
    }
    .map_err(|error| observation_fetch_error(error, AdmissionReason::InvalidCredentialsRelation))?;

    rows.into_iter()
        .map(|row| {
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
                reauth_required: row
                    .try_get("reauth_required")
                    .map_err(|_| unsupported_error(AdmissionReason::InvalidCredentialsRelation))?,
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
