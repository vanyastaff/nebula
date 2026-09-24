//! Refresh-claim conformance for the SQLite deployment backend.
//!
//! Every case runs against a fresh in-memory database whose schema comes from
//! the ordered migration catalog, so the adapter is exercised against the real
//! `credential_refresh_claims` and `credential_sentinel_events` constraints —
//! including the incident-identity index migration 0039 installs and the
//! resolution columns migration 0053 adds.
//!
//! SQLite is its own lease-clock authority: expiry is decided in the database,
//! not by the test process, so this fixture backdates rows rather than moving a
//! clock. `expires_at` is `INTEGER` milliseconds here, which is also the unit
//! `age_incidents` shifts incident rows by.
//!
//! The failure-injection case is real on this backend: a test-only `BEFORE
//! UPDATE` trigger aborts the resolution write after the claim-row delete has
//! already happened, inside the adapter's own transaction. No production code
//! grows a failure seam; the trigger is created and dropped by the fixture.
//!
//! This runner asserts **all 30** shared cases. The in-memory runner declares
//! the failure-injection one skipped, and postgres asserts all 30 only when it
//! can reach a database (see that runner's module doc).

#![cfg(feature = "sqlite")]

#[macro_use]
#[path = "support/refresh_claim_oracle.rs"]
mod oracle;
use oracle::RefreshClaimFixture as _;

use std::{str::FromStr, sync::Mutex, time::Duration};

use chrono::{Duration as ChronoDuration, Utc};
use nebula_storage::credential::refresh_claim::{
    CredentialOperationDecision, CredentialOperationIntent, RefreshAdjudication,
    RefreshClaimAdjudicationError, RefreshClaimAdjudicator,
};
use nebula_storage::credential::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, ReauthEscalation,
    RefreshClaimReclaimer, RefreshClaimRepo, ReplicaId, RepoError, SentinelEscalationPolicy,
    SqliteRefreshClaimRepo,
};
use nebula_storage_port::CredentialSelector;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{AssertSqlSafe, SqlitePool};

/// An isolated in-memory database with the ordered migration catalog applied.
///
/// The shared cache keeps every pooled connection on the same database, and the
/// pool holds more than one connection because the concurrency cases mean to
/// contend at the SQL layer rather than at the pool.
async fn fresh_pool() -> SqlitePool {
    let database = format!("nebula-claim-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{database}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("in-memory SQLite URL must parse")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect to in-memory SQLite");
    nebula_storage::sqlite::init_schema(&pool)
        .await
        .expect("apply the ordered SQLite migration catalog");
    pool
}

/// The SQLite adapter plus the two seams a shared case cannot reach through the
/// port: backdating a row, and failing a resolution write.
struct SqliteRefreshClaimFixture {
    repo: SqliteRefreshClaimRepo,
    pool: SqlitePool,
    namespace: String,
    /// Trigger names this fixture installed and has not dropped yet, one per
    /// credential it was asked to fail a resolution write for.
    injected: Mutex<Vec<String>>,
}

impl SqliteRefreshClaimFixture {
    async fn new() -> Self {
        let pool = fresh_pool().await;
        Self {
            repo: SqliteRefreshClaimRepo::new(pool.clone()),
            pool,
            namespace: uuid::Uuid::new_v4().simple().to_string(),
            injected: Mutex::new(Vec::new()),
        }
    }

    /// A trigger name for `credential`, which the caller has already checked is
    /// identifier-safe.
    fn trigger_name(&self, credential: &CredentialSelector) -> String {
        format!("refresh_claim_inject_{}", credential.credential_id())
    }
}

/// The identifier-safe spelling of `credential`, or a panic naming what made it
/// unsafe.
///
/// Trigger names and trigger bodies are both rendered by interpolation, so the
/// credential is validated rather than escaped: `CredentialSelector`'s `Display` is
/// Crockford base32 behind a `cred_` prefix, and this fails loudly if that ever
/// stops being true instead of interpolating something a SQL parser reads
/// differently than this test intends.
fn sql_literal(credential: &CredentialSelector) -> String {
    let rendered = credential.credential_id().to_string();
    assert!(
        rendered
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_'),
        "a credential id must be identifier-safe to be interpolated"
    );
    rendered
}

#[async_trait::async_trait]
impl RefreshClaimRepo for SqliteRefreshClaimFixture {
    async fn try_claim(
        &self,
        selector: &CredentialSelector,
        holder: &ReplicaId,
        ttl: Duration,
        intent: CredentialOperationIntent,
    ) -> Result<ClaimAttempt, RepoError> {
        sqlx::query(
            "INSERT OR IGNORE INTO credentials (id, owner_id, credential_key, state_kind, \
             state_version, data, version, material_epoch, created_at, updated_at, \
             reauth_required, metadata, record_state) \
             VALUES (?1, ?2, 'test.key', 'test.state', 1, X'00', 1, 1, ?3, ?3, 0, '{}', 'live')",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .bind(Utc::now().timestamp_millis())
        .execute(&self.pool)
        .await
        .map_err(|_| RepoError::Storage)?;
        self.repo.try_claim(selector, holder, ttl, intent).await
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        self.repo.heartbeat(token, ttl).await
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        self.repo.release(token).await
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        self.repo.mark_sentinel(token).await
    }
}

#[async_trait::async_trait]
impl RefreshClaimReclaimer for SqliteRefreshClaimFixture {
    async fn reclaim_stuck(
        &self,
        policy: SentinelEscalationPolicy,
    ) -> Result<Vec<ExpiredClaim>, RepoError> {
        self.repo.reclaim_stuck(policy).await
    }
}

#[async_trait::async_trait]
impl RefreshClaimAdjudicator for SqliteRefreshClaimFixture {
    async fn adjudicate(
        &self,
        selector: &CredentialSelector,
        decision: CredentialOperationDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, RefreshClaimAdjudicationError> {
        self.repo.adjudicate(selector, decision, evidence).await
    }
}

#[async_trait::async_trait]
impl oracle::RefreshClaimFixture for SqliteRefreshClaimFixture {
    fn credential(&self, case: &str) -> CredentialSelector {
        oracle::case_credential(&self.namespace, case)
    }

    fn replica(&self, seed: u8) -> ReplicaId {
        ReplicaId::new(format!("replica-{seed:02x}"))
    }

    fn claim_ttl(&self) -> Duration {
        // The production lease. Nothing has to wait for it: `expire_claims`
        // puts the row past its deadline in the database, and every predicate
        // the adapter applies compares against the database's own clock.
        Duration::from_secs(30)
    }

    async fn expire_claims(&self, credential: &CredentialSelector) {
        let expired_at = (Utc::now() - ChronoDuration::seconds(1)).timestamp_millis();
        sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = ?1 \
             WHERE owner_id = ?2 AND credential_id = ?3",
        )
        .bind(expired_at)
        .bind(credential.owner().as_str())
        .bind(credential.credential_id().to_string())
        .execute(&self.pool)
        .await
        .expect("backdating the claim row must not fail");
    }

    async fn seed_unresolved_incidents(&self, credential: &CredentialSelector, count: u32) {
        oracle::replay_poisoned_lifecycles(self, credential, count).await;
    }

    async fn incident_count(&self, credential: &CredentialSelector) -> u64 {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM credential_sentinel_events \
             WHERE owner_id = ?1 AND credential_id = ?2",
        )
        .bind(credential.owner().as_str())
        .bind(credential.credential_id().to_string())
        .fetch_one(&self.pool)
        .await
        .expect("the sentinel count must be readable");
        u64::try_from(count).expect("a row count is not negative")
    }

    async fn count_sentinel_events_in_window(
        &self,
        credential: &CredentialSelector,
        window: Duration,
    ) -> Result<u32, RepoError> {
        if window.is_zero() {
            return Ok(0);
        }
        let window_ms = i64::try_from(window.as_millis()).map_err(|_| RepoError::InvalidState)?;
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM credential_sentinel_events \
             WHERE owner_id = ?1 AND credential_id = ?2 \
               AND detected_at > (unixepoch('now') * 1000 \
                 + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER) - ?3)",
        )
        .bind(credential.owner().as_str())
        .bind(credential.credential_id().to_string())
        .bind(window_ms)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| RepoError::Storage)?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    async fn resolved_incident_count(&self, credential: &CredentialSelector) -> u64 {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM credential_sentinel_events \
             WHERE owner_id = ?1 AND credential_id = ?2 AND adjudicated_at IS NOT NULL",
        )
        .bind(credential.owner().as_str())
        .bind(credential.credential_id().to_string())
        .fetch_one(&self.pool)
        .await
        .expect("counting decisions must not fail");
        u64::try_from(count).expect("a row count is not negative")
    }

    async fn age_incidents(&self, credential: &CredentialSelector, by: Duration) {
        let by_ms = i64::try_from(by.as_millis()).expect("an age fits an i64");
        sqlx::query(
            "UPDATE credential_sentinel_events \
             SET detected_at = detected_at - ?1 \
             WHERE owner_id = ?2 AND credential_id = ?3",
        )
        .bind(by_ms)
        .bind(credential.owner().as_str())
        .bind(credential.credential_id().to_string())
        .execute(&self.pool)
        .await
        .expect("aging an incident must not fail");
    }

    async fn inject_decision_write_failure(&self, credential: &CredentialSelector) {
        let name = self.trigger_name(credential);
        // Interpolation is safe: the trigger name is built from a validated
        // credential, and the body's only other literal is that same value.
        let create = format!(
            "CREATE TRIGGER {name} \
             BEFORE UPDATE ON credential_sentinel_events \
             WHEN OLD.adjudicated_at IS NULL \
               AND NEW.adjudicated_at IS NOT NULL \
               AND OLD.credential_id = '{}' \
             BEGIN \
                 SELECT RAISE(ABORT, 'injected resolution write failure'); \
             END",
            sql_literal(credential)
        );
        sqlx::query(AssertSqlSafe(format!("DROP TRIGGER IF EXISTS {name}")))
            .execute(&self.pool)
            .await
            .expect("dropping a stale trigger must not fail");
        sqlx::query(AssertSqlSafe(create))
            .execute(&self.pool)
            .await
            .expect("installing the failure trigger must not fail");
        self.injected
            .lock()
            .expect("the injected-trigger list is never poisoned")
            .push(name);
    }

    async fn clear_decision_write_failure(&self, credential: &CredentialSelector) {
        // The named trigger goes first so the credential argument cannot be
        // ignored; the retained list is what proves one was installed.
        let name = self.trigger_name(credential);
        self.injected
            .lock()
            .expect("the injected-trigger list is never poisoned")
            .retain(|installed| installed != &name);
        sqlx::query(AssertSqlSafe(format!("DROP TRIGGER IF EXISTS {name}")))
            .execute(&self.pool)
            .await
            .expect("dropping the failure trigger must not fail");
    }

    async fn poisoned_claim_exists(&self, credential: &CredentialSelector) -> bool {
        // Character for character the predicate the adapter answers
        // `OutcomeUnknown` with: an expired `sentinel = 1` row, compared
        // against the same millisecond clock the adapter binds.
        let (exists,): (i64,) = sqlx::query_as(
            "SELECT EXISTS ( \
                 SELECT 1 FROM credential_refresh_claims \
                 WHERE owner_id = ?1 AND credential_id = ?2 \
                   AND expires_at < ?3 AND sentinel = 1 \
             )",
        )
        .bind(credential.owner().as_str())
        .bind(credential.credential_id().to_string())
        .bind(Utc::now().timestamp_millis())
        .fetch_one(&self.pool)
        .await
        .expect("reading the poison predicate must not fail");
        exists != 0
    }
}

async fn fixture() -> Option<SqliteRefreshClaimFixture> {
    Some(SqliteRefreshClaimFixture::new().await)
}

refresh_claim_conformance_suite!(fixture());

refresh_claim_case!(
    a_failed_decision_write_leaves_the_claim_poisoned_and_unrecorded,
    0x2D,
    fixture()
);

#[tokio::test]
async fn threshold_incident_atomically_advances_reauth_authority_once() {
    let fixture = SqliteRefreshClaimFixture::new().await;
    let selector =
        fixture.credential("threshold_incident_atomically_advances_reauth_authority_once");
    let claim = match fixture
        .try_refresh_claim(&selector, &fixture.replica(1), fixture.claim_ttl())
        .await
        .expect("claim")
    {
        ClaimAttempt::Acquired(claim) => claim,
        other => panic!("fresh aggregate must be claimable: {other:?}"),
    };
    fixture
        .mark_sentinel(&claim.token)
        .await
        .expect("mark egress");
    fixture.expire_claims(&selector).await;

    let policy =
        SentinelEscalationPolicy::new(1, Duration::from_hours(1)).expect("valid threshold policy");
    let outcomes = fixture.reclaim_stuck(policy).await.expect("atomic reclaim");
    assert!(matches!(
        outcomes.as_slice(),
        [ExpiredClaim::OutcomeUnknownAccounted {
            event_count: 1,
            escalation: ReauthEscalation::ReauthRequired {
                changed: true,
                version,
                material_epoch,
            },
            ..
        }] if version.get() == 2 && material_epoch.get() == 2
    ));

    type CredentialAuthorityRow = (
        i64,
        i64,
        i64,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: CredentialAuthorityRow = sqlx::query_as(
        "SELECT reauth_required, version, material_epoch, refresh_retry_mode, \
             refresh_retry_not_before, refresh_retry_phase, refresh_retry_kind, \
             refresh_retry_diagnostic_code FROM credentials \
             WHERE owner_id = ?1 AND id = ?2",
    )
    .bind(selector.owner().as_str())
    .bind(selector.credential_id().to_string())
    .fetch_one(&fixture.pool)
    .await
    .expect("durable credential state");
    assert_eq!(row, (1, 2, 2, None, None, None, None, None));
    assert!(
        fixture
            .reclaim_stuck(policy)
            .await
            .expect("idempotent sweep")
            .is_empty()
    );
}
