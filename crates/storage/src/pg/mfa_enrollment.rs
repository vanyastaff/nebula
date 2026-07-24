//! PostgreSQL pending-MFA-enrollment repository.
//!
//! Starting enrollment replaces only the candidate row. Installing a
//! candidate deletes the exact live row and updates `users` in one
//! transaction, making success single-use under replay and concurrency.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::{Pool, Postgres};

use crate::{
    StorageError,
    identity_secret::{IdentitySecretCodec, TotpSecretPurpose},
    pg::map_db_err,
    repos::{MfaEnrollmentCandidate, MfaEnrollmentInstallOutcome, MfaEnrollmentRepo},
};

/// PostgreSQL-backed pending MFA enrollment repository.
#[derive(Clone)]
pub struct PgMfaEnrollmentRepo {
    pool: Pool<Postgres>,
    identity_secrets: Arc<IdentitySecretCodec>,
}

impl PgMfaEnrollmentRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>, identity_secrets: Arc<IdentitySecretCodec>) -> Self {
        Self {
            pool,
            identity_secrets,
        }
    }
}

type CandidateTuple = (Vec<u8>, Vec<u8>, Vec<u8>, DateTime<Utc>, DateTime<Utc>);

fn tuple_to_candidate(row: CandidateTuple) -> Result<MfaEnrollmentCandidate, StorageError> {
    let enrollment_id: [u8; 32] = row.1.try_into().map_err(|_: Vec<u8>| {
        StorageError::Serialization("MFA enrollment id is not 32 bytes".to_owned())
    })?;
    MfaEnrollmentCandidate::new(enrollment_id, row.0, row.2, row.3, row.4)
}

impl MfaEnrollmentRepo for PgMfaEnrollmentRepo {
    #[tracing::instrument(level = "debug", skip(self, candidate))]
    async fn replace_candidate(
        &self,
        candidate: &MfaEnrollmentCandidate,
    ) -> Result<(), StorageError> {
        // Authenticate the envelope before it becomes durable. If an
        // explicitly configured legacy key was used, normalize to the
        // current key while preserving the pending-purpose AAD domain.
        let opened = self
            .identity_secrets
            .open_totp_seed(
                TotpSecretPurpose::EnrollmentCandidate,
                candidate.user_id(),
                candidate.secret_envelope(),
            )
            .map_err(identity_secret_storage_error)?;
        let envelope = opened
            .replacement_envelope
            .as_deref()
            .unwrap_or_else(|| candidate.secret_envelope());
        sqlx::query(
            "INSERT INTO mfa_enrollment_candidates \
             (user_id, enrollment_id, secret_envelope, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (user_id) DO UPDATE SET \
               enrollment_id = EXCLUDED.enrollment_id, \
               secret_envelope = EXCLUDED.secret_envelope, \
               created_at = EXCLUDED.created_at, \
               expires_at = EXCLUDED.expires_at",
        )
        .bind(candidate.user_id())
        .bind(candidate.enrollment_id().as_slice())
        .bind(envelope)
        .bind(candidate.created_at())
        .bind(candidate.expires_at())
        .execute(&self.pool)
        .await
        .map_err(|error| map_db_err("mfa_enrollment_candidate", error))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, user_id))]
    async fn get_live_candidate(
        &self,
        user_id: &[u8],
    ) -> Result<Option<MfaEnrollmentCandidate>, StorageError> {
        let row = sqlx::query_as::<_, CandidateTuple>(
            "SELECT user_id, enrollment_id, secret_envelope, created_at, expires_at \
             FROM mfa_enrollment_candidates \
             WHERE user_id = $1 AND expires_at > NOW()",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| map_db_err("mfa_enrollment_candidate", error))?;
        row.map(tuple_to_candidate).transpose()
    }

    #[tracing::instrument(level = "debug", skip(self, user_id, enrollment_id))]
    async fn install_candidate(
        &self,
        user_id: &[u8],
        enrollment_id: &[u8; 32],
    ) -> Result<MfaEnrollmentInstallOutcome, StorageError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| map_db_err("mfa_enrollment_candidate", error))?;
        let secret_envelope = sqlx::query_scalar::<_, Vec<u8>>(
            "DELETE FROM mfa_enrollment_candidates \
             WHERE user_id = $1 AND enrollment_id = $2 AND expires_at > NOW() \
             RETURNING secret_envelope",
        )
        .bind(user_id)
        .bind(enrollment_id.as_slice())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| map_db_err("mfa_enrollment_candidate", error))?;

        let Some(pending_envelope) = secret_envelope else {
            transaction
                .rollback()
                .await
                .map_err(|error| map_db_err("mfa_enrollment_candidate", error))?;
            return Ok(MfaEnrollmentInstallOutcome::CandidateUnavailable);
        };

        // A pending ciphertext is never copied into the active column. The
        // exact candidate is authenticated for its owner and pending purpose,
        // then re-sealed under the active-purpose AAD domain inside the same
        // transaction that consumes it.
        let opened = self
            .identity_secrets
            .open_totp_seed(
                TotpSecretPurpose::EnrollmentCandidate,
                user_id,
                &pending_envelope,
            )
            .map_err(identity_secret_storage_error)?;
        let active_envelope = self
            .identity_secrets
            .seal_totp_seed(TotpSecretPurpose::Active, user_id, &opened.plaintext)
            .map_err(identity_secret_storage_error)?;

        let updated = sqlx::query(
            "UPDATE users SET \
               mfa_secret_envelope = $2, mfa_enabled = TRUE, version = version + 1 \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(active_envelope)
        .execute(&mut *transaction)
        .await
        .map_err(|error| map_db_err("user", error))?
        .rows_affected();
        if updated != 1 {
            transaction
                .rollback()
                .await
                .map_err(|error| map_db_err("mfa_enrollment_candidate", error))?;
            return Err(StorageError::not_found("user", "MFA enrollment owner"));
        }

        transaction
            .commit()
            .await
            .map_err(|error| map_db_err("mfa_enrollment_candidate", error))?;
        Ok(MfaEnrollmentInstallOutcome::Installed)
    }
}

fn identity_secret_storage_error(_: crate::identity_secret::IdentitySecretError) -> StorageError {
    StorageError::Serialization("identity secret envelope operation failed".to_owned())
}

#[cfg(all(test, feature = "postgres"))]
mod tests {

    use chrono::Duration;
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::{
        credential::{EnvKeyProvider, KeyProvider},
        pg::PgUserRepo,
        repos::UserRepo,
        test_support::{random_id, test_user},
    };

    const TEST_KEY_B64: &str = "ERERERERERERERERERERERERERERERERERERERERERE=";

    fn codec() -> Arc<IdentitySecretCodec> {
        let provider = EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid test key");
        Arc::new(
            IdentitySecretCodec::new(Arc::new(provider) as Arc<dyn KeyProvider>)
                .expect("valid identity codec"),
        )
    }

    static SPEC16_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
    static SCHEMA_READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    async fn pool() -> Option<Pool<Postgres>> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => return None,
            Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
        };
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect");
        SCHEMA_READY
            .get_or_init(|| async {
                SPEC16_MIGRATOR
                    .run(&pool)
                    .await
                    .expect("spec-16 postgres migrations");
            })
            .await;
        Some(pool)
    }

    async fn seed_user(pool: &Pool<Postgres>, label: &str) -> Vec<u8> {
        let repo = PgUserRepo::new(pool.clone());
        let suffix = hex::encode(&random_id()[..4]);
        let user = test_user(&format!("{label}-{suffix}@example.test"));
        repo.create(&user).await.expect("seed user");
        user.id
    }

    fn candidate(
        codec: &IdentitySecretCodec,
        user_id: Vec<u8>,
        enrollment_id: [u8; 32],
        secret: &[u8],
    ) -> MfaEnrollmentCandidate {
        let now = Utc::now();
        let envelope = codec
            .seal_totp_seed(TotpSecretPurpose::EnrollmentCandidate, &user_id, secret)
            .expect("seal pending candidate");
        MfaEnrollmentCandidate::new(
            enrollment_id,
            user_id,
            envelope,
            now,
            now + Duration::minutes(10),
        )
        .expect("valid candidate")
    }

    fn enrollment_id() -> [u8; 32] {
        let left = random_id();
        let right = random_id();
        let mut id = [0_u8; 32];
        id[..16].copy_from_slice(&left);
        id[16..].copy_from_slice(&right);
        id
    }

    #[tokio::test]
    async fn install_is_atomic_and_single_use() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "mfa-install").await;
        let codec = codec();
        let repo = PgMfaEnrollmentRepo::new(pool.clone(), Arc::clone(&codec));
        let candidate = candidate(
            &codec,
            user_id.clone(),
            enrollment_id(),
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
        );
        repo.replace_candidate(&candidate)
            .await
            .expect("store candidate");

        assert_eq!(
            repo.install_candidate(&user_id, candidate.enrollment_id())
                .await
                .expect("install candidate"),
            MfaEnrollmentInstallOutcome::Installed
        );
        assert_eq!(
            repo.install_candidate(&user_id, candidate.enrollment_id())
                .await
                .expect("replay candidate"),
            MfaEnrollmentInstallOutcome::CandidateUnavailable
        );

        let row = PgUserRepo::new(pool)
            .get(&user_id)
            .await
            .expect("load user")
            .expect("user exists");
        assert!(row.mfa_enabled);
        let active = codec
            .open_totp_seed(
                TotpSecretPurpose::Active,
                &user_id,
                row.mfa_secret_envelope.as_deref().expect("active envelope"),
            )
            .expect("open active envelope");
        assert_eq!(
            active.plaintext.as_slice(),
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"
        );
        assert!(
            codec
                .open_totp_seed(
                    TotpSecretPurpose::EnrollmentCandidate,
                    &user_id,
                    row.mfa_secret_envelope.as_deref().expect("active envelope"),
                )
                .is_err(),
            "active envelope must not authenticate in the pending domain"
        );
    }

    #[tokio::test]
    async fn concurrent_install_has_exactly_one_winner() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "mfa-concurrent").await;
        let codec = codec();
        let repo = Arc::new(PgMfaEnrollmentRepo::new(pool, Arc::clone(&codec)));
        let candidate = candidate(
            &codec,
            user_id.clone(),
            enrollment_id(),
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
        );
        repo.replace_candidate(&candidate)
            .await
            .expect("store candidate");
        let enrollment_id = *candidate.enrollment_id();

        let left = {
            let repo = Arc::clone(&repo);
            let user_id = user_id.clone();
            tokio::spawn(async move { repo.install_candidate(&user_id, &enrollment_id).await })
        };
        let right = {
            let repo = Arc::clone(&repo);
            let user_id = user_id.clone();
            tokio::spawn(async move { repo.install_candidate(&user_id, &enrollment_id).await })
        };
        let outcomes = [
            left.await.expect("left join").expect("left install"),
            right.await.expect("right join").expect("right install"),
        ];

        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == MfaEnrollmentInstallOutcome::Installed)
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| {
                    **outcome == MfaEnrollmentInstallOutcome::CandidateUnavailable
                })
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn wrong_candidate_id_does_not_consume_or_install() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "mfa-wrong-id").await;
        let codec = codec();
        let repo = PgMfaEnrollmentRepo::new(pool.clone(), Arc::clone(&codec));
        let candidate = candidate(
            &codec,
            user_id.clone(),
            enrollment_id(),
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
        );
        repo.replace_candidate(&candidate)
            .await
            .expect("store candidate");

        assert_eq!(
            repo.install_candidate(&user_id, &enrollment_id())
                .await
                .expect("reject mismatched candidate"),
            MfaEnrollmentInstallOutcome::CandidateUnavailable
        );
        assert!(
            repo.get_live_candidate(&user_id)
                .await
                .expect("load candidate")
                .is_some()
        );
        let user = PgUserRepo::new(pool)
            .get(&user_id)
            .await
            .expect("load user")
            .expect("user exists");
        assert!(!user.mfa_enabled);
        assert!(user.mfa_secret_envelope.is_none());
    }

    #[tokio::test]
    async fn expired_candidate_is_unavailable_and_cannot_install() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "mfa-expired").await;
        let codec = codec();
        let repo = PgMfaEnrollmentRepo::new(pool, Arc::clone(&codec));
        let now = Utc::now();
        let envelope = codec
            .seal_totp_seed(
                TotpSecretPurpose::EnrollmentCandidate,
                &user_id,
                b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
            )
            .expect("seal expired candidate");
        let candidate = MfaEnrollmentCandidate::new(
            enrollment_id(),
            user_id.clone(),
            envelope,
            now - Duration::minutes(20),
            now - Duration::minutes(10),
        )
        .expect("well-ordered expired candidate");
        let candidate_id = *candidate.enrollment_id();
        repo.replace_candidate(&candidate)
            .await
            .expect("store expired candidate");

        assert!(
            repo.get_live_candidate(&user_id)
                .await
                .expect("load expired candidate")
                .is_none()
        );
        assert_eq!(
            repo.install_candidate(&user_id, &candidate_id)
                .await
                .expect("reject expired candidate"),
            MfaEnrollmentInstallOutcome::CandidateUnavailable
        );
    }
}
