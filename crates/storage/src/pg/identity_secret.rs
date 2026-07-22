//! Fail-closed PostgreSQL startup migration for Plane-A TOTP envelopes.
//!
//! Migration `0038_identity_secret_authority.sql` renames the historical
//! `users.mfa_secret_envelope` bytes in place. This runtime pass then distinguishes the
//! bounded legacy Base32 representation from self-identifying `EncryptedData`
//! envelopes, encrypts legacy seeds in crash-resumable batches, authenticates
//! every existing active and pending envelope, and persists current-key
//! replacements for explicitly configured legacy keys.
//!
//! One session-level advisory lock serializes the pass across new replicas.
//! The first-party composition root must await [`PgIdentitySecretMigrator::run`]
//! before constructing or exposing `PgAuthBackend`.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Pool, Postgres};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::identity_secret::{
    IdentitySecretCodec, IdentitySecretError, TotpSecretPurpose, is_canonical_totp_seed,
};

const ADVISORY_LOCK_NAMESPACE: i32 = 0x4e42_554c;
const ADVISORY_LOCK_KEY: i32 = 0x4944_5345;
const MIGRATION_BATCH_SIZE: i64 = 128;
const MAX_CONVERGENCE_PASSES: usize = 4;

/// Secret-free startup-migration failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IdentitySecretMigrationError {
    /// A database operation failed. The stage is a fixed code location, never
    /// SQL text, bound data, a connection URL, or a secret-bearing row.
    #[error("identity secret migration database operation failed: {stage}")]
    Database {
        /// Fixed operation label for operator correlation.
        stage: &'static str,
    },
    /// Stored bytes were neither a supported envelope nor a valid legacy seed.
    #[error("identity secret migration rejected stored secret material: {reason} [{correlation}]")]
    StoredSecretRejected {
        /// Fixed non-secret rejection category.
        reason: IdentitySecretRejectionReason,
        /// Fixed stage or truncated one-way owner fingerprint; never a raw id.
        correlation: String,
    },
    /// Concurrent legacy writes prevented the bounded pass from converging.
    #[error("identity secret migration did not converge")]
    DidNotConverge,
}

/// Safe operator-facing category for a rejected stored identity secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdentitySecretRejectionReason {
    /// A persisted owner id violated the 16-byte domain shape.
    InvalidOwner,
    /// A persisted row exceeded a storage bound.
    InvalidBounds,
    /// Bytes were neither the canonical historical seed nor a v1 envelope.
    MalformedEnvelope,
    /// The envelope names a key outside the explicit current/legacy keyring.
    LegacyKeyUnavailable,
    /// AEAD authentication rejected the ciphertext, owner, purpose, or key.
    AuthenticationFailed,
    /// Authenticated plaintext was not a canonical v1 TOTP seed.
    InvalidPlaintext,
    /// The process key provider/keyring could not satisfy the operation.
    CodecUnavailable,
}

impl std::fmt::Display for IdentitySecretRejectionReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::InvalidOwner => "invalid_owner",
            Self::InvalidBounds => "invalid_bounds",
            Self::MalformedEnvelope => "malformed_envelope",
            Self::LegacyKeyUnavailable => "legacy_key_unavailable",
            Self::AuthenticationFailed => "authentication_failed",
            Self::InvalidPlaintext => "invalid_plaintext",
            Self::CodecUnavailable => "codec_unavailable",
        };
        formatter.write_str(label)
    }
}

/// Non-secret counts from one completed startup migration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[must_use]
pub struct IdentitySecretMigrationReport {
    /// Legacy active factors encrypted in place.
    pub active_legacy_converted: u64,
    /// Legacy pending candidates encrypted in place.
    pub pending_legacy_converted: u64,
    /// Old-key envelopes replaced with current-key envelopes.
    pub envelopes_rotated: u64,
}

impl IdentitySecretMigrationReport {
    fn merge(&mut self, pass: MigrationPass) {
        self.active_legacy_converted = self
            .active_legacy_converted
            .saturating_add(pass.active_legacy_converted);
        self.pending_legacy_converted = self
            .pending_legacy_converted
            .saturating_add(pass.pending_legacy_converted);
        self.envelopes_rotated = self
            .envelopes_rotated
            .saturating_add(pass.envelopes_rotated);
    }
}

#[derive(Default)]
struct MigrationPass {
    active_legacy_converted: u64,
    pending_legacy_converted: u64,
    envelopes_rotated: u64,
    cas_contentions: u64,
}

impl MigrationPass {
    const fn rewrites(&self) -> u64 {
        self.active_legacy_converted
            .saturating_add(self.pending_legacy_converted)
            .saturating_add(self.envelopes_rotated)
    }

    const fn settled(&self) -> bool {
        self.rewrites() == 0 && self.cas_contentions == 0
    }

    fn record_active_rewrite(&mut self, updated: u64, was_legacy: bool) {
        if updated != 1 {
            self.cas_contentions = self.cas_contentions.saturating_add(1);
            return;
        }
        if was_legacy {
            self.active_legacy_converted = self.active_legacy_converted.saturating_add(1);
        } else {
            self.envelopes_rotated = self.envelopes_rotated.saturating_add(1);
        }
    }

    fn record_pending_rewrite(&mut self, updated: u64, was_legacy: bool) {
        if updated != 1 {
            self.cas_contentions = self.cas_contentions.saturating_add(1);
            return;
        }
        if was_legacy {
            self.pending_legacy_converted = self.pending_legacy_converted.saturating_add(1);
        } else {
            self.envelopes_rotated = self.envelopes_rotated.saturating_add(1);
        }
    }
}

enum SecretRewrite {
    Current,
    LegacyPlaintext(Vec<u8>),
    Rotated(Vec<u8>),
}

/// PostgreSQL startup migrator for active and pending TOTP secrets.
pub struct PgIdentitySecretMigrator {
    pool: Pool<Postgres>,
    codec: Arc<IdentitySecretCodec>,
}

impl PgIdentitySecretMigrator {
    /// Bind the migrator to the auth database and shared identity codec.
    #[must_use]
    pub fn new(pool: Pool<Postgres>, codec: Arc<IdentitySecretCodec>) -> Self {
        Self { pool, codec }
    }

    /// Convert legacy seeds, authenticate every envelope, and rotate old keys.
    ///
    /// Work is memory-bounded to 128 rows per query and crash-resumable: every
    /// row rewrite is an equality-guarded update committed independently. A
    /// later pass verifies the resulting envelope. The function returns only
    /// after a full pass observes no legacy or old-key rows.
    ///
    /// # Errors
    ///
    /// Fails closed on database errors, malformed/tampered/wrong-key envelopes,
    /// invalid legacy Base32, or bounded-pass non-convergence. Errors carry no
    /// row bytes, user ids, key ids, database URLs, or driver messages.
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(operation = "identity_secret_migrate")
    )]
    pub async fn run(&self) -> Result<IdentitySecretMigrationReport, IdentitySecretMigrationError> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|_| database_error("acquire_connection"))?;
        // Advisory locks are session-scoped. This startup-only connection is
        // deliberately retired instead of returned to the pool, so task
        // cancellation or panic between lock and explicit unlock cannot
        // poison the pool with an invisibly locked session.
        connection.close_on_drop();
        sqlx::query("SELECT pg_advisory_lock($1, $2)")
            .bind(ADVISORY_LOCK_NAMESPACE)
            .bind(ADVISORY_LOCK_KEY)
            .execute(&mut *connection)
            .await
            .map_err(|_| database_error("acquire_advisory_lock"))?;

        let migration = self.run_locked(&mut connection).await;
        let unlocked: Result<bool, _> = sqlx::query_scalar("SELECT pg_advisory_unlock($1, $2)")
            .bind(ADVISORY_LOCK_NAMESPACE)
            .bind(ADVISORY_LOCK_KEY)
            .fetch_one(&mut *connection)
            .await;
        match (migration, unlocked) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(_) | Ok(false)) => Err(database_error("release_advisory_lock")),
            (Ok(report), Ok(true)) => {
                tracing::info!(
                    active_legacy_converted = report.active_legacy_converted,
                    pending_legacy_converted = report.pending_legacy_converted,
                    envelopes_rotated = report.envelopes_rotated,
                    "identity secret migration converged"
                );
                Ok(report)
            },
        }
    }

    async fn run_locked(
        &self,
        connection: &mut PgConnection,
    ) -> Result<IdentitySecretMigrationReport, IdentitySecretMigrationError> {
        sqlx::query("DELETE FROM mfa_enrollment_candidates WHERE expires_at <= NOW()")
            .execute(&mut *connection)
            .await
            .map_err(|_| database_error("delete_expired_candidates"))?;
        self.validate_stored_bounds(connection).await?;

        let mut report = IdentitySecretMigrationReport::default();
        for _ in 0..MAX_CONVERGENCE_PASSES {
            let pass = self.run_pass(connection).await?;
            let settled = pass.settled();
            report.merge(pass);
            if settled {
                return Ok(report);
            }
        }
        Err(IdentitySecretMigrationError::DidNotConverge)
    }

    async fn validate_stored_bounds(
        &self,
        connection: &mut PgConnection,
    ) -> Result<(), IdentitySecretMigrationError> {
        let invalid_active: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users \
             WHERE octet_length(id) <> 16 \
                OR (mfa_secret_envelope IS NOT NULL \
                    AND octet_length(mfa_secret_envelope) NOT BETWEEN 1 AND 4096))",
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| database_error("validate_active_bounds"))?;
        let invalid_pending: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM mfa_enrollment_candidates \
             WHERE octet_length(user_id) <> 16 \
                OR octet_length(secret_envelope) NOT BETWEEN 1 AND 4096)",
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| database_error("validate_candidate_bounds"))?;
        if invalid_active || invalid_pending {
            Err(IdentitySecretMigrationError::StoredSecretRejected {
                reason: IdentitySecretRejectionReason::InvalidBounds,
                correlation: if invalid_active {
                    "active_bounds".to_owned()
                } else {
                    "candidate_bounds".to_owned()
                },
            })
        } else {
            Ok(())
        }
    }

    async fn run_pass(
        &self,
        connection: &mut PgConnection,
    ) -> Result<MigrationPass, IdentitySecretMigrationError> {
        let mut pass = MigrationPass::default();
        let mut cursor: Option<Vec<u8>> = None;
        loop {
            let rows = sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(
                "SELECT id, mfa_secret_envelope FROM users \
                 WHERE mfa_secret_envelope IS NOT NULL \
                   AND ($1::bytea IS NULL OR id > $1) \
                 ORDER BY id LIMIT $2",
            )
            .bind(cursor.as_deref())
            .bind(MIGRATION_BATCH_SIZE)
            .fetch_all(&mut *connection)
            .await
            .map_err(|_| database_error("read_active_envelopes"))?;
            if rows.is_empty() {
                break;
            }
            for (user_id, stored_bytes) in rows {
                cursor = Some(user_id.clone());
                let stored_bytes = Zeroizing::new(stored_bytes);
                match classify_secret(
                    &self.codec,
                    TotpSecretPurpose::Active,
                    &user_id,
                    &stored_bytes,
                )? {
                    SecretRewrite::Current => {},
                    SecretRewrite::LegacyPlaintext(replacement)
                    | SecretRewrite::Rotated(replacement) => {
                        let was_legacy = looks_like_legacy_base32(&stored_bytes);
                        let updated = sqlx::query(
                            "UPDATE users SET mfa_secret_envelope = $3, version = version + 1 \
                             WHERE id = $1 AND mfa_secret_envelope = $2",
                        )
                        .bind(&user_id)
                        .bind(stored_bytes.as_slice())
                        .bind(replacement)
                        .execute(&mut *connection)
                        .await
                        .map_err(|_| database_error("rewrite_active_envelope"))?
                        .rows_affected();
                        pass.record_active_rewrite(updated, was_legacy);
                    },
                }
            }
        }

        let mut cursor: Option<Vec<u8>> = None;
        loop {
            let rows = sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(
                "SELECT user_id, secret_envelope FROM mfa_enrollment_candidates \
                 WHERE $1::bytea IS NULL OR user_id > $1 \
                 ORDER BY user_id LIMIT $2",
            )
            .bind(cursor.as_deref())
            .bind(MIGRATION_BATCH_SIZE)
            .fetch_all(&mut *connection)
            .await
            .map_err(|_| database_error("read_candidate_envelopes"))?;
            if rows.is_empty() {
                break;
            }
            for (user_id, stored_bytes) in rows {
                cursor = Some(user_id.clone());
                let stored_bytes = Zeroizing::new(stored_bytes);
                match classify_secret(
                    &self.codec,
                    TotpSecretPurpose::EnrollmentCandidate,
                    &user_id,
                    &stored_bytes,
                )? {
                    SecretRewrite::Current => {},
                    SecretRewrite::LegacyPlaintext(replacement)
                    | SecretRewrite::Rotated(replacement) => {
                        let was_legacy = looks_like_legacy_base32(&stored_bytes);
                        let updated = sqlx::query(
                            "UPDATE mfa_enrollment_candidates SET secret_envelope = $3 \
                             WHERE user_id = $1 AND secret_envelope = $2",
                        )
                        .bind(&user_id)
                        .bind(stored_bytes.as_slice())
                        .bind(replacement)
                        .execute(&mut *connection)
                        .await
                        .map_err(|_| database_error("rewrite_candidate_envelope"))?
                        .rows_affected();
                        pass.record_pending_rewrite(updated, was_legacy);
                    },
                }
            }
        }
        Ok(pass)
    }
}

impl std::fmt::Debug for PgIdentitySecretMigrator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PgIdentitySecretMigrator")
            .field("pool", &"[redacted]")
            .field("codec", &"[redacted]")
            .finish()
    }
}

fn classify_secret(
    codec: &IdentitySecretCodec,
    purpose: TotpSecretPurpose,
    user_id: &[u8],
    stored_bytes: &[u8],
) -> Result<SecretRewrite, IdentitySecretMigrationError> {
    match codec.open_totp_seed(purpose, user_id, stored_bytes) {
        Ok(opened) => match opened.replacement_envelope {
            Some(replacement) => Ok(SecretRewrite::Rotated(replacement)),
            None => Ok(SecretRewrite::Current),
        },
        Err(_) if looks_like_legacy_base32(stored_bytes) => codec
            .seal_totp_seed(purpose, user_id, stored_bytes)
            .map(SecretRewrite::LegacyPlaintext)
            .map_err(|error| stored_secret_rejection(error, user_id)),
        Err(error) => Err(stored_secret_rejection(error, user_id)),
    }
}

fn stored_secret_rejection(
    error: IdentitySecretError,
    user_id: &[u8],
) -> IdentitySecretMigrationError {
    let reason = match error {
        IdentitySecretError::InvalidOwner => IdentitySecretRejectionReason::InvalidOwner,
        IdentitySecretError::InvalidEnvelope => IdentitySecretRejectionReason::MalformedEnvelope,
        IdentitySecretError::LegacyKeyUnavailable => {
            IdentitySecretRejectionReason::LegacyKeyUnavailable
        },
        IdentitySecretError::AuthenticationFailed => {
            IdentitySecretRejectionReason::AuthenticationFailed
        },
        IdentitySecretError::InvalidPlaintext => IdentitySecretRejectionReason::InvalidPlaintext,
        IdentitySecretError::KeyUnavailable(_)
        | IdentitySecretError::InvalidKeyring
        | IdentitySecretError::EncryptionFailed => IdentitySecretRejectionReason::CodecUnavailable,
    };
    IdentitySecretMigrationError::StoredSecretRejected {
        reason,
        correlation: owner_fingerprint(user_id),
    }
}

fn owner_fingerprint(user_id: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"nebula:plane-a:identity-migration-owner:v1\0");
    hasher.update(user_id);
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

fn looks_like_legacy_base32(bytes: &[u8]) -> bool {
    is_canonical_totp_seed(bytes)
}

const fn database_error(stage: &'static str) -> IdentitySecretMigrationError {
    IdentitySecretMigrationError::Database { stage }
}

#[cfg(test)]
mod tests {
    use super::looks_like_legacy_base32;

    #[test]
    fn legacy_classifier_accepts_only_canonical_twenty_byte_base32() {
        assert!(looks_like_legacy_base32(
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"
        ));
        assert!(!looks_like_legacy_base32(b"JBSWY3DPEHPK3PXP"));
        assert!(!looks_like_legacy_base32(b"short"));
        assert!(!looks_like_legacy_base32(b"JBSWY3DPEHPK3PX="));
        assert!(!looks_like_legacy_base32(
            br#"{"version":1,"key_id":"identity-key-1"}"#
        ));
    }
}

#[cfg(all(test, feature = "postgres"))]
mod postgres_tests {
    use std::{str::FromStr, sync::Arc};

    use chrono::{Duration, Utc};
    use nebula_crypto::EncryptedData;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

    use super::*;
    use crate::{
        credential::{EnvKeyProvider, KeyProvider},
        pg::{PgUserRepo, identity_secret::IdentitySecretRejectionReason},
        repos::UserRepo,
        test_support::{random_id, test_user},
    };

    const CURRENT_KEY_B64: &str = "VVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVU=";
    const OLD_KEY_B64: &str = "ZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmY=";
    const CANONICAL_SEED: &[u8] = b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
    struct IsolatedDatabase {
        admin: Pool<Postgres>,
        pool: Pool<Postgres>,
        schema: String,
    }

    impl IsolatedDatabase {
        async fn cleanup(self) {
            self.pool.close().await;
            let statement = format!("DROP SCHEMA {} CASCADE", self.schema);
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&self.admin)
                .await
                .expect("drop isolated identity schema");
            self.admin.close().await;
        }
    }

    async fn database() -> Option<IsolatedDatabase> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => return None,
            Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
        };
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect");
        let schema = format!("identity_secret_{}", hex::encode(random_id()));
        let statement = format!("CREATE SCHEMA {schema}");
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&admin)
            .await
            .expect("create isolated identity schema");
        let options = PgConnectOptions::from_str(&url)
            .expect("parse DATABASE_URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(6)
            .connect_with(options)
            .await
            .expect("connect isolated schema");
        MIGRATOR
            .run(&pool)
            .await
            .expect("identity schema migrations");
        Some(IsolatedDatabase {
            admin,
            pool,
            schema,
        })
    }

    fn provider(encoded_key: &str) -> Arc<dyn KeyProvider> {
        Arc::new(EnvKeyProvider::from_base64(encoded_key).expect("valid test key"))
    }

    fn current_codec() -> Arc<IdentitySecretCodec> {
        Arc::new(IdentitySecretCodec::new(provider(CURRENT_KEY_B64)).expect("current codec"))
    }

    async fn seed_user_with_secret(pool: &Pool<Postgres>, label: &str, secret: Vec<u8>) -> Vec<u8> {
        let mut user = test_user(&format!(
            "{label}-{}@identity-migration.test",
            hex::encode(&random_id()[..5])
        ));
        user.mfa_enabled = true;
        user.mfa_secret_envelope = Some(secret);
        PgUserRepo::new(pool.clone())
            .create(&user)
            .await
            .expect("seed identity user");
        user.id
    }

    #[tokio::test]
    async fn converts_active_and_pending_legacy_then_is_idempotent() {
        let Some(database) = database().await else {
            return;
        };
        let pool = database.pool.clone();
        let user_id = seed_user_with_secret(&pool, "legacy", CANONICAL_SEED.to_vec()).await;
        let enrollment_id = [0x71_u8; 32];
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO mfa_enrollment_candidates \
             (user_id, enrollment_id, secret_envelope, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&user_id)
        .bind(enrollment_id.as_slice())
        .bind(CANONICAL_SEED)
        .bind(now)
        .bind(now + Duration::minutes(10))
        .execute(&pool)
        .await
        .expect("seed pending legacy secret");

        let codec = current_codec();
        let report = PgIdentitySecretMigrator::new(pool.clone(), Arc::clone(&codec))
            .run()
            .await
            .expect("convert legacy identity secrets");
        assert_eq!(report.active_legacy_converted, 1);
        assert_eq!(report.pending_legacy_converted, 1);

        let (active, version): (Vec<u8>, i64) =
            sqlx::query_as("SELECT mfa_secret_envelope, version FROM users WHERE id = $1")
                .bind(&user_id)
                .fetch_one(&pool)
                .await
                .expect("load active envelope");
        let pending: Vec<u8> = sqlx::query_scalar(
            "SELECT secret_envelope FROM mfa_enrollment_candidates WHERE user_id = $1",
        )
        .bind(&user_id)
        .fetch_one(&pool)
        .await
        .expect("load pending envelope");
        assert_eq!(version, 1, "active rewrite must fence stale full-row CAS");
        assert_eq!(
            codec
                .open_totp_seed(TotpSecretPurpose::Active, &user_id, &active)
                .expect("active authenticates")
                .plaintext
                .as_slice(),
            CANONICAL_SEED
        );
        assert_eq!(
            codec
                .open_totp_seed(TotpSecretPurpose::EnrollmentCandidate, &user_id, &pending,)
                .expect("pending authenticates")
                .plaintext
                .as_slice(),
            CANONICAL_SEED
        );

        let second = PgIdentitySecretMigrator::new(pool.clone(), codec)
            .run()
            .await
            .expect("second run is idempotent");
        assert_eq!(second, IdentitySecretMigrationReport::default());
        database.cleanup().await;
    }

    #[tokio::test]
    async fn rotates_explicit_old_key_and_rejects_tamper_with_safe_category() {
        let Some(database) = database().await else {
            return;
        };
        let pool = database.pool.clone();
        let old_provider = provider(OLD_KEY_B64);
        let old_snapshot = old_provider.current().expect("old snapshot");
        let old_key_id = old_snapshot.key_id().to_owned();
        let (_, old_key) = old_snapshot.into_parts();
        let old_codec = IdentitySecretCodec::new(old_provider).expect("old codec");

        let rotating_user = random_id();
        let old_envelope = old_codec
            .seal_totp_seed(TotpSecretPurpose::Active, &rotating_user, CANONICAL_SEED)
            .expect("old envelope");
        let rotating_user = seed_user_with_secret(&pool, "rotate", old_envelope).await;
        // Re-seal for the actual fixture user because AAD includes its id.
        let old_envelope = old_codec
            .seal_totp_seed(TotpSecretPurpose::Active, &rotating_user, CANONICAL_SEED)
            .expect("user-bound old envelope");
        sqlx::query("UPDATE users SET mfa_secret_envelope = $2 WHERE id = $1")
            .bind(&rotating_user)
            .bind(&old_envelope)
            .execute(&pool)
            .await
            .expect("bind old envelope to user");

        let current = Arc::new(
            IdentitySecretCodec::with_legacy_keys(
                provider(CURRENT_KEY_B64),
                vec![(old_key_id, old_key)],
            )
            .expect("rotating codec"),
        );
        let report = PgIdentitySecretMigrator::new(pool.clone(), Arc::clone(&current))
            .run()
            .await
            .expect("rotate old key");
        assert_eq!(report.envelopes_rotated, 1);

        let tamper_user = seed_user_with_secret(&pool, "tamper", CANONICAL_SEED.to_vec()).await;
        let envelope = current
            .seal_totp_seed(TotpSecretPurpose::Active, &tamper_user, CANONICAL_SEED)
            .expect("tamper fixture envelope");
        let mut parsed: EncryptedData = serde_json::from_slice(&envelope).expect("parse envelope");
        parsed.ciphertext[0] ^= 0x80;
        let tampered = serde_json::to_vec(&parsed).expect("serialize tampered envelope");
        sqlx::query("UPDATE users SET mfa_secret_envelope = $2 WHERE id = $1")
            .bind(&tamper_user)
            .bind(tampered)
            .execute(&pool)
            .await
            .expect("persist owner-bound tampered envelope");
        let error = PgIdentitySecretMigrator::new(pool.clone(), current)
            .run()
            .await
            .expect_err("tampered envelope must fail closed");
        match error {
            IdentitySecretMigrationError::StoredSecretRejected {
                reason,
                correlation,
            } => {
                assert_eq!(reason, IdentitySecretRejectionReason::AuthenticationFailed);
                assert_eq!(correlation, owner_fingerprint(&tamper_user));
            },
            other => panic!("unexpected migration error: {other:?}"),
        }
        database.cleanup().await;
    }

    #[tokio::test]
    async fn concurrent_migrators_serialize_and_converge_once() {
        let Some(database) = database().await else {
            return;
        };
        let pool = database.pool.clone();
        let user_id = seed_user_with_secret(&pool, "concurrent", CANONICAL_SEED.to_vec()).await;
        let codec = current_codec();
        let first = PgIdentitySecretMigrator::new(pool.clone(), Arc::clone(&codec));
        let second = PgIdentitySecretMigrator::new(pool.clone(), Arc::clone(&codec));

        let (first_report, second_report) = tokio::join!(first.run(), second.run());
        let first_report = first_report.expect("first migrator converges");
        let second_report = second_report.expect("second migrator converges");
        assert_eq!(
            first_report
                .active_legacy_converted
                .saturating_add(second_report.active_legacy_converted),
            1,
            "the advisory lock must serialize conversion so exactly one migrator rewrites the row"
        );

        let envelope: Vec<u8> =
            sqlx::query_scalar("SELECT mfa_secret_envelope FROM users WHERE id = $1")
                .bind(&user_id)
                .fetch_one(&pool)
                .await
                .expect("load converged envelope");
        assert_eq!(
            codec
                .open_totp_seed(TotpSecretPurpose::Active, &user_id, &envelope)
                .expect("concurrent migration leaves an authenticated envelope")
                .plaintext
                .as_slice(),
            CANONICAL_SEED
        );
        database.cleanup().await;
    }

    #[tokio::test]
    async fn schema_rejects_unbounded_identity_authorities() {
        let Some(database) = database().await else {
            return;
        };
        let mut user = test_user(&format!(
            "bounds-{}@identity-migration.test",
            hex::encode(&random_id()[..5])
        ));
        user.mfa_enabled = true;
        user.mfa_secret_envelope = Some(vec![b'A'; 4097]);
        let error = PgUserRepo::new(database.pool.clone())
            .create(&user)
            .await
            .expect_err("oversized secret must fail at the schema boundary");
        assert!(!error.to_string().contains(&"A".repeat(32)));
        database.cleanup().await;
    }
}
