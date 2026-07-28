//! Admitted PostgreSQL migration operator.

#![forbid(unsafe_code)]
#![expect(
    clippy::print_stderr,
    reason = "binary edge: bounded startup diagnostics must reach stderr without Debug rendering"
)]

use nebula_storage::{
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

async fn run() -> Result<(), MigrationOperatorError> {
    let database_url = std::env::var("DATABASE_URL")
        .map(SecretString::from)
        .map_err(|_| MigrationOperatorError::MissingDatabaseUrl)?;
    let pool = PgPoolOptions::new()
        .connect(database_url.expose_secret())
        .await
        .map_err(|_| MigrationOperatorError::DatabaseUnavailable)?;

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
    use super::{MigrationOperatorError, MigrationRoute, migration_route};
    use nebula_storage::credential::CredentialStoreStartupError;
    use nebula_storage_port::StorageError;

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
