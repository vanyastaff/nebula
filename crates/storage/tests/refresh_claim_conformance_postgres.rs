//! Refresh-claim conformance for the PostgreSQL deployment backend.
//!
//! PostgreSQL is a deployment backend, so its absence is a job failure, never a
//! silent substitution. With `NEBULA_REQUIRE_POSTGRES=1` and no `DATABASE_URL`,
//! every case fails; without it a developer without a database still sees every
//! case fail loudly naming the unreachable backend. A green run of this runner
//! therefore always means the cases ran against a live database — a run that
//! asserts nothing cannot look green.
//!
//! PostgreSQL is also the lease-clock authority — acquisition, heartbeat,
//! sentinel admission, and reclaim all compare against `CURRENT_TIMESTAMP` — so
//! this fixture backdates rows instead of moving a clock, exactly as the SQLite
//! one does.
//!
//! # Why this fixture holds a session lock for its whole life
//!
//! `reclaim_stuck` is a global sweep: it walks every expired claim row in the
//! database. Cases share one database, and nextest runs one case per process,
//! so a case that sweeps while another case is mid-lifecycle can account that
//! case's poison before its own sweep sees it — and the case being interfered
//! with would fail on an assertion about *its* accounting, not on the behaviour
//! it names. Identity (a credential per case per run) keeps cases from meeting
//! each other's rows; it cannot keep a global sweep off them. A session
//! advisory lock held for the fixture's lifetime is what serializes the cases,
//! so each one sweeps in a database where no other case has an expired row.
//! Within a case the lock is not used again, so the two concurrency cases still
//! contend at the SQL layer.
//!
//! The failure-injection case is real on this backend: a test-only `BEFORE
//! UPDATE` trigger raises an exception inside the adapter's own transaction,
//! which aborts it, so the claim-row delete rolls back with the resolution
//! write. No production code grows a failure seam.
//!
//! This runner asserts **all 30** shared cases when the backend is reachable.
//! Without a database every case fails naming the backend, so the denominator
//! can never hold green cases that asserted nothing.

#![cfg(feature = "postgres")]

#[macro_use]
#[path = "support/refresh_claim_oracle.rs"]
mod oracle;

use std::{sync::Mutex, time::Duration};

use nebula_core::CredentialId;
use nebula_storage::credential::refresh_claim::{
    RefreshAdjudication, RefreshClaimAdjudicationError, RefreshClaimAdjudicator,
    RefreshOutcomeDecision,
};
use nebula_storage::credential::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, PgRefreshClaimRepo, RefreshClaimRepo,
    ReplicaId, RepoError,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool, Postgres, pool::PoolConnection};
use tokio::sync::OnceCell;

static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// The reclaim-test advisory lock key, shared with every process that sweeps.
const RECLAIM_TEST_LOCK_KEY: i64 = 0x4E42_5246_434C_414D;

/// Connect to `DATABASE_URL` and apply the ordered migration catalog, or return
/// `None`, which every case turns into a loud failure naming the backend.
///
/// Cases share one database; the fixture keeps them independent by identity and
/// by the session lock below.
async fn pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1: \
                 PostgreSQL is a deployment backend and is never substituted"
            );
            return None;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    SCHEMA_READY
        .get_or_init(|| async {
            nebula_storage::postgres::init_schema(&pool)
                .await
                .expect("apply the ordered PostgreSQL migration catalog");
        })
        .await;
    Some(pool)
}

/// Serialize the global reclaim sweep across cases, which may be separate
/// processes.
///
/// `close_on_drop` guarantees a panicking case retires its session instead of
/// returning a still-locked connection to the pool.
async fn acquire_reclaim_test_lock(pool: &PgPool) -> PoolConnection<Postgres> {
    let mut connection = pool
        .acquire()
        .await
        .expect("acquire the reclaim-test connection");
    connection.close_on_drop();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(RECLAIM_TEST_LOCK_KEY)
        .execute(&mut *connection)
        .await
        .expect("acquire the reclaim-test advisory lock");
    connection
}

/// The PostgreSQL adapter plus the reclaim-test lock and the two seams a shared
/// case cannot reach through the port.
struct PgRefreshClaimFixture {
    repo: PgRefreshClaimRepo,
    pool: PgPool,
    namespace: String,
    /// Trigger names this fixture installed and has not dropped yet.
    injected: Mutex<Vec<String>>,
    /// Held for the fixture's lifetime, and only for that: it makes each case's
    /// global sweep exclusive, and it is dropped before the fixture is.
    #[expect(
        dead_code,
        reason = "the lock is held for its drop, never read by a case"
    )]
    reclaim_lock: PoolConnection<Postgres>,
}

impl PgRefreshClaimFixture {
    async fn new(pool: PgPool) -> Self {
        let reclaim_lock = acquire_reclaim_test_lock(&pool).await;
        Self {
            repo: PgRefreshClaimRepo::new(pool.clone()),
            pool,
            namespace: uuid::Uuid::new_v4().simple().to_string(),
            injected: Mutex::new(Vec::new()),
            reclaim_lock,
        }
    }

    /// The names this fixture's injection uses for `credential`, which the
    /// caller has already checked is identifier-safe.
    fn injection_names(&self, credential: &CredentialId) -> (String, String) {
        (
            format!("refresh_claim_reject_{credential}"),
            format!("refresh_claim_inject_{credential}"),
        )
    }
}

/// The identifier-safe spelling of `credential`, or a panic naming what made it
/// unsafe.
///
/// Identifiers here are rendered by interpolation, so the credential is
/// validated rather than escaped: `CredentialId`'s `Display` is Crockford
/// base32 behind a `cred_` prefix, and this fails loudly if that ever stops
/// being true instead of interpolating something a SQL parser reads differently
/// than this test intends.
fn sql_identifier(credential: &CredentialId) -> String {
    let rendered = credential.to_string();
    assert!(
        rendered
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_'),
        "a credential id must be identifier-safe to be interpolated"
    );
    rendered
}

#[async_trait::async_trait]
impl RefreshClaimRepo for PgRefreshClaimFixture {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        self.repo.try_claim(credential_id, holder, ttl).await
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

    async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RepoError> {
        self.repo.reclaim_stuck().await
    }

    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window: Duration,
    ) -> Result<u32, RepoError> {
        self.repo
            .count_sentinel_events_in_window(credential_id, window)
            .await
    }
}

#[async_trait::async_trait]
impl RefreshClaimAdjudicator for PgRefreshClaimFixture {
    async fn adjudicate(
        &self,
        credential_id: &CredentialId,
        decision: RefreshOutcomeDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, RefreshClaimAdjudicationError> {
        self.repo
            .adjudicate(credential_id, decision, evidence)
            .await
    }
}

#[async_trait::async_trait]
impl oracle::RefreshClaimFixture for PgRefreshClaimFixture {
    fn credential(&self, case: &str) -> CredentialId {
        oracle::case_credential(&self.namespace, case)
    }

    fn replica(&self, seed: u8) -> ReplicaId {
        ReplicaId::new(format!("replica-{seed:02x}"))
    }

    fn claim_ttl(&self) -> Duration {
        // The production lease. Nothing waits for it: `expire_claims` puts the
        // row past its deadline in the database, and every predicate the
        // adapter applies compares against the database's own clock.
        Duration::from_secs(30)
    }

    async fn expire_claims(&self, credential: &CredentialId) {
        sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = CURRENT_TIMESTAMP - INTERVAL '1 second' \
             WHERE credential_id = $1",
        )
        .bind(credential.to_string())
        .execute(&self.pool)
        .await
        .expect("backdating the claim row must not fail");
    }

    async fn seed_unresolved_incidents(&self, credential: &CredentialId, count: u32) {
        oracle::replay_poisoned_lifecycles(self, credential, count).await;
    }

    async fn incident_count(&self, credential: &CredentialId) -> u64 {
        u64::from(
            self.count_sentinel_events_in_window(credential, oracle::INCIDENT_WINDOW)
                .await
                .expect("the sentinel count must be readable"),
        )
    }

    async fn resolved_incident_count(&self, credential: &CredentialId) -> u64 {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM credential_sentinel_events \
             WHERE credential_id = $1 AND adjudicated_at IS NOT NULL",
        )
        .bind(credential.to_string())
        .fetch_one(&self.pool)
        .await
        .expect("counting decisions must not fail");
        u64::try_from(count).expect("a row count is not negative")
    }

    async fn age_incidents(&self, credential: &CredentialId, by: Duration) {
        let by_micros = i64::try_from(by.as_micros()).expect("an age fits an i64");
        sqlx::query(
            "UPDATE credential_sentinel_events \
             SET detected_at = detected_at - ($2::bigint * INTERVAL '1 microsecond') \
             WHERE credential_id = $1",
        )
        .bind(credential.to_string())
        .bind(by_micros)
        .execute(&self.pool)
        .await
        .expect("aging an incident must not fail");
    }

    async fn inject_decision_write_failure(&self, credential: &CredentialId) {
        let (function, trigger) = self.injection_names(credential);
        let identifier = sql_identifier(credential);
        // Interpolation is safe: both names are built from a validated
        // credential, and the body's only literal is that same value.
        let body = format!(
            "CREATE OR REPLACE FUNCTION {function}() RETURNS trigger AS $$ \
             BEGIN \
                 RAISE EXCEPTION 'injected resolution write failure'; \
             END; \
             $$ LANGUAGE plpgsql"
        );
        let attach = format!(
            "CREATE TRIGGER {trigger} \
             BEFORE UPDATE ON credential_sentinel_events \
             FOR EACH ROW \
             WHEN ( \
                 OLD.adjudicated_at IS NULL \
                 AND NEW.adjudicated_at IS NOT NULL \
                 AND OLD.credential_id = '{identifier}' \
             ) \
             EXECUTE FUNCTION {function}()"
        );
        // A stale half-installed injection must not survive a retry of this
        // case, so the installation starts from the cleared state.
        oracle::RefreshClaimFixture::clear_decision_write_failure(self, credential).await;
        sqlx::query(AssertSqlSafe(body))
            .execute(&self.pool)
            .await
            .expect("installing the failure function must not fail");
        sqlx::query(AssertSqlSafe(attach))
            .execute(&self.pool)
            .await
            .expect("installing the failure trigger must not fail");
        self.injected
            .lock()
            .expect("the injected-trigger list is never poisoned")
            .push(trigger);
    }

    async fn clear_decision_write_failure(&self, credential: &CredentialId) {
        let (function, trigger) = self.injection_names(credential);
        // The trigger goes first: the function cannot be dropped while a
        // trigger still depends on it.
        sqlx::query(AssertSqlSafe(format!(
            "DROP TRIGGER IF EXISTS {trigger} ON credential_sentinel_events"
        )))
        .execute(&self.pool)
        .await
        .expect("dropping the failure trigger must not fail");
        sqlx::query(AssertSqlSafe(format!(
            "DROP FUNCTION IF EXISTS {function}()"
        )))
        .execute(&self.pool)
        .await
        .expect("dropping the failure function must not fail");
        self.injected
            .lock()
            .expect("the injected-trigger list is never poisoned")
            .retain(|installed| installed != &trigger);
    }

    async fn poisoned_claim_exists(&self, credential: &CredentialId) -> bool {
        // The predicate the adapter answers `OutcomeUnknown` with: an expired
        // `sentinel = 1` row, compared against the database clock the adapter
        // compares against.
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS ( \
                 SELECT 1 FROM credential_refresh_claims \
                 WHERE credential_id = $1 \
                   AND expires_at < CURRENT_TIMESTAMP \
                   AND sentinel = 1 \
             )",
        )
        .bind(credential.to_string())
        .fetch_one(&self.pool)
        .await
        .expect("reading the poison predicate must not fail");
        exists
    }
}

async fn fixture() -> Option<PgRefreshClaimFixture> {
    let pool = pool().await?;
    Some(PgRefreshClaimFixture::new(pool).await)
}

refresh_claim_conformance_suite!(fixture());

refresh_claim_case!(
    a_failed_decision_write_leaves_the_claim_poisoned_and_unrecorded,
    0x2D,
    fixture()
);
