# Nebula — Production-Ready 1.0 Roadmap

> Strategic milestones to take Nebula from "core stable, engine+API in active
> development" (per `DESCRIPTION.md`) to **production-ready 1.0**. Granular
> tasks live under `paths.plans/<milestone-or-branch>.md` after running
> `/aif-plan full` on each milestone — this file stays a checklist.
>
> **Maintenance:** add a milestone with one-line evidence (file:line) when a
> new gap is discovered; tick it off only after the exit criteria below are
> verifiably met. README-driven product claims (canon §4.5 honesty) override
> roadmap optimism.

## Status Snapshot (2026-05-07)

- **Cross-cutting layer** (`error`, `log`, `eventbus`, `metrics`,
  `resilience`, `system`) — **stable, no pending breaks**. `nebula-telemetry`
  was merged into `nebula-metrics` 2026-05-06 per ADR-0046 (PRs #652–#656);
  the lock-free primitives + label interning now live as intra-crate modules
  inside `nebula-metrics` (no `cargo deny` wrapper for the boundary; the
  former `MetricsAdapter` bridge type was deleted in #654).
- **Core layer** (`core`, `validator`, `expression`, `workflow`, `execution`,
  `schema`, `metadata`) — **stable**. Expression #590 (regex_cache LRU)
  closed in PR #625 via `moka::sync::Cache` migration —
  see `crates/expression/src/eval.rs` (regex_cache field on `Evaluator`).
  `schema` `#[param]` namespace renamed to `#[field]` (M6 Phase 2).
- **Business layer** (`credential`, `resource`, `action`, `plugin`) — **§M6
  + §M11 closed 2026-04-29.** v4 trait shapes shipped: `Action: Sized + type
  Input/Output + static metadata + slot-binding derive + FromWorkflowNode
  factory + ErasedAction dispatch`; `Resource` drops `type Credential` per
  ADR-0044 — resources declare credentials as `#[credential]` slot fields;
  `Credential::Properties` companion struct replaces in-metadata schema.
  All 3 M6.3 examples (Pool / Resident / Service+cross-workflow) ship in
  workspace `examples/`. `resource` plans 06 + 10 + prototypes marked
  SUPERSEDED.
- **Exec layer** — `storage` is production-ready for execution/workflow
  (Layer 1 traits stable, 23 PG migrations + 9 common, CAS, outbox, reclaim).
  `engine` is ~85% — orchestration solid, **§11.5 durability debts closed
  via M0** (budget + workflow_input persistence shipped under #289 / #311;
  explicit-termination wiring landed in M0.3); **§10 conditional-flow
  correctness verified via M1** (skip-propagation tests + dead-field
  cleanup, 2026-04-28); **§11.2 closed via M2.1 layered-retry shipping**
  (action-internal retry stays in `nebula-resilience`; engine-level node
  retry now wired end-to-end via `NodeDefinition.retry_policy` →
  `NodeExecutionState::next_attempt_at` → `WaitingRetry` parking →
  frontier-loop re-dispatch, 2026-04-29); **§11.5 Layer-1 lease
  enforcement closed via M2.2** (heartbeat-driven `lease_holder` /
  `lease_expires_at` fence verified by engine + PG + loom + chaos,
  2026-04-29); **§M6.2 scoped resources storage + lifecycle primitives
  shipped 2026-04-30** (engine frontier-loop wiring deferred per
  `.ai-factory/PHASE7_BLOCKED.md`). Layer 2 (`claimed_by`/`claimed_until`
  + spec-16 row model) remains Sprint E (1.1) scaffolding.
  `sandbox` is correctness-grade; capability discovery enforcement gap (canon
  §4.5).
- **API layer** — routing wired; **2 remaining feature gaps** (tracing
  context propagation §M3.5 and shift-left `validate_workflow` audit
  §M3.6). Auth backend and webhook dispatch closed via PR #638
  (`feat(api): Plane-A auth, slug webhooks, idempotency`, merged
  2026-05-04). **OpenAPI 3.1 closed via M3.2** (`feat/api-openapi-spec`,
  2026-05-07) — utoipa-axum mounts every handler through `routes!()` so
  spec drift is a compile error; Stub Endpoint Policy (ADR-0047) tags
  class-(c) handlers as `deprecated` + 501 so canon §4.5 stays
  mechanically verifiable; full drift / 3.1-validation test suite in
  `crates/api/tests/openapi_*.rs`. **M3.4 closed 2026-05-07
  (`feat/api-m3-4-idempotency`)** — `IdempotencyLayer` mounted in
  `build_app` on api routes only (webhook ingress keeps its own dedup
  contract); ADR-0048 ratifies the hybrid store backend (`InMemory` for
  dev/tests, `Pg` for production); `nebula_api_idempotency_*` metrics
  + tracing span fields land in `crates/metrics/src/naming.rs`; e2e
  tests in `crates/api/tests/idempotency_e2e.rs` exercise the full
  `build_app` stack. Webhook handlers ship; production
  trigger-registration wiring through the storage layer remains a
  separate follow-up flagged in `crates/api/src/routes/webhook.rs:25-28`.
- **GitHub:** 19+ open issues, all p2/p3 needs-triage/discussion. No p0/p1.
  5 open dependabot PRs.

## Definition of Done — production-ready 1.0

A milestone exits only when **all** of these hold for its scope:

- [ ] **Behaviour**: every claim in README / canon / per-crate docs is backed
      by code that exercises the path; no "false capability" per canon §4.5.
- [ ] **Observability** (DoD per `feedback_observability_as_completion.md`):
      typed `thiserror` variant + `tracing` span / event + invariant check on
      every new state, error, or hot path.
- [ ] **Tests**: unit + integration + (where applicable) loom / chaos / fuzz.
      No `#[ignore]` outside benches and intentionally slow chaos suites.
- [ ] **Layer hygiene**: `cargo deny check` green, no new wrapper edges
      without `reason` in `deny.toml`.
- [ ] **Lint / docs**: `cargo clippy --workspace --all-targets -- -D warnings`
      green; `cargo test --workspace --doc` green; rustdoc broken-intra-doc
      links forbid (`-D rustdoc::broken_intra_doc_links`).
- [ ] **Security**: secrets policy (AES-256-GCM + AAD, zeroize-on-Drop,
      redacted Debug) holds end-to-end; CODEOWNERS gate on credential /
      webhook paths (already enforced).
- [ ] **CI parity**: `lefthook pre-push` mirrors required CI jobs (per
      `feedback_lefthook_mirrors_ci.md`).

## Milestones

### M0 — Engine durability debts (canon §11.5)

**Why first.** Without these, "resume" and "replay" claims in canon are false.
Pure data debt, no architectural rework needed.

- [x] **M0.1** ~~Persist `ExecutionBudget`~~ — **DONE** (verified 2026-04-28).
      Field `budget: Option<ExecutionBudget>` lives in
      `crates/execution/src/state.rs:170` (issue #289 closed); persisted at
      `engine.rs:677, 1003`; restored at `engine.rs:1433-1444`. Tests:
      `resume_restores_persisted_budget` (engine.rs:6637-6717),
      `resume_falls_back_to_default_budget_on_legacy_state` (6723).
      Migration `00000000000009_add_resume_persistence.sql`.
- [x] **M0.2** ~~Persist original workflow input~~ — **DONE** (verified
      2026-04-28). Field `workflow_input: Option<Value>` in
      `crates/execution/src/state.rs:158` (issue #311 closed); persisted at
      `engine.rs:674, 998`; restored at `engine.rs:1487-1497`. Test:
      `resume_restores_original_workflow_input` (engine.rs:6583).
- [x] **M0.3** ~~Wire `ExecutionTerminationReason::ExplicitStop` /
      `ExplicitFail`~~ — **DONE** (closed 2026-04-28).
      `set_terminated_by` at `state.rs:240`; engine wiring at
      `engine.rs:1986-area`; `determine_final_status` priority ladder at
      `engine.rs:3590`; surfaced via `ExecutionResult.termination_reason`
      and `ExecutionEvent::ExecutionFinished.termination_reason`.
      11 tests cover the priority-ladder branches + state setter + serde
      compat.
- [x] **M0.4** ~~Sync stale debt docs~~ — **DONE** (closed 2026-04-28).
      `engine/README.md` L4 debt block updated (M0.1/M0.2/M0.3 moved to
      "Recently closed debts" table); `action/src/result.rs:206-219` and
      `execution/src/status.rs:85-94` docstrings rewritten to describe
      shipped wiring; workspace-wide Grep for "Phase 3 ControlAction" /
      "not yet wired" / "v1 wiring status" returns 0 hits in M0 scope
      (remaining hits are unrelated: sandbox capability discovery (M4),
      WebSocket endpoint (1.1 deferred)).

**Exit:** M0.3 closes the Phase-3 ControlAction-plan termination wiring with
test (`Terminate` → `ExplicitStop` round-trips through `ExecutionResult` and
`ExecutionEvent`); M0.4 brings README/canon back in sync.

### M1 — Engine correctness verification + cleanup (canon §10)

**Why re-scoped.** The original M1.1 entry described a "local-edge gating"
defect that recon (2026-04-28) showed didn't exist — `propagate_skip`
(engine.rs:3267-3313) was already full-graph recursive via the
`resolved == required && activated == 0` ladder. M1.2 option A ("wire
dynamic edge conditions") contradicted Spec 28 §2.2 which already settled
conditional routing via explicit `ControlAction` nodes. Re-scoped to
verification + dead-field cleanup + doc audit.

- [x] **M1.1** ~~Verify full-graph skip propagation in non-trivial
      topologies~~ — **DONE** (closed 2026-04-28). Added 5 integration tests
      covering transitive 3-hop chain, diamond with one skipped branch,
      mixed-source aggregate, all-sources-skipped aggregate, and multi-hop
      skip with sibling activation (`crates/engine/tests/integration.rs`).
      All pass on the existing `propagate_skip` recursion.
- [x] **M1.2** ~~Remove dead `WorkflowEngine.expression_engine` field~~ —
      **DONE** (closed 2026-04-28). Field at `engine.rs:125-130` (annotated
      `#[expect(dead_code)]`) removed; the shared `Arc<ExpressionEngine>`
      lives in `ParamResolver` (the only consumer, used for parameter
      expression / template resolution). Spec 28 §2.2 already settled the
      conditional-routing question via `ControlAction` nodes — there is no
      engine-level edge expression to evaluate.
- [x] **M1.3** ~~Sync canon §10 / docs with Spec 28 §2.2 port-driven
      routing~~ — **DONE** (closed 2026-04-28). Updated
      `crates/workflow/README.md` Public API section to describe `Connection`
      as a pure wire (no `EdgeCondition` / `ResultMatcher` / `ErrorMatcher`);
      added stale-doc warning + drift table to
      `crates/workflow/docs/Architecture.md` (880-line pre-Spec-28 planning
      doc). `connection.rs` and `builder.rs` already frame the removed
      types as historical context (verified).

**Exit:** skip-propagation correctness verified by tests; no
`#[expect(dead_code)]` in engine; docs match shipping code.

### M2 — Engine retry semantics + node attempts

- [x] **M2.1** ~~Decide engine-retry direction for 1.0~~ — **DONE** (closed
      2026-04-29 via the layered-retry exit per ADR-0042). Two retry
      surfaces, disjoint by trigger boundary:
      - **Layer 1 — action-internal** (`nebula-resilience::retry_with`)
        stays in action source code for in-call recoverable failures.
      - **Layer 2 — engine-level node retry**
        (`NodeDefinition.retry_policy`) is now real end-to-end:
        `NodeExecutionState::next_attempt_at` parks the node in the
        `NodeState::WaitingRetry` state (added in M2.1 T2), the
        frontier loop's retry-pending min-heap re-dispatches at the
        scheduled time, and `ExecutionBudget.max_total_retries`
        provides a global cap (canon §11.2). Cancel/terminate/budget
        guards drain parked retries to `Cancelled` without
        re-dispatching.
      - Sequencing across two PRs:
        **PR #627 (foundation, 2026-04-28)** — T0 dropped
        `ActionResult::Retry` + `unstable-retry-scheduler`; T1 landed
        ADR-0042; T8 added shift-left `validate_workflow` for
        `RetryConfig` (rejects `max_attempts=0`, non-finite multiplier,
        `max_delay < initial_delay`, etc.).
        **PR (this one, 2026-04-29)** — T2 added `next_attempt_at` +
        `total_retries` + `WaitingRetry` (forward-compat via serde
        defaults); T3 verified Layer-1 storage is JSONB-only so no
        column migration is required; T4 wired engine retry decision
        (per-node policy → workflow default → budget cap, with
        `NodeAttempt` push for idempotency-key differentiation); T5
        wired the frontier loop's `retry_heap` + `tokio::select!`
        across join_set / cancel / wall-clock / retry-timer; T6
        landed 9 integration tests covering core path + cancel +
        terminate + budget + idempotency + per-node-vs-workflow
        resolution + one-shot fallback. Total: ~146 unit-test deltas
        + 9 integration tests, all green.
- [x] **M2.2** ~~Verify `execution_leases` heartbeat enforcement across runner
      restarts~~ — **DONE** (closed 2026-04-29, Layer 1 only — Layer 2
      remains Sprint E (1.1) per "Out of scope for 1.0").
      Sequencing across five commits on
      `feature/m2-2-execution-leases-heartbeat`:
      - **Commit 1** (`309e773c`) — T0 verification + T1' Sprint-E
        boundary doc-comments on `repos/execution.rs` and migration
        0011 SQL files; T3 in-memory engine takeover test.
      - **Commit 2** (`45d79088`) — T4 cancel-redeliver across runner
        restart (control-queue reclaim sweep with engine_b's
        `EngineControlDispatch`); T5 `replay_execution` lease-less
        invariant + doc-comment.
      - **Commit 3** (`7e6ee685`) — T6 PG `acquire_lease`/`renew_lease`/
        `release_lease` lifecycle tests (7 cases) + T7 PG multi-runner
        takeover test, all DATABASE_URL-gated and skipping silently
        when not set.
      - **Commit 4** (`d37c88fe`) — T8 loom probe `lease_handoff` (3
        exhaustive-schedule tests) + T9 InMemoryExecutionRepo chaos
        test (high-contention holder-uniqueness, `#[ignore]` by default).
      - **Commit 5** (this one) — T10 storage-layer tracing sweep
        (PgExecutionRepo + InMemoryExecutionRepo parity) + T11/T12
        documentation (durability matrix split into Layer 1 enforced
        vs Layer 2 Sprint E scaffolding) + T13 ROADMAP closure +
        T14 lefthook parity check.
- [x] **M2.2 — Original T1+T2 (drop legacy schema/trait) superseded.**
      T0 verification revealed that `repos/execution.rs` and migration
      0011 lease columns are Sprint E (1.1) Layer-2 scaffolding per
      `crates/storage/src/lib.rs:16-30, 65-87`, not legacy. T1' (add
      Sprint-E boundary doc-comments) replaced the planned drops; the
      `feat!` marker on the closer commit was downgraded to
      `feat(engine):` because no breaking change shipped.

**Exit:** retry path is real with tests — **closed 2026-04-29 via
ADR-0042 layered-retry exit.** Workflow authors get the operator-level
retry policy that canon §4.5 used to claim falsely; action authors keep
`nebula-resilience::retry_with` for in-call retries. §11.2 now reads as
"two layers, disjoint by trigger boundary: in-call vs post-finalisation."

### M3 — API surface completion

The largest 1.0 area. Six blocks; can be parallelized once unblocked.

**1.0 closure criteria** (apply on top of the global DoD checklist):

- Every shipping route reachable through `build_app` — no
  «middleware crate-local, but not mounted» states.
- OpenAPI 3.1 spec is the authoritative contract; SDK + doc UI
  consume it as a single source.
- Distributed-trace context flows API request → engine span → action
  `Context` → resource calls (W3C Trace Context).
- Every state-changing endpoint replay-protected; the dedup store
  survives process restart in production deployments.
- Every `/execute` path runs `validate_workflow` before engine
  dispatch (no «engine validates then explodes» surprises).
- Production `AuthBackend` ships (not just the in-memory dev impl).

#### M3.1 Auth backend

- [x] Plane-A handlers shipped (PR #638): signup / login / logout /
      forgot-password / reset-password / verify-email / mfa-enroll /
      mfa-verify / oauth-start / oauth-callback in
      `crates/api/src/handlers/auth.rs`.
- [x] PAT lookup wired in `middleware/auth.rs:145-149`
      (`backend.lookup_pat`).
- [x] Session store wired through `AuthBackend`
      (`create_session` / `get_principal_by_session` / `revoke_session`).
- [x] All 10 routes mounted in `routes/auth.rs:13-28`.
- [ ] **Production `AuthBackend` impl.** The live backend today is
      `crates/api/src/auth/in_memory.rs` — usable for dev but loses
      state on restart. Ship a PG-backed `AuthBackend` (users,
      credentials hash, sessions, PATs, MFA secrets, OAuth state).
- [ ] **CSRF enforcement** for state-changing endpoints under `/auth/*`
      and `/api/v1/*` — the `csrf_cookie` is issued, but no middleware
      validates the matching `X-CSRF-Token` header.
- [ ] **OAuth providers loaded from operator secrets.** Today provider
      configs live in the in-memory backend. 1.0 needs a registry that
      pulls secrets from the credential store at startup.
- [ ] **Lockout + rate-limit integration tests** against the PG backend
      (in-memory tests already cover the unit-level path).
- [ ] **Observability:** per-handler `tracing::Span` (already present),
      typed `AuthError` → `ApiError` mapping audit, and a
      `nebula_api_auth_*` metrics family for failed/locked-out
      attempts.

#### M3.2 OpenAPI 3.1 spec generation — ✅ CLOSED 2026-05-07

- [x] Replace the `openapi.rs:9-10` stub with a real generator —
      utoipa 5.5 + utoipa-axum 0.2 (ADR-0047).
- [x] Annotate every handler with request/response schemas (auth,
      webhook, execution, workflow, credential, resource, me, org,
      catalog, system).
- [x] Spec served at `GET /api/v1/openapi.json` matches the actual
      route table at runtime — `OpenApiRouter::routes(routes!(...))`
      is the only mounting path, so handlers without `#[utoipa::path]`
      fail to compile (structural drift detection).
- [x] Swagger UI mounted at `GET /api/v1/docs` (handler returns the
      self-contained UI shell that fetches `/api/v1/openapi.json`).
- [x] Integration test: spec validates against the OpenAPI 3.1 schema
      via `oas3::OpenApiV3Spec` round-trip in
      `crates/api/tests/openapi_spec.rs`.
- [x] CI lint: every router-mounted route appears in the generated
      spec — drift smoke + operationId uniqueness + `$ref` resolution
      + canon §4.5 stub-honesty gate
      (`tests/openapi_canon_compliance.rs`).

#### M3.3 Webhook handler dispatch ✅ closed 2026-05-07 (ADR-0049)

ADR-0049 records the convergence: the legacy slug pipeline
(`WebhookDispatcher` + `NoopSink`) is **deleted**; both URL shapes —
programmatic `(uuid, nonce)` and operator-configured
`(org, ws, slug)` — funnel through `WebhookTransport::dispatch_inner`.

- [x] **Single dispatch pipe.** `crates/api/src/webhook/*`,
      `routes/webhook.rs`, `handlers/webhook.rs`, and
      `middleware/webhook_ratelimit.rs` removed. Both URL shapes go
      through `WebhookTransport`'s `dispatch_inner`.
- [x] **Storage bootstrap.** `bootstrap_webhook_activations` walks
      `WebhookActivationRepo::list_active()` at startup and registers
      each as `transport.activate_slug(...)`. PG impl uses the
      `webhook_path` indexed column (migration 0018) for O(1)
      reverse lookup; migration 0025 documents the kind-namespaced
      `triggers.config` JSONB contract.
- [x] **Replay window enforcement.** `RequiredPolicy.replay_window`
      (default 5 min) consumed by `RequiredPolicy::verify_with` —
      shared between the programmatic and slug paths.
      `MockClock` drives deterministic tests.
- [x] **Per-key rate-limit / abuse protection.** Transport's
      sliding-window limiter buckets by `WebhookKey`. Flooding one
      slug does not affect another; LRU-capped path table
      (#271 mitigation).
- [x] **Provider-specific URL-verification challenges.** Slack
      `url_verification`, Stripe `pending_webhook` ping, Generic
      `?challenge=…` GET — all handled in `WebhookAction::pre_handle`
      hooks (`crates/action/src/webhook/providers/`).
- [x] **Observability.** `NEBULA_WEBHOOK_*` namespace extended
      (G1 + G2): replay-rejection / rate-limit-rejection / bootstrap
      counters + label vocabulary + Prometheus HELP. Cardinality
      budget documented; `tenant_id` and `webhook_key_kind` are the
      bounded label keys. Per-outcome `REQUESTS_TOTAL` +
      `LATENCY_SECONDS` histogram and the cardinality regression
      test land as a 1.0 follow-up (ADR-0049 § "Out of scope").
- [x] **Lifecycle subscriber.** `TriggerLifecycleEvent` consumer
      ships; producer-side wiring deferred (ADR-0049 § "Out of
      scope").
- [x] **Admin reload.** `POST /internal/v1/webhooks/reload` swaps the
      slug map atomically via `transport.replace_slug_map`. Internal
      auth via `X-Internal-Token`.

#### M3.4 Idempotency-Key dedup

- [x] `IdempotencyLayer` middleware shipped (PR #638) — full impl:
      `Idempotency-Key` header, `moka` cache, 24h TTL, 10k entries,
      SHA-256 body fingerprint, 422 on body mismatch, only 2xx/4xx
      cached.
- [x] `crates/api/tests/idempotency_middleware.rs` covers core paths
      against minimal routers.
- [x] **Mount `IdempotencyLayer` in `crate::app::build_app`** — wired on
      api routes BEFORE the webhook transport merge (webhook ingress has its
      own dedup contract per ROADMAP §M3.3). `crates/api/src/app.rs`.
- [x] **Shared-store decision (in-memory vs PG-backed) + ADR.**
      ADR-0048 — hybrid backend: `InMemoryIdempotencyStore` for dev/tests,
      `StorageBackedIdempotencyStore<PgIdempotencyStore>` for production.
      Selection via `ApiConfig.idempotency.backend` (`API_IDEMPOTENCY_BACKEND`).
      `docs/adr/0048-idempotency-store-backend.md`.
- [x] **PG-backed: migration + repo trait + concurrency tests.** Trait
      `IdempotencyStoreRepo` in `crates/storage/src/repos/idempotency.rs`;
      impl `PgIdempotencyStore` in `crates/storage/src/pg/idempotency.rs`;
      migration `0024_add_idempotency_dedup.sql` (PG + SQLite parity);
      `DATABASE_URL`-gated tests in `crates/storage/tests/pg_idempotency.rs`
      (round-trip, concurrent first-writer-wins, body-mismatch race, TTL).
- [x] **End-to-end integration test** against the real `build_app` router
      (not minimal test routers) covering replay + body-mismatch + 5xx
      bypass + per-principal scope. `crates/api/tests/idempotency_e2e.rs`.
- [x] **Observability:** `nebula_api_idempotency_{hits,misses,rejects,store_saturation_ppm,latency_ms}`
      live in `crates/metrics/src/naming.rs`; middleware records them on
      every outcome branch. Tracing span carries `cache_key_prefix`,
      `identity_prefix`, `body_size_bytes`, `outcome`.

#### M3.5 Tracing context propagation

- [x] Adopt W3C Trace Context on the HTTP edge: parse `traceparent` /
      `tracestate` in `crates/api/src/middleware/trace_w3c.rs` (request
      ID middleware remains `X-Request-ID` only).
- [x] Attach extracted span context to the per-request `tracing::Span`
      (`TraceLayer` + `tracing_opentelemetry`; see ADR-0050). API binaries
      install the required `OpenTelemetryLayer` via
      `nebula_api::init_api_telemetry` so the attach is not a no-op without
      OTLP.
- [ ] Propagate span context into engine via `ExecutionContext` for
      every synchronous API→engine touchpoint (queue row stamping
      ships today; field on `ExecutionContext` exists for downstream).
- [x] Engine / action boundary: `ActionRuntimeContext` exposes
      `resource_http_request_span` / `instrument_resource_http_request`
      for outbound resource HTTP (callers wrap `nebula-resilience`).
- [x] Emit `traceparent` / `tracestate` on responses where policy allows
      (CORS allow/expose aligned).
- [ ] Integration test: full stack API → engine → action → resource with
      one root span (engine `control_trace` unit test + lease_takeover
      cancel path cover subsets today).
- [ ] OpenTelemetry exporter wired (depends on M9.2 verification).

#### M3.6 Shift-left workflow validation

- [ ] Audit every `/execute` and `/workflows/{id}/run` handler — call
      `validate_workflow` before engine handoff per
      `crates/workflow/README.md:82-84` contract. Today
      `validate_workflow` is referenced in `handlers/workflow.rs`,
      `routes/workflow.rs`, and integration tests — coverage map is
      unknown.
- [ ] Encode «must validate before dispatch» as a typed boundary —
      e.g. `validated::ValidatedWorkflow` newtype that the engine
      `Submit` API requires (impossible to call without validating).
- [ ] Map `WorkflowValidationError` to `400 Bad Request` with
      field-level error details in problem+json body.
- [ ] Integration test: malformed workflow returns 400 before any
      engine state mutation.
- [ ] Lint or CI test that catches a future handler skipping
      `validate_workflow` (dead-code-style detection or grep gate).

**Exit:** every shipping route reachable from `build_app`; every
state-changing endpoint replay-protected end-to-end; OpenAPI spec is
the authoritative contract; trace context flows API → engine → action
→ resource; production-grade `AuthBackend` ships; no `/execute` path
bypasses validation.

### M4 — Sandbox capability discovery enforcement

**1.0 closure criteria** (apply on top of the global DoD checklist):

- The §4.5 «false capability» disclaimer at `crates/sandbox/src/lib.rs:20-21`
  is removed because the gap is closed, not deprioritized.
- Capability mismatches reject registration before any sandbox spawn
  — never «discovered then errored after spawn».
- Sandbox README Appendix TODO closed; canon §12.6 honesty
  preserved (in-process is correctness-only; child-process is the
  isolation boundary).
- OS-sandbox primitives (`os_sandbox` module) document what is and
  is not enforced per platform; release notes flag the partial paths.

#### M4.1 Discovery validation

- [ ] Parse `plugin.toml` `[capabilities]` table at registration in
      `crates/sandbox/src/discovery.rs` (today `discovery` exists but
      capability comparison does not run).
- [ ] Compare manifest declarations against runtime
      `PluginCapabilities` returned by the loaded binary.
- [ ] Typed `SandboxError::CapabilityMismatch { declared, runtime }`
      variant + `tracing` event with both sets at WARN level.
- [ ] Mismatch rejects registration **before** any spawn; existing
      `discovered_plugin::DiscoveredPlugin` API surfaces the error.
- [ ] Integration test: malformed `plugin.toml` → typed reject.
- [ ] Integration test: runtime claims a capability missing from
      manifest → reject.
- [ ] Integration test: runtime declares **subset** of manifest → allow.
- [ ] Sandbox README Appendix entry rewritten to describe shipping
      enforcement; «false capability» disclaimer at `lib.rs:20-21`
      removed.
- [ ] §4.5 operational-honesty audit: workspace-wide grep for
      «capability allowlist» / «false capability» returns 0 hits in
      sandbox scope.

#### M4.2 OS-sandbox primitives parity (1.0 honesty)

- [ ] Audit `crates/sandbox/src/os_sandbox/` per platform (Linux
      seccomp / namespace; macOS sandbox-exec; Windows job objects)
      — document what is shipping vs «best-effort, partial» per
      `lib.rs:23`.
- [ ] Each primitive has a typed enable/disable switch + tracing
      observation + per-platform integration test (or explicit
      «not supported» marker).
- [ ] Release notes section: «what sandbox isolation guarantees on
      each platform».

**Exit:** §M4.1 closes the registration-time enforcement loop; §M4.2
brings OS-sandbox honesty to 1.0 grade. Sandbox README Appendix TODO
closed; canon §4.5 sandbox grep returns 0 hits; integration tests
cover the four reject/allow paths.

### M5 — Plugin ABI + Engine-Plugin contract

Decision point first; coding task only after the ADR lands.

**1.0 closure criteria** (apply on top of the global DoD checklist):

- The decision is captured in an accepted ADR — not in a per-PR
  description or an issue thread.
- Plugin loader behaviour matches the ADR: either validates
  `nebula_version` at load time, or rejects the field as unknown.
- Plugin authors can answer «what versions of nebula does my
  plugin support» from `plugin.toml` + plugin-sdk README alone.
- Migration story for the in-tree first-party plugins
  (`crates/credential-builtin`, future built-ins) documented.

#### M5.1 ABI-promise ADR

- [ ] Draft ADR-NNNN: **A** (engine semver bound in manifest +
      deprecation policy) vs **B** (no ABI promise; community plugins
      rebuild per engine minor).
- [ ] Cost / benefit table: maintenance burden, plugin author DX,
      engine refactor freedom (per `feedback_hard_breaking_changes`
      «expert-level architecture, not junior patches»).
- [ ] Review by maintainer + acceptance.
- [ ] ADR linked from `crates/plugin-sdk/README.md`.

#### M5.2 Loader behaviour to match the ADR

- [ ] **If A:** parse `nebula_version` semver requirement from
      `plugin.toml` at load time in `crates/plugin-sdk/src/loader.rs`
      (or equivalent).
- [ ] **If A:** typed `PluginLoadError::EngineVersionMismatch` +
      `tracing` event + integration test covering 3 cases (compatible,
      patch-mismatch, major-mismatch).
- [ ] **If A:** deprecation policy section in plugin-sdk README
      («N-1 minor support window» or similar).
- [ ] **If B:** plugin.toml schema rejects `nebula_version` as
      unrecognized field with a typed error pointing at the ADR.
- [ ] **If B:** plugin-sdk README has a «1.0 ABI policy» section
      that says «rebuild every minor» with rationale.

#### M5.3 In-tree plugin migration

- [ ] Audit `crates/credential-builtin` and any other first-party
      plugins for ABI assumptions; align with chosen path.
- [ ] Each plugin `Cargo.toml` references the engine version per
      ADR convention.

**Exit:** ADR accepted; plugin-sdk README + `plugin.toml` schema
reflect the choice; loader behaviour matches; integration tests
cover the version-mismatch paths chosen by the ADR.

### M6 — Resource layer finalization

- [x] **M6.1** ~~Plan **06-action-integration**~~ — **DONE** (closed
      2026-04-29 via M6 + §M11 cascade Phases 1-6). Slot-binding pattern
      lands typed `ResourceAction` wiring through `#[derive(Action)]` +
      `FromWorkflowNode` factory + typed `ctx.acquire_resource_by_id::<R>`
      / `ctx.resolve_credential_by_id::<C>` helpers. Evidence:
      `crates/action/src/from_workflow_node.rs`,
      `crates/action/src/context.rs:675-705` (typed acquire),
      `crates/action/src/context.rs:722-745` (typed resolve),
      `crates/engine/src/runtime/runtime.rs::dispatch_action` (factory
      registry path), `crates/engine/src/scoped_resources.rs`
      (closest-ancestor scoped→global fallback). Plan
      `crates/resource/plans/06-action-integration.md` marked SUPERSEDED.
- [x] **M6.2** ~~Plan **10-scoped-resources**~~ — **DONE** (closed
      2026-04-29 via Phase 7). `DashScopedResourceMap` per-branch storage
      with closest-ancestor walk + cycle defense; `CleanupOutcome` typed
      enum + `ExecutionEvent::ScopedResourceCleanupTimeout` + 30s default
      cleanup timeout; 17 integration tests cover the
      `crates/resource/plans/10-scoped-resources.md` use-case matrix.
      **Storage + lifecycle primitives ship complete; engine
      frontier-loop wiring (`ResourceAction::configure` /
      `cleanup` per-branch dispatch) is deferred per
      `.ai-factory/PHASE7_BLOCKED.md`** — depends on a branch-tree
      dominator analysis that exceeds the M6 budget. Evidence:
      `crates/engine/src/scoped_resources.rs` (full module),
      `crates/engine/tests/scoped_resources.rs` (17 tests),
      `crates/engine/src/event.rs::ScopedResourceCleanupTimeout`. Plan
      `crates/resource/plans/10-scoped-resources.md` marked SUPERSEDED.
- [x] **M6.3** ~~Move `resource-prototypes`~~ — **DONE** (closed
      2026-04-29 via Phase 10). 3 runnable examples ship in
      `examples/examples/` covering Pool / Resident / Service
      (cross-workflow) topologies; `resource-prototypes.md` marked
      SUPERSEDED with topology selection guidance distilled into
      `crates/resource/docs/topology-reference.md`. Evidence:
      `examples/examples/m6_postgres_pool.rs`,
      `examples/examples/m6_resident_http.rs`,
      `examples/examples/m6_telegram_multi_workflow.rs`. Run via
      `cargo run -p nebula-examples --example m6_*`.
- [ ] **M6.4** *(deferred — candidate)* `EventTrigger` DX wrapper around
      `nebula_engine::daemon::EventSource` + `TriggerAction` per ADR-0045.
      Phase 10 Telegram example uses raw `EventSource` directly. Wrapper
      DX is post-M6 candidate work; no commitment.

**Exit:** §M6.1, §M6.2, §M6.3 DONE; §M6.4 deferred per ADR-0045 with
explicit candidate marker. **M6 closes 2026-04-29.** Per-slot
credential rotation reverse-index + fan-out subsystem (originally part
of §M6.2) is split out as candidate §M11.5 per
`.ai-factory/PHASE4_BLOCKED.md`.

### M7 — Storage operationalization

**1.0 closure criteria** (apply on top of the global DoD checklist):

- A production composition root exists and uses PG-backed repos by
  default; in-memory impls remain for dev/tests, never as a quiet
  prod fallback.
- Multi-process deployment limits + Sprint-E deferrals captured in
  1.0 release notes (so operators know what they get vs what's deferred).
- Loom probe runs nightly green; failures gate the 1.0 tag.

#### M7.1 PgControlQueueRepo as default composition root

- [ ] Identify or create the production composition root (today
      `crates/api/examples/simple_server.rs:25-30` explicitly defers
      this — it pulls in-memory only). Decide where it lives
      (`nebula-cli` vs a dedicated `crates/server` vs lifted into
      `nebula-api`).
- [ ] Composition root accepts a `ControlQueueRepo` impl from config
      and wires `PgControlQueueRepo` by default.
- [ ] `simple_server.rs` keeps the in-memory path for dev with an
      explicit `--queue=memory` opt-in (and a startup warning).
- [ ] Config schema (`crate::config::ApiConfig` or successor)
      surfaces queue backend selection + connection-string source.
- [ ] Restart-survival integration test: spawn → enqueue cancel →
      kill engine → respawn → cancel processes (currently lost on
      restart per Status Snapshot).
- [ ] Operator doc: «in-memory queue is dev-only» banner in
      `crates/storage/README.md` + release notes.
- [ ] `task db:migrate` includes the control-queue migration by
      default (verify the queue table is in 23 PG migrations).

#### M7.2 Multi-process deployment limits documentation

- [ ] Release-notes section: «Nebula 1.0 supports multi-runner
      deployment with PG; what's enforced (Layer 1 lease) vs what's
      Sprint-E (Layer 2 spec-16 row model)».
- [ ] Operator runbook: detecting + recovering from runner restart
      (lease expiry, takeover, in-flight command replay).
- [ ] Empirical scale-out numbers: max concurrent runners, throughput
      ceiling per PG instance, lease renewal frequency tradeoffs
      (with reproducible benchmark scripts under `benches/`).
- [ ] `crates/storage/src/lib.rs` doc-comment block points operators
      at the runbook + release notes.

#### M7.3 Loom probe nightly CI

- [ ] Add `.github/workflows/loom-nightly.yml` (cron + `workflow_dispatch`).
- [ ] Run `cargo test -p nebula-storage-loom-probe --features loom`
      against pinned toolchain.
- [ ] Failure surfaces as required check (or escalates to issue auto-create
      after 3 consecutive red runs).
- [ ] CODEOWNERS notification on storage-path failures.
- [ ] Docs entry: how to reproduce locally + how to debug a loom red.
- [ ] Update `.ai-factory/rules/base.md` (if it covers CI) to reference
      the nightly contract.

**Exit:** prod deployments wired through `PgControlQueueRepo` by default;
restart-survival integration test green; nightly Loom job runs and
gates 1.0 tag; operator runbook + release notes cover Layer-2 deferrals
honestly.

### M8 — Engine concurrency verification

**1.0 closure criteria** (apply on top of the global DoD checklist):

- Engine runs under `cfg(loom)` exhaustive-schedule coverage for
  the three concurrency seams that have historically caused bugs
  (lease handoff, registry mutate, cancel-token handoff).
- DashMap loom-hostility resolved either via `cfg(loom)` substitute
  or by extracting a lock-free struct that loom can model.
- Property tests cover lease fence + registration nonce invariants
  beyond what example-based tests verify.
- Nightly runs gate the 1.0 tag (shared with M7.3).

#### M8.1 Loom in nebula-engine

- [ ] Add `loom` feature flag to `crates/engine/Cargo.toml` mirroring
      `nebula-storage-loom-probe` setup.
- [ ] Substitute `DashMap` under `cfg(loom)` (either with a
      loom-friendly map or extract the call-site into a single-writer
      struct that loom can model exhaustively).
- [ ] Loom probe: lease renewal across simulated runner takeover
      (mirror M2.2 Layer-1 path under exhaustive scheduling).
- [ ] Loom probe: `running_registry` insert/remove during cancel
      (per `engine.rs:196-251` — the historically subtle path).
- [ ] Loom probe: cancel-token handoff (parent → child cancellation
      under arbitrary scheduling).
- [ ] Each probe also runs in M7.3 nightly CI.

#### M8.2 Property tests for invariants

- [ ] Property test: lease fence (`lease_holder` + `lease_expires_at`)
      — never two concurrent holders for the same execution.
- [ ] Property test: registration nonce — duplicate registration
      with the same nonce rejected; different nonce accepted.
- [ ] Property test (third candidate, TBD on runtime audit): retry
      policy decisions deterministic under shuffled wall-clock
      (M2.1 retry path).
- [ ] Tests run under `task dev:check` if fast enough; otherwise
      under nightly with a clear escape-hatch flag.

**Exit:** loom suite green under nightly; property tests cover the
three invariants; multi-runner deployments have model-checked
concurrency contracts.

### M9 — Observability + DoD audit pass

**1.0 closure criteria** (apply on top of the global DoD checklist):

- Every hot-path boundary in the workspace has the
  typed-error + tracing-span + invariant-check triple per
  `feedback_observability_as_completion`. Gaps are filed and closed
  before tag, not deferred to «we'll add metrics later».
- OpenTelemetry exporter end-to-end verified against a real OTLP
  collector (`task obs:up`) — no «config exists, never tested» state.
- The two known mutex hot-paths (#595 / #590) either have
  proven low contention with a recorded measurement, or are
  refactored to lock-free.

#### M9.1 Hot-path observability sweep

- [ ] Inventory hot-path boundaries: engine state transitions, control
      dispatch, sandbox spawn, storage CAS retries, retry-loop entry,
      lease acquire/renew, action dispatch, expression evaluation
      hot path, eventbus publish/subscribe.
- [ ] For each boundary verify: typed `thiserror` variant present
      (or extend the parent error enum with a new variant rather than
      `String` strings).
- [ ] For each boundary verify: `tracing::Span` with structured
      fields (no string interpolation) or `tracing::event!` at the
      right level (DEBUG for hot, INFO for state changes, WARN for
      degradations, ERROR for invariant breaks).
- [ ] For each boundary verify: invariant check (`debug_assert!` /
      `eyre::ensure!` / typed-error early return) so silent corruption
      is impossible.
- [ ] Per-crate gap report committed under `crates/<crate>/docs/observability.md`
      (or appended to existing crate README observability section).
- [ ] Track each gap as a sub-PR linked to this milestone; no «we'll
      file an issue later».
- [ ] Verification gate: workspace-wide `clippy::missing_errors_doc`
      and an audit checklist (manual + grep heuristics for `String` in
      error variants, missing `#[instrument]`, missing
      `debug_assert!` near lock acquisitions).

#### M9.2 OpenTelemetry bridge verification

- [ ] Read #598 issue + comments to capture the open question.
- [ ] Inventory current OTLP setup in `nebula-metrics` and `nebula-log`:
      what's implemented vs what's documented in
      `crates/api/docs/REST_API_AXUM_GUIDE.md`.
- [ ] If bridge missing: implement OTLP exporter (metrics + traces)
      wired into the `MetricsRegistry` snapshot path.
- [ ] Integration test: trace + metrics flow into a local OTLP
      collector started via `task obs:up`, verified via Jaeger UI
      probe in test or via OTLP collector debug output.
- [ ] Cross-dependency: M3.5 trace-context propagation provides the
      span tree this exporter ships.
- [ ] Operator doc: how to point `OTEL_EXPORTER_OTLP_ENDPOINT` at a
      real collector + what fields appear (with worked example).

#### M9.3 Hot-path Mutex audit

- [ ] **#595 (metrics OTLP label allocation):** measure allocation
      cost on the metrics export path; if hot, switch to
      pre-interned `LabelKey` pool or arena allocation. If cold,
      mark Out of Scope with the measurement attached.
- [ ] **#590 (expression regex_cache Mutex):** verify the moka
      migration (PR #625) actually closed the contention — add a
      stress test that exercises concurrent regex compilation. Update
      Status Snapshot if regression found.
- [ ] For each #issue: comment with measurement + decision; close the
      issue or move to 1.1 with explicit rationale.

- [x] **M9.4** ~~Metrics / telemetry crate boundary~~ — **DONE**
      (closed 2026-05-06 via ADR-0046, PRs #652–#656). The structurally
      unenforced `nebula-telemetry` ↔ `nebula-metrics` cross-crate
      boundary (canon `[L1-§3.10]`) was replaced with intra-crate module
      discipline inside `nebula-metrics`: primitives (`Counter`/`Gauge`/
      `Histogram`/`MetricsRegistry`/`LabelInterner`) absorbed via #653,
      `MetricsAdapter` bridge type deleted via #654, re-audit findings
      and cardinality / HELP-catalog quick wins closed via #655 and #656.
      Exporter + `nebula_*` naming + `LabelAllowlist` continue to live
      in the same crate. Intra-doc rustdoc links repaired across the
      merged tree.

**Exit:** observability gap report committed and gaps closed; OTLP
exporter verified end-to-end against `task obs:up`; the three mutex
hot-paths each carry a measurement + decision; spans/metrics/errors
triple present at every new boundary.

### M10 — Documentation + DX + release process

**1.0 closure criteria** (apply on top of the global DoD checklist):

- A new contributor can build, test, and ship a plugin starting from
  `README.md` + `examples/` alone — no «ask the maintainer» step.
- The release procedure is documented end-to-end (tag → publish →
  announce) and either fully manual with a runbook OR ships a
  minimal automation; mixed states are out.
- `lefthook pre-push` mirrors every CI required job — no per-PR
  surprise where green-locally turns red on the runner.
- `cargo doc --no-deps --workspace` is warning-free and broken
  intra-doc links forbidden in CI.

#### M10.1 Root `examples/` workspace member

- [ ] `examples/workflow_action/` — minimal end-to-end: define an
      action, register it, run a workflow that uses it; output
      compared against expected.
- [ ] `examples/credential/` — declare + register a credential
      type; round-trip encryption + AAD binding + zeroize-on-Drop
      with a runnable assertion.
- [ ] `examples/plugin/` — third-party-style plugin built against
      `nebula-plugin-sdk` only (no internal imports); load + invoke
      from a host driver.
- [ ] `examples/resource_topology/` — Pool / Resident / Service
      topologies (M6.3 `m6_postgres_pool` / `m6_resident_http` /
      `m6_telegram_multi_workflow` already cover these — promote /
      polish their READMEs).
- [ ] Each example has a top-level README with a runnable command
      (`cargo run -p nebula-examples --example <name>`) and the
      expected output snippet.
- [ ] CI: `task examples:check` builds + runs every example.

#### M10.2 Per-crate README quality pass

- [ ] Audit each `crates/*/README.md` against a shared template
      (Purpose / Public API / Usage / Status / Invariants / Related
      ADRs). Today templates are inconsistent across the workspace.
- [ ] Compile-checked examples in doctests for every public-API
      crate (`nebula-sdk`, `nebula-plugin-sdk`, `nebula-credential`,
      `nebula-action`, `nebula-resilience`, `nebula-metrics`,
      `nebula-error`).
- [ ] Cross-link to relevant ADRs and canon sections (no «see
      canon §X» without a link).
- [ ] `cargo test --workspace --doc` green (already a CI gate;
      verify after this pass).

#### M10.3 Release process resolution

- [ ] ADR-NNNN: pick the path explicitly (manual + runbook vs minimal
      tag-triggered automation). Per `project_no_release_workflow.md`
      `release.yml` was removed deliberately — re-adding it is a
      decision, not a default.
- [ ] If automation: minimal `release.yml` for tag → `cargo publish`
      per crate in dependency order, dry-run first; no Actions noise
      beyond the publish step.
- [ ] If manual: runbook in `CONTRIBUTING.md` (or `AGENTS.md`)
      covering tag, version bump, changelog, publish order, and
      post-publish verification.
- [ ] Post-1.0 versioning policy doc (semver scope: what is API,
      what is internal, deprecation window).
- [ ] CHANGELOG strategy: pick git-cliff (#599) vs hand-curated +
      `CHANGELOG.md` template; record decision in the same ADR.

#### M10.4 Lefthook == CI parity

- [ ] Compare `.github/workflows/ci.yml` required jobs vs
      `lefthook.yml` `pre-push` hooks per
      `feedback_lefthook_mirrors_ci`.
- [ ] Update lefthook to match (or vice versa) with no silent drift.
- [ ] CI parity-check job: a script lints divergence between the two
      configs and fails the workflow on drift.
- [ ] Refresh procedure documented inline in `lefthook.yml` so the
      next maintainer doesn't reintroduce drift.

#### M10.5 cargo doc cleanliness

- [ ] `cargo doc --no-deps --workspace` returns 0 warnings on the
      pinned toolchain.
- [ ] `-D rustdoc::broken_intra_doc_links` enforced workspace-wide
      via `[workspace.lints]` (verify present; add if missing).
- [ ] CI required job: `cargo doc` cleanliness gate.
- [ ] Per-crate README + per-crate doc-comment links validated
      (no 404s on `crates.io` / external links — a cron-based
      external-link check counts).

**Exit:** root `examples/` ships 4+ runnable examples with output
checks; per-crate READMEs follow the template; release process is
captured by an ADR + runbook (or automation); lefthook == CI verified
in CI; `cargo doc` is green with broken-intra-doc-links forbidden.

### M11 — Dependency Redesign (action / resource / credential v4)

> **Status: DONE (closed 2026-04-29).** Tracked separately from §M6
> because the dependency-redesign cascade is bigger than M6 itself; M6
> consumed the new APIs while this milestone delivered them. Sub-tasks
> below land per the `m6-resource-finalization-integration-audit.md`
> plan, Phases 0-10.

- [x] **M11.1** ~~Slot-binding pattern~~ — **DONE**.
      `#[resource(key = …)]` / `#[credential(key = …)]` per-field
      attributes on Action / Resource structs replace the previous
      `DeclaresDependencies` boilerplate and `Resource::Credential`
      singular type. Field-type matrix: bare guard, `Option<Guard>`,
      `Lazy<Guard>`, `Option<Lazy<Guard>>` — type-driven optional+lazy
      detection in the derive. Evidence: `crates/action/macros/src/field_slots.rs`,
      `crates/resource/macros/src/field_slots.rs`,
      `crates/core/src/dependencies.rs::SlotField`.
- [x] **M11.2** ~~`type Input` / `type Output` on base `Action`~~ —
      **DONE**. Variant A trait shape ships `Action: Sized + type Input
      + type Output + static metadata`; sub-traits inherit
      `<Self as Action>::Input/Output`. Closes the action-redesign
      Strategy `2026-04-24-action-redesign-strategy.md` (a) path.
      Evidence: `crates/action/src/action.rs`,
      `crates/action/src/stateless.rs`.
- [x] **M11.3** ~~Supersede ADR-0036~~ — **DONE** via
      [`docs/adr/0044-supersede-0036-resource-credential-singular.md`](../docs/adr/0044-supersede-0036-resource-credential-singular.md).
      `Resource::Credential` associated type removed; resources declare
      credentials via `#[credential]` slot fields; per-slot rotation
      hook `Resource::on_credential_refresh(&mut self, slot_name)`
      replaces the singular ADR-0036 hook signature. Evidence:
      `crates/resource/src/resource.rs:243-296`.
- [x] **M11.4** ~~`FromWorkflowNode` async factory~~ — **DONE**.
      Per-execution Action instances are constructed by
      `FromWorkflowNode::from_workflow_node(node, ctx).await` —
      `#[derive(Action)]` emits the body so plugin authors never write it
      by hand. Engine dispatch in `ActionRuntime::dispatch_action`
      consults `ActionRegistry::get_factory()` first; legacy
      `Arc<dyn XxxHandler>` retained for 4 production paths
      (webhook routing, sandbox discovery, SDK runtime, EventSource adapter)
      per `.ai-factory/PHASE3_BLOCKED.md`. Evidence:
      `crates/action/src/from_workflow_node.rs`,
      `crates/engine/src/runtime/runtime.rs`.
- [x] **M11.5** *(deferred — candidate)* Per-slot credential rotation
      reverse-index + fan-out dispatch. Trait shape exists
      (`on_credential_refresh(&mut self, slot_name)`); engine-side
      machinery to map `(CredentialId, ResourceKey, slot_name)` triples
      and dispatch through `&mut self` reentrancy is deferred per
      `.ai-factory/PHASE4_BLOCKED.md`. Scope is comparable to ADR-0036
      Wave-2 + Tech Spec §3.2-§3.5; warrants own milestone.
- [x] **M11.6** ~~Derive macros + dispatch infrastructure~~ — **DONE**.
      5 macros total: `#[derive(Action)]`, `#[derive(Resource)]`,
      `#[derive(Credential)]`, `#[derive(Schema)]` (renamed `#[param]`
      → `#[field]` in Phase 2), `#[action]` attribute (advanced
      phantom-shim cases). `ActionFactory` + `ErasedAction` enum +
      generic factories (`GenericStatelessFactory<A>` etc.) ship the
      object-safe dispatch facade for the engine registry. Evidence:
      `crates/action/macros/src/`, `crates/resource/macros/src/`,
      `crates/credential/macros/src/`, `crates/schema/macros/src/`,
      `crates/action/src/factory.rs`, `crates/action/src/erased.rs`.

**Exit:** §M11.1-§M11.4 + §M11.6 DONE 2026-04-29; §M11.5 deferred as
candidate. Verification gate: `cargo deny + clippy + test + doc + build
--examples` green; trybuild probes for all 5 macros pass.

### M12 — Business-layer crate hardening (frontier → stable)

> Why this exists: M11 shipped the dependency-redesign cascade
> (slot binding, FromWorkflowNode factory, Variant A trait shape).
> But the business-layer crates that consume those primitives stayed
> at `status: frontier` (action, resource) or `status: partial`
> (plugin) per their README frontmatter — and `nebula-credential-builtin`
> contains only an empty П1 scaffold (`crates/credential-builtin/src/`
> is just `lib.rs`). M12 closes those gaps so business-layer crates
> reach `status: stable` for 1.0.

**1.0 closure criteria** (apply on top of the global DoD checklist):

- Every business-layer crate flips `status: frontier|partial → stable`
  in README frontmatter (action, credential, credential-builtin,
  resource, plugin) — supported by a recorded dashboard row in
  `docs/MATURITY.md` (M14.1 dependency).
- The `frontier`/`partial` markers in per-crate Maturity sections are
  removed because the gaps below are closed, not silently relaxed.
- Cross-trait integration tests cover the action × credential ×
  resource × plugin matrix that today reads as «partial» in
  `crates/action/README.md:227`.

#### M12.1 nebula-action — frontier → stable

- [ ] Land `CheckpointPolicy` field on `ActionMetadata` per the
      planned status in `crates/action/README.md:204` + propagate
      through engine consumption end-to-end (today «do not document
      it as a current capability»).
- [ ] Cross-trait integration tests for `PaginatedAction` /
      `BatchAction` / `WebhookAction` / `PollAction` × resource +
      credential slot bindings (currently «cross-action-type
      integration tests: partial»).
- [ ] Decide `ActionHandler` enum + per-variant `XxxHandler`
      retirement strategy per `.ai-factory/PHASE3_BLOCKED.md` (4
      production paths still on legacy: webhook routing, sandbox
      discovery, SDK runtime, EventSource adapter). Either close the
      4 paths to factory-only or document the dual-surface as 1.0
      contract.
- [ ] Action README frontmatter: `status: frontier → stable`.

#### M12.2 nebula-credential — finish hardening

- [ ] Close §M11.5 (deferred candidate): per-slot credential rotation
      reverse-index + fan-out dispatch through `&mut self` reentrancy
      per `.ai-factory/PHASE4_BLOCKED.md`. Trait shape exists
      (`Resource::on_credential_refresh(&mut self, slot_name)`) —
      engine-side machinery to map `(CredentialId, ResourceKey, slot_name)`
      triples does not.
- [ ] Audit ADR-0032 storage layer composition (`EncryptionLayer`,
      `CacheLayer`, `AuditLayer`, `ScopeLayer`) for production wiring
      gaps — every layer has a typed-error surface, observability
      triple, and integration test against PG-backed store.
- [ ] AAD-policy completeness check (SEC-11 hardening 2026-04-27
      removed AAD-free `encrypt`; verify call-sites all carry AAD).
- [ ] Capability sub-trait coverage matrix (`Interactive`,
      `Refreshable`, `Revocable`, `Testable`, `Dynamic` per Tech Spec
      §15.4) — each capability has at least one in-tree concrete
      type that exercises it.
- [ ] Credential README frontmatter stays `stable`; capability
      subtrait coverage documented in `docs/MATURITY.md`.

#### M12.3 nebula-credential-builtin — П3 concrete types

- [ ] **ADR:** what concrete credential types ship in 1.0
      `nebula-credential-builtin`? Today the crate is the П1 scaffold
      (`src/lib.rs` only) — README lists `SlackOAuth2 / BitbucketOAuth2 /
      BitbucketPat / BitbucketAppPassword / AnthropicApiKey / AwsSigV4`
      as planned, none landed. 1.0 needs the «what's actually shipping»
      decision (likely a small generic core: `GenericOAuth2`,
      `GenericPat`, `GenericApiKey`, `GenericBasicAuth`, `AwsSigV4`)
      vs the original full vendor list.
- [ ] Implement the chosen set with the standard pattern:
      `#[plugin_credential(...)]` + sealed capability traits per
      ADR-0035 §3 + integration tests + token refresh where applicable.
- [ ] Each shipped type has: integration test, observability triple,
      doctest in the crate README.
- [ ] OAuth provider configs surfaced via operator-facing config
      schema (cross-dependency with M3.1 production `AuthBackend`).
- [ ] Credential-builtin README rewritten to describe shipped surface
      (no more «П1 scaffold» disclaimer); frontmatter
      `status: scaffold → stable`.
- [ ] Out-of-Scope item «Telegram / OAuth provider integrations beyond
      what `crates/credential-builtin` already covers» reworded after
      this ships (today it implies coverage that does not exist).

#### M12.4 nebula-resource — frontier → stable

- [ ] Audit `crates/resource/plans/` for non-SUPERSEDED plans
      (`01-core`, `02-topology`, `03-infrastructure`,
      `04-recovery-resilience`, `05-manager`, `07-implementation`,
      `08-correctness`, `09-topology-guide`, `naming-audit`,
      `type-cross-reference`, `resource-author-contracts`,
      `resource-hld`) — for each, decide: shipped, ship-for-1.0, or
      formally deferred to 1.1.
- [ ] Pull «v2 deferred» items into 1.0 scope where they materially
      affect 1.0 grade: `ConnectionAware` disconnect detection,
      `InfraProvider` nested lifecycle, `ResourceGroup` (07-implementation
      lines 35-378), `Authenticate<C>` design (07-implementation:437) —
      OR explicitly defer in Out-of-Scope with rationale.
- [ ] Pre-expiry credential refresh (proactive) — currently v1 is
      reactive via `EventBus<CredentialRotatedEvent>`
      (`07-implementation:504`); decide if proactive ships in 1.0.
- [ ] Engine frontier-loop wiring of `ResourceAction::configure` /
      `cleanup` per branch per `.ai-factory/PHASE7_BLOCKED.md` —
      requires branch-tree dominator analysis. Either ship or defer
      with explicit «scoped resources require manual driver» banner.
- [ ] Resource README frontmatter: `status: frontier → stable`.

#### M12.5 nebula-plugin — slice B (partial → stable)

- [ ] Land ADR-0027 (`ResolvedPlugin`, namespace invariant, registry
      accessors) — file location pre-announced as
      `docs/adr/0027-plugin-load-path-stable.md`
      (`crates/plugin/README.md:66`).
- [ ] Replace legacy API with the slice-B replacement
      (`PluginManifest` canonical home in `nebula-metadata`,
      re-exported from `nebula-plugin` for source compat); remove
      the «If the README and the code disagree, trust the code»
      disclaimer at `crates/plugin/README.md:14-18`.
- [ ] `ResolvedPlugin::from(impl Plugin)` validation hardening:
      duplicate-key detection + namespace invariant integration tests.
- [ ] `PluginRegistry` accessors (`all_*`, `resolve_*`) covered by
      integration tests across multiple registered plugins.
- [ ] Cross-dependency with M5 (ABI promise) for the loader
      validation behaviour.
- [ ] Plugin README frontmatter: `status: partial → stable`;
      disclaimer block removed.

**Exit:** action / credential / credential-builtin / resource / plugin
all reach `status: stable`; 1.0 ships a defined credential-types set
(not «scaffold + planned»); §M11.5 closes; M12.4 either lands or
formally defers each non-SUPERSEDED resource plan.

### M13 — Core-layer 1.0 polish (frontier → stable)

> Why this exists: core-layer crates carry types every other crate
> imports (IDs, schemas, expressions, workflow definitions). The
> `frontier` markers on `nebula-core` (Role/Permission/Tenancy «may
> see breaking changes») and `nebula-metadata` mean any caller's
> 1.0 contract is fragile unless these stabilize.

**1.0 closure criteria** (apply on top of the global DoD checklist):

- Core / metadata flip `status: frontier → stable`; the «may see
  breaking changes» disclaimers in their READMEs are removed.
- Schema / validator / expression have a coverage matrix recorded in
  `docs/MATURITY.md` (M14.1).
- Workflow / execution invariants are documented + tested
  (state-machine transitions, validation completeness).

#### M13.1 nebula-core — Role/Permission/Tenancy stability lock-in

- [ ] Audit Role / Permission / Tenancy / slug modules for breaking
      changes still on the table (per `crates/core/README.md:63`).
- [ ] Land an ADR that pins the public surface of these modules; any
      future change goes through deprecation per the M5 / M10.3
      versioning policy.
- [ ] Compat fixture set covering identifier round-trips (prefixed-ULID
      parse + format + reject malformed inputs).
- [ ] `SecretString` Debug redaction integration test (per
      `crates/core/README.md:50` invariant) — explicit test that
      forbids leaking through structured fields.
- [ ] Core README frontmatter: `status: frontier → stable`.

#### M13.2 nebula-metadata — finalize semantics

- [ ] `MaturityLevel` semantics document: what each level means
      operationally + how the engine / catalog consumes it.
- [ ] Deprecation flow integration test: registering a deprecated
      component surfaces a warning, blocks new dependents, etc.
- [ ] `BaseMetadata<K>` derive-emission audit (used by every catalog
      citizen — Action / Credential / Resource).
- [ ] Metadata README frontmatter: `status: frontier → stable`.

#### M13.3 nebula-schema — derive completeness coverage

- [ ] `#[derive(Schema)]` (renamed from `#[param]` in M11 Phase 2)
      edge case matrix: nested types, `Option<T>` / `Vec<T>` /
      `HashMap<K,V>`, optional with default, sensitive (redacted)
      fields, custom validators.
- [ ] Each edge case has trybuild probes (compile-fail + compile-pass).
- [ ] Validation cross-link with `nebula-validator`: schema-level
      validation invokes validator combinators correctly.
- [ ] Schema README frontmatter stays `stable`; coverage row in
      `docs/MATURITY.md`.

#### M13.4 nebula-validator — combinator coverage

- [ ] Audit combinator coverage against
      `crates/validator/tests/fixtures/compat/error_registry_v1.json`
      (canonical machine-readable registry of stable error codes).
- [ ] Every combinator has at least one fixture row + unit test +
      doctest.
- [ ] Validator README frontmatter stays `stable`; coverage row in
      `docs/MATURITY.md`.

#### M13.5 nebula-expression — language extension decisions

- [ ] Decide expression-language surface for 1.0: lock current
      grammar OR ship a v2 grammar before tag.
- [ ] If lock: ADR pinning grammar; deprecation policy for syntax
      changes.
- [ ] If v2: spec + migration guide for v1→v2 expressions in stored
      workflows.
- [ ] Verify regex_cache moka migration (PR #625) under stress (per
      M9.3 #590 follow-up).
- [ ] Expression README frontmatter stays `stable`; coverage row in
      `docs/MATURITY.md`.

#### M13.6 nebula-workflow — DAG validation completeness

- [ ] Audit `validate_workflow` against the WorkflowDefinition shape:
      every constraint required at activation time has a typed error
      variant + invariant check + test.
- [ ] Spec-28 §2.2 port-driven routing (M1.3 closed) verified by an
      integration test set that exercises every routing pattern in
      `crates/workflow/docs/Architecture.md`.
- [ ] Workflow README frontmatter stays `stable`; «historical context»
      block in `connection.rs`/`builder.rs` archived to
      `docs/HISTORICAL.md` if not already.

#### M13.7 nebula-execution — state machine invariants

- [ ] State-machine transition diagram in
      `crates/execution/docs/state-machine.md` (or section in README)
      that matches the actual `transition_node` allow-list.
- [ ] Property test: every legal transition is reachable; every
      illegal transition rejected with typed error.
- [ ] Invariant audit per `feedback_direct_state_mutation` —
      `let _ = transition_node(...)` patterns rejected by lint or
      grep gate.
- [ ] Execution README frontmatter stays `stable`.

**Exit:** core / metadata flip frontier → stable; schema /
validator / expression / workflow / execution have coverage matrices
in `docs/MATURITY.md`; state-machine invariants documented + tested.

### M14 — Cross-cutting maturation + Public API freeze

> Why this exists: cross-cutting crates carry the seams every other
> crate uses (eventbus, log, metrics, resilience, system) plus the
> public surface plugin and integration authors consume (sdk,
> plugin-sdk). They are mostly `stable` but carry known gaps the
> roadmap has not tracked: `ExecutionEvent` still on raw mpsc, no
> Health trait abstraction, ExecutionCommandService still inline in
> the cancel handler, `docs/MATURITY.md` referenced by every README
> but the file does not exist.

**1.0 closure criteria** (apply on top of the global DoD checklist):

- `docs/MATURITY.md` exists and is the authoritative dashboard for
  every crate's status, test coverage, and known gaps. Today every
  README links to it; the file is missing.
- `ExecutionEvent` migrates from raw `mpsc` to `nebula-eventbus` so
  multi-subscriber consumers stop reinventing the broadcast pattern
  (memory `project_eventbus_status`).
- `nebula-sdk` and `nebula-plugin-sdk` flip `status: partial → stable`
  with a frozen public surface for 1.0.
- Cross-cutting refinements landed: Health trait extracted, command
  service extracted, system probe contracts hardened.

#### M14.1 docs/MATURITY.md — actually write the dashboard

- [ ] Create `docs/MATURITY.md` (workspace-level — every per-crate
      README links to it as the single source of truth, but the file
      does not exist).
- [ ] Schema: row per crate × columns (`status`, `API stability`,
      `test coverage`, `loom coverage`, `chaos coverage`, `known
      gaps`, `1.0 readiness`).
- [ ] Each per-crate README's «See `docs/MATURITY.md` row for
      `nebula-XXX`» link resolves correctly.
- [ ] CI gate: file existence + link resolution from per-crate
      READMEs (catches future regressions).

#### M14.2 nebula-eventbus — ExecutionEvent migration

- [ ] Migrate `ExecutionEvent` from raw `mpsc` to `EventBus<ExecutionEvent>`
      so multi-subscriber consumers stop reinventing the channel
      (memory `project_eventbus_status`: «ExecutionEvent still on raw
      mpsc; migration needed for multi-subscriber»).
- [ ] Engine publishes via the eventbus; no direct `mpsc::send` from
      engine to event consumers.
- [ ] At least 2 subscribers in tests (e.g. metrics emitter + log
      tailer) verify the broadcast semantics.
- [ ] Refine `Registry` and `Scope` patterns based on actual engine
      usage (per `crates/eventbus/README.md:59`: «may be refined as
      engine usage grows»).

#### M14.3 nebula-sdk — partial → stable (1.0 public surface freeze)

- [ ] Audit `prelude` exports: every type in the prelude has a 1.0
      stability commitment.
- [ ] `WorkflowBuilder` / `ActionBuilder` / `TestRuntime` API frozen
      via ADR.
- [ ] `testing` module + `TestRuntime` harness coverage filled in
      (`crates/sdk/README.md:101` says «usable but harness coverage
      is still growing»).
- [ ] `simple_action!` macro coverage decision: extend to stateful /
      trigger / resource-backed shapes, OR document constraint
      explicitly per the README note.
- [ ] `anyhow` re-export decision: keep (current ergonomics choice
      per README:103-105) with a typed-error opt-in path, OR retire
      for 1.0 with migration guidance.
- [ ] SDK README frontmatter: `status: partial → stable`.

#### M14.4 nebula-plugin-sdk — partial → stable

- [ ] Land ADR-0006 slice 1d: `PluginCtx` broker RPC accessors +
      `PluginSupervisor` (`crates/plugin-sdk/README.md:81-82` flags
      `PluginCtx` as placeholder with «no methods»).
- [ ] Capability negotiation in handshake (cross-dependency with
      M4.1 sandbox capability discovery).
- [ ] Protocol versioning becomes a tested contract
      (`crates/plugin-sdk/README.md:84`).
- [ ] Retire the 1 panic site flagged at
      `crates/plugin-sdk/README.md:85` to a typed error variant.
- [ ] Test coverage lift: handshake + duplex envelope happy/error
      paths covered by integration tests (today «light»).
- [ ] Plugin-sdk README frontmatter: `status: partial → stable`.

#### M14.5 nebula-resilience — feature gap closure

- [ ] Cross-dependency with M2 (engine retry layered design): action-
      internal retry stays here; verify no engine-internal retry
      regression on the Layer-1 path.
- [ ] Audit `nebula-resilience` against the recent fix-pipeline-retry
      PR (#639) follow-ups for residual hardening.
- [ ] Resilience README frontmatter stays `stable`; observability
      triple verified per M9.1.

#### M14.6 nebula-log — refinements

- [ ] `nebula-log`: file rolling + runtime reload audit
      (`crates/log/README.md:58` claims stable; verify under load).
- [ ] Log README frontmatter stays `stable`.

#### M14.7 Health trait extraction (per memory)

- [ ] Generalize `TriggerHealth` into a `Health` trait
      (memory `project_health_trait`: «when Resource/Agent need it»).
- [ ] Resource health / Agent health surfaces consume the same trait
      so operators get a uniform health surface across the catalog.
- [ ] Engine exposes aggregate health for the `/healthz` endpoint
      surfacing per-resource / per-agent state.

#### M14.8 ExecutionCommandService extraction (per memory)

- [ ] Extract the §12.2 orphan contract from inline `cancel_execution`
      handler into an `ExecutionCommandService` per memory
      `project_cancel_handler_inline`. Required before any new
      transport (queue, gRPC, CLI) can issue commands without
      duplicating the inline contract.
- [ ] Cancel handler refactored to delegate to the service.
- [ ] Service has typed error surface + observability triple +
      integration test covering the orphan contract that was inline.

**Exit:** every crate carries a real row in `docs/MATURITY.md`;
sdk + plugin-sdk reach `status: stable`; ExecutionEvent flows through
the eventbus; Health trait + ExecutionCommandService extracted.

## Out of Scope for 1.0

These are **explicit deferrals** — must not silently slip into 1.0 scope:

- Storage Layer 2 / spec-16 multi-tenant row model ("Sprint E"). Document as
  1.1 milestone in release notes.
- ADR-0013 compile-time modes (build.rs / mode-* features). Per
  `project_adr0013_unimplemented.md`, accepted but unimplemented; not a 1.0
  blocker.
- Telegram / OAuth provider integrations beyond the small generic-core set
  M12.3 ships in `crates/credential-builtin` (vendor-specific provider
  packs are 1.1+; the 1.0 surface is the chosen generic types per the
  M12.3 ADR).
- WebSocket endpoint (`crates/api/src/handlers/websocket.rs:32` returns 501) —
  ship 1.0 without realtime API; document as 1.1.
- Performance regression testing harness (#600 loadgen-rs investigation).
- ADR-0024 dyn-trait migration (#601).
- Automated CHANGELOG generation via git-cliff (#599).

## Sub-Project Ordering Rationale

These milestones are **not all parallelizable**. Suggested ordering
(M0–M2, M6, M11 closed 2026-04-29 → 2026-05-04; remaining in flight):

1. **M0 (durability)** first — small, foundational, removes false claims.
   ✅ DONE.
2. **M3 (API)** in parallel with M0 once M0 owner is on it — biggest
   user-facing gap, can be sliced (M3.1 auth ✅, M3.2 OpenAPI, M3.3
   webhook ✅, M3.4 idempotency partial, M3.5 tracing, M3.6 validation
   each as own sub-project).
3. **M1, M2 (engine correctness + retry)** after M0 lands — they touch
   the same `engine.rs` paths. ✅ DONE.
4. **M4, M5 (sandbox + plugin contract)** can run in parallel with M3.
5. **M6 (resource finalization)** independent. ✅ DONE.
6. **M7 (storage ops), M8 (loom)** — late, after engine work settles.
7. **M9 (observability sweep)** — late; M9.4 already closed via ADR-0046.
8. **M11 (dependency redesign)** — landed before M6 closure. ✅ DONE.
9. **M12 (business-layer hardening)** — runs in parallel with M3
   completion; M12 needs M11 (DONE), and M12.5 depends on M5 ABI ADR.
10. **M13 (core-layer polish)** — independent of M3-M9; can start any
    time. M13.1 (core stability lock-in) gates anyone consuming
    Role/Permission/Tenancy.
11. **M14 (cross-cutting + public API freeze)** — gates 1.0 tag.
    Requires M12 + M13 substantially done so MATURITY.md (M14.1) and
    SDK / plugin-sdk freeze (M14.3 / M14.4) reflect a stable surface.
12. **M10 (docs/release)** — last; gate before tag, after M14
    finalises the public surface.

## Next Step

Open `/aif-plan full <milestone>` for the chosen first sub-project. The
roadmap entry's bullets become the tasks; `/aif-plan` adds dependencies,
testing/logging policy, and (optionally) a worktree + branch.
