# Nebula 1.0 Research (Active)

> Persistent research log for Nebula's road to production-ready 1.0
> ("z8run-class" workflow engine). Built up by AI Factory orchestration on
> 2026-04-28 from layer audits, targeted code zonds, and verification of
> stale memory entries.
>
> Maintenance: prepend a new `## Sessions / ### YYYY-MM-DD` block when a
> meaningful research pass happens. Keep `## Active Summary` as a
> one-screen snapshot; move stale findings down into Sessions.

## Active Summary (input for /aif-plan)

- **Topic:** Nebula → production-ready 1.0 (orchestrator-led).
- **Goal state:** every claim in README / canon / per-crate docs is backed
  by code that exercises the path; observability triple (typed error +
  tracing span + invariant check) at every new boundary; CI green incl.
  loom nightly; out-of-scope items explicitly deferred to 1.1.
- **Status:** cross-cutting + core + most business layers stable; engine
  ~70% (durability debts mostly closed, explicit-termination wiring open);
  api routes wired but five large feature gaps (auth, OpenAPI, webhook
  dispatch, idempotency, tracing-context); sandbox correctness-grade only
  (capability discovery enforcement gap); storage Layer 1 ready, Layer 2
  deferred to "Sprint E"/1.1.
- **Decisions taken (this session):**
  - 11 ROADMAP milestones M0–M10 in `.ai-factory/ROADMAP.md`.
  - First sub-project: **M0 — Engine durability debts**. Verification on
    2026-04-28 collapsed M0.1 / M0.2 (budget + workflow_input persist) to
    DONE; remaining work is M0.3 (Terminate → ExecutionTerminationReason
    wiring) + M0.4 (doc sync).
  - For M0.3: store first-write-wins signal in `ExecutionState.terminated_by`,
    extend (not replace) `ExecutionEvent::ExecutionFinished`, **include**
    sibling cancellation per Phase 3 spec.
- **Out of scope for 1.0** (must not slip silently):
  - Storage Layer 2 / spec-16 multi-tenant row model.
  - ADR-0013 compile-time modes (no build.rs / mode-* features yet).
  - WebSocket endpoint (`api/handlers/websocket.rs:32` returns 501).
  - ADR-0024 dyn-trait migration (#601).
  - Performance regression harness (#600 loadgen-rs investigation).
  - git-cliff / automated CHANGELOG (#599).
- **Hard constraints (DoD):**
  - Typed `thiserror` only in libraries; no `unwrap` / `expect` / `panic` /
    `Box<dyn Error>` in lib code (clippy gate).
  - `tracing` span + invariant check + typed error variant on every new
    state, error, or hot path (per `feedback_observability_as_completion.md`).
  - `lefthook pre-push` mirrors CI required jobs (per
    `feedback_lefthook_mirrors_ci.md`); never let them diverge.
  - No backwards-compat shims (per `feedback_no_shims.md`); breaking
    changes OK if spec-correct (per `feedback_hard_breaking_changes.md`).
  - `cargo deny check` `[wrappers]` enforces layer boundaries.
  - Runnable examples live in root-level `examples/` workspace member only
    (per `feedback_examples_location.md`).

## Sessions

### 2026-04-28 — Initial M0 audit, roadmap, plan

#### Layer audits (4 parallel zonds)

**Engine + execution (zond a07a5e5f).**
- Works end-to-end: workflow run, frontier with bounded concurrency,
  8-state machine (CAS-protected via `ExecutionRepo`), node lifecycle via
  `nebula_eventbus::EventBus<ExecutionEvent>` (capacity 1024, `engine.rs:56`),
  cancellation via `EngineControlDispatch::dispatch_cancel`, full control
  queue consumption (Start/Resume/Restart/Cancel/Terminate, `reclaim_stuck`
  per ADR-0017), durable journal, idempotency keys, credential access
  deny-by-default per canon §4.5.
- Partial / open: `ExecutionTerminationReason::ExplicitStop/ExplicitFail`
  defined but **not wired** (canon §4.5 honesty gap, status.rs:85-94 +
  result.rs:206-219); engine-level retry from `ActionResult::Retry` is
  "planned" (canon §11.2); downstream-edge gate is **local-only**
  (`engine.rs:1808`) — multi-hop conditional flows can read stale data;
  `expression_engine` field is `#[expect(dead_code)]` (engine.rs:124-128);
  `support_inputs` port-driven routing reserved (spec 28).
- Tests: 20 engine integration tests (5557 lines); 211 unit tests in
  execution+workflow; no loom probes for engine concurrency; chaos test
  `refresh_coordinator_chaos.rs` `#[ignore]`'d (~6s CI weight).
- **STALE-CLAIM ALERT in this report:** said "ExecutionBudget not persisted"
  / "workflow_input not persisted" citing engine README L4-debt — verified
  WRONG against state.rs and engine.rs; both fields shipped (issues #289 /
  #311 closed). See pivot below.

**API + SDK (zond a7e3fe61).**
- Axum routes wired (`/health`, `/ready`, `/metrics`, `/api/v1/{auth,me,
  orgs,workspaces,catalog}`), RFC 9457 ProblemDetails, cursor pagination
  with tests.
- Five 1.0-blocker gaps:
  1. Auth backend (9 stub handlers in `handlers/auth.rs:22-113`,
     register/login/verify_email/reset_password/totp/oauth + PAT lookup
     stub at `middleware/auth.rs:134`).
  2. OpenAPI 3.1 spec generation (`handlers/openapi.rs:9-16` stub).
  3. Webhook dispatch (`handlers/webhook.rs:21-34` stubs — transport real,
     handlers not).
  4. Idempotency-Key dedup absent — cancel claims idempotency
     (`handlers/execution.rs:450`) but no header dedup store.
  5. Tracing context propagation: `middleware/request_id.rs` sets header,
     does NOT attach to `tracing::Span` or hand off to engine.
- SDK shape clean; no leaky internals from lower layers.
- Integration tests: knife.rs canon §13 e2e + oauth2 flow + webhook
  signature + RFC 9457 errors + body limits. Auth endpoints untested.

**Storage + sandbox + plugin (zond a5a3de3b).**
- Storage Layer 1 (PG + SQLite) **production-ready 1.0**. 23 PG migrations
  + 9 common. Optimistic CAS, outbox pattern, `FOR UPDATE SKIP LOCKED`,
  `reclaim_stuck` window per ADR-0017. Loom probe for credential
  refresh-claim (ADR-0041 DoD) exists, not in CI.
- Layer 2 (`ControlQueueRepo` only wired; rest of spec-16 row model is
  trait stubs) — explicit "Sprint E" / 1.1 deferral.
- Sandbox = correctness boundary, **not attacker-grade**. Linux-only
  Landlock 5.13+ ACLs, rlimits (addr=512 MB / nofile=256 / cpu=30s /
  nproc=1 / core=0), env_clear + allowlist, stderr 8 KiB cap, envelope
  1 MiB cap with poison transport. Capability discovery enforcement
  **INCOMPLETE** — canon §4.5 honesty gap (`sandbox/lib.rs:21`).
- Plugin trait stable; maturity model
  (Experimental/Beta/Stable/Deprecated). **No engine-plugin ABI
  versioning** — `nebula_version` field exists in manifest but unvalidated.
  Decision needed for 1.0 (M5 ROADMAP).
- ControlQueueRepo: `PgControlQueueRepo` available but `simple_server` +
  tests use `InMemoryControlQueueRepo` — restart loses pending commands.

**Cross-cutting + resource (zond abce7305).**
- All 7 cross-cutting crates (`error`, `log`, `eventbus`, `telemetry`,
  `metrics`, `resilience`, `system`) **stable, no breaks pending**.
- Resource: 12/15 plans DONE; 3 PARTIAL: `06-action-integration` (ResourceAction
  trait + scoped resources), `10-scoped-resources` (per-execution
  isolation), `resource-prototypes` (Postgres Pool / HTTP Resident /
  Telegram Service skeletons need adapter impls or move to
  `examples/`).

#### Repo-wide signals

- **GitHub:** 19+ open issues, all p2/p3 needs-triage/discussion; no
  p0/p1 stop-ship. 5 open Dependabot PRs (toml, mockall, lru, rstest,
  rust-minor-patch group).
- **TODO markers:** 61 occurrences across 21 files. 77% concentrated in
  API handlers (auth ×10, webhook ×4, org ×9, me ×6, credential ×12,
  openapi ×2, execution ×2). Engine: 2. Sandbox: 3. Validator: 1.
- **`#[ignore]` tests:** only benches (`expression/`) and intentional slow
  chaos suite (`refresh_coordinator_chaos`) and sandbox fixture-gated
  test. Not blockers.

#### Targeted M0 zonds

**ExecutionState + ExecutionRepo + migrations (zond acdf5bb5).**
- `ExecutionState` (`crates/execution/src/state.rs:120-171`) — 13 fields.
  CRITICAL: `workflow_input: Option<Value>` (line 158, `#[serde(default)]`,
  closes #311) and `budget: Option<ExecutionBudget>` (line 170,
  `#[serde(default)]`, closes #289) **already present**.
- Setters: `set_workflow_input` (state.rs:206-208), `set_budget`
  (state.rs:218-220).
- `ExecutionRepo` trait (`crates/storage/src/repos/execution.rs:11-119`):
  `transition(id, expected_version, status, state_patch)`, `get`,
  `set_output`, `acquire_lease`, `renew_lease`, `release_lease`. CAS via
  `expected_version`. PG impl `crates/storage/src/backend/pg_execution.rs:88-107`.
- Engine ↔ repo: `checkpoint_node_output()` (engine.rs:2638-2703) and
  `persist_final_state()` (engine.rs:2741-2850); CAS conflict mapped to
  `EngineError::CasConflict` (`engine/error.rs:143-150`,
  `ENGINE:CAS_CONFLICT`).
- Migration pattern: `<NNNN>_<desc>.sql` under
  `crates/storage/migrations/{postgres,sqlite,common}`. Idempotent
  (`IF NOT EXISTS`); ADR-0009 forward-compat
  (`DEFAULT N`, new cols nullable). Latest:
  `00000000000009_add_resume_persistence.sql` (Layer 1) → adds `input
  JSONB` to executions; `0020_add_resume_result_persistence.sql` (PG +
  SQLite parity) → adds `result_schema_version` / `result_kind` /
  `result` to `execution_nodes`.

**ExecutionBudget lifecycle (zond aec80e21).**
- Definition: `crates/execution/src/context.rs:41-64`. Fully serde-ready
  (`Clone/Debug/PartialEq/Eq/Serialize/Deserialize`); `max_duration` via
  `crate::serde_duration_opt`. No tokio handles, no `Instant` in struct —
  serialization-safe.
- Construction: `engine.rs:980-1003` (execute_workflow),
  `engine.rs:608-677` (replay_execution), `engine.rs:1433-1444`
  (resume_execution — reads from `exec_state.budget`, fallback to default
  with warning).
- Enforcement: `check_budget()` at `engine.rs:3219-3241`; called from the
  frontier loop (`engine.rs:1711`) and a wall-clock select arm
  (`engine.rs:1874-1900`).
- Tests: `budget_max_duration_exceeded` (4561), `budget_max_output_bytes_exceeded`
  (4597), `budget_max_total_retries_exceeded` (4629), unlimited variant
  (4655); resume round-trips at `engine.rs:6637` and 6723.

**ActionResult::Terminate → ExplicitStop wiring (zond aad39840).**
- `ActionResult` (`crates/action/src/result.rs:198-223`) — 15 variants.
  Termination twins: `Terminate { reason: TerminationReason::Success {
  note } }` (Stop) and `Terminate { reason: Failure { code, message } }`
  (Fail). Constructed via `terminate_success()` /
  `terminate_failure()` (lines 514-529).
- Engine consumes at `engine.rs:1986` (`Ok((task_id, (node_key,
  Ok(action_result))))`); persist + idempotency record + emit
  `NodeCompleted` + `evaluate_edge`. **`evaluate_edge` (`engine.rs:3141`)
  gates Terminate identical to Skip — that is the entire current
  engine-side wiring per docstring.**
- `determine_final_status` (`engine.rs:3545-3585`) takes `failed_node`,
  `cancel_token`, `exec_state`. Terminate **never reaches it**.
- `ExecutionTerminationReason` (`crates/execution/src/status.rs:98-142`):
  5 variants — `NaturalCompletion`, `ExplicitStop {by_node, note}`,
  `ExplicitFail {by_node, code, message}`, `Cancelled`, `SystemError`.
  Serde round-trips proven (status.rs:290-343, contracts.rs:302).
  **NEVER POPULATED from engine.**
- `ExecutionResult.termination_reason: Option<ExecutionTerminationReason>`
  field already present (`crates/execution/src/result.rs:66`); builder
  `with_termination_reason` at 91-92; serde + legacy tests at 210-258.
  **Never set by engine path.**
- `ExecutionEvent` (`crates/engine/src/event.rs`): 6 variants —
  `NodeStarted`, `NodeCompleted`, `NodeFailed`, `NodeSkipped`,
  `FrontierIntegrityViolation`, `ExecutionFinished {success: bool,
  elapsed}`. **No termination event; success is binary — cannot express
  ExplicitFail vs system failure.**
- Phase 3 ControlAction plan referenced in docstrings; **no separate
  plan file exists** — this session's plan
  (`.ai-factory/plans/m0-explicit-termination.md`) is first-source.

#### Pivot finding (M0 scope reduction)

The first engine audit's claim that `ExecutionBudget` and workflow input
were not persisted was sourced from `crates/engine/README.md` "L4 debt"
text, which **lags reality**. Verified against:

- `state.rs:158, 170` — fields present with `#[serde(default)]`.
- `engine.rs:674, 677` (replay_execution), `998, 1003` (execute_workflow),
  `1433-1444` (resume budget restore), `1487-1497` (resume input restore).
- Tests: `resume_restores_persisted_budget` (engine.rs:6637-6717),
  `resume_falls_back_to_default_budget_on_legacy_state` (6723+),
  `resume_restores_original_workflow_input` (6583).
- Migration `00000000000009_add_resume_persistence.sql:10` adds `input
  JSONB`.

ROADMAP M0.1 / M0.2 marked DONE; M0 plan reduced to M0.3 (Terminate
wiring) + M0.4 (sync README + canon refs + docstrings to match shipped
code). Lesson: per `feedback_review_verify_claims.md`, README L4-debt
blocks are point-in-time; verify against current code before planning on
them.

#### Memory updates

- `project_eventbus_status.md` — refreshed: `ExecutionEvent` migrated to
  `nebula_eventbus::EventBus` (`engine.rs:56`, capacity 1024). The "still
  on raw mpsc" framing is stale; left a note that `expression_engine`
  remains dead-code (M1.2).
- `project_cancel_handler_inline.md` — re-verified, **still accurate**:
  no `ExecutionCommandService` exists; `cancel_execution` at
  `crates/api/src/handlers/execution.rs:359-492` continues to inline CAS
  + terminal guard + `completed_at` backfill + control-queue enqueue +
  503/500 shaping. Remains a prerequisite for any second transport
  issuing control commands.

#### Open architectural decisions (forward-looking)

- **Plugin ABI / engine semver coupling (ROADMAP M5):** commit to
  `Plugin` trait stability via `nebula_version` validation in plugin
  manifests, OR document explicit no-promise + rebuild-each-minor.
  Requires ADR before 1.0 ships.
- **Storage `ControlQueueRepo` default composition root (ROADMAP M7):**
  switch `simple_server` to PG; document multi-process restart limits in
  release notes.
- **Sandbox capability discovery enforcement (ROADMAP M4):** small
  scope, closes canon §4.5 gap; nice candidate for second sub-project.
- **Engine `expression_engine` dead-code (ROADMAP M1.2):** wire dynamic
  edge conditions OR remove field + downgrade canon §10 claims.
- **`ExecutionEvent` extension pattern (set in M0.3):** extend existing
  variants rather than add new event types when the change is
  in-process-only — keeps subscriber surface small. Reuse for
  M3.5 tracing-context propagation.

## Pointers

- `.ai-factory/ROADMAP.md` — milestones (M0–M10), DoD, out-of-scope.
- `.ai-factory/plans/m0-explicit-termination.md` — current plan
  (Phase 3 ControlAction wiring).
- `.ai-factory/DESCRIPTION.md` — agent-facing project summary.
- `.ai-factory/ARCHITECTURE.md` — agent-actionable architecture subset.
- `.ai-factory/rules/base.md` — distilled coding rules.
- `crates/engine/README.md` — engine "L4 debt" block (see M0.4 — sync
  pending).
- `crates/action/src/result.rs:206-219` — Terminate docstring (M0.4
  cleanup target).
- `crates/execution/src/status.rs:85-94` — termination reason docstring
  (M0.4 cleanup target).
