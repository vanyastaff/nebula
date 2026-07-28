//! Admitted PostgreSQL migration operator.

#![forbid(unsafe_code)]
#![expect(
    clippy::print_stderr,
    reason = "binary edge: bounded startup diagnostics must reach stderr without Debug rendering"
)]

use nebula_storage::{
    LedgerAdoptionError, LedgerAdoptionOutcome,
    credential::{CredentialStoreStartupError, PgCredentialPersistence},
    postgres,
};
use nebula_storage_port::StorageError;
use secrecy::{ExposeSecret as _, SecretString};
use sqlx::postgres::PgPoolOptions;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationRoute {
    Complete,
    AggregateOwnerAdmission,
}

#[derive(Debug, Error, PartialEq, Eq)]
enum MigrationOperatorError {
    #[error("DATABASE_URL is not configured")]
    MissingDatabaseUrl,
    #[error("database is unavailable")]
    DatabaseUnavailable,
    #[error("database schema setup failed")]
    GeneralAdmissionFailed,
    #[error("aggregate-owned schema admission failed: {0}")]
    AggregateAdmissionFailed(#[source] CredentialStoreStartupError),
    #[error("usage: nebula-db-migrate [migrate | adopt <through-version>]")]
    UnknownCommand,
    #[error("adopt baseline `{argument}` is not a migration version")]
    InvalidBaseline { argument: String },
    #[error("ledger adoption failed: {0}")]
    AdoptionFailed(#[source] LedgerAdoptionError),
}

fn migration_route(
    general_admission: Result<(), StorageError>,
) -> Result<MigrationRoute, MigrationOperatorError> {
    match general_admission {
        Ok(()) => Ok(MigrationRoute::Complete),
        Err(StorageError::Configuration(_)) => Ok(MigrationRoute::AggregateOwnerAdmission),
        Err(_) => Err(MigrationOperatorError::GeneralAdmissionFailed),
    }
}

/// What the operator asked this invocation to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    /// Apply pending migrations through catalog and aggregate admission.
    Migrate,
    /// Stamp a ledger onto a database provisioned before ordered migrations.
    Adopt { through_version: i64 },
}

fn parse_command(arguments: &[String]) -> Result<Command, MigrationOperatorError> {
    match arguments {
        [] => Ok(Command::Migrate),
        [verb] if verb == "migrate" => Ok(Command::Migrate),
        [verb, version] if verb == "adopt" => version
            .parse()
            .map(|through_version| Command::Adopt { through_version })
            .map_err(|_| MigrationOperatorError::InvalidBaseline {
                argument: version.clone(),
            }),
        _ => Err(MigrationOperatorError::UnknownCommand),
    }
}

async fn adopt(pool: &sqlx::PgPool, through_version: i64) -> Result<(), MigrationOperatorError> {
    match postgres::adopt_ledger(pool, through_version).await {
        Ok(LedgerAdoptionOutcome::Adopted { through_version }) => {
            eprintln!("adopted: ledger stamped through migration {through_version}");
            Ok(())
        },
        Ok(LedgerAdoptionOutcome::AlreadyLedgered) => {
            eprintln!("no change: database already has a migration ledger");
            Ok(())
        },
        Ok(LedgerAdoptionOutcome::FreshDatabase) => {
            eprintln!("no change: database is empty, run `migrate` instead");
            Ok(())
        },
        Err(error) => Err(MigrationOperatorError::AdoptionFailed(error)),
    }
}

async fn run() -> Result<(), MigrationOperatorError> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let command = parse_command(&arguments)?;

    let database_url = std::env::var("DATABASE_URL")
        .map(SecretString::from)
        .map_err(|_| MigrationOperatorError::MissingDatabaseUrl)?;
    let pool = PgPoolOptions::new()
        .connect(database_url.expose_secret())
        .await
        .map_err(|_| MigrationOperatorError::DatabaseUnavailable)?;

    if let Command::Adopt { through_version } = command {
        let result = adopt(&pool, through_version).await;
        pool.close().await;
        return result;
    }

    match migration_route(postgres::init_schema(&pool).await)? {
        MigrationRoute::Complete => {
            pool.close().await;
        },
        MigrationRoute::AggregateOwnerAdmission => {
            pool.close().await;
            let ready_store = PgCredentialPersistence::connect(database_url.expose_secret())
                .await
                .map_err(MigrationOperatorError::AggregateAdmissionFailed)?;
            drop(ready_store);
        },
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error}");

        let mut source = std::error::Error::source(&error);
        while let Some(source_error) = source {
            eprintln!("  caused by: {source_error}");
            source = std::error::Error::source(source_error);
        }

        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, MigrationOperatorError, MigrationRoute, migration_route, parse_command};
    use nebula_storage::credential::CredentialStoreStartupError;
    use nebula_storage_port::StorageError;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn no_arguments_still_means_migrate() {
        assert_eq!(parse_command(&arguments(&[])), Ok(Command::Migrate));
        assert_eq!(
            parse_command(&arguments(&["migrate"])),
            Ok(Command::Migrate)
        );
    }

    #[test]
    fn adopt_requires_an_explicit_numeric_baseline() {
        assert_eq!(
            parse_command(&arguments(&["adopt", "40"])),
            Ok(Command::Adopt {
                through_version: 40
            }),
            "the operator states which migration level the live schema is at"
        );
        assert_eq!(
            parse_command(&arguments(&["adopt", "head"])),
            Err(MigrationOperatorError::InvalidBaseline {
                argument: "head".to_owned()
            }),
            "a non-numeric baseline must be refused rather than guessed"
        );
        assert_eq!(
            parse_command(&arguments(&["adopt"])),
            Err(MigrationOperatorError::UnknownCommand),
            "adoption must never default to a baseline the operator did not state"
        );
    }

    #[test]
    fn unknown_verbs_are_refused() {
        assert_eq!(
            parse_command(&arguments(&["revert"])),
            Err(MigrationOperatorError::UnknownCommand)
        );
    }

    #[test]
    fn successful_general_admission_completes_without_fallback() {
        assert_eq!(migration_route(Ok(())), Ok(MigrationRoute::Complete));
    }

    #[test]
    fn only_configuration_rejection_routes_to_aggregate_owner() {
        assert_eq!(
            migration_route(Err(StorageError::Configuration(
                "private schema detail".to_owned()
            ))),
            Ok(MigrationRoute::AggregateOwnerAdmission)
        );
    }

    #[test]
    fn operational_and_unknown_general_failures_never_fallback() {
        for error in [
            StorageError::Connection("private driver detail".to_owned()),
            StorageError::Internal("private invariant detail".to_owned()),
        ] {
            assert_eq!(
                migration_route(Err(error)),
                Err(MigrationOperatorError::GeneralAdmissionFailed)
            );
        }
    }

    #[test]
    fn operator_errors_are_closed_and_secret_free() {
        for error in [
            MigrationOperatorError::MissingDatabaseUrl,
            MigrationOperatorError::DatabaseUnavailable,
            MigrationOperatorError::GeneralAdmissionFailed,
            MigrationOperatorError::AggregateAdmissionFailed(
                CredentialStoreStartupError::Unavailable,
            ),
        ] {
            let display = error.to_string();
            let debug = format!("{error:?}");
            assert!(!display.contains("private"));
            assert!(!debug.contains("private"));
        }
    }

    #[test]
    fn aggregate_admission_error_preserves_the_closed_typed_source() {
        let error = MigrationOperatorError::AggregateAdmissionFailed(
            CredentialStoreStartupError::Unavailable,
        );
        let source = std::error::Error::source(&error)
            .expect("aggregate admission failure must preserve its safe source");

        assert_eq!(source.to_string(), "credential store unavailable");
    }
}
