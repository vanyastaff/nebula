//! Deterministic shared-cache lock coverage for SQLite database discovery.

use std::str::FromStr as _;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use super::{
    ClassifiedAttempt, SQLITE_DATABASE_DISCOVERY_RETRY, SQLITE_MIGRATOR,
    SqliteDatabaseDiscoveryAttempt, SqliteDatabaseDiscoveryProbe, SqliteDatabaseDiscoveryRetry,
    SqliteDatabaseRows, UnobservedSqliteDatabaseDiscovery, catalog,
    retry_sqlite_database_discovery, setup_sqlite_pool_with_discovery_probe,
};

#[derive(Debug, PartialEq, Eq)]
enum DiscoveryObservation {
    Success,
    DatabaseError { numeric_code: Option<u32> },
}

struct ControlledDiscovery {
    query_arrived: Arc<tokio::sync::Barrier>,
    query_released: Arc<tokio::sync::Barrier>,
    observation_sender: tokio::sync::mpsc::UnboundedSender<DiscoveryObservation>,
    has_observed_first_result: AtomicBool,
    retry_wait_started: tokio::sync::Notify,
    query_attempt_sender: tokio::sync::watch::Sender<u32>,
}

impl ControlledDiscovery {
    fn new() -> (
        Self,
        tokio::sync::mpsc::UnboundedReceiver<DiscoveryObservation>,
        tokio::sync::watch::Receiver<u32>,
    ) {
        let (observation_sender, observation_receiver) = tokio::sync::mpsc::unbounded_channel();
        let (query_attempt_sender, query_attempt_receiver) = tokio::sync::watch::channel(0);
        (
            Self {
                query_arrived: Arc::new(tokio::sync::Barrier::new(2)),
                query_released: Arc::new(tokio::sync::Barrier::new(2)),
                observation_sender,
                has_observed_first_result: AtomicBool::new(false),
                retry_wait_started: tokio::sync::Notify::new(),
                query_attempt_sender,
            },
            observation_receiver,
            query_attempt_receiver,
        )
    }
}

impl SqliteDatabaseDiscoveryProbe for ControlledDiscovery {
    async fn before_first_attempt(&self) {
        self.query_arrived.wait().await;
        self.query_released.wait().await;
    }

    fn observe_attempt(&self, result: &ClassifiedAttempt<SqliteDatabaseRows>) {
        let observed_attempts = (*self.query_attempt_sender.borrow()).saturating_add(1);
        self.query_attempt_sender.send_replace(observed_attempts);
        if self.has_observed_first_result.swap(true, Ordering::Relaxed) {
            return;
        }
        let observation = match result {
            ClassifiedAttempt::Success(_) => DiscoveryObservation::Success,
            ClassifiedAttempt::Transient { numeric_code }
            | ClassifiedAttempt::NonRetryable { numeric_code } => {
                DiscoveryObservation::DatabaseError {
                    numeric_code: *numeric_code,
                }
            },
        };
        let _receiver_may_finish_after_first_observation =
            self.observation_sender.send(observation);
    }

    fn before_retry_wait(&self, _attempts: u32) {
        self.retry_wait_started.notify_one();
    }
}

async fn controlled_shared_memory_pool(
    fixture_name: &str,
) -> (sqlx::SqlitePool, sqlx::pool::PoolConnection<sqlx::Sqlite>) {
    let database_name = format!("nebula-{fixture_name}-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{database_name}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("controlled SQLite URL must be valid")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("controlled shared-memory pool must open");
    let blocker = pool
        .acquire()
        .await
        .expect("first pool slot must be reserved for the controlled transaction");
    let discovery_connection = pool
        .acquire()
        .await
        .expect("second pool slot must be opened before the controlled race");
    drop(discovery_connection);
    (pool, blocker)
}

#[derive(Debug, Clone, Copy)]
enum ControlledTransaction {
    PendingSchema,
    Committed,
    ReadOnly,
}

async fn observe_controlled_setup(
    pool: &sqlx::SqlitePool,
    discovery: &ControlledDiscovery,
    mut observation_receiver: tokio::sync::mpsc::UnboundedReceiver<DiscoveryObservation>,
    mut blocker: sqlx::pool::PoolConnection<sqlx::Sqlite>,
    controlled_transaction: ControlledTransaction,
    retry: SqliteDatabaseDiscoveryRetry,
) -> (Result<(), catalog::CatalogSetupError>, DiscoveryObservation) {
    let setup_finished = Arc::new(tokio::sync::Notify::new());
    let setup_finished_signal = Arc::clone(&setup_finished);
    let setup = async {
        let result = setup_sqlite_pool_with_discovery_probe(pool.clone(), discovery, retry).await;
        setup_finished_signal.notify_one();
        result
    };
    let control = async {
        discovery.query_arrived.wait().await;
        sqlx::query("BEGIN")
            .execute(&mut *blocker)
            .await
            .expect("controlled transaction must begin");
        match controlled_transaction {
            ControlledTransaction::PendingSchema => {
                sqlx::query("CREATE TABLE discovery_lock (value INTEGER NOT NULL)")
                    .execute(&mut *blocker)
                    .await
                    .expect("schema transaction must acquire the main schema lock");
            },
            ControlledTransaction::Committed => {
                sqlx::query("CREATE TABLE discovery_lock (value INTEGER NOT NULL)")
                    .execute(&mut *blocker)
                    .await
                    .expect("schema transaction must acquire the main schema lock");
                sqlx::query("DROP TABLE discovery_lock")
                    .execute(&mut *blocker)
                    .await
                    .expect("committed control must leave the database empty");
                sqlx::query("COMMIT")
                    .execute(&mut *blocker)
                    .await
                    .expect("committed control must release its schema lock");
            },
            ControlledTransaction::ReadOnly => {
                let observed: i64 = sqlx::query_scalar("SELECT 1")
                    .fetch_one(&mut *blocker)
                    .await
                    .expect("non-schema control transaction must remain readable");
                assert_eq!(observed, 1);
            },
        }
        discovery.query_released.wait().await;
        let observation = tokio::select! {
            observation = observation_receiver.recv() => {
                observation.expect("controlled query must publish one observation")
            },
            () = setup_finished.notified() => {
                panic!("setup completed before database discovery published its first result");
            },
        };
        if !matches!(controlled_transaction, ControlledTransaction::Committed) {
            sqlx::query("ROLLBACK")
                .execute(&mut *blocker)
                .await
                .expect("controlled transaction must release its lock independently");
        }
        drop(blocker);
        observation
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(setup, control)
    })
    .await
    .expect("controlled discovery must complete without scheduler or pool starvation")
}

/// An already-open connection can acquire SQLite's shared-cache schema lock
/// after setup acquires its own connection. Setup must wait on that same
/// connection and reach the canonical catalog after the peer releases it.
#[tokio::test]
async fn database_discovery_waits_for_an_uncommitted_schema_transaction() {
    let (pool, blocker) = controlled_shared_memory_pool("schema-lock").await;
    let (discovery, observation_receiver, _query_attempt_receiver) = ControlledDiscovery::new();

    let (setup_result, observation) = observe_controlled_setup(
        &pool,
        &discovery,
        observation_receiver,
        blocker,
        ControlledTransaction::PendingSchema,
        SQLITE_DATABASE_DISCOVERY_RETRY,
    )
    .await;

    assert_eq!(
        observation,
        DiscoveryObservation::DatabaseError {
            numeric_code: Some(262),
        },
        "the controlled failure must be SQLITE_LOCKED_SHAREDCACHE, not a guessed lock class"
    );
    assert!(
        setup_result.is_ok(),
        "a transient schema lock must not fail setup after the peer releases it; \
         observed {observation:?} at main_database_discovery, result: {setup_result:?}"
    );
    let observed_head: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&pool)
            .await
            .expect("the recovered setup must leave a readable migration ledger");
    assert_eq!(observed_head, Some(catalog::catalog_head(&SQLITE_MIGRATOR)));
}

/// Committing the same schema transaction before discovery is the positive
/// control: no lock remains and setup reaches the canonical catalog head.
#[tokio::test]
async fn database_discovery_accepts_a_committed_schema_transaction() {
    let (pool, blocker) = controlled_shared_memory_pool("committed-schema").await;
    let (discovery, observation_receiver, _query_attempt_receiver) = ControlledDiscovery::new();

    let (setup_result, observation) = observe_controlled_setup(
        &pool,
        &discovery,
        observation_receiver,
        blocker,
        ControlledTransaction::Committed,
        SQLITE_DATABASE_DISCOVERY_RETRY,
    )
    .await;

    assert_eq!(observation, DiscoveryObservation::Success);
    assert!(setup_result.is_ok(), "setup failed: {setup_result:?}");
}

/// A live transaction alone does not cause the failure: a transaction that
/// has not changed schema remains compatible with database-list discovery.
#[tokio::test]
async fn database_discovery_accepts_a_non_schema_transaction() {
    let (pool, blocker) = controlled_shared_memory_pool("non-schema").await;
    let (discovery, observation_receiver, _query_attempt_receiver) = ControlledDiscovery::new();

    let (setup_result, observation) = observe_controlled_setup(
        &pool,
        &discovery,
        observation_receiver,
        blocker,
        ControlledTransaction::ReadOnly,
        SQLITE_DATABASE_DISCOVERY_RETRY,
    )
    .await;

    assert_eq!(observation, DiscoveryObservation::Success);
    assert!(setup_result.is_ok(), "setup failed: {setup_result:?}");
}

struct ImmediateTransientAttempt {
    attempt_sender: tokio::sync::watch::Sender<u32>,
}

impl SqliteDatabaseDiscoveryAttempt for ImmediateTransientAttempt {
    async fn run(&mut self) -> ClassifiedAttempt<SqliteDatabaseRows> {
        let attempts = (*self.attempt_sender.borrow()).saturating_add(1);
        self.attempt_sender.send_replace(attempts);
        ClassifiedAttempt::Transient {
            numeric_code: Some(262),
        }
    }
}

struct PendingAttempt {
    attempt_sender: tokio::sync::watch::Sender<u32>,
}

struct TransientThenSuccessAttempt {
    attempt_sender: tokio::sync::watch::Sender<u32>,
}

impl SqliteDatabaseDiscoveryAttempt for TransientThenSuccessAttempt {
    async fn run(&mut self) -> ClassifiedAttempt<SqliteDatabaseRows> {
        let attempts = (*self.attempt_sender.borrow()).saturating_add(1);
        self.attempt_sender.send_replace(attempts);
        if attempts == 1 {
            ClassifiedAttempt::Transient {
                numeric_code: Some(262),
            }
        } else {
            ClassifiedAttempt::Success(Vec::new())
        }
    }
}

impl SqliteDatabaseDiscoveryAttempt for PendingAttempt {
    async fn run(&mut self) -> ClassifiedAttempt<SqliteDatabaseRows> {
        let attempts = (*self.attempt_sender.borrow()).saturating_add(1);
        self.attempt_sender.send_replace(attempts);
        std::future::pending().await
    }
}

struct ImmediateNonRetryableAttempt {
    attempts: u32,
}

impl SqliteDatabaseDiscoveryAttempt for ImmediateNonRetryableAttempt {
    async fn run(&mut self) -> ClassifiedAttempt<SqliteDatabaseRows> {
        self.attempts = self.attempts.saturating_add(1);
        ClassifiedAttempt::NonRetryable {
            numeric_code: Some(1),
        }
    }
}

struct RetryWaitProbe {
    retry_wait_started: Arc<tokio::sync::Notify>,
}

impl SqliteDatabaseDiscoveryProbe for RetryWaitProbe {
    fn before_retry_wait(&self, _attempts: u32) {
        self.retry_wait_started.notify_one();
    }
}

fn assert_unavailable(result: Result<SqliteDatabaseRows, catalog::CatalogSetupError>) {
    match result {
        Err(catalog::CatalogSetupError::Unavailable) => {},
        Err(error) => panic!("database discovery returned the wrong error: {error:?}"),
        Ok(databases) => panic!(
            "database discovery unexpectedly succeeded with {} database rows",
            databases.len()
        ),
    }
}

/// Backoff and every later attempt share the original budget rather than
/// renewing it after a transient lock.
#[tokio::test(start_paused = true)]
async fn database_discovery_expires_one_fixed_deadline_during_backoff() {
    let (attempt_sender, mut attempt_receiver) = tokio::sync::watch::channel(0_u32);
    let retry_wait_started = Arc::new(tokio::sync::Notify::new());
    let retry_probe = RetryWaitProbe {
        retry_wait_started: Arc::clone(&retry_wait_started),
    };
    let retry = SqliteDatabaseDiscoveryRetry {
        timeout: Duration::from_millis(50),
        poll_interval: Duration::from_millis(10),
    };
    let discovery = tokio::spawn(async move {
        let mut attempt = ImmediateTransientAttempt { attempt_sender };
        retry_sqlite_database_discovery(&mut attempt, &retry_probe, retry).await
    });

    retry_wait_started.notified().await;
    tokio::time::advance(Duration::from_millis(10)).await;
    attempt_receiver
        .wait_for(|attempts| *attempts >= 2)
        .await
        .expect("retry driver must retain its attempt observer");
    tokio::time::advance(Duration::from_millis(40)).await;
    tokio::task::yield_now().await;
    assert!(
        discovery.is_finished(),
        "the retry deadline was renewed after a later attempt"
    );
    let discovery_result = discovery
        .await
        .expect("database discovery task must not panic");
    assert_unavailable(discovery_result);
    assert!(*attempt_receiver.borrow() > 1);
}

/// A ready success cannot win `timeout_at`'s polling order once the fixed
/// deadline has elapsed: the driver must reject it before starting the attempt.
#[tokio::test(start_paused = true)]
async fn database_discovery_does_not_start_a_ready_attempt_at_the_deadline() {
    let (attempt_sender, attempt_receiver) = tokio::sync::watch::channel(0_u32);
    let retry_wait_started = Arc::new(tokio::sync::Notify::new());
    let retry_probe = RetryWaitProbe {
        retry_wait_started: Arc::clone(&retry_wait_started),
    };
    let retry = SqliteDatabaseDiscoveryRetry {
        timeout: Duration::from_millis(50),
        poll_interval: Duration::from_millis(50),
    };
    let discovery = tokio::spawn(async move {
        let mut attempt = TransientThenSuccessAttempt { attempt_sender };
        retry_sqlite_database_discovery(&mut attempt, &retry_probe, retry).await
    });

    retry_wait_started.notified().await;
    tokio::time::advance(Duration::from_millis(50)).await;
    tokio::task::yield_now().await;
    assert!(
        discovery.is_finished(),
        "database discovery remained live after its fixed deadline"
    );
    assert_unavailable(
        discovery
            .await
            .expect("database discovery task must not panic"),
    );
    assert_eq!(
        *attempt_receiver.borrow(),
        1,
        "the ready success attempt must not start at the expired deadline"
    );
}

/// One fixed deadline also bounds an individual attempt that never completes.
#[tokio::test(start_paused = true)]
async fn database_discovery_expires_one_fixed_deadline_during_an_attempt() {
    let (attempt_sender, mut attempt_receiver) = tokio::sync::watch::channel(0_u32);
    let retry = SqliteDatabaseDiscoveryRetry {
        timeout: Duration::from_millis(50),
        poll_interval: Duration::from_millis(10),
    };
    let discovery = tokio::spawn(async move {
        let mut attempt = PendingAttempt { attempt_sender };
        retry_sqlite_database_discovery(&mut attempt, &UnobservedSqliteDatabaseDiscovery, retry)
            .await
    });
    attempt_receiver
        .wait_for(|attempts| *attempts == 1)
        .await
        .expect("retry driver must retain its attempt observer");
    tokio::time::advance(Duration::from_millis(50)).await;
    tokio::task::yield_now().await;
    assert!(
        discovery.is_finished(),
        "an outstanding discovery attempt exceeded the fixed deadline"
    );
    assert_unavailable(
        discovery
            .await
            .expect("database discovery task must not panic"),
    );
    assert_eq!(*attempt_receiver.borrow(), 1);
}

/// Permanent failures return immediately and never enter the retry path.
#[tokio::test]
async fn database_discovery_does_not_retry_a_non_lock_failure() {
    let mut attempt = ImmediateNonRetryableAttempt { attempts: 0 };
    let result = retry_sqlite_database_discovery(
        &mut attempt,
        &UnobservedSqliteDatabaseDiscovery,
        SQLITE_DATABASE_DISCOVERY_RETRY,
    )
    .await;
    assert_unavailable(result);
    assert_eq!(attempt.attempts, 1);
}

/// Dropping the caller-owned setup future during backoff must return its held
/// pool connection even while the peer still owns the schema lock.
#[tokio::test]
async fn cancelling_database_discovery_returns_its_pool_slot() {
    let (pool, mut blocker) = controlled_shared_memory_pool("cancelled-discovery").await;
    let (discovery, _observation_receiver, query_attempt_receiver) = ControlledDiscovery::new();
    let cancellation_retry = SqliteDatabaseDiscoveryRetry {
        timeout: SQLITE_DATABASE_DISCOVERY_RETRY.timeout,
        poll_interval: SQLITE_DATABASE_DISCOVERY_RETRY.timeout,
    };
    let mut setup = Box::pin(setup_sqlite_pool_with_discovery_probe(
        pool.clone(),
        &discovery,
        cancellation_retry,
    ));
    let mut hold_lock_until_retry = Box::pin(async {
        discovery.query_arrived.wait().await;
        sqlx::query("BEGIN")
            .execute(&mut *blocker)
            .await
            .expect("controlled transaction must begin");
        sqlx::query("CREATE TABLE discovery_lock (value INTEGER NOT NULL)")
            .execute(&mut *blocker)
            .await
            .expect("schema transaction must acquire the main schema lock");
        discovery.query_released.wait().await;
        discovery.retry_wait_started.notified().await;
    });

    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            () = &mut hold_lock_until_retry => {},
            result = &mut setup => {
                panic!("setup returned before its first retry wait: {result:?}");
            },
        }
    })
    .await
    .expect("controlled discovery must enter retry wait");
    let attempts_before_cancellation = *query_attempt_receiver.borrow();
    assert_eq!(attempts_before_cancellation, 1);
    drop(setup);
    drop(hold_lock_until_retry);

    let returned_slot = tokio::time::timeout(Duration::from_secs(1), pool.acquire())
        .await
        .expect("cancelling discovery must return its held pool slot")
        .expect("the returned pool slot must remain usable while its peer is locked");
    tokio::task::yield_now().await;
    assert_eq!(
        *query_attempt_receiver.borrow(),
        attempts_before_cancellation,
        "dropping the caller-owned future must stop all later attempts"
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .expect("controlled schema transaction must release its lock");
    drop(blocker);
    drop(returned_slot);
}
