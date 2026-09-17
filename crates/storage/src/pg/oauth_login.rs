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
#[path = "oauth_login_tests.rs"]
mod tests;
