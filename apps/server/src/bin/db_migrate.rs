//! Apply the PostgreSQL migration catalog to `DATABASE_URL`.
//!
//! The same admission every server start runs: a fresh database is migrated
//! to head, a database whose ledger is a canonical prefix is brought forward,
//! and any other database is refused with the reason.

#![forbid(unsafe_code)]
#![expect(
    clippy::print_stderr,
    reason = "binary edge: bounded startup diagnostics must reach stderr without Debug rendering"
)]

use nebula_storage::postgres;
use nebula_storage_port::StorageError;
use secrecy::{ExposeSecret as _, SecretString};
use sqlx::postgres::PgPoolOptions;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
enum MigrationOperatorError {
    #[error("DATABASE_URL is not configured")]
    MissingDatabaseUrl,
    #[error("database is unavailable")]
    DatabaseUnavailable,
    #[error("usage: nebula-db-migrate [migrate]")]
    UnknownCommand,
    /// The schema setup message is value-free by construction: it names
    /// migration numbers and the remedy, never URLs or driver text.
    #[error("{0}")]
    SchemaRejected(String),
    #[error("database schema setup failed")]
    SetupFailed,
}

fn parse_arguments(arguments: &[String]) -> Result<(), MigrationOperatorError> {
    match arguments {
        [] => Ok(()),
        [verb] if verb == "migrate" => Ok(()),
        _ => Err(MigrationOperatorError::UnknownCommand),
    }
}

fn setup_failure(error: StorageError) -> MigrationOperatorError {
    match error {
        StorageError::Configuration(reason) => MigrationOperatorError::SchemaRejected(reason),
        StorageError::Connection(_) => MigrationOperatorError::DatabaseUnavailable,
        _ => MigrationOperatorError::SetupFailed,
    }
}

async fn run() -> Result<(), MigrationOperatorError> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    parse_arguments(&arguments)?;

    let database_url = std::env::var("DATABASE_URL")
        .map(SecretString::from)
        .map_err(|_| MigrationOperatorError::MissingDatabaseUrl)?;
    let pool = PgPoolOptions::new()
        .connect(database_url.expose_secret())
        .await
        .map_err(|_| MigrationOperatorError::DatabaseUnavailable)?;

    let result = postgres::init_schema(&pool).await.map_err(setup_failure);
    pool.close().await;
    result
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{MigrationOperatorError, parse_arguments, setup_failure};
    use nebula_storage_port::StorageError;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn no_arguments_or_migrate_runs_the_catalog() {
        assert_eq!(parse_arguments(&arguments(&[])), Ok(()));
        assert_eq!(parse_arguments(&arguments(&["migrate"])), Ok(()));
    }

    #[test]
    fn unknown_verbs_are_refused() {
        for verb in [&["revert"][..], &["adopt", "40"][..]] {
            assert_eq!(
                parse_arguments(&arguments(verb)),
                Err(MigrationOperatorError::UnknownCommand)
            );
        }
    }

    #[test]
    fn a_schema_rejection_keeps_its_value_free_reason() {
        let error = setup_failure(StorageError::Configuration(
            "database schema rejected: migration 0001 has a different checksum".to_owned(),
        ));
        assert_eq!(
            error.to_string(),
            "database schema rejected: migration 0001 has a different checksum"
        );
    }

    #[test]
    fn operational_failures_never_echo_driver_text() {
        for error in [
            StorageError::Connection("private driver detail".to_owned()),
            StorageError::Internal("private invariant detail".to_owned()),
        ] {
            let rendered = setup_failure(error).to_string();
            assert!(!rendered.contains("private"), "{rendered}");
        }
    }
}
