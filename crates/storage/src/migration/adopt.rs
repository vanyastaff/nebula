//! Adoption of databases provisioned before the ordered migration ledger.
//!
//! Ordered migrations are authoritative: setup admits a database only when its
//! `_sqlx_migrations` ledger is a canonical prefix. Databases created by the
//! previous idempotent `init_schema` carry the `port_*` schema with no ledger
//! at all, so they classify as `UnledgeredDatabase` and their processes refuse
//! to start. Recreating them is not an option for a deployment holding data.
//!
//! Adoption is the documented way out, and it is deliberately an explicit
//! operator action rather than something startup performs on its own: stamping
//! a ledger asserts "the schema this database already has is what migrations
//! `1..=through_version` produce", and only an operator can know that. Startup
//! keeps failing closed.

use sqlx::migrate::{Migrate, Migrator};

use super::catalog::{CatalogObservation, MigrationLedger};

/// What adoption did to a database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerAdoptionOutcome {
    /// A ledger was created and stamped through this version.
    Adopted {
        /// Highest migration version recorded as already applied.
        through_version: i64,
    },
    /// A ledger already existed; nothing was changed.
    AlreadyLedgered,
    /// The database has no user relations, so ordinary setup already covers it
    /// and there is nothing to adopt.
    FreshDatabase,
}

/// Why adoption refused to stamp a ledger.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LedgerAdoptionError {
    /// The database could not be inspected or written.
    #[error("database is unavailable for adoption")]
    Unavailable,
    /// The requested baseline names no migration in the canonical catalog.
    #[error("requested baseline {requested} is not a canonical migration version")]
    UnknownBaseline {
        /// The version the operator asked to stamp through.
        requested: i64,
    },
    /// Stamping produced a ledger that setup would still reject, so the work
    /// was rolled back and the database left exactly as it was.
    #[error("stamped ledger would still be rejected by schema setup")]
    RejectedAfterStamp,
}

/// Whether this database needs stamping, decided before any write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AdoptionPlan {
    /// Stamp migrations up to and including this version.
    Stamp { through_version: i64 },
    /// Nothing to do; report this outcome unchanged.
    Skip(LedgerAdoptionOutcome),
}

/// Decide what adoption should do, from an observation alone.
///
/// Pure so the refusal rules are testable without a database.
pub(crate) fn plan_adoption(
    migrator: &Migrator,
    observation: &CatalogObservation,
    through_version: i64,
) -> Result<AdoptionPlan, LedgerAdoptionError> {
    if !migrator
        .iter()
        .any(|migration| migration.version == through_version)
    {
        return Err(LedgerAdoptionError::UnknownBaseline {
            requested: through_version,
        });
    }
    if matches!(observation.migration_ledger, MigrationLedger::Present(_)) {
        return Ok(AdoptionPlan::Skip(LedgerAdoptionOutcome::AlreadyLedgered));
    }
    if !observation.has_user_relations {
        return Ok(AdoptionPlan::Skip(LedgerAdoptionOutcome::FreshDatabase));
    }
    Ok(AdoptionPlan::Stamp { through_version })
}

/// Record migrations up to `through_version` as already applied.
///
/// Uses sqlx's own `skip`, so each stamped row carries the checksum a real run
/// would have written and ledger validation cannot drift from hand-rolled DDL.
/// The caller is responsible for running this inside a transaction and for
/// verifying the result before committing.
pub(crate) async fn stamp_ledger<Connection>(
    connection: &mut Connection,
    migrator: &Migrator,
    through_version: i64,
) -> Result<(), LedgerAdoptionError>
where
    Connection: Migrate + Send,
{
    connection
        .ensure_migrations_table(&migrator.table_name)
        .await
        .map_err(|_| LedgerAdoptionError::Unavailable)?;

    let applied = connection
        .list_applied_migrations(&migrator.table_name)
        .await
        .map_err(|_| LedgerAdoptionError::Unavailable)?;
    if !applied.is_empty() {
        // Another adopter won between observation and this write.
        return Err(LedgerAdoptionError::RejectedAfterStamp);
    }

    for migration in migrator
        .iter()
        .filter(|migration| migration.version <= through_version)
    {
        connection
            .skip(&migrator.table_name, migration)
            .await
            .map_err(|_| LedgerAdoptionError::Unavailable)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::catalog::{CatalogObservation, MigrationLedger};
    use super::{AdoptionPlan, LedgerAdoptionError, LedgerAdoptionOutcome, plan_adoption};

    fn observation(ledger: MigrationLedger, has_user_relations: bool) -> CatalogObservation {
        CatalogObservation {
            migration_ledger: ledger,
            has_user_relations,
        }
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn unledgered_database_with_relations_is_stamped() {
        let migrator = &super::super::SQLITE_MIGRATOR;
        let plan = plan_adoption(
            migrator,
            &observation(MigrationLedger::Absent, true),
            super::super::GENERAL_CATALOG_SUPPORTED_FLOOR,
        );
        assert_eq!(
            plan,
            Ok(AdoptionPlan::Stamp {
                through_version: super::super::GENERAL_CATALOG_SUPPORTED_FLOOR
            }),
            "a database with the old schema and no ledger is exactly what adoption is for"
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn empty_database_is_left_to_ordinary_setup() {
        let migrator = &super::super::SQLITE_MIGRATOR;
        assert_eq!(
            plan_adoption(
                migrator,
                &observation(MigrationLedger::Absent, false),
                super::super::GENERAL_CATALOG_SUPPORTED_FLOOR
            ),
            Ok(AdoptionPlan::Skip(LedgerAdoptionOutcome::FreshDatabase)),
            "stamping an empty database would claim a schema it does not have"
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn already_ledgered_database_is_never_restamped() {
        let migrator = &super::super::SQLITE_MIGRATOR;
        assert_eq!(
            plan_adoption(
                migrator,
                &observation(MigrationLedger::Present(Vec::new()), true),
                super::super::GENERAL_CATALOG_SUPPORTED_FLOOR
            ),
            Ok(AdoptionPlan::Skip(LedgerAdoptionOutcome::AlreadyLedgered)),
            "an existing ledger is authoritative and must not be overwritten"
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn baseline_outside_the_canonical_catalog_is_refused() {
        let migrator = &super::super::SQLITE_MIGRATOR;
        assert_eq!(
            plan_adoption(migrator, &observation(MigrationLedger::Absent, true), 9_999),
            Err(LedgerAdoptionError::UnknownBaseline { requested: 9_999 }),
            "a baseline must name a real migration or the ledger would be a fiction"
        );
    }
}
