//! Several `nebula-worker` processes racing over one PostgreSQL database.
//!
//! Every other lease and control-queue proof runs inside one process: the
//! storage chaos suite contends on the in-memory reference adapter, the loom
//! probe checks the in-memory model, and the PostgreSQL conformance suites drive
//! the SQL adapters from one test process. This test runs the real worker binary
//! three times against one fresh database and checks the durable invariants a
//! multi-replica deployment relies on:
//!
//! 1. A first worker runs alone and takes the first batch of executions, so
//!    every lease on that batch is provably its own.
//! 2. Two more workers join, and all three race on the same control queue for a
//!    second batch of short executions and a batch of long ones.
//! 3. The long executions are cancelled through the durable command service.
//!    The process that drains a `Cancel` is often not the lease holder, so the
//!    cancel has to reach another process through the recorded request.
//! 4. The first worker is killed with `SIGKILL` while it still holds leases on
//!    parked executions. Nothing tells the others; only lease expiry does.
//!
//! Then every short execution has completed and every long one is cancelled,
//! each `Start` was accepted by exactly one owner turn, every execution the
//! dead worker held was re-fenced by a survivor, each submitted command was
//! consumed exactly once with none left pending, and both survivors are still
//! running.
//!
//! The cancels go out before the kill on purpose. A cancel recorded while a
//! dead worker's lease is still live is acknowledged, and is only acted on when
//! a survivor next acquires that execution: at its next durable wake, which
//! for the long executions here is ten minutes away.
//!
//! The test needs a live PostgreSQL in `DATABASE_URL` (a role that may create
//! databases) and takes about a minute, most of it waiting out the 30-second
//! lease of the killed worker. It is ignored by default; the nightly chaos
//! workflow runs it with
//! `cargo nextest run -p nebula-worker-bin --features postgres
//! --test postgres_multiprocess --run-ignored only`.

#![cfg(feature = "postgres")]

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nebula_core::{WorkflowId, id::ExecutionId, node_key};
use nebula_storage::postgres::{
    PgControlQueue, PgExecutionStore, PgPlanFlavorCatalog, PgStartAcceptanceStore,
    PgTenantProvisioningStore, PgWorkflowStore, PgWorkflowVersionStore,
};
use nebula_storage_port::{
    Scope,
    dto::{
        PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate, TenantProvisioningOutcome,
        TenantProvisioningRequest, WorkflowRecord,
    },
    store::{TenantProvisioningStore, WorkflowStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, NodeDefinition, ParamValue, Version, WorkflowConfig, WorkflowDefinition,
};
use serde_json::json;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};

/// Worker processes sharing the database; the first is the one killed.
const WORKERS: u8 = 3;
/// Short executions started while only the first worker runs.
const VICTIM_BATCH: usize = 4;
/// Short executions started once every worker races on the queue.
const RACE_BATCH: usize = 8;
/// Long executions started with the race batch and cancelled before the kill.
const CANCEL_BATCH: usize = 3;
/// How long a short execution parks. Long enough that the first worker still
/// holds its batch when it is killed, after the other workers boot and the
/// cancels land.
const SHORT_DELAY_MS: u64 = 45_000;
/// How long a long execution would park; only a cancel ends it in time.
const LONG_DELAY_MS: u64 = 600_000;
/// Bound on a worker process becoming ready (schema admission included).
const READY_BOUND: Duration = Duration::from_mins(1);
/// Bound on a submitted batch being claimed and leased.
const LEASE_BOUND: Duration = Duration::from_secs(30);
/// Bound on a cancel reaching a lease holder on another process, which reads
/// it from its next heartbeat (every 10 s).
const CANCEL_BOUND: Duration = Duration::from_secs(30);
/// Bound on every execution settling after the kill: the dead worker's lease
/// (30 s) has to expire before a survivor takes its executions over.
const SETTLE_BOUND: Duration = Duration::from_mins(2);

/// Artifact identity every process agrees on: 32 bytes of `0x71`.
const ARTIFACT_DIGEST_BYTE: u8 = 0x71;
/// Fixed base64 AES-256 credential key for this test only.
const CREDENTIAL_KEY_BASE64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "multi-process PostgreSQL workload (about a minute); nightly CI runs it with --run-ignored only"]
async fn worker_processes_share_one_postgres_without_losing_or_repeating_work() {
    let admin_url = std::env::var("DATABASE_URL").expect(
        "postgres_multiprocess needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
    );
    let database = Arc::new(ScratchDatabase::create(&admin_url).await);
    let logs = tempfile::tempdir().expect("worker log directory");

    // Spawned so a panic inside the scenario still reaches the cleanup below;
    // the workers die with it through `kill_on_drop`.
    let scenario = tokio::spawn({
        let database = Arc::clone(&database);
        let logs = logs.path().to_owned();
        async move { run_scenario(&database, &logs).await }
    });
    let outcome = match scenario.await {
        Ok(outcome) => outcome,
        Err(join) => Err(format!("scenario panicked: {join}")),
    };

    let cleanup = match Arc::try_unwrap(database) {
        Ok(database) => database.remove().await,
        Err(_) => Err("the scratch database is still shared after the scenario".to_owned()),
    };
    if let Err(failure) = outcome {
        panic!("{failure}\n{}", worker_logs(logs.path()));
    }
    if let Err(failure) = cleanup {
        panic!("{failure}");
    }
}

/// The scenario proper. Returns a failure message instead of panicking so the
/// caller can always drop the scratch database and attach the worker logs.
async fn run_scenario(database: &ScratchDatabase, logs: &Path) -> Result<(), String> {
    let pool = &database.pool;
    nebula_storage::postgres::init_schema(pool)
        .await
        .map_err(|error| format!("schema setup failed: {error}"))?;
    let scope = scope();
    provision_scope(pool, &scope).await;
    let short_workflow = activate_delay_workflow(pool, &scope, "short", SHORT_DELAY_MS).await;
    let long_workflow = activate_delay_workflow(pool, &scope, "long", LONG_DELAY_MS).await;

    // Phase 1: the victim runs alone, so every lease on its batch is its own.
    let mut victim = WorkerProcess::spawn(0, &database.url, logs);
    victim.wait_ready().await?;
    let victim_batch = start_executions(pool, &scope, short_workflow, VICTIM_BATCH).await;
    wait_until_leased(pool, &victim_batch).await?;
    let victim_holders = lease_holders(pool, &victim_batch).await;
    let [victim_holder] = <[String; 1]>::try_from(victim_holders.into_iter().collect::<Vec<_>>())
        .map_err(|holders| {
        format!("one process must hold every lease of its batch, saw {holders:?}")
    })?;

    // Phase 2: every worker races on the same control queue.
    let mut survivors = Vec::new();
    for index in 1..WORKERS {
        survivors.push(WorkerProcess::spawn(index, &database.url, logs));
    }
    for survivor in &mut survivors {
        survivor.wait_ready().await?;
    }
    let race_batch = start_executions(pool, &scope, short_workflow, RACE_BATCH).await;
    let cancel_batch = start_executions(pool, &scope, long_workflow, CANCEL_BATCH).await;
    let raced: Vec<_> = race_batch.iter().chain(&cancel_batch).copied().collect();
    wait_until_leased(pool, &raced).await?;

    // Phase 3: cancel the long executions through the durable command path
    // while every worker runs. The consumer that drains a Cancel is often not
    // the lease holder, so the cancel has to cross processes to land.
    let commands = nebula_engine::ExecutionCommandService::new(
        Arc::new(PgExecutionStore::new(pool.clone())),
        Arc::new(PgControlQueue::new(pool.clone())),
    );
    for execution_id in &cancel_batch {
        let receipt = commands
            .cancel(&scope, *execution_id, None)
            .await
            .map_err(|error| format!("cancel of {execution_id} refused: {error}"))?;
        if !receipt.enqueued {
            return Err(format!("cancel of {execution_id} was not enqueued"));
        }
    }
    wait_until_terminal(pool, &cancel_batch, CANCEL_BOUND).await?;

    // Phase 4: kill the victim while it holds leases. Only expiry frees them.
    let fenced_at_kill = held_by(pool, &victim_holder).await;
    if !victim_batch
        .iter()
        .all(|execution| fenced_at_kill.contains_key(&execution.to_string()))
    {
        return Err(format!(
            "the victim must still hold its whole batch when killed; it holds {fenced_at_kill:?}"
        ));
    }
    victim.kill().await;

    // Phase 5: the survivors take the dead worker's executions over.
    let short: Vec<_> = victim_batch.iter().chain(&race_batch).copied().collect();
    let all: Vec<_> = short.iter().chain(&cancel_batch).copied().collect();
    wait_until_terminal(pool, &short, SETTLE_BOUND).await?;

    let mut violations = Vec::new();
    check_statuses(pool, &short, "completed", &mut violations).await;
    check_statuses(pool, &cancel_batch, "cancelled", &mut violations).await;
    check_node_states(pool, &short, "completed", &mut violations).await;
    check_node_states(pool, &cancel_batch, "cancelled", &mut violations).await;
    check_takeover(pool, &fenced_at_kill, &victim_holder, &mut violations).await;
    check_control_queue(pool, &all, &cancel_batch, &mut violations).await;
    check_start_accepted_once(pool, &all, &mut violations).await;
    for survivor in &mut survivors {
        if let Some(status) = survivor.exited() {
            violations.push(format!(
                "worker {} exited during the run: {status}",
                survivor.index
            ));
        }
    }
    for mut survivor in survivors {
        survivor.kill().await;
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "multi-process invariants violated:\n  {}",
            violations.join("\n  ")
        ))
    }
}

// ── Scratch database ──────────────────────────────────────────────────────────

/// A database created for one run and dropped after it.
///
/// A fresh database rather than a private schema: the worker binary takes only
/// a DSN, so it cannot be pinned to a schema the way an in-process pool is.
struct ScratchDatabase {
    admin_url: String,
    name: String,
    url: String,
    pool: PgPool,
}

impl ScratchDatabase {
    async fn create(admin_url: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after the epoch")
            .as_millis();
        // `[a-z0-9_]` only, so interpolating it into DDL cannot inject SQL.
        let name = format!("nebula_multiprocess_{}_{stamp}", std::process::id());
        let admin = admin_pool(admin_url).await;
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("DATABASE_URL role must be allowed to create the scratch database");
        admin.close().await;
        let url = with_database(admin_url, &name);
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect to the scratch database");
        Self {
            admin_url: admin_url.to_owned(),
            name,
            url,
            pool,
        }
    }

    async fn remove(self) -> Result<(), String> {
        self.pool.close().await;
        let admin = admin_pool(&self.admin_url).await;
        let dropped = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        )))
        .execute(&admin)
        .await;
        admin.close().await;
        dropped
            .map(drop)
            .map_err(|error| format!("could not drop scratch database {}: {error}", self.name))
    }
}

async fn admin_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .expect("connect to DATABASE_URL")
}

/// `url` with its database path replaced by `database`, query string kept.
fn with_database(url: &str, database: &str) -> String {
    let (base, query) = url
        .split_once('?')
        .map_or((url, None), |(base, query)| (base, Some(query)));
    let authority = base.find("://").map_or(0, |index| index + 3);
    let base = base[authority..]
        .find('/')
        .map_or(base, |slash| &base[..authority + slash]);
    match query {
        Some(query) => format!("{base}/{database}?{query}"),
        None => format!("{base}/{database}"),
    }
}

// ── Worker processes ──────────────────────────────────────────────────────────

/// One `nebula-worker` process with a fixed processor id and its own log file.
struct WorkerProcess {
    index: u8,
    child: tokio::process::Child,
    log: PathBuf,
}

impl WorkerProcess {
    fn spawn(index: u8, database_url: &str, logs: &Path) -> Self {
        let log = logs.join(format!("worker-{index}.log"));
        let stderr = std::fs::File::create(&log).expect("create worker log");
        let workdir = logs.join(format!("worker-{index}"));
        std::fs::create_dir_all(&workdir).expect("create worker working directory");
        let processor_id = format!("{:02x}", 0xa0 + index).repeat(16);
        let artifact_digest = format!("{ARTIFACT_DIGEST_BYTE:02x}").repeat(32);
        let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_nebula-worker"))
            .env_clear()
            .env("NEBULA_WORKER_ARTIFACT_SET_DIGEST", artifact_digest)
            .env("NEBULA_WORKER_DATABASE_URL", database_url)
            .env("NEBULA_WORKER_DB_MAX_CONNECTIONS", "4")
            .env("NEBULA_WORKER_PROCESSOR_ID", processor_id)
            .env("NEBULA_CRED_MASTER_KEY", CREDENTIAL_KEY_BASE64)
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .current_dir(workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .kill_on_drop(true)
            .spawn()
            .expect("launch the worker binary");
        Self { index, child, log }
    }

    /// Wait for the binary's "runtime ready" line, failing if it exits first.
    async fn wait_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + READY_BOUND;
        loop {
            let log = std::fs::read_to_string(&self.log).unwrap_or_default();
            if log.contains("core-flavor runtime ready") {
                return Ok(());
            }
            if let Some(status) = self.exited() {
                return Err(format!(
                    "worker {} exited before ready: {status}",
                    self.index
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "worker {} not ready within {READY_BOUND:?}",
                    self.index
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// `SIGKILL`: no drain, no lease release, nothing flushed.
    async fn kill(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

fn worker_logs(logs: &Path) -> String {
    (0..WORKERS)
        .map(|index| {
            let path = logs.join(format!("worker-{index}.log"));
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let lines: Vec<_> = text.lines().collect();
            let tail = &lines[lines.len().saturating_sub(60)..];
            format!(
                "── worker {index} (last {} lines) ──\n{}",
                tail.len(),
                tail.join("\n")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Seeding through the owners' own services ──────────────────────────────────

fn scope() -> Scope {
    Scope::new(
        nebula_core::WorkspaceId::from_bytes([0x22; 16]).to_string(),
        nebula_core::OrgId::from_bytes([0x11; 16]).to_string(),
    )
}

async fn provision_scope(pool: &PgPool, scope: &Scope) {
    let org = TenantOrgCreate::new(
        scope.org_id.clone(),
        scope.org_id.clone(),
        "Multiprocess".into(),
        "multiprocess".into(),
        "free".into(),
        None,
        json!({}),
    )
    .expect("org values");
    let workspace = TenantDefaultWorkspaceCreate::new(
        scope.workspace_id.clone(),
        "default".into(),
        "Default".into(),
        None,
        "multiprocess".into(),
        json!({}),
    )
    .expect("workspace values");
    let request = TenantProvisioningRequest::new(
        org,
        workspace,
        PrincipalKind::User,
        "multiprocess-owner".into(),
        None,
    )
    .expect("provisioning request");
    let outcome = PgTenantProvisioningStore::new(pool.clone())
        .provision_tenant(request)
        .await
        .expect("provision the test tenant");
    assert!(matches!(outcome, TenantProvisioningOutcome::Created));
}

/// The frozen core registry the workers derive their flavor from.
fn core_registry() -> Arc<nebula_plugin::FrozenPluginRegistry> {
    nebula_deployment::CoreRelease::new(nebula_core::ArtifactSetDigest::from_bytes(
        [ARTIFACT_DIGEST_BYTE; 32],
    ))
    .expect("core release")
    .into_registry()
}

/// Activate a one-node workflow that parks on `core.delay` for `delay_ms`.
///
/// While the timer is pending the owning process keeps the frontier, and so
/// the lease, alive; killing it there leaves a lease only expiry can free.
async fn activate_delay_workflow(
    pool: &PgPool,
    scope: &Scope,
    slug: &str,
    delay_ms: u64,
) -> WorkflowId {
    let workflow_id = WorkflowId::new();
    let now = chrono::Utc::now();
    let delay = NodeDefinition::new(node_key!("delay"), "Park", "core", "core.delay")
        .expect("delay node")
        .with_parameter("mode", ParamValue::literal(json!("for")))
        .with_parameter("amount", ParamValue::literal(json!(delay_ms)))
        .with_parameter("unit", ParamValue::literal(json!("milliseconds")))
        .with_parameter("data", ParamValue::literal(json!({"workflow": slug})));
    let definition = WorkflowDefinition {
        id: workflow_id,
        name: format!("multiprocess-{slug}"),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![delay],
        connections: Vec::new(),
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    let workflows = Arc::new(PgWorkflowStore::new(pool.clone()));
    workflows
        .create(
            scope,
            WorkflowRecord {
                id: workflow_id.to_string(),
                scope: scope.clone(),
                version: 0,
                slug: format!("multiprocess-{slug}"),
            },
        )
        .await
        .expect("create workflow");
    nebula_engine::WorkflowActivationService::new(
        workflows,
        Arc::new(PgWorkflowVersionStore::new(pool.clone())),
        core_registry(),
        nebula_engine::PlanFlavorRevisionInstaller::new(Arc::new(PgPlanFlavorCatalog::new(
            pool.clone(),
            &nebula_metrics::MetricsRegistry::new(),
        ))),
        Arc::new(nebula_core::accessor::SystemClock),
    )
    .activate(scope, workflow_id, 0, definition)
    .await
    .unwrap_or_else(|error| match &error {
        nebula_engine::WorkflowActivationError::Compilation(compilation) => {
            panic!("activation refused: {:?}", compilation.diagnostics())
        },
        _ => panic!("activation refused: {error:?}"),
    });
    workflow_id
}

/// Start `count` executions; each persists its execution row and one `Start`.
async fn start_executions(
    pool: &PgPool,
    scope: &Scope,
    workflow_id: WorkflowId,
    count: usize,
) -> Vec<ExecutionId> {
    let service = nebula_engine::WorkflowStartService::new(
        nebula_engine::WorkflowStores {
            workflow: Arc::new(PgWorkflowStore::new(pool.clone())),
            versions: Arc::new(PgWorkflowVersionStore::new(pool.clone())),
        },
        Arc::new(PgExecutionStore::new(pool.clone())),
        Arc::new(PgStartAcceptanceStore::new(pool.clone())),
        nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(PgPlanFlavorCatalog::new(
            pool.clone(),
            &nebula_metrics::MetricsRegistry::new(),
        ))),
        core_registry(),
        Arc::new(nebula_core::accessor::SystemClock),
        nebula_execution::ExecutionBudget::default(),
    )
    .expect("start owner accepts the default budget");
    let mut started = Vec::with_capacity(count);
    for _ in 0..count {
        let receipt = service
            .start(scope, workflow_id, Some(json!({})), None, None)
            .await
            .expect("start owner persists the execution and its Start");
        started.push(receipt.state().execution_id);
    }
    started
}

// ── Observation ───────────────────────────────────────────────────────────────

fn ids(executions: &[ExecutionId]) -> Vec<String> {
    executions.iter().map(ToString::to_string).collect()
}

/// Wait until every execution is `running` under a live lease.
async fn wait_until_leased(pool: &PgPool, executions: &[ExecutionId]) -> Result<(), String> {
    let deadline = Instant::now() + LEASE_BOUND;
    loop {
        let leased: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM executions WHERE id = ANY($1) AND status = 'running' \
             AND lease_holder IS NOT NULL AND lease_expires_at > now()",
        )
        .bind(ids(executions))
        .fetch_one(pool)
        .await
        .map_err(|error| format!("lease query failed: {error}"))?;
        if usize::try_from(leased).is_ok_and(|leased| leased == executions.len()) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "only {leased} of {} executions were leased within {LEASE_BOUND:?}",
                executions.len()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn lease_holders(pool: &PgPool, executions: &[ExecutionId]) -> BTreeSet<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT DISTINCT lease_holder FROM executions WHERE id = ANY($1)",
    )
    .bind(ids(executions))
    .fetch_all(pool)
    .await
    .expect("lease holder query")
    .into_iter()
    .flatten()
    .collect()
}

/// Executions `holder` leases right now, with the fencing generation it holds.
async fn held_by(pool: &PgPool, holder: &str) -> BTreeMap<String, i64> {
    sqlx::query(
        "SELECT id, fencing_generation FROM executions \
         WHERE lease_holder = $1 AND lease_expires_at > now()",
    )
    .bind(holder)
    .fetch_all(pool)
    .await
    .expect("held lease query")
    .into_iter()
    .map(|row| (row.get("id"), row.get("fencing_generation")))
    .collect()
}

async fn wait_until_terminal(
    pool: &PgPool,
    executions: &[ExecutionId],
    bound: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + bound;
    loop {
        let open: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, status FROM executions WHERE id = ANY($1) \
             AND status NOT IN ('completed', 'failed', 'cancelled', 'timed_out')",
        )
        .bind(ids(executions))
        .fetch_all(pool)
        .await
        .map_err(|error| format!("status query failed: {error}"))?;
        if open.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("executions still open after {bound:?}: {open:?}"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

// ── Invariants ────────────────────────────────────────────────────────────────

async fn check_statuses(
    pool: &PgPool,
    executions: &[ExecutionId],
    expected: &str,
    violations: &mut Vec<String>,
) {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, status FROM executions WHERE id = ANY($1)")
            .bind(ids(executions))
            .fetch_all(pool)
            .await
            .expect("status query");
    for (id, status) in rows {
        if status != expected {
            violations.push(format!("{id} ended {status}, expected {expected}"));
        }
    }
}

/// The `delay` node of each execution ended `expected`.
async fn check_node_states(
    pool: &PgPool,
    executions: &[ExecutionId],
    expected: &str,
    violations: &mut Vec<String>,
) {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, state->'node_states'->'delay'->>'state' FROM executions WHERE id = ANY($1)",
    )
    .bind(ids(executions))
    .fetch_all(pool)
    .await
    .expect("node state query");
    for (id, state) in rows {
        if state.as_deref() != Some(expected) {
            violations.push(format!("{id}: node ended {state:?}, expected {expected}"));
        }
    }
}

/// Every execution the killed worker held was re-fenced by another holder.
async fn check_takeover(
    pool: &PgPool,
    fenced_at_kill: &BTreeMap<String, i64>,
    victim_holder: &str,
    violations: &mut Vec<String>,
) {
    let ids: Vec<_> = fenced_at_kill.keys().cloned().collect();
    let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, fencing_generation, lease_holder FROM executions WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
    .expect("fencing query");
    for (id, generation, holder) in rows {
        let at_kill = fenced_at_kill.get(&id).copied().unwrap_or_default();
        if generation <= at_kill {
            violations.push(format!(
                "{id}: fencing generation {generation} never advanced past {at_kill}, \
                 so no survivor took over the dead worker's lease"
            ));
        }
        if holder.as_deref() == Some(victim_holder) {
            violations.push(format!("{id}: still leased to the dead worker"));
        }
    }
}

/// Exactly one `Start` per execution and one `Cancel` per cancelled one, each
/// consumed; nothing left pending, in flight or failed.
async fn check_control_queue(
    pool: &PgPool,
    executions: &[ExecutionId],
    cancelled: &[ExecutionId],
    violations: &mut Vec<String>,
) {
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT execution_id, command, status, count(*) FROM execution_control_queue \
         WHERE execution_id = ANY($1) GROUP BY 1, 2, 3",
    )
    .bind(ids(executions))
    .fetch_all(pool)
    .await
    .expect("control queue query");
    let cancelled: BTreeSet<_> = ids(cancelled).into_iter().collect();
    for execution in ids(executions) {
        let mut expected = vec![("Start", 1)];
        if cancelled.contains(&execution) {
            expected.push(("Cancel", 1));
        }
        for (command, want) in expected {
            let consumed: i64 = rows
                .iter()
                .filter(|(id, kind, status, _)| {
                    id == &execution && kind == command && status == "Completed"
                })
                .map(|row| row.3)
                .sum();
            if consumed != want {
                violations.push(format!(
                    "{execution}: {consumed} consumed {command} commands, expected {want}"
                ));
            }
        }
    }
    for (execution, command, status, count) in &rows {
        if status != "Completed" {
            violations.push(format!(
                "{execution}: {count} {command} command(s) left {status}"
            ));
        }
    }
}

/// Each execution's `Start` row was accepted by exactly one owner turn: the
/// journal holds one `control_accepted` observation naming that row.
async fn check_start_accepted_once(
    pool: &PgPool,
    executions: &[ExecutionId],
    violations: &mut Vec<String>,
) {
    let starts: Vec<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT execution_id, id FROM execution_control_queue \
         WHERE execution_id = ANY($1) AND command = 'Start'",
    )
    .bind(ids(executions))
    .fetch_all(pool)
    .await
    .expect("start row query");
    let accepted: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT execution_id, payload->'observation'->'source'->'row_id' \
         FROM execution_journal WHERE execution_id = ANY($1) \
         AND payload->>'event' = 'control_observed' \
         AND payload->'observation'->'reason'->>'code' = 'control_accepted'",
    )
    .bind(ids(executions))
    .fetch_all(pool)
    .await
    .expect("control observation query");
    for (execution, row_id) in starts {
        let row_id: Vec<serde_json::Value> =
            row_id.into_iter().map(serde_json::Value::from).collect();
        let seen = accepted
            .iter()
            .filter(|(id, source)| id == &execution && source.as_array() == Some(&row_id))
            .count();
        if seen != 1 {
            violations.push(format!(
                "{execution}: its Start was accepted {seen} times, expected once"
            ));
        }
    }
}

#[test]
fn scratch_database_url_keeps_credentials_and_query() {
    assert_eq!(
        with_database(
            "postgres://postgres:postgres@localhost:5432/nebula_test?sslmode=disable",
            "scratch"
        ),
        "postgres://postgres:postgres@localhost:5432/scratch?sslmode=disable"
    );
    assert_eq!(
        with_database("postgres://postgres@localhost:54329", "scratch"),
        "postgres://postgres@localhost:54329/scratch"
    );
}
