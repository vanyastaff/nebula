//! PostgreSQL atomic finalizer for Plane-A OAuth login completion.
//!
//! The provider exchange and userinfo requests finish before this module
//! is entered. It owns the one transaction that resolves or creates the
//! local user, converges the stable external-identity link under races,
//! and persists exactly one authority artifact: a browser session or a
//! one-time local MFA challenge. No network operation is performed while
//! database locks are held.

use sqlx::{Pool, Postgres, Transaction};

use crate::{
    StorageError,
    pg::user::{SELECT_COLS, UserTuple, tuple_to_row},
    repos::{
        OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome, OAuthLoginFinalized,
        OAuthLoginSessionDraft,
    },
    rows::UserRow,
    session_token::session_token_digest,
};

const USER_ID_BYTES: usize = 16;
const MAX_PROVIDER_BYTES: usize = 64;
const MAX_SUBJECT_BYTES: usize = 255;
const MAX_EMAIL_BYTES: usize = 254;

const INSERT_CANDIDATE_USER_SQL: &str = "INSERT INTO users \
     (id, email, email_verified_at, display_name, avatar_url, password_hash, \
      created_at, last_login_at, locked_until, failed_login_count, mfa_enabled, \
      mfa_secret_envelope, version, deleted_at) \
     VALUES ($1, $2, $3, $4, $5, NULL, $3, NULL, NULL, 0, FALSE, NULL, 0, NULL) \
     ON CONFLICT (LOWER(email)) WHERE deleted_at IS NULL DO NOTHING \
     RETURNING id, email, email_verified_at, display_name, avatar_url, password_hash, \
      created_at, last_login_at, locked_until, failed_login_count, mfa_enabled, \
      mfa_secret_envelope, version, deleted_at";

const INSERT_EXTERNAL_IDENTITY_SQL: &str = "INSERT INTO external_identities \
     (provider, subject, user_id, email) VALUES ($1, $2, $3, $4) \
     ON CONFLICT (provider, subject) DO NOTHING RETURNING user_id";

const INSERT_SESSION_SQL: &str = "INSERT INTO sessions \
     (token_digest, user_id, created_at, last_active_at, expires_at, ip_address, user_agent, revoked_at) \
     VALUES ($1, $2, $3, $4, $5, $6::inet, $7, NULL)";

const INSERT_MFA_CHALLENGE_SQL: &str = "INSERT INTO verification_tokens \
     (token_hash, user_id, kind, payload, created_at, expires_at, consumed_at) \
     VALUES ($1, $2, 'mfa_challenge', NULL, $3, $4, NULL)";

enum LinkedUser {
    Absent,
    Active(Box<UserRow>),
    Unavailable,
}

enum TransactionDecision {
    Commit(Box<UserRow>),
    CommitMfaRequired,
    Rollback(OAuthLoginRejection),
}

enum OAuthLoginRejection {
    VerifiedEmailRequired,
    AccountLinkRequired,
    LinkedUserUnavailable,
}

impl OAuthLoginRejection {
    const fn outcome_label(&self) -> &'static str {
        match self {
            Self::VerifiedEmailRequired => "verified_email_required",
            Self::AccountLinkRequired => "account_link_required",
            Self::LinkedUserUnavailable => "linked_user_unavailable",
        }
    }
}

impl From<OAuthLoginRejection> for OAuthLoginFinalizeOutcome {
    fn from(rejection: OAuthLoginRejection) -> Self {
        match rejection {
            OAuthLoginRejection::VerifiedEmailRequired => Self::VerifiedEmailRequired,
            OAuthLoginRejection::AccountLinkRequired => Self::AccountLinkRequired,
            OAuthLoginRejection::LinkedUserUnavailable => Self::LinkedUserUnavailable,
        }
    }
}

/// PostgreSQL-backed atomic OAuth login finalizer.
#[derive(Clone)]
pub struct PgOAuthLoginFinalizer {
    pool: Pool<Postgres>,
}

impl PgOAuthLoginFinalizer {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    /// Atomically converge one verified provider identity into a local
    /// user, stable external link, and exactly one session/MFA artifact.
    ///
    /// `(provider, subject)` is authoritative whenever it already
    /// exists. Otherwise a provider-attested email is required. Races
    /// are resolved inside the transaction: callers never need to retry
    /// a duplicate user or link error to discover the canonical user.
    #[tracing::instrument(level = "info", skip_all, fields(operation = "oauth_login_finalize"))]
    pub async fn finalize(
        &self,
        command: OAuthLoginFinalizeCommand,
    ) -> Result<OAuthLoginFinalizeOutcome, StorageError> {
        validate_common_command(&command)?;
        let mut transaction = self.pool.begin().await.map_err(begin_error)?;
        match finalize_in_transaction(&mut transaction, &command).await {
            Ok(TransactionDecision::Commit(user)) => {
                commit(transaction).await?;
                tracing::info!(outcome = "finalized", "OAuth login finalized");
                Ok(OAuthLoginFinalizeOutcome::Finalized(Box::new(
                    OAuthLoginFinalized {
                        user: *user,
                        session_token: command.session.token,
                        session_expires_at: command.session.expires_at,
                    },
                )))
            },
            Ok(TransactionDecision::CommitMfaRequired) => {
                commit(transaction).await?;
                tracing::info!(outcome = "mfa_required", "OAuth login requires MFA");
                Ok(OAuthLoginFinalizeOutcome::MfaRequired)
            },
            Ok(TransactionDecision::Rollback(rejection)) => {
                rollback(transaction).await?;
                tracing::warn!(outcome = rejection.outcome_label(), "OAuth login rejected");
                Ok(rejection.into())
            },
            Err(error) => {
                rollback(transaction).await?;
                tracing::error!(outcome = "storage_error", "OAuth login finalization failed");
                Err(error)
            },
        }
    }
}

async fn finalize_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    command: &OAuthLoginFinalizeCommand,
) -> Result<TransactionDecision, StorageError> {
    match load_linked_user(transaction, &command.provider, &command.subject).await? {
        LinkedUser::Active(user) => {
            return finalize_for_canonical_user(transaction, *user, command).await;
        },
        LinkedUser::Unavailable => {
            return Ok(TransactionDecision::Rollback(
                OAuthLoginRejection::LinkedUserUnavailable,
            ));
        },
        LinkedUser::Absent => {},
    }

    let Some(verified_email) = command
        .verified_email
        .as_deref()
        .filter(|email| !email.trim().is_empty())
    else {
        return Ok(TransactionDecision::Rollback(
            OAuthLoginRejection::VerifiedEmailRequired,
        ));
    };

    let Some(selected_user) = insert_candidate_user(transaction, command, verified_email).await?
    else {
        // A concurrent same-subject finalization may have created both the
        // email owner and authoritative link while this INSERT waited on the
        // unique email index. Recheck the subject before classifying the
        // collision. A different/unlinked subject must never inherit an
        // existing account merely because the provider reports its email.
        return match load_linked_user(transaction, &command.provider, &command.subject).await? {
            LinkedUser::Active(user) => {
                finalize_for_canonical_user(transaction, *user, command).await
            },
            LinkedUser::Unavailable => Ok(TransactionDecision::Rollback(
                OAuthLoginRejection::LinkedUserUnavailable,
            )),
            LinkedUser::Absent => Ok(TransactionDecision::Rollback(
                OAuthLoginRejection::AccountLinkRequired,
            )),
        };
    };

    let linked_user_id: Option<Vec<u8>> = sqlx::query_scalar(INSERT_EXTERNAL_IDENTITY_SQL)
        .bind(&command.provider)
        .bind(&command.subject)
        .bind(&selected_user.id)
        .bind(verified_email)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| operation_error())?;

    let canonical_user = if linked_user_id.is_some() {
        selected_user
    } else {
        match load_linked_user(transaction, &command.provider, &command.subject).await? {
            LinkedUser::Active(user) => {
                if user.id != selected_user.id {
                    delete_candidate_user(transaction, &selected_user.id).await?;
                }
                *user
            },
            LinkedUser::Unavailable => {
                return Ok(TransactionDecision::Rollback(
                    OAuthLoginRejection::LinkedUserUnavailable,
                ));
            },
            LinkedUser::Absent => return Err(operation_error()),
        }
    };

    finalize_for_canonical_user(transaction, canonical_user, command).await
}

async fn finalize_for_canonical_user(
    transaction: &mut Transaction<'_, Postgres>,
    user: UserRow,
    command: &OAuthLoginFinalizeCommand,
) -> Result<TransactionDecision, StorageError> {
    if user.mfa_enabled {
        if user
            .mfa_secret_envelope
            .as_deref()
            .is_none_or(<[u8]>::is_empty)
        {
            return Err(operation_error());
        }
        insert_mfa_challenge(transaction, &user.id, command).await?;
        return Ok(TransactionDecision::CommitMfaRequired);
    }

    insert_session(transaction, &user.id, &command.session).await?;
    Ok(TransactionDecision::Commit(Box::new(user)))
}

fn validate_common_command(command: &OAuthLoginFinalizeCommand) -> Result<(), StorageError> {
    let valid = valid_provider(&command.provider)
        && valid_subject(&command.subject)
        && command
            .verified_email
            .as_deref()
            .is_none_or(valid_canonical_verified_email)
        && command.candidate_user.id.len() == USER_ID_BYTES
        && !command.candidate_user.display_name.trim().is_empty()
        && !command.session.token.is_empty()
        && command.session.created_at <= command.session.last_active_at
        && command.session.last_active_at < command.session.expires_at
        && command
            .mfa_challenge
            .token_hash
            .iter()
            .any(|byte| *byte != 0)
        && command.mfa_challenge.created_at < command.mfa_challenge.expires_at;
    if valid {
        Ok(())
    } else {
        Err(invalid_command_error())
    }
}

fn valid_provider(provider: &str) -> bool {
    !provider.is_empty()
        && provider.len() <= MAX_PROVIDER_BYTES
        && provider.trim() == provider
        && provider.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn valid_subject(subject: &str) -> bool {
    !subject.is_empty()
        && subject.len() <= MAX_SUBJECT_BYTES
        && subject.trim() == subject
        && !subject.chars().any(char::is_control)
}

fn valid_canonical_verified_email(email: &str) -> bool {
    if email.is_empty()
        || email.len() > MAX_EMAIL_BYTES
        || email.trim() != email
        || email.to_lowercase() != email
        || email.chars().any(char::is_whitespace)
        || email.chars().any(char::is_control)
    {
        return false;
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain)) = (parts.next(), parts.next()) else {
        return false;
    };
    let local_valid = !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..");
    let domain_valid = !domain.is_empty()
        && parts.next().is_none()
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    local_valid && domain_valid
}

fn validate_active_user(user: &UserRow) -> Result<(), StorageError> {
    if user.id.len() == USER_ID_BYTES {
        Ok(())
    } else {
        Err(operation_error())
    }
}

async fn load_linked_user(
    transaction: &mut Transaction<'_, Postgres>,
    provider: &str,
    subject: &str,
) -> Result<LinkedUser, StorageError> {
    let linked_user_id: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT user_id FROM external_identities \
         WHERE provider = $1 AND subject = $2 FOR UPDATE",
    )
    .bind(provider)
    .bind(subject)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| operation_error())?;

    let Some(linked_user_id) = linked_user_id else {
        return Ok(LinkedUser::Absent);
    };
    match load_active_user_by_id(transaction, &linked_user_id).await? {
        Some(user) => Ok(LinkedUser::Active(Box::new(user))),
        None => Ok(LinkedUser::Unavailable),
    }
}

async fn load_active_user_by_id(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: &[u8],
) -> Result<Option<UserRow>, StorageError> {
    let sql =
        format!("SELECT {SELECT_COLS} FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE");
    let row = sqlx::query_as::<_, UserTuple>(sqlx::AssertSqlSafe(sql))
        .bind(user_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| operation_error())?;
    let row = row.map(tuple_to_row);
    if let Some(user) = row.as_ref() {
        validate_active_user(user)?;
    }
    Ok(row)
}

async fn insert_candidate_user(
    transaction: &mut Transaction<'_, Postgres>,
    command: &OAuthLoginFinalizeCommand,
    verified_email: &str,
) -> Result<Option<UserRow>, StorageError> {
    let row = sqlx::query_as::<_, UserTuple>(INSERT_CANDIDATE_USER_SQL)
        .bind(&command.candidate_user.id)
        .bind(verified_email)
        .bind(command.candidate_user.created_at)
        .bind(&command.candidate_user.display_name)
        .bind(command.candidate_user.avatar_url.as_deref())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| operation_error())?;
    Ok(row.map(tuple_to_row))
}

async fn delete_candidate_user(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: &[u8],
) -> Result<(), StorageError> {
    let deleted = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&mut **transaction)
        .await
        .map_err(|_| operation_error())?
        .rows_affected();
    if deleted == 1 {
        Ok(())
    } else {
        Err(operation_error())
    }
}

async fn insert_session(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: &[u8],
    draft: &OAuthLoginSessionDraft,
) -> Result<(), StorageError> {
    let digest = session_token_digest(&draft.token);
    sqlx::query(INSERT_SESSION_SQL)
        .bind(digest.as_bytes().as_slice())
        .bind(user_id)
        .bind(draft.created_at)
        .bind(draft.last_active_at)
        .bind(draft.expires_at)
        .bind(draft.ip_address.as_deref())
        .bind(draft.user_agent.as_deref())
        .execute(&mut **transaction)
        .await
        .map_err(|_| operation_error())?;
    Ok(())
}

async fn insert_mfa_challenge(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: &[u8],
    command: &OAuthLoginFinalizeCommand,
) -> Result<(), StorageError> {
    sqlx::query(INSERT_MFA_CHALLENGE_SQL)
        .bind(command.mfa_challenge.token_hash.as_slice())
        .bind(user_id)
        .bind(command.mfa_challenge.created_at)
        .bind(command.mfa_challenge.expires_at)
        .execute(&mut **transaction)
        .await
        .map_err(|_| operation_error())?;
    Ok(())
}

async fn rollback(transaction: Transaction<'_, Postgres>) -> Result<(), StorageError> {
    transaction.rollback().await.map_err(|_| {
        StorageError::Connection("OAuth login finalization rollback failed".to_owned())
    })
}

async fn commit(transaction: Transaction<'_, Postgres>) -> Result<(), StorageError> {
    transaction.commit().await.map_err(|_| {
        StorageError::Connection("OAuth login finalization commit outcome is unknown".to_owned())
    })
}

fn begin_error(_: sqlx::Error) -> StorageError {
    StorageError::Connection("OAuth login finalization unavailable".to_owned())
}

fn invalid_command_error() -> StorageError {
    StorageError::Internal("invalid OAuth login finalization command".to_owned())
}

fn operation_error() -> StorageError {
    StorageError::Internal("OAuth login finalization failed".to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{Duration, Utc};
    use sqlx::postgres::PgPoolOptions;
    use tokio::sync::Barrier;

    use super::*;
    use crate::{
        StorageError,
        pg::{PgExternalIdentityRepo, PgSessionRepo, PgUserRepo},
        repos::{
            ExternalIdentityRepo, OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome,
            OAuthLoginFinalized, OAuthLoginMfaChallengeDraft, OAuthLoginSessionDraft,
            OAuthLoginUserDraft, SessionRepo, UserRepo,
        },
        rows::SessionDraft,
        test_support::{random_id, test_user},
    };

    static SPEC16_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
    static SCHEMA_READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    async fn pool() -> Option<Pool<Postgres>> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => return None,
            Err(err) => panic!("DATABASE_URL is set but invalid: {err}"),
        };
        let pool = PgPoolOptions::new()
            .max_connections(8)
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

    fn unique(prefix: &str) -> String {
        format!("{prefix}-{}", hex::encode(&random_id()[..6]))
    }

    fn command(
        provider: &str,
        subject: &str,
        verified_email: Option<&str>,
    ) -> OAuthLoginFinalizeCommand {
        let now = Utc::now();
        OAuthLoginFinalizeCommand {
            provider: provider.to_owned(),
            subject: subject.to_owned(),
            verified_email: verified_email.map(str::to_owned),
            candidate_user: OAuthLoginUserDraft {
                id: random_id(),
                display_name: "OAuth test user".to_owned(),
                avatar_url: None,
                created_at: now,
            },
            session: OAuthLoginSessionDraft {
                token: random_id(),
                created_at: now,
                last_active_at: now,
                expires_at: now + Duration::hours(2),
                ip_address: Some("192.0.2.42".to_owned()),
                user_agent: Some("nebula-storage-oauth-test/1.0".to_owned()),
            },
            mfa_challenge: OAuthLoginMfaChallengeDraft {
                token_hash: random_id()
                    .repeat(2)
                    .try_into()
                    .expect("32-byte challenge hash"),
                created_at: now,
                expires_at: now + Duration::minutes(5),
            },
        }
    }

    fn finalized(outcome: OAuthLoginFinalizeOutcome) -> OAuthLoginFinalized {
        match outcome {
            OAuthLoginFinalizeOutcome::Finalized(finalized) => *finalized,
            _ => panic!("expected finalized OAuth login"),
        }
    }

    #[test]
    fn user_insert_converges_on_the_active_email_index() {
        let sql = INSERT_CANDIDATE_USER_SQL
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(sql.contains("ON CONFLICT (LOWER(email)) WHERE deleted_at IS NULL DO NOTHING"));
    }

    #[test]
    fn identity_insert_converges_without_rebinding_the_winner() {
        let sql = INSERT_EXTERNAL_IDENTITY_SQL
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(sql.contains("ON CONFLICT (provider, subject) DO NOTHING RETURNING user_id"));
        assert!(!sql.contains("DO UPDATE"));
    }

    #[test]
    fn command_validation_rejects_noncanonical_or_oversized_identity_inputs() {
        let valid = command("google", "subject-1", Some("alice@example.test"));
        assert!(validate_common_command(&valid).is_ok());

        let invalid_emails = [
            "Alice@example.test".to_owned(),
            " alice@example.test".to_owned(),
            format!("{}@example.test", "a".repeat(MAX_EMAIL_BYTES)),
        ];
        for email in invalid_emails {
            let invalid = command("google", "subject-1", Some(&email));
            assert!(validate_common_command(&invalid).is_err());
        }

        let invalid_provider = command("Google", "subject-1", Some("alice@example.test"));
        assert!(validate_common_command(&invalid_provider).is_err());
        let oversized_subject = "s".repeat(MAX_SUBJECT_BYTES + 1);
        let invalid_subject = command("google", &oversized_subject, Some("alice@example.test"));
        assert!(validate_common_command(&invalid_subject).is_err());
        let mut invalid_user_id = command("google", "subject-1", Some("alice@example.test"));
        invalid_user_id.candidate_user.id.pop();
        assert!(validate_common_command(&invalid_user_id).is_err());
        let mut zero_challenge = command("google", "subject-1", Some("alice@example.test"));
        zero_challenge.mfa_challenge.token_hash = [0; 32];
        assert!(validate_common_command(&zero_challenge).is_err());
    }

    #[tokio::test]
    async fn same_subject_and_email_race_converges_to_one_user_and_link() {
        let Some(pool) = pool().await else { return };
        let provider = unique("same-subject-provider");
        let subject = unique("same-subject");
        let email = format!("{}@example.test", unique("same-subject"));
        let left_command = command(&provider, &subject, Some(&email));
        let right_command = command(&provider, &subject, Some(&email));
        let left_session_token = left_command.session.token.clone();
        let right_session_token = right_command.session.token.clone();
        let left_session_digest = session_token_digest(&left_session_token);
        let right_session_digest = session_token_digest(&right_session_token);
        let left_challenge_hash = left_command.mfa_challenge.token_hash;
        let right_challenge_hash = right_command.mfa_challenge.token_hash;
        let barrier = Arc::new(Barrier::new(2));
        let left_barrier = Arc::clone(&barrier);
        let right_barrier = Arc::clone(&barrier);
        let left_finalizer = PgOAuthLoginFinalizer::new(pool.clone());
        let right_finalizer = left_finalizer.clone();

        let (left, right) = tokio::join!(
            async move {
                left_barrier.wait().await;
                left_finalizer.finalize(left_command).await
            },
            async move {
                right_barrier.wait().await;
                right_finalizer.finalize(right_command).await
            }
        );
        let left = finalized(left.expect("left finalization"));
        let right = finalized(right.expect("right finalization"));

        assert_eq!(left.user.id, right.user.id);
        assert!(!left.user.mfa_enabled);
        assert!(!right.user.mfa_enabled);
        assert_eq!(left.session_token, left_session_token);
        assert_eq!(right.session_token, right_session_token);
        let user_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL",
        )
        .bind(&email)
        .fetch_one(&pool)
        .await
        .expect("count users");
        assert_eq!(user_count, 1);
        let linked_user: Vec<u8> = sqlx::query_scalar(
            "SELECT user_id FROM external_identities WHERE provider = $1 AND subject = $2",
        )
        .bind(&provider)
        .bind(&subject)
        .fetch_one(&pool)
        .await
        .expect("linked user");
        assert_eq!(linked_user, left.user.id);
        let session_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sessions WHERE token_digest = $1 OR token_digest = $2",
        )
        .bind(left_session_digest.as_bytes().as_slice())
        .bind(right_session_digest.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count sessions");
        assert_eq!(session_count, 2);
        let challenge_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM verification_tokens WHERE token_hash = $1 OR token_hash = $2",
        )
        .bind(left_challenge_hash.as_slice())
        .bind(right_challenge_hash.as_slice())
        .fetch_one(&pool)
        .await
        .expect("count unused MFA challenges");
        assert_eq!(challenge_count, 0);
    }

    #[tokio::test]
    async fn different_subjects_with_the_same_email_require_explicit_linking() {
        let Some(pool) = pool().await else { return };
        let provider = unique("shared-email-provider");
        let left_subject = unique("left-subject");
        let right_subject = unique("right-subject");
        let email = format!("{}@example.test", unique("shared-email"));
        let left_command = command(&provider, &left_subject, Some(&email));
        let right_command = command(&provider, &right_subject, Some(&email));
        let left_session_token = left_command.session.token.clone();
        let right_session_token = right_command.session.token.clone();
        let left_session_digest = session_token_digest(&left_session_token);
        let right_session_digest = session_token_digest(&right_session_token);
        let barrier = Arc::new(Barrier::new(2));
        let left_barrier = Arc::clone(&barrier);
        let right_barrier = Arc::clone(&barrier);
        let left_finalizer = PgOAuthLoginFinalizer::new(pool.clone());
        let right_finalizer = left_finalizer.clone();

        let (left, right) = tokio::join!(
            async move {
                left_barrier.wait().await;
                left_finalizer.finalize(left_command).await
            },
            async move {
                right_barrier.wait().await;
                right_finalizer.finalize(right_command).await
            }
        );
        let left = left.expect("left finalization");
        let right = right.expect("right finalization");
        let finalized = match (left, right) {
            (
                OAuthLoginFinalizeOutcome::Finalized(finalized),
                OAuthLoginFinalizeOutcome::AccountLinkRequired,
            )
            | (
                OAuthLoginFinalizeOutcome::AccountLinkRequired,
                OAuthLoginFinalizeOutcome::Finalized(finalized),
            ) => *finalized,
            _ => panic!("exactly one subject must claim an unowned email"),
        };

        let user_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL",
        )
        .bind(&email)
        .fetch_one(&pool)
        .await
        .expect("count users");
        assert_eq!(user_count, 1);
        let link_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_identities WHERE provider = $1 AND subject IN ($2, $3)",
        )
        .bind(&provider)
        .bind(&left_subject)
        .bind(&right_subject)
        .fetch_one(&pool)
        .await
        .expect("count links");
        assert_eq!(link_count, 1);
        let session_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sessions WHERE token_digest = $1 OR token_digest = $2",
        )
        .bind(left_session_digest.as_bytes().as_slice())
        .bind(right_session_digest.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count sessions");
        assert_eq!(session_count, 1);
        assert!(
            finalized.session_token == left_session_token
                || finalized.session_token == right_session_token
        );
    }

    #[tokio::test]
    async fn same_subject_with_different_emails_removes_the_losing_candidate() {
        let Some(pool) = pool().await else { return };
        let provider = unique("different-email-provider");
        let subject = unique("different-email-subject");
        let left_email = format!("{}@example.test", unique("left-email"));
        let right_email = format!("{}@example.test", unique("right-email"));
        let left_command = command(&provider, &subject, Some(&left_email));
        let right_command = command(&provider, &subject, Some(&right_email));
        let barrier = Arc::new(Barrier::new(2));
        let left_barrier = Arc::clone(&barrier);
        let right_barrier = Arc::clone(&barrier);
        let left_finalizer = PgOAuthLoginFinalizer::new(pool.clone());
        let right_finalizer = left_finalizer.clone();

        let (left, right) = tokio::join!(
            async move {
                left_barrier.wait().await;
                left_finalizer.finalize(left_command).await
            },
            async move {
                right_barrier.wait().await;
                right_finalizer.finalize(right_command).await
            }
        );
        let left = finalized(left.expect("left finalization"));
        let right = finalized(right.expect("right finalization"));

        assert_eq!(left.user.id, right.user.id);
        let candidate_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE (LOWER(email) = LOWER($1) OR LOWER(email) = LOWER($2)) AND deleted_at IS NULL",
        )
        .bind(&left_email)
        .bind(&right_email)
        .fetch_one(&pool)
        .await
        .expect("count candidate users");
        assert_eq!(candidate_count, 1);
    }

    #[tokio::test]
    async fn existing_link_does_not_require_email_and_keeps_its_original_user() {
        let Some(pool) = pool().await else { return };
        let provider = unique("existing-link-provider");
        let subject = unique("existing-link-subject");
        let mut user = test_user(&format!("{}@example.test", unique("existing-link")));
        user.email_verified_at = Some(Utc::now());
        PgUserRepo::new(pool.clone())
            .create(&user)
            .await
            .expect("seed user");
        PgExternalIdentityRepo::new(pool.clone())
            .link_external(&user.id, &provider, &subject, Some(&user.email))
            .await
            .expect("seed identity link");

        let command = command(&provider, &subject, None);
        let session_token = command.session.token.clone();
        let outcome = PgOAuthLoginFinalizer::new(pool)
            .finalize(command)
            .await
            .expect("finalize existing link");
        let finalized = finalized(outcome);

        assert_eq!(finalized.user.id, user.id);
        assert_eq!(finalized.session_token, session_token);
    }

    #[tokio::test]
    async fn existing_link_with_mfa_never_creates_a_session() {
        let Some(pool) = pool().await else { return };
        let provider = unique("existing-mfa-link-provider");
        let subject = unique("existing-mfa-link-subject");
        let mut user = test_user(&format!("{}@example.test", unique("existing-mfa-link")));
        user.email_verified_at = Some(Utc::now());
        user.mfa_enabled = true;
        user.mfa_secret_envelope = Some(b"test-envelope".to_vec());
        PgUserRepo::new(pool.clone())
            .create(&user)
            .await
            .expect("seed MFA user");
        PgExternalIdentityRepo::new(pool.clone())
            .link_external(&user.id, &provider, &subject, Some(&user.email))
            .await
            .expect("seed identity link");
        let command = command(&provider, &subject, None);
        let session_digest = session_token_digest(&command.session.token);
        let challenge_hash = command.mfa_challenge.token_hash;

        let outcome = PgOAuthLoginFinalizer::new(pool.clone())
            .finalize(command)
            .await
            .expect("finalize existing MFA link");

        assert!(matches!(outcome, OAuthLoginFinalizeOutcome::MfaRequired));
        let session_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
                .bind(session_digest.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count sessions");
        assert_eq!(session_count, 0);
        let challenge: (Vec<u8>, Vec<u8>, String, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
            "SELECT token_hash, user_id, kind, consumed_at \
                 FROM verification_tokens WHERE token_hash = $1",
        )
        .bind(challenge_hash.as_slice())
        .fetch_one(&pool)
        .await
        .expect("load MFA challenge");
        assert_eq!(challenge.0, challenge_hash);
        assert_eq!(challenge.1, user.id);
        assert_eq!(challenge.2, "mfa_challenge");
        assert!(challenge.3.is_none());
    }

    #[tokio::test]
    async fn linked_mfa_user_without_a_secret_rolls_back_every_authority_artifact() {
        let Some(pool) = pool().await else { return };

        for (case, secret) in [("missing", None), ("empty", Some(Vec::new()))] {
            let provider = unique(&format!("invalid-mfa-{case}-provider"));
            let subject = unique(&format!("invalid-mfa-{case}-subject"));
            let mut user = test_user(&format!(
                "{}@example.test",
                unique(&format!("invalid-mfa-{case}"))
            ));
            user.email_verified_at = Some(Utc::now());
            user.mfa_enabled = true;
            user.mfa_secret_envelope = secret;
            PgUserRepo::new(pool.clone())
                .create(&user)
                .await
                .expect("seed invalid MFA user");
            PgExternalIdentityRepo::new(pool.clone())
                .link_external(&user.id, &provider, &subject, Some(&user.email))
                .await
                .expect("seed identity link");
            let command = command(&provider, &subject, None);
            let session_digest = session_token_digest(&command.session.token);
            let challenge_hash = command.mfa_challenge.token_hash;

            let result = PgOAuthLoginFinalizer::new(pool.clone())
                .finalize(command)
                .await;

            assert!(matches!(result, Err(StorageError::Internal(_))));
            let session_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
                    .bind(session_digest.as_bytes().as_slice())
                    .fetch_one(&pool)
                    .await
                    .expect("count sessions");
            assert_eq!(session_count, 0, "{case} secret created a session");
            let challenge_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM verification_tokens WHERE token_hash = $1",
            )
            .bind(challenge_hash.as_slice())
            .fetch_one(&pool)
            .await
            .expect("count challenges");
            assert_eq!(challenge_count, 0, "{case} secret created a challenge");
        }
    }

    #[tokio::test]
    async fn mfa_challenge_hash_collision_fails_without_creating_a_session() {
        let Some(pool) = pool().await else { return };
        let provider = unique("challenge-collision-provider");
        let subject = unique("challenge-collision-subject");
        let mut user = test_user(&format!("{}@example.test", unique("challenge-collision")));
        user.email_verified_at = Some(Utc::now());
        user.mfa_enabled = true;
        user.mfa_secret_envelope = Some(b"test-envelope".to_vec());
        PgUserRepo::new(pool.clone())
            .create(&user)
            .await
            .expect("seed MFA user");
        PgExternalIdentityRepo::new(pool.clone())
            .link_external(&user.id, &provider, &subject, Some(&user.email))
            .await
            .expect("seed identity link");
        let command = command(&provider, &subject, None);
        let session_digest = session_token_digest(&command.session.token);
        let challenge_hash = command.mfa_challenge.token_hash;
        sqlx::query(
            "INSERT INTO verification_tokens \
             (token_hash, user_id, kind, payload, created_at, expires_at, consumed_at) \
             VALUES ($1, $2, 'mfa_challenge', NULL, NOW(), NOW() + INTERVAL '5 minutes', NULL)",
        )
        .bind(challenge_hash.as_slice())
        .bind(&user.id)
        .execute(&pool)
        .await
        .expect("seed colliding challenge hash");

        let result = PgOAuthLoginFinalizer::new(pool.clone())
            .finalize(command)
            .await;

        assert!(matches!(result, Err(StorageError::Internal(_))));
        let session_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
                .bind(session_digest.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count sessions");
        assert_eq!(session_count, 0);
        let challenge_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM verification_tokens WHERE token_hash = $1")
                .bind(challenge_hash.as_slice())
                .fetch_one(&pool)
                .await
                .expect("count challenges");
        assert_eq!(
            challenge_count, 1,
            "collision must not overwrite the existing token"
        );
    }

    #[tokio::test]
    async fn malformed_linked_user_id_is_rejected_before_session_commit() {
        let Some(pool) = pool().await else { return };
        let provider = unique("malformed-link-provider");
        let subject = unique("malformed-link-subject");
        let mut user = test_user(&format!("{}@example.test", unique("malformed-link")));
        user.id.pop();
        user.email_verified_at = Some(Utc::now());
        PgUserRepo::new(pool.clone())
            .create(&user)
            .await
            .expect("seed malformed user");
        PgExternalIdentityRepo::new(pool.clone())
            .link_external(&user.id, &provider, &subject, Some(&user.email))
            .await
            .expect("seed malformed identity link");
        let command = command(&provider, &subject, None);
        let session_digest = session_token_digest(&command.session.token);

        let result = PgOAuthLoginFinalizer::new(pool.clone())
            .finalize(command)
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("malformed linked user id must fail closed"),
        };
        assert_eq!(
            error.to_string(),
            "internal: OAuth login finalization failed"
        );
        let session_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
                .bind(session_digest.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count sessions");
        assert_eq!(session_count, 0);
    }

    #[tokio::test]
    async fn soft_deleted_linked_user_is_not_rebound_by_email() {
        let Some(pool) = pool().await else { return };
        let provider = unique("deleted-link-provider");
        let subject = unique("deleted-link-subject");
        let mut user = test_user(&format!("{}@example.test", unique("deleted-link")));
        user.email_verified_at = Some(Utc::now());
        let users = PgUserRepo::new(pool.clone());
        users.create(&user).await.expect("seed user");
        PgExternalIdentityRepo::new(pool.clone())
            .link_external(&user.id, &provider, &subject, Some(&user.email))
            .await
            .expect("seed identity link");
        users.soft_delete(&user.id).await.expect("soft delete user");
        let command = command(&provider, &subject, Some("replacement@example.test"));
        let session_digest = session_token_digest(&command.session.token);

        let outcome = PgOAuthLoginFinalizer::new(pool.clone())
            .finalize(command)
            .await
            .expect("semantic rejection");
        assert!(matches!(
            outcome,
            OAuthLoginFinalizeOutcome::LinkedUserUnavailable
        ));
        let session_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
                .bind(session_digest.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count sessions");
        assert_eq!(session_count, 0);
        let replacement_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL",
        )
        .bind("replacement@example.test")
        .fetch_one(&pool)
        .await
        .expect("count replacement users");
        assert_eq!(replacement_count, 0);
    }

    #[tokio::test]
    async fn existing_local_email_requires_explicit_linking_without_writes() {
        let Some(pool) = pool().await else { return };
        let provider = unique("unverified-provider");
        let subject = unique("unverified-subject");
        let mut user = test_user(&format!("{}@example.test", unique("existing-email")));
        user.email_verified_at = Some(Utc::now());
        PgUserRepo::new(pool.clone())
            .create(&user)
            .await
            .expect("seed user");
        let command = command(&provider, &subject, Some(&user.email));
        let session_digest = session_token_digest(&command.session.token);

        let outcome = PgOAuthLoginFinalizer::new(pool.clone())
            .finalize(command)
            .await
            .expect("semantic rejection");
        assert!(matches!(
            outcome,
            OAuthLoginFinalizeOutcome::AccountLinkRequired
        ));
        let link_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_identities WHERE provider = $1 AND subject = $2",
        )
        .bind(&provider)
        .bind(&subject)
        .fetch_one(&pool)
        .await
        .expect("count links");
        assert_eq!(link_count, 0);
        let session_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
                .bind(session_digest.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count sessions");
        assert_eq!(session_count, 0);
    }

    #[tokio::test]
    async fn session_failure_rolls_back_new_user_and_identity_link() {
        let Some(pool) = pool().await else { return };
        let provider = unique("rollback-provider");
        let subject = unique("rollback-subject");
        let email = format!("{}@example.test", unique("rollback"));
        let mut seed_user = test_user(&format!("{}@example.test", unique("session-owner")));
        seed_user.email_verified_at = Some(Utc::now());
        PgUserRepo::new(pool.clone())
            .create(&seed_user)
            .await
            .expect("seed session owner");
        let now = Utc::now();
        let duplicate_session_token = random_id();
        PgSessionRepo::new(pool.clone())
            .create(
                &duplicate_session_token,
                &SessionDraft {
                    user_id: seed_user.id.clone(),
                    created_at: now,
                    last_active_at: now,
                    expires_at: now + Duration::hours(1),
                    ip_address: None,
                    user_agent: None,
                    revoked_at: None,
                },
            )
            .await
            .expect("seed colliding session");
        let mut command = command(&provider, &subject, Some(&email));
        let candidate_user_id = command.candidate_user.id.clone();
        command.session.token = duplicate_session_token;

        let result = PgOAuthLoginFinalizer::new(pool.clone())
            .finalize(command)
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("session collision must fail the transaction"),
        };
        assert!(matches!(&error, StorageError::Internal(_)));
        assert_eq!(
            error.to_string(),
            "internal: OAuth login finalization failed"
        );
        let candidate_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
            .bind(&candidate_user_id)
            .fetch_one(&pool)
            .await
            .expect("count candidate user");
        assert_eq!(candidate_count, 0);
        let email_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL",
        )
        .bind(&email)
        .fetch_one(&pool)
        .await
        .expect("count candidate email");
        assert_eq!(email_count, 0);
        let link_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_identities WHERE provider = $1 AND subject = $2",
        )
        .bind(&provider)
        .bind(&subject)
        .fetch_one(&pool)
        .await
        .expect("count links");
        assert_eq!(link_count, 0);
    }
}
