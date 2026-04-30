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

## Status Snapshot (2026-04-28)

- **Cross-cutting layer** (`error`, `log`, `eventbus`, `telemetry`, `metrics`,
  `resilience`, `system`) — **stable, no pending breaks**.
- **Core layer** (`core`, `validator`, `expression`, `workflow`, `execution`,
  `schema`, `metadata`) — **stable**. Expression #590 (regex_cache LRU)
  closed in PR #625 via `moka::sync::Cache` migration —
  see `crates/expression/src/eval.rs` (regex_cache field on `Evaluator`).
- **Business layer** (`credential`, `resource`, `action`, `plugin`) — mostly
  stable. `resource` plans 06 + 10 + prototypes are PARTIAL.
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
  2026-04-29). Layer 2 (`claimed_by`/`claimed_until` + spec-16 row
  model) remains Sprint E (1.1) scaffolding.
  `sandbox` is correctness-grade; capability discovery enforcement gap (canon
  §4.5).
- **API layer** — routing wired; **5 sizable feature gaps** (auth backend,
  OpenAPI 3.1, webhook dispatch, idempotency, tracing context propagation).
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

The largest 1.0 area. Five blocks; can be parallelized once unblocked.

- [ ] **M3.1 Auth backend.** 9 stub handlers in
      `crates/api/src/handlers/auth.rs:22-113` (register, login,
      verify_email, reset_password, totp_*, oauth_*) + PAT lookup TODO in
      `crates/api/src/middleware/auth.rs:134`. Wire session store.
- [ ] **M3.2 OpenAPI 3.1 spec generation.** Today
      `crates/api/src/handlers/openapi.rs:9-16` is a stub. Required for SDK
      doc discovery and 1.0 contract.
- [ ] **M3.3 Webhook handler dispatch.** Stubs in
      `crates/api/src/handlers/webhook.rs:21-34` (validate per-trigger
      auth, enqueue trigger event, return 202). Transport layer is real;
      handlers are not.
- [ ] **M3.4 Idempotency-Key dedup.** Cancel endpoint claims idempotency
      (`crates/api/src/handlers/execution.rs:450`) but no header handling
      or dedup store. POST endpoints lack replay protection.
- [ ] **M3.5 Tracing context propagation.** Request-ID middleware
      (`crates/api/src/middleware/request_id.rs`) sets a header but does
      not attach to `tracing::Span` or propagate to engine execution. No
      distributed-trace handoff.
- [ ] **M3.6 Shift-left workflow validation.** Audit all `/execute`
      handlers to call `validate_workflow` before passing to engine
      (per `crates/workflow/README.md:82-84` contract). Add lint or test
      that catches unvalidated paths.

**Exit:** every stub-marked handler has either a real implementation +
integration test, OR is removed from the route table; `cargo doc` for
`nebula-api` reflects only shipping endpoints.

### M4 — Sandbox capability discovery enforcement

- [ ] **M4.1** Validate declared `PluginCapabilities` against `plugin.toml`
      at registration; reject mismatch before sandbox spawn. Today
      `crates/sandbox/src/lib.rs:21` notes "capability allowlist is a false
      capability until discovery wires up" — this is a canon §4.5
      operational-honesty gap.

**Exit:** sandbox README appendix TODO closed; capability mismatch produces
a typed error and rejected registration; integration test covers it.

### M5 — Plugin ABI + Engine-Plugin contract

Decision point, not a coding task by itself. Pick **A or B** and document.

- [ ] **M5.1** Either: **(A)** commit to `Plugin` trait stability via engine
      semver constraint in plugin manifests (`nebula_version` field is in
      manifest but not validated at load time), with deprecation policy
      doc'd; **OR** **(B)** ship 1.0 explicitly without ABI promise and
      document that community plugins must rebuild against each engine
      minor.

**Exit:** ADR landed; plugin-sdk README and per-plugin.toml schema reflect
the choice; loader either validates `nebula_version` or rejects it as
unrecognized field.

### M6 — Resource layer finalization

- [ ] **M6.1** Plan **06-action-integration**: complete `ResourceAction`
      trait wiring (currently PARTIAL per
      `crates/resource/plans/06-action-integration.md` vs source).
- [ ] **M6.2** Plan **10-scoped-resources**: per-execution isolation +
      credential rotation paths (currently PARTIAL).
- [ ] **M6.3** Move `resource-prototypes` (Postgres Pool, HTTP Resident,
      Telegram Service) to root `examples/` workspace member or to a
      separate dev fixture crate; do not leave them as planning-only
      skeletons.

**Exit:** all 15 resource plans are DONE; one runnable example per
topology in root `examples/`.

### M7 — Storage operationalization

- [ ] **M7.1** Wire **Postgres** `PgControlQueueRepo` as the default
      composition root (currently `simple_server` and tests pick
      `InMemoryControlQueueRepo`; restart loses pending commands).
- [ ] **M7.2** Document multi-process deployment limits and the
      Sprint-E (spec-16 Layer 2) deferral in 1.0 release notes.
- [ ] **M7.3** Add Loom probe job to nightly CI (ADR-0041 DoD —
      `crates/storage-loom-probe`). Probe exists; CI run does not.

**Exit:** any production deployment without in-memory control queue
fallback; Loom job runs nightly green.

### M8 — Engine concurrency verification

- [ ] **M8.1** Add `loom` feature to `nebula-engine`; cover lease renewal,
      running-registry insert/remove, and cancel-token handoff
      (`crates/engine/src/engine.rs:196-251`). `DashMap` is loom-hostile —
      either substitute on `cfg(loom)` or extract a lock-free struct.
- [ ] **M8.2** Add 2-3 property tests for lease fence + registration
      nonce.

**Exit:** loom suite green nightly; multi-runner deployments have
verified concurrency invariants.

### M9 — Observability + DoD audit pass

- [ ] **M9.1** Sweep all hot paths (engine state transitions, control
      dispatch, sandbox spawn, storage CAS retries) for the
      typed-error + tracing-span + invariant-check triple. Document gaps
      and fill in. Per `feedback_observability_as_completion.md`,
      observability is part of DoD, not follow-up.
- [ ] **M9.2** Verify OpenTelemetry bridge against #598 (telemetry: verify
      OpenTelemetry setup against bridge-pattern guide).
- [ ] **M9.3** Address #595 (metrics OTLP label allocation) and #591
      (system NETWORK_STATS Mutex) and #590 (expression regex_cache
      Mutex) if they sit on hot paths used by 1.0 surfaces.

**Exit:** issue triage report attached; spans/metrics/errors triple
present at each new boundary.

### M10 — Documentation + DX + release process

- [ ] **M10.1** Root-level `examples/` workspace member with at least:
      one workflow + action example, one credential example, one plugin
      author example, one resource topology example.
      Per `feedback_examples_location.md`.
- [ ] **M10.2** Per-crate `README.md` quality pass (compile-checked
      examples in doctests where possible).
- [ ] **M10.3** Resolve `release.yml` strategy. Per
      `project_no_release_workflow.md` it was removed deliberately — for
      1.0, decide: stay manual + tag-driven, OR ship a minimal
      tag-triggered crates.io publish workflow. Don't re-add Actions noise
      without a reason.
- [ ] **M10.4** Verify `lefthook pre-push` mirrors **every** CI required
      job per `feedback_lefthook_mirrors_ci.md`.
- [ ] **M10.5** `cargo doc --no-deps --workspace` clean of broken
      intra-doc links and warnings.

**Exit:** new contributor can build, test, and ship a plugin from
`README.md` + `examples/` alone.

## Out of Scope for 1.0

These are **explicit deferrals** — must not silently slip into 1.0 scope:

- Storage Layer 2 / spec-16 multi-tenant row model ("Sprint E"). Document as
  1.1 milestone in release notes.
- ADR-0013 compile-time modes (build.rs / mode-* features). Per
  `project_adr0013_unimplemented.md`, accepted but unimplemented; not a 1.0
  blocker.
- Telegram / OAuth provider integrations beyond what `crates/credential-builtin`
  already covers.
- WebSocket endpoint (`crates/api/src/handlers/websocket.rs:32` returns 501) —
  ship 1.0 without realtime API; document as 1.1.
- Performance regression testing harness (#600 loadgen-rs investigation).
- ADR-0024 dyn-trait migration (#601).
- Automated CHANGELOG generation via git-cliff (#599).

## Sub-Project Ordering Rationale

These milestones are **not all parallelizable**. Suggested ordering:

1. **M0 (durability)** first — small, foundational, removes false claims.
2. **M3 (API)** in parallel with M0 once M0 owner is on it — biggest user-facing
   gap, can be sliced (M3.1 auth, M3.2 OpenAPI, M3.3 webhook each as own
   sub-project).
3. **M1, M2 (engine correctness + retry)** after M0 lands — they touch the
   same `engine.rs` paths.
4. **M4, M5 (sandbox + plugin contract)** can run in parallel with M3.
5. **M6 (resource finalization)** independent; can start any time.
6. **M7 (storage ops), M8 (loom)** — late, after engine work settles.
7. **M9 (observability sweep), M10 (docs/release)** — last; gate before tag.

## Next Step

Open `/aif-plan full <milestone>` for the chosen first sub-project. The
roadmap entry's bullets become the tasks; `/aif-plan` adds dependencies,
testing/logging policy, and (optionally) a worktree + branch.
