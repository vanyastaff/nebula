//! Backend-neutral migration-catalog admission and physical ledger probes.

use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackendKind {
    #[cfg(any(test, feature = "sqlite"))]
    Sqlite,
    #[cfg(any(test, feature = "postgres"))]
    Postgres,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MigrationSpec {
    pub(crate) version: i64,
    pub(crate) description: String,
    pub(crate) checksum: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct CatalogPolicy {
    pub(crate) current_version: i64,
    pub(crate) canonical: Vec<MigrationSpec>,
    pub(crate) reserved_other_backend_versions: BTreeSet<i64>,
}

#[derive(Clone)]
pub(crate) struct MigrationLedgerRow {
    pub(crate) version: i64,
    pub(crate) description: String,
    pub(crate) checksum: Vec<u8>,
    pub(crate) success: bool,
}

#[derive(Clone)]
pub(crate) enum MigrationLedger {
    Absent,
    Present(Vec<MigrationLedgerRow>),
}

pub(crate) struct CatalogObservation {
    pub(crate) migration_ledger: MigrationLedger,
    pub(crate) has_user_relations: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CatalogAdmission {
    Fresh,
    CanonicalPrefix { latest: i64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CatalogRejection {
    EmptyLedger,
    UnledgeredDatabase,
    #[cfg_attr(
        not(any(feature = "sqlite", feature = "postgres")),
        expect(
            dead_code,
            reason = "physical ledger probes are compiled only with a database backend"
        )
    )]
    InvalidMigrationLedger,
    BelowSupportedFloor {
        latest: i64,
    },
    FailedMigration {
        migration: i64,
    },
    DuplicateMigration {
        migration: i64,
    },
    NonCanonicalOrder {
        expected: i64,
        actual: i64,
    },
    ReservedForOtherBackend {
        migration: i64,
    },
    UnknownMigration {
        migration: i64,
    },
    DescriptionMismatch {
        migration: i64,
    },
    ChecksumMismatch {
        migration: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) enum CatalogSetupError {
    Rejected(CatalogRejection),
    Unavailable,
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
impl From<CatalogRejection> for CatalogSetupError {
    fn from(rejection: CatalogRejection) -> Self {
        Self::Rejected(rejection)
    }
}

pub(crate) fn classify(
    policy: &CatalogPolicy,
    observation: &CatalogObservation,
    supported_floor: i64,
) -> Result<CatalogAdmission, CatalogRejection> {
    let rows = match &observation.migration_ledger {
        MigrationLedger::Absent => {
            if observation.has_user_relations {
                return rejected(CatalogRejection::UnledgeredDatabase);
            }
            return Ok(CatalogAdmission::Fresh);
        },
        MigrationLedger::Present(rows) if rows.is_empty() => {
            return rejected(CatalogRejection::EmptyLedger);
        },
        MigrationLedger::Present(rows) => rows,
    };

    validate_ledger(policy, rows)?;
    let latest = rows
        .last()
        .map(|row| row.version)
        .ok_or(CatalogRejection::EmptyLedger)?;
    if latest < supported_floor {
        return rejected(CatalogRejection::BelowSupportedFloor { latest });
    }
    Ok(CatalogAdmission::CanonicalPrefix { latest })
}

fn validate_ledger(
    policy: &CatalogPolicy,
    rows: &[MigrationLedgerRow],
) -> Result<(), CatalogRejection> {
    let mut observed_versions = HashSet::with_capacity(rows.len());
    for row in rows {
        if !observed_versions.insert(row.version) {
            return rejected(CatalogRejection::DuplicateMigration {
                migration: row.version,
            });
        }
    }

    for row in rows {
        if !row.success {
            return rejected(CatalogRejection::FailedMigration {
                migration: row.version,
            });
        }
        if policy
            .reserved_other_backend_versions
            .contains(&row.version)
        {
            return rejected(CatalogRejection::ReservedForOtherBackend {
                migration: row.version,
            });
        }
        if row.version > policy.current_version {
            return rejected(CatalogRejection::UnknownMigration {
                migration: row.version,
            });
        }
        let Some(canonical) = policy
            .canonical
            .iter()
            .find(|migration| migration.version == row.version)
        else {
            return rejected(CatalogRejection::UnknownMigration {
                migration: row.version,
            });
        };
        if row.description != canonical.description {
            return rejected(CatalogRejection::DescriptionMismatch {
                migration: row.version,
            });
        }
        if row.checksum != canonical.checksum {
            return rejected(CatalogRejection::ChecksumMismatch {
                migration: row.version,
            });
        }
    }

    for (index, row) in rows.iter().enumerate() {
        let Some(expected) = policy.canonical.get(index) else {
            return rejected(CatalogRejection::UnknownMigration {
                migration: row.version,
            });
        };
        if row.version != expected.version {
            return rejected(CatalogRejection::NonCanonicalOrder {
                expected: expected.version,
                actual: row.version,
            });
        }
    }

    Ok(())
}

fn rejected<T>(rejection: CatalogRejection) -> Result<T, CatalogRejection> {
    Err(rejection)
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn catalog_head(migrator: &sqlx::migrate::Migrator) -> i64 {
    migrator
        .iter()
        .next_back()
        .map_or(0, |migration| migration.version)
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn policy_from_migrator(backend: BackendKind, migrator: &sqlx::migrate::Migrator) -> CatalogPolicy {
    let canonical = migrator
        .iter()
        .map(|migration| MigrationSpec {
            version: migration.version,
            description: migration.description.to_string(),
            checksum: migration.checksum.to_vec(),
        })
        .collect();
    let reserved_other_backend_versions = match backend {
        #[cfg(any(test, feature = "sqlite"))]
        BackendKind::Sqlite => BTreeSet::from([29, 36, 37, 38]),
        #[cfg(any(test, feature = "postgres"))]
        BackendKind::Postgres => BTreeSet::new(),
    };

    CatalogPolicy {
        current_version: catalog_head(migrator),
        canonical,
        reserved_other_backend_versions,
    }
}

#[cfg(feature = "sqlite")]
pub(crate) fn sqlite_policy() -> CatalogPolicy {
    policy_from_migrator(BackendKind::Sqlite, &super::SQLITE_MIGRATOR)
}

#[cfg(feature = "postgres")]
pub(crate) fn postgres_policy() -> CatalogPolicy {
    policy_from_migrator(BackendKind::Postgres, &super::POSTGRES_MIGRATOR)
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn observation_fetch_error(error: sqlx::Error, rejection: CatalogRejection) -> CatalogSetupError {
    match error {
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnIndexOutOfBounds { .. }
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::RowNotFound
        | sqlx::Error::TypeNotFound { .. } => CatalogSetupError::Rejected(rejection),
        _ => CatalogSetupError::Unavailable,
    }
}

#[cfg(feature = "sqlite")]
mod sqlite {
    use sqlx::{Row as _, SqliteConnection};

    use super::{
        CatalogAdmission, CatalogObservation, CatalogRejection, CatalogSetupError, MigrationLedger,
        MigrationLedgerRow, classify, sqlite_policy,
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

    const LEDGER_SHAPE: [ExpectedColumnShape; 6] = [
        ExpectedColumnShape {
            name: "version",
            declared_type: "BIGINT",
            not_null: false,
            default: None,
            primary_key_position: 1,
        },
        ExpectedColumnShape {
            name: "description",
            declared_type: "TEXT",
            not_null: true,
            default: None,
            primary_key_position: 0,
        },
        ExpectedColumnShape {
            name: "installed_on",
            declared_type: "TIMESTAMP",
            not_null: true,
            default: Some("CURRENT_TIMESTAMP"),
            primary_key_position: 0,
        },
        ExpectedColumnShape {
            name: "success",
            declared_type: "BOOLEAN",
            not_null: true,
            default: None,
            primary_key_position: 0,
        },
        ExpectedColumnShape {
            name: "checksum",
            declared_type: "BLOB",
            not_null: true,
            default: None,
            primary_key_position: 0,
        },
        ExpectedColumnShape {
            name: "execution_time",
            declared_type: "BIGINT",
            not_null: true,
            default: None,
            primary_key_position: 0,
        },
    ];

    pub(crate) async fn admit(
        connection: &mut SqliteConnection,
        supported_floor: i64,
    ) -> Result<CatalogAdmission, CatalogSetupError> {
        let policy = sqlite_policy();
        let observation = observe(connection).await?;
        classify(&policy, &observation, supported_floor).map_err(Into::into)
    }

    pub(crate) async fn observe(
        connection: &mut SqliteConnection,
    ) -> Result<CatalogObservation, CatalogSetupError> {
        let ledger_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM sqlite_schema
                 WHERE type = 'table' AND name = '_sqlx_migrations'
             )",
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| CatalogSetupError::Unavailable)?;
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
        .map_err(|_| CatalogSetupError::Unavailable)?;

        if !ledger_exists {
            return Ok(CatalogObservation {
                migration_ledger: MigrationLedger::Absent,
                has_user_relations,
            });
        }

        let rows = sqlx::query("PRAGMA table_info('_sqlx_migrations')")
            .fetch_all(&mut *connection)
            .await
            .map_err(|_| CatalogSetupError::Unavailable)?;
        let columns = rows
            .into_iter()
            .map(|row| {
                Ok(ColumnShape {
                    name: row.try_get("name").map_err(|error| {
                        super::observation_fetch_error(
                            error,
                            CatalogRejection::InvalidMigrationLedger,
                        )
                    })?,
                    declared_type: row.try_get("type").map_err(|error| {
                        super::observation_fetch_error(
                            error,
                            CatalogRejection::InvalidMigrationLedger,
                        )
                    })?,
                    not_null: row.try_get::<i64, _>("notnull").map_err(|error| {
                        super::observation_fetch_error(
                            error,
                            CatalogRejection::InvalidMigrationLedger,
                        )
                    })? == 1,
                    default: row.try_get("dflt_value").map_err(|error| {
                        super::observation_fetch_error(
                            error,
                            CatalogRejection::InvalidMigrationLedger,
                        )
                    })?,
                    primary_key_position: row.try_get("pk").map_err(|error| {
                        super::observation_fetch_error(
                            error,
                            CatalogRejection::InvalidMigrationLedger,
                        )
                    })?,
                })
            })
            .collect::<Result<Vec<_>, CatalogSetupError>>()?;
        if columns.len() != LEDGER_SHAPE.len()
            || !columns.iter().zip(&LEDGER_SHAPE).all(|(actual, expected)| {
                actual.name == expected.name
                    && actual.declared_type == expected.declared_type
                    && actual.not_null == expected.not_null
                    && actual.default.as_deref() == expected.default
                    && actual.primary_key_position == expected.primary_key_position
            })
        {
            return Err(CatalogSetupError::Rejected(
                CatalogRejection::InvalidMigrationLedger,
            ));
        }

        let ledger_rows = sqlx::query_as::<_, (i64, String, bool, Vec<u8>)>(
            "SELECT version, description, success, checksum
             FROM _sqlx_migrations
             ORDER BY version",
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| {
            super::observation_fetch_error(error, CatalogRejection::InvalidMigrationLedger)
        })?
        .into_iter()
        .map(
            |(version, description, success, checksum)| MigrationLedgerRow {
                version,
                description,
                checksum,
                success,
            },
        )
        .collect();

        Ok(CatalogObservation {
            migration_ledger: MigrationLedger::Present(ledger_rows),
            has_user_relations,
        })
    }
}

#[cfg(feature = "sqlite")]
pub(crate) use sqlite::{admit as admit_sqlite, observe as observe_sqlite};

#[cfg(feature = "postgres")]
mod postgres {
    use sqlx::PgConnection;

    use super::{
        CatalogAdmission, CatalogObservation, CatalogRejection, CatalogSetupError, MigrationLedger,
        MigrationLedgerRow, classify, postgres_policy,
    };

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

    const LEDGER_COLUMNS: [(&str, &str, bool, Option<&str>); 6] = [
        ("version", "int8", false, None),
        ("description", "text", false, None),
        ("installed_on", "timestamptz", false, Some("now()")),
        ("success", "bool", false, None),
        ("checksum", "bytea", false, None),
        ("execution_time", "int8", false, None),
    ];

    pub(crate) async fn admit(
        connection: &mut PgConnection,
        supported_floor: i64,
    ) -> Result<CatalogAdmission, CatalogSetupError> {
        let policy = postgres_policy();
        let observation = observe(connection).await?;
        classify(&policy, &observation, supported_floor).map_err(Into::into)
    }

    pub(crate) async fn observe(
        connection: &mut PgConnection,
    ) -> Result<CatalogObservation, CatalogSetupError> {
        let ledger_exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind("_sqlx_migrations")
            .fetch_one(&mut *connection)
            .await
            .map_err(|_| CatalogSetupError::Unavailable)?;
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
        .map_err(|_| CatalogSetupError::Unavailable)?;

        if !ledger_exists {
            return Ok(CatalogObservation {
                migration_ledger: MigrationLedger::Absent,
                has_user_relations,
            });
        }

        let columns: Vec<ColumnShape> = sqlx::query_as(
            "SELECT column_name AS name,
                    udt_name AS data_type,
                    is_nullable = 'YES' AS nullable,
                    column_default AS default_value
             FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name = '_sqlx_migrations'
             ORDER BY ordinal_position",
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| {
            super::observation_fetch_error(error, CatalogRejection::InvalidMigrationLedger)
        })?;
        if columns.len() != LEDGER_COLUMNS.len()
            || !columns.iter().zip(&LEDGER_COLUMNS).all(
                |(actual, (name, data_type, nullable, default_value))| {
                    actual.name == *name
                        && actual.data_type == *data_type
                        && actual.nullable == *nullable
                        && actual.default_value.as_deref() == *default_value
                },
            )
        {
            return Err(CatalogSetupError::Rejected(
                CatalogRejection::InvalidMigrationLedger,
            ));
        }

        let constraints: Vec<ConstraintShape> = sqlx::query_as(
            "SELECT schema_constraint.conname AS name,
                    schema_constraint.contype::text AS kind,
                    pg_get_constraintdef(schema_constraint.oid, true) AS definition
             FROM pg_constraint AS schema_constraint
             JOIN pg_class AS relation ON relation.oid = schema_constraint.conrelid
             JOIN pg_namespace AS namespace ON namespace.oid = relation.relnamespace
             WHERE namespace.nspname = current_schema()
               AND relation.relname = '_sqlx_migrations'
             ORDER BY schema_constraint.conname",
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| {
            super::observation_fetch_error(error, CatalogRejection::InvalidMigrationLedger)
        })?;
        let constraint_matches = matches!(
            constraints.as_slice(),
            [ConstraintShape { name, kind, definition }]
                if name == "_sqlx_migrations_pkey"
                    && kind == "p"
                    && definition == "PRIMARY KEY (version)"
        );
        if !constraint_matches {
            return Err(CatalogSetupError::Rejected(
                CatalogRejection::InvalidMigrationLedger,
            ));
        }

        let ledger_rows = sqlx::query_as::<_, (i64, String, bool, Vec<u8>)>(
            "SELECT version, description, success, checksum
             FROM _sqlx_migrations
             ORDER BY version",
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| {
            super::observation_fetch_error(error, CatalogRejection::InvalidMigrationLedger)
        })?
        .into_iter()
        .map(
            |(version, description, success, checksum)| MigrationLedgerRow {
                version,
                description,
                checksum,
                success,
            },
        )
        .collect();

        Ok(CatalogObservation {
            migration_ledger: MigrationLedger::Present(ledger_rows),
            has_user_relations,
        })
    }
}

#[cfg(feature = "postgres")]
pub(crate) use postgres::{admit as admit_postgres, observe as observe_postgres};
