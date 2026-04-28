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
  `schema`, `metadata`) — **stable**; expression has p2 perf concern (#590).
- **Business layer** (`credential`, `resource`, `action`, `plugin`) — mostly
  stable. `resource` plans 06 + 10 + prototypes are PARTIAL.
- **Exec layer** — `storage` is production-ready for execution/workflow
  (Layer 1 traits stable, 23 PG migrations + 9 common, CAS, outbox, reclaim).
  `engine` is ~70% — orchestration solid, **§11.5 durability debts** open.
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
- [ ] **M0.3** Wire `ExecutionTerminationReason::ExplicitStop` /
      `ExplicitFail` from `ActionResult::Terminate` through to
      `determine_final_status`. Today the variants exist in
      `crates/execution/src/status.rs:85-94` but are never populated. The
      engine docstring at `crates/action/src/result.rs:206-219` explicitly
      flags "Phase 3 of the ControlAction plan and is **not yet wired**".
      Audit log loses intent vs system-driven termination (canon §4.5).
- [ ] **M0.4** **Sync stale debt docs** — `crates/engine/README.md` "L4
      debt" block + canon §11.5 references still describe M0.1 and M0.2 as
      open; update to match shipped reality. Per
      `feedback_active_dev_mode.md`: do not leave docs trailing the code.

**Exit:** M0.3 closes the Phase-3 ControlAction-plan termination wiring with
test (`Terminate` → `ExplicitStop` round-trips through `ExecutionResult` and
`ExecutionEvent`); M0.4 brings README/canon back in sync.

### M1 — Engine correctness: full-graph edge gating

- [ ] **M1.1** Replace local-edge skip propagation with full-graph
      transitive blocking. Today `crates/engine/src/engine.rs:1808`
      blocks only direct successors; multi-hop conditional flows can
      read stale outputs from skipped branches.
- [ ] **M1.2** Decide and document fate of `expression_engine` field
      (`crates/engine/src/engine.rs:124-128`, currently
      `#[expect(dead_code)]`): wire dynamic edge conditions for 1.0 OR
      remove the field and downgrade canon §10 claims.

**Exit:** §10 conditional-flow text matches behaviour; no `#[expect(dead_code)]`
in engine.

### M2 — Engine retry semantics + node attempts

- [ ] **M2.1** Implement engine-level re-execution from `ActionResult::Retry`
      (canon §11.2 "planned"). Today `NodeAttempt` tracks counts but no
      durable re-execution loop. Either ship it for 1.0 OR remove the
      retry-engine claims from canon and confine retry to `nebula-resilience`
      inside actions.
- [ ] **M2.2** Verify `execution_leases` heartbeat enforcement across runner
      restarts (per `crates/execution/README.md:138`).

**Exit:** retry path is either real (with tests) or removed from canon.

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
