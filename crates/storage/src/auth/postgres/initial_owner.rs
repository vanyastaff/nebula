//! Atomic first-owner reservation on the admitted PostgreSQL deployment pool.

use super::PgAccountLifecycle;
use crate::{
    StorageError,
    auth::{
        InitialOwnerBegin, InitialOwnerRegistration,
        initial_owner::{Enrollment, decode_enrollment, validate_registration},
    },
    sql_error::{storage_error, storage_error_for},
};

impl PgAccountLifecycle {
    /// Create the initial account and permanently freeze its tenant command.
    /// Repeated begin never changes credentials, replaces the command or grants
    /// tenant authority. Email verification remains unset and no token is minted.
    ///
    /// # Errors
    /// Invalid owner bindings fail before SQL. Database failures are value-free;
    /// an error during commit leaves the outcome unknown. Inspect or resume the
    /// durable enrollment instead of interpreting that error as rollback.
    #[tracing::instrument(skip_all)]
    pub async fn begin_initial_owner(
        &self,
        registration: &InitialOwnerRegistration<'_>,
    ) -> Result<InitialOwnerBegin, StorageError> {
        validate_registration(registration)?;
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let (state, user_id, request): (String, Option<Vec<u8>>, Option<serde_json::Value>) =
            sqlx::query_as(
                "SELECT state, user_id, tenant_request
                 FROM initial_owner_enrollment WHERE singleton = 1 FOR UPDATE",
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(storage_error)?;
        match decode_enrollment(state, user_id, request)? {
            Enrollment::Pending { .. } => {
                tracing::debug!(outcome = "already_started", "initial owner begin rejected");
                return Ok(InitialOwnerBegin::AlreadyStarted);
            },
            Enrollment::Sealed => {
                tracing::debug!(outcome = "unavailable", "initial owner begin rejected");
                return Ok(InitialOwnerBegin::Unavailable);
            },
            Enrollment::Available => {},
        }
        let used: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users)
                 OR EXISTS (SELECT 1 FROM orgs)
                 OR EXISTS (SELECT 1 FROM tenant_provisioning_receipts)",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(storage_error)?;
        if used {
            sqlx::query(
                "UPDATE initial_owner_enrollment
                 SET state = 'sealed', recorded_at = clock_timestamp() WHERE singleton = 1",
            )
            .execute(&mut *tx)
            .await
            .map_err(storage_error)?;
            tx.commit().await.map_err(storage_error)?;
            tracing::debug!(outcome = "unavailable", "initial owner enrollment sealed");
            return Ok(InitialOwnerBegin::Unavailable);
        }
        let user_id = registration.user_id.as_bytes();
        sqlx::query(
            "UPDATE initial_owner_enrollment
             SET state = 'enrolled', user_id = $1, tenant_request = $2,
                 recorded_at = clock_timestamp() WHERE singleton = 1",
        )
        .bind(user_id.as_slice())
        .bind(crate::tenant_provisioning::encode_request(
            registration.tenant_request,
        ))
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        sqlx::query(
            "INSERT INTO users (id, email, display_name, password_hash, created_at)
             VALUES ($1, $2, $3, $4, clock_timestamp())",
        )
        .bind(user_id.as_slice())
        .bind(registration.email)
        .bind(registration.display_name)
        .bind(registration.password_hash)
        .execute(&mut *tx)
        .await
        .map_err(|error| storage_error_for("user", error))?;
        tx.commit().await.map_err(storage_error)?;
        tracing::info!(outcome = "begun", "initial owner enrollment recorded");
        Ok(InitialOwnerBegin::Begun)
    }
}
