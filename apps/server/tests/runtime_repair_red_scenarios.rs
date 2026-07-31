//! Genuine expected-RED scenarios for the first-party runtime-repair profile.
//!
//! Every test enters through HTTP and observes either the public response or
//! the authority-free lifecycle observer. A setup or supervisor failure emits
//! no `EXPECTED_RED` marker; only a reached, independently checked product
//! defect is classified as expected RED.

#![cfg(feature = "runtime-repair-red")]

#[path = "runtime_repair_red_scenarios/component_c7.rs"]
mod component_c7;

use std::{net::SocketAddr, time::Duration};

use nebula_core::{NodeKey, id::ExecutionId};
use nebula_server::runtime_repair_red::{
    EvidenceIntegrityError, RuntimeRepairHarness, RuntimeRepairProfileConfig,
    RuntimeRepairProfileHandle,
};
use reqwest::{
    Client, Response, StatusCode,
    header::{COOKIE, HeaderMap, HeaderValue, SET_COOKIE},
};
use secrecy::SecretString;
use serde_json::{Value, json};

const API_PREFIX: &str = "/api/v1/orgs/runtime-repair/workspaces/runtime-repair";
const PROFILE_EMAIL: &str = "runtime-repair@nebula.invalid";
const PROFILE_PASSWORD: &str = "runtime-repair-evidence-password";
const SESSION_COOKIE: &str = "__Host-nebula-session";
const CSRF_COOKIE: &str = "__Host-nebula-csrf";
const DELAY_NODE: &str = "delay";
const OBSERVATION_BOUND: Duration = Duration::from_millis(750);

/// How long to wait for the runtime to act on a command it was handed through
/// the durable control queue.
///
/// Deliberately larger than [`OBSERVATION_BOUND`], which bounds in-process
/// lifecycle observations. This one waits on a full round trip — the producing
/// request returns, a consumer polls the queue, claims the row, acquires the
/// execution lease, and commits — so sizing it like an in-process signal would
/// measure poll cadence rather than whether the runtime honored the command.
///
/// Generous on purpose. The failure this guards against is "the runtime never
/// honors the command at all" — the shape a consumer that blocks on a parked
/// dispatch produces, where nothing arrives however long you wait. A tighter
/// bound would add cold-start flakiness without catching anything that this
/// one misses.
const RUNTIME_COMMAND_BOUND: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
enum Backend {
    InMemory,
    FileSqlite,
    LivePostgres,
}

impl Backend {
    fn harness(self) -> RuntimeRepairHarness {
        match self {
            Self::InMemory => RuntimeRepairProfileConfig::in_memory().into_harness(),
            Self::FileSqlite => RuntimeRepairProfileConfig::file_sqlite().into_harness(),
            Self::LivePostgres => {
                let database_dsn = std::env::var("DATABASE_URL")
                    .expect("SETUP: live PostgreSQL requires DATABASE_URL");
                RuntimeRepairProfileConfig::live_postgres(SecretString::from(database_dsn))
                    .into_harness()
            },
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::InMemory => "in-memory",
            Self::FileSqlite => "file-sqlite",
            Self::LivePostgres => "live-postgresql",
        }
    }
}

struct AuthenticatedClient {
    client: Client,
    address: SocketAddr,
    cookie: HeaderValue,
    csrf_token: HeaderValue,
}

impl AuthenticatedClient {
    async fn login(address: SocketAddr) -> Self {
        let client = Client::builder()
            .build()
            .expect("SETUP: construct HTTP client");
        let response = client
            .post(format!("http://{address}/api/v1/auth/login"))
            .json(&json!({
                "email": PROFILE_EMAIL,
                "password": PROFILE_PASSWORD,
            }))
            .send()
            .await
            .expect("SETUP: login request reaches profile");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "SETUP: closed profile login must succeed"
        );

        let cookie = cookie_header(response.headers());
        let body: Value = response
            .json()
            .await
            .expect("SETUP: login response is valid JSON");
        let csrf_token = body
            .get("csrf_token")
            .and_then(Value::as_str)
            .expect("SETUP: login response carries csrf_token");

        Self {
            client,
            address,
            cookie: HeaderValue::from_str(&cookie)
                .expect("SETUP: profile cookies form one valid header"),
            csrf_token: HeaderValue::from_str(csrf_token)
                .expect("SETUP: profile CSRF token forms one valid header"),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{API_PREFIX}{path}", self.address)
    }

    fn mutation(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request
            .header(COOKIE, self.cookie.clone())
            .header("x-csrf-token", self.csrf_token.clone())
    }

    async fn create_delay_workflow(&self, scenario: &str) -> String {
        let response = self
            .mutation(self.client.post(self.url("/workflows")))
            .json(&json!({
                "name": format!("runtime-repair-{scenario}"),
                "description": "Task 7 deterministic park/restart scenario",
                "definition": {
                    "nodes": [{
                        "id": DELAY_NODE,
                        "name": "Park",
                        "plugin_key": "core",
                        "action_key": "core.delay",
                        "parameters": {
                            "mode": {"type": "literal", "value": "for"},
                            "amount": {"type": "literal", "value": 60_000},
                            "unit": {"type": "literal", "value": "milliseconds"},
                            "data": {
                                "type": "literal",
                                "value": {"scenario": scenario}
                            }
                        },
                        "enabled": true
                    }],
                    "connections": []
                }
            }))
            .send()
            .await
            .expect("SETUP: workflow create request reaches profile");
        let body = expect_json_status(response, StatusCode::CREATED, "create workflow").await;
        let workflow_id = required_text(&body, "id", "created workflow id").to_owned();

        let response = self
            .mutation(
                self.client
                    .post(self.url(&format!("/workflows/{workflow_id}/activate"))),
            )
            .send()
            .await
            .expect("SETUP: workflow activation request reaches profile");
        expect_json_status(response, StatusCode::OK, "activate workflow").await;
        workflow_id
    }

    async fn start(&self, workflow_id: &str, idempotency_key: &str, input: Value) -> Response {
        self.mutation(
            self.client
                .post(self.url(&format!("/workflows/{workflow_id}/executions"))),
        )
        .header("Idempotency-Key", idempotency_key)
        .json(&json!({"input": input}))
        .send()
        .await
        .expect("SETUP: execution start request reaches profile")
    }

    async fn running_total(&self, workflow_id: &str) -> usize {
        let response = self
            .client
            .get(self.url(&format!(
                "/workflows/{workflow_id}/executions?page=1&page_size=100"
            )))
            .header(COOKIE, self.cookie.clone())
            .send()
            .await
            .expect("SETUP: execution list request reaches profile");
        let body = expect_json_status(response, StatusCode::OK, "list workflow executions").await;
        body.get("total")
            .and_then(Value::as_u64)
            .and_then(|total| usize::try_from(total).ok())
            .expect("SETUP: execution list carries a bounded total")
    }

    async fn get_execution(&self, execution_id: &str) -> Value {
        let response = self
            .client
            .get(self.url(&format!("/executions/{execution_id}")))
            .header(COOKIE, self.cookie.clone())
            .send()
            .await
            .expect("SETUP: execution get request reaches profile");
        expect_json_status(response, StatusCode::OK, "get execution").await
    }

    async fn cancel(&self, execution_id: &str) -> Value {
        let response = self
            .mutation(
                self.client
                    .delete(self.url(&format!("/executions/{execution_id}"))),
            )
            .send()
            .await
            .expect("SETUP: cancellation request reaches profile");
        expect_json_status(response, StatusCode::ACCEPTED, "cancel execution").await
    }
}

struct RunningProfile {
    handle: RuntimeRepairProfileHandle,
    http: AuthenticatedClient,
}

impl RunningProfile {
    async fn launch(harness: &RuntimeRepairHarness) -> Self {
        let handle = harness
            .launch()
            .await
            .expect("SETUP: closed profile must launch");
        handle
            .readiness()
            .await
            .expect("SETUP: all profile components must become ready");
        let http = AuthenticatedClient::login(handle.addr()).await;
        Self { handle, http }
    }

    async fn shutdown(self) {
        self.handle.shutdown();
        self.handle
            .join()
            .await
            .expect("SETUP: profile shutdown must join every supervised component");
    }

    #[expect(
        clippy::print_stderr,
        reason = "the strict JUnit verifier consumes this one machine-readable RED marker"
    )]
    async fn expected_red(self, reason: &'static str) -> ! {
        self.shutdown().await;
        eprintln!("EXPECTED_RED:{reason}");
        panic!("runtime-repair behavioral oracle is expected to be RED");
    }
}

fn cookie_header(headers: &HeaderMap) -> String {
    let mut session = None;
    let mut csrf = None;
    for value in headers.get_all(SET_COOKIE) {
        let raw = value
            .to_str()
            .expect("SETUP: Set-Cookie header is valid ASCII");
        let pair = raw
            .split(';')
            .next()
            .expect("SETUP: Set-Cookie has a name/value pair");
        if pair.starts_with(&format!("{SESSION_COOKIE}=")) {
            session = Some(pair.to_owned());
        } else if pair.starts_with(&format!("{CSRF_COOKIE}=")) {
            csrf = Some(pair.to_owned());
        }
    }
    format!(
        "{}; {}",
        session.expect("SETUP: login sets the host-bound session cookie"),
        csrf.expect("SETUP: login sets the host-bound CSRF cookie")
    )
}

async fn expect_json_status(response: Response, expected: StatusCode, operation: &str) -> Value {
    let actual = response.status();
    let body = response
        .json::<Value>()
        .await
        .expect("SETUP: API response is valid JSON");
    assert_eq!(
        actual,
        expected,
        "SETUP: {operation} returned an unexpected status with problem code {:?}",
        body.get("code").and_then(Value::as_str)
    );
    body
}

fn required_text<'a>(body: &'a Value, field: &str, context: &str) -> &'a str {
    body.get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("SETUP: {context} is present"))
}

async fn startkey_scenario(backend: Backend) {
    let harness = backend.harness();
    let running = RunningProfile::launch(&harness).await;
    let workflow_id = running
        .http
        .create_delay_workflow(&format!("startkey-{}", backend.label()))
        .await;
    let idempotency_key = format!("runtime-repair-startkey-{workflow_id}");
    let original_input = json!({"scenario": "startkey", "revision": 1});

    let first = running
        .http
        .start(&workflow_id, &idempotency_key, original_input.clone())
        .await;
    let first = expect_json_status(first, StatusCode::ACCEPTED, "first keyed start").await;
    let first_execution = required_text(&first, "id", "first execution id").to_owned();
    let first_started_at = first
        .get("started_at")
        .and_then(Value::as_i64)
        .expect("SETUP: acceptance receipt carries a timestamp");

    let replay = running
        .http
        .start(&workflow_id, &idempotency_key, original_input)
        .await;
    let replay = expect_json_status(replay, StatusCode::ACCEPTED, "same-fingerprint replay").await;
    let replay_execution = required_text(&replay, "id", "replayed execution id");
    if replay_execution != first_execution {
        running
            .expected_red("startkey-same-fingerprint-duplicated")
            .await;
    }
    // The receipt is the *original* one, so its timestamp must be the original
    // too. Substituting only the id would date the execution to whenever the
    // retry arrived — a fabricated fact, and one that silently skews any
    // latency measured from this field.
    assert_eq!(
        replay.get("started_at").and_then(Value::as_i64),
        Some(first_started_at),
        "a replayed receipt must carry the original acceptance timestamp"
    );
    // The whole receipt describes the persisted execution, not the request
    // that was refused. Reading `status` from the state this retry built would
    // report `created` for an execution that has since started or finished.
    let persisted = running.http.get_execution(&first_execution).await;
    assert_eq!(
        replay.get("status").and_then(Value::as_str),
        persisted.get("status").and_then(Value::as_str),
        "a replayed receipt must report the execution's persisted status"
    );

    let before_mismatch = running.http.running_total(&workflow_id).await;
    assert_eq!(
        before_mismatch, 1,
        "SETUP: same-fingerprint precondition must expose exactly one execution"
    );
    let mismatch = running
        .http
        .start(
            &workflow_id,
            &idempotency_key,
            json!({"scenario": "startkey", "revision": 2}),
        )
        .await;
    let mismatch_status = mismatch.status();
    let mismatch_body = mismatch
        .json::<Value>()
        .await
        .expect("SETUP: mismatch response is valid JSON");

    if mismatch_status.is_success() {
        running.expected_red("startkey-mismatch-accepted").await;
    }
    if mismatch_status != StatusCode::CONFLICT
        || mismatch_body.get("code").and_then(Value::as_str) != Some("operation_mismatch")
    {
        running
            .expected_red("startkey-wrong-mismatch-contract")
            .await;
    }

    if running.http.running_total(&workflow_id).await != before_mismatch {
        running.expected_red("startkey-execution-delta").await;
    }
    running.shutdown().await;
}

async fn c0_park_restart_resume_scenario(backend: Backend) {
    let harness = backend.harness();
    let running = RunningProfile::launch(&harness).await;
    let workflow_id = running
        .http
        .create_delay_workflow(&format!("c0-{}", backend.label()))
        .await;
    let started = running
        .http
        .start(
            &workflow_id,
            &format!("runtime-repair-c0-{workflow_id}"),
            json!({"scenario": "c0", "revision": 1}),
        )
        .await;
    let started = expect_json_status(started, StatusCode::ACCEPTED, "C0 start").await;
    let execution_text = required_text(&started, "id", "C0 execution id").to_owned();
    let execution_id =
        ExecutionId::parse(&execution_text).expect("SETUP: API returned a valid execution id");
    let node_key = NodeKey::new(DELAY_NODE).expect("SETUP: delay node key is valid");

    match harness
        .evidence_controls()
        .observations()
        .await_node_parked(execution_id, node_key.clone(), OBSERVATION_BOUND)
        .await
    {
        Ok(()) => {},
        Err(EvidenceIntegrityError::ObservationTimedOut) => {
            running.expected_red("c0-drive-not-connected").await;
        },
        Err(error) => {
            running.shutdown().await;
            panic!("SETUP: lifecycle observer failed before C0 park: {error}");
        },
    }

    running.shutdown().await;
    let running = RunningProfile::launch(&harness).await;
    harness
        .evidence_controls()
        .clock()
        .advance_by_millis(60_001)
        .expect("SETUP: injected clock advances across the delay");

    match harness
        .evidence_controls()
        .observations()
        .await_node_wait_completed(execution_id, node_key, OBSERVATION_BOUND)
        .await
    {
        Ok(()) => {},
        Err(EvidenceIntegrityError::ObservationTimedOut) => {
            running.expected_red("c0-resume-not-connected").await;
        },
        Err(error) => {
            running.shutdown().await;
            panic!("SETUP: lifecycle observer failed before C0 resume: {error}");
        },
    }

    match harness
        .evidence_controls()
        .observations()
        .await_execution_finished(execution_id, OBSERVATION_BOUND)
        .await
    {
        Ok(_) => {},
        Err(EvidenceIntegrityError::ObservationTimedOut) => {
            running.expected_red("c0-completion-not-observed").await;
        },
        Err(error) => {
            running.shutdown().await;
            panic!("SETUP: lifecycle observer failed before C0 completion: {error}");
        },
    }

    let execution = running.http.get_execution(&execution_text).await;
    if execution.get("status").and_then(Value::as_str) != Some("completed") {
        running.expected_red("c0-wrong-terminal-state").await;
    }
    running.shutdown().await;
}

async fn cancellation_scenario(backend: Backend) {
    let harness = backend.harness();
    let running = RunningProfile::launch(&harness).await;
    let workflow_id = running
        .http
        .create_delay_workflow(&format!("cancel-{}", backend.label()))
        .await;
    let started = running
        .http
        .start(
            &workflow_id,
            &format!("runtime-repair-cancel-{workflow_id}"),
            json!({"scenario": "cancel-before-handler-completion"}),
        )
        .await;
    let started = expect_json_status(started, StatusCode::ACCEPTED, "cancellation start").await;
    let execution_id = required_text(&started, "id", "cancellation execution id");
    let execution_text = execution_id.to_owned();
    let cancelled = running.http.cancel(&execution_text).await;

    // The response must not claim the run is over. The API submits the cancel
    // through the control queue and writes nothing, so a terminal status or a
    // completion timestamp here could only have been asserted by the handler
    // on the runtime's behalf.
    if cancelled.get("status").and_then(Value::as_str) == Some("cancelled")
        || cancelled
            .get("finished_at")
            .is_some_and(|value| !value.is_null())
    {
        running.expected_red("c1-cancel-terminalized-by-api").await;
    }

    // …and the cancel must still actually happen. The worker drains the
    // command, acquires the lease, and terminalizes the execution itself. This
    // half is what keeps the assertion above from being satisfiable by simply
    // dropping the cancel on the floor.
    let execution_id =
        ExecutionId::parse(&execution_text).expect("SETUP: API returned a valid execution id");
    match harness
        .evidence_controls()
        .observations()
        .await_execution_finished(execution_id, RUNTIME_COMMAND_BOUND)
        .await
    {
        Ok(_) => {},
        Err(EvidenceIntegrityError::ObservationTimedOut) => {
            running
                .expected_red("c1-cancel-not-honored-by-runtime")
                .await;
        },
        Err(error) => {
            running.shutdown().await;
            panic!("SETUP: lifecycle observer failed awaiting the cancel: {error}");
        },
    }

    let execution = running.http.get_execution(&execution_text).await;
    if execution.get("status").and_then(Value::as_str) != Some("cancelled") {
        running.expected_red("c1-cancel-wrong-terminal-state").await;
    }
    running.shutdown().await;
}

#[tokio::test]
async fn startkey_in_memory() {
    startkey_scenario(Backend::InMemory).await;
}

#[tokio::test]
async fn startkey_file_sqlite() {
    startkey_scenario(Backend::FileSqlite).await;
}

#[tokio::test]
async fn startkey_live_postgresql() {
    startkey_scenario(Backend::LivePostgres).await;
}

#[tokio::test]
async fn c0_file_sqlite_park_restart_resume() {
    c0_park_restart_resume_scenario(Backend::FileSqlite).await;
}

#[tokio::test]
async fn c0_live_postgresql_park_restart_resume() {
    c0_park_restart_resume_scenario(Backend::LivePostgres).await;
}

#[tokio::test]
async fn c1_file_sqlite_cancel_before_handler_completion() {
    cancellation_scenario(Backend::FileSqlite).await;
}

#[tokio::test]
async fn c1_live_postgresql_cancel_before_handler_completion() {
    cancellation_scenario(Backend::LivePostgres).await;
}
