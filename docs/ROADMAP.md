# Nebula — Production-Ready 1.0 Roadmap

> Strategic milestones taking Nebula from "core stable, engine + API in active
> development" to **production-ready 1.0**. This is a **capability /
> dependency** checklist, not a calendar plan — sequencing follows proof
> points and dependencies per [`STRATEGY.md`](../STRATEGY.md) ("capability
> completeness and engine truth" is the planning unit, not human-months).
>
> Granular tasks live under `docs/plans/<milestone-or-branch>.md`; this file
> stays a checklist. README-driven product claims (canon §4.5 honesty)
> override roadmap optimism: a milestone ticks off only when its exit criteria
> are *verifiably* met.
>
> **Provenance:** this doc was relocated from the retired `.ai-factory/`
> tree (AI-Factory is being removed; `ce-*` is the default skill set). It is
> now the git-tracked roadmap of record under `docs/`. The crate / layer
> inventory is sourced from [`CLAUDE.md`](../CLAUDE.md) (canonical), not the
> agent-summary files.
>
> **Maintenance:** add a milestone with one-line evidence (`file:line` or
> `#PR`) when a new gap is discovered; tick it off only after the exit
> criteria below are verifiably met.

## Status snapshot — 2026-05-18

- **Cross-cutting layer** (`error`, `log`, `eventbus`, `metrics`,
  `resilience`) — **stable, no pending breaks**. `nebula-telemetry` was
  merged into `nebula-metrics` per ADR-0046 (PRs #652–#656); the lock-free
  primitives + label interning are intra-crate modules in `nebula-metrics`
  (no `cargo deny` wrapper for that boundary; `MetricsAdapter` deleted in
  #654). The orphan `nebula-system` crate was removed (#668) — it is no
  longer a workspace member.
- **Core layer** (`core`, `validator`, `expression`, `workflow`, `execution`,
  `schema`, `metadata`, `storage-port`) — **stable**. Expression #590
  (`regex_cache` LRU) closed in PR #625 via `moka::sync::Cache`. `schema`
  `#[param]` namespace renamed to `#[field]` (M6 Phase 2). **ADR-0052
  cascade complete**: visibility/required moved into `nebula-validator`
  (P1 #670), validator is the sole error emitter with root-rule secret
  scrub + single crossing (P2 #672), `HasSchema` convergence replaced the
  per-trait `*_schema` methods with `schema_of` (P3 #676). `nebula-storage-port`
  is the new Core seam — the spec-16 object-safe row-model contract every
  storage consumer depends on (ADR-0072 deleted the legacy `ExecutionRepo` /
  `WorkflowRepo` surface).
- **Business layer** (`credential`, `credential-builtin`, `resource`,
  `action`, `plugin`, `tenancy`) — **§M6 + §M11 closed 2026-04-29.** v4
  trait shapes shipped: `Action: Sized + type Input/Output + static
  metadata + slot-binding derive + FromWorkflowNode factory + ErasedAction
  dispatch`; `Resource` drops `type Credential` (ADR-0044). **nebula-resource
  finalized** (ADR-0067, #688): `SlotCell` slot, `&self`/`&Runtime`
  rotation hook, structural dedup, resource CRUD/status HTTP API. **Rotation
  fan-out dispatch is live** (#690 + 3 latent P1 fixes; #703 wired resource
  acquire through the `acquire_erased` lease pipeline) — but
  **bind-population is still deferred** (no production credential→slot
  resolver; `register_and_bind` has a quiesce contract and zero callers),
  so `nebula-resource` stays `frontier` (tracked under M12.4).
  `nebula-tenancy` is the new scope-enforcing storage decorator (substitutes
  a tenant scope on every call before it reaches a handler).
- **Exec layer** — `storage` is production-ready for execution/workflow and
  is now the **sole spec-16 adapter** (InMemory + SQLite + Postgres) behind
  `nebula-storage-port`. `engine` is ~85% — orchestration solid; §11.5
  durability debts closed (M0); §10 conditional-flow verified (M1); §11.2
  layered-retry shipped (M2.1); §11.5 Layer-1 lease enforcement verified by
  engine + PG + loom + chaos (M2.2); §M6.2 scoped-resources storage +
  lifecycle primitives shipped. `sandbox` is correctness-grade; the
  Plugin-Proto tier (`plugin-sdk`, `sandbox`) was decomposed for honesty
  (#669); the capability-discovery enforcement gap (canon §4.5) is still
  open (M4).
- **API layer** — fully restructured (`lib` + `apps/server`, domain modules;
  #671) with §4.5-honest stub completion. Auth backend + webhook dispatch +
  idempotency landed (M3.1 partial / M3.3 ✅ / M3.4 ✅); OpenAPI 3.1
  closed (M3.2); W3C trace context flows API → control queue → engine
  (M3.5, #661); PAT-scope access kernel landed (#702); tenant credential
  security hardened (`b2a59ea8`). Credential write-path validation +
  port-backed catalog + public projection closed the §4.5 fail-open
  (ADR-0052 P4 #677). Remaining: production `AuthBackend`, CSRF, OAuth
  secrets registry (M3.1); full-stack one-root-span trace test + OTLP
  exporter (M3.5 / M9.2); shift-left validation audit (M3.6).
- **Shared infra** — `nebula-credential` is consumed by Exec + API +
  Business (the `deny.toml` `[wrappers]` allowlist locks the consumer set).
  The `nebula-credential-runtime` crate shipped (ADR-0066, #678):
  `CredentialService<B,PS>` facade, `DispatchOps`, facade-level `owner_id`
  tenant isolation; `External = ExternalSourceNotWired` (the ADR-0051
  external-source bridge is unbuilt). Wiring `nebula-api` onto the
  credential services is the next credential-side increment.
- **Agent-discipline gate** — ADR-0083 (intent-honesty gate): the
  deterministic structural-budget tier shipped (#705); the semantic LLM
  tier is an unbuilt sequenced follow-up, not a 1.0 blocker.
- **GitHub:** open issues are p2/p3 needs-triage/discussion; no p0/p1.
  Open issues frequently already have fixes landed (squash-merge
  auto-close misses) — verify with `git log --grep="#N"` before planning.

## Definition of Done — production-ready 1.0

A milestone exits only when **all** of these hold for its scope:

- [ ] **Behaviour**: every claim in README / canon / per-crate docs is backed
      by code that exercises the path; no "false capability" per canon §4.5.
- [ ] **Observability** (per CLAUDE.md "Observability is part of Definition
      of Done"): typed `thiserror` variant + `tracing` span / event +
      invariant check on every new state, error, or hot path.
- [ ] **Tests**: unit + integration + (where applicable) loom / chaos / fuzz.
      No `#[ignore]` outside benches and intentionally slow chaos suites.
- [ ] **Layer hygiene**: `cargo deny check` green, no new wrapper edges
      without `reason` in `deny.toml`.
- [ ] **Lint / docs**: `cargo clippy --workspace --all-targets -- -D warnings`
      green; `cargo test --workspace --doc` green; rustdoc broken-intra-doc
      links forbidden (`-D rustdoc::broken_intra_doc_links`).
- [ ] **Security**: secrets policy (AES-256-GCM + AAD, zeroize-on-Drop,
      redacted Debug) holds end-to-end; CODEOWNERS gate on credential /
      webhook paths (already enforced).
- [ ] **CI parity**: `lefthook pre-push` mirrors every CI required job.

## Milestones

### M0 — Engine durability debts (canon §11.5) — ✅ DONE

**Why first.** Without these, "resume" and "replay" claims in canon are false.
Pure data debt, no architectural rework needed.

- [x] **M0.1** Persist `ExecutionBudget` — DONE (2026-04-28). Field
      `budget: Option<ExecutionBudget>` in `crates/execution/src/state.rs`
      (issue #289); persisted + restored in `engine.rs`; tests
      `resume_restores_persisted_budget` /
      `resume_falls_back_to_default_budget_on_legacy_state`. Migration
      `00000000000009_add_resume_persistence.sql`.
- [x] **M0.2** Persist original workflow input — DONE (2026-04-28). Field
      `workflow_input: Option<Value>` (issue #311); persisted + restored;
      test `resume_restores_original_workflow_input`.
- [x] **M0.3** Wire `ExecutionTerminationReason::ExplicitStop`/`ExplicitFail`
      — DONE (2026-04-28). `set_terminated_by` + `determine_final_status`
      priority ladder; surfaced via `ExecutionResult.termination_reason` and
      `ExecutionEvent::ExecutionFinished.termination_reason`; 11 tests.
- [x] **M0.4** Sync stale debt docs — DONE (2026-04-28). `engine/README.md`
      L4 debt block + `action/src/result.rs` / `execution/src/status.rs`
      docstrings rewritten; workspace grep for stale "not yet wired" markers
      clean in M0 scope.

**Exit:** termination wiring round-trips through `ExecutionResult` and
`ExecutionEvent`; README/canon back in sync. **Closed.**

### M1 — Engine correctness verification + cleanup (canon §10) — ✅ DONE

Re-scoped to verification + dead-field cleanup + doc audit (the original
"local-edge gating" defect did not exist; option-A "wire dynamic edge
conditions" contradicted Spec 28 §2.2, which settled conditional routing
via explicit `ControlAction` nodes).

- [x] **M1.1** Verify full-graph skip propagation — DONE (2026-04-28). 5
      integration tests: transitive 3-hop, diamond with one skipped branch,
      mixed-source aggregate, all-sources-skipped, multi-hop with sibling
      activation (`crates/engine/tests/integration.rs`).
- [x] **M1.2** Remove dead `WorkflowEngine.expression_engine` field — DONE
      (2026-04-28). The shared `Arc<ExpressionEngine>` lives in
      `ParamResolver` (the only consumer).
- [x] **M1.3** Sync canon §10 / docs with Spec 28 §2.2 port-driven routing —
      DONE (2026-04-28). `crates/workflow/README.md` + `Architecture.md`
      drift table updated.

**Exit:** skip-propagation verified; no `#[expect(dead_code)]` in engine;
docs match shipping code. **Closed.**

### M2 — Engine retry semantics + node attempts — ✅ DONE

- [x] **M2.1** Engine-retry direction for 1.0 — DONE (2026-04-29, ADR-0042).
      Two disjoint retry surfaces by trigger boundary: **Layer 1** —
      action-internal `nebula-resilience::retry_with` (in-call); **Layer 2**
      — engine-level node retry (`NodeDefinition.retry_policy` →
      `NodeExecutionState::next_attempt_at` → `NodeState::WaitingRetry`
      parking → frontier-loop min-heap re-dispatch), with
      `ExecutionBudget.max_total_retries` global cap (canon §11.2).
      Cancel/terminate/budget guards drain parked retries to `Cancelled`.
      Shipped across PR #627 (foundation) + PR #628 (wiring); ~146
      unit-test deltas + 9 integration tests.
- [x] **M2.2** Verify `execution_leases` heartbeat enforcement across runner
      restarts — DONE (2026-04-29, Layer 1; #629). In-memory takeover,
      cancel-redeliver across restart, PG lease lifecycle (DATABASE_URL
      -gated), loom probe `lease_handoff`, chaos holder-uniqueness,
      storage-layer tracing parity. Original "drop legacy schema/trait"
      sub-task superseded — those columns are forward Layer-2 scaffolding,
      not legacy; replaced with boundary doc-comments.

**Exit:** retry path is real with tests; §11.2 reads as "two layers,
disjoint by trigger boundary". **Closed 2026-04-29.**

### M3 — API surface completion

The largest 1.0 area. Closure criteria (on top of the global DoD):

- Every shipping route reachable through `build_app` — no
  "middleware crate-local, but not mounted" states.
- OpenAPI 3.1 spec is the authoritative contract; SDK + doc UI consume it.
- Distributed-trace context flows API → engine span → action `Context` →
  resource calls (W3C Trace Context).
- Every state-changing endpoint replay-protected; the dedup store survives
  process restart in production.
- Every `/execute` path runs `validate_workflow` before engine dispatch.
- Production `AuthBackend` ships (not just the in-memory dev impl).

#### M3.1 Auth backend

- [x] Plane-A handlers shipped (PR #638): signup / login / logout /
      forgot-password / reset-password / verify-email / mfa-enroll /
      mfa-verify / oauth-start / oauth-callback.
- [x] PAT lookup wired (`backend.lookup_pat`); session store wired through
      `AuthBackend`; all 10 routes mounted. **PAT-scope access kernel
      landed** (#702); the API restructure (#671) moved auth into the
      domain-module layout; tenant credential security hardened
      (`b2a59ea8`).
- [ ] **Production `AuthBackend` impl** — the live backend is the in-memory
      dev impl (loses state on restart). Ship a PG-backed `AuthBackend`
      (users, credential hash, sessions, PATs, MFA secrets, OAuth state).
- [ ] **CSRF enforcement** for state-changing `/auth/*` and `/api/v1/*` —
      the `csrf_cookie` is issued but no middleware validates the matching
      `X-CSRF-Token` header.
- [ ] **OAuth providers loaded from operator secrets** — a registry that
      pulls secrets from the credential store at startup (cross-dep with
      M12.3 + the `nebula-credential-runtime` wiring increment).
- [ ] **Lockout + rate-limit integration tests** against the PG backend.
- [ ] **Observability:** typed `AuthError` → `ApiError` mapping audit +
      `nebula_api_auth_*` metrics family for failed/locked-out attempts.

#### M3.2 OpenAPI 3.1 spec generation — ✅ CLOSED 2026-05-07

- [x] Real generator (utoipa 5.5 + utoipa-axum 0.2, ADR-0047); every
      handler annotated; spec served at `GET /api/v1/openapi.json` matches
      the route table (handlers without `#[utoipa::path]` fail to compile);
      Swagger UI at `GET /api/v1/docs`; OpenAPI 3.1 round-trip test; CI
      drift + operationId-uniqueness + `$ref` + §4.5 stub-honesty gate.

#### M3.3 Webhook handler dispatch — ✅ CLOSED 2026-05-07 (ADR-0049)

- [x] Single dispatch pipe (legacy slug pipeline deleted; both URL shapes
      funnel through `WebhookTransport::dispatch_inner`); storage bootstrap
      via `WebhookActivationRepo::list_active()`; replay-window enforcement;
      per-key sliding-window rate-limit; provider URL-verification
      challenges (Slack/Stripe/Generic); observability namespace extended;
      lifecycle subscriber (producer-side deferred per ADR-0049 §
      out-of-scope); admin reload endpoint. Per-outcome `REQUESTS_TOTAL` +
      latency histogram + cardinality regression test land as a 1.0
      follow-up.

#### M3.4 Idempotency-Key dedup — ✅ CLOSED 2026-05-07

- [x] `IdempotencyLayer` shipped + mounted in `build_app` (api routes only;
      webhook ingress keeps its own dedup contract); ADR-0048 hybrid
      backend (`InMemory` dev/tests, `Pg` production); migration
      `0024_add_idempotency_dedup.sql` (PG + SQLite parity); DATABASE_URL
      -gated concurrency tests; e2e test against real `build_app`;
      `nebula_api_idempotency_*` metrics + tracing span fields.

#### M3.5 Tracing context propagation

- [x] W3C Trace Context on the HTTP edge (`traceparent`/`tracestate` parse,
      `crates/api/src/middleware/trace_w3c.rs`); span context attached to
      the per-request `tracing::Span` (`TraceLayer` + `tracing_opentelemetry`,
      ADR-0050); API binaries install `OpenTelemetryLayer` so the attach is
      not a no-op without OTLP; `traceparent`/`tracestate` emitted on
      responses (CORS-aligned).
- [x] Propagate span context into the engine via the control queue +
      `ExecutionContext` — **DONE via #661** (M3.5 W3C trace context
      through control queue + engine).
- [x] Engine/action boundary: `ActionRuntimeContext` exposes
      `resource_http_request_span` / `instrument_resource_http_request` for
      outbound resource HTTP.
- [ ] Integration test: full stack API → engine → action → resource with
      one root span (engine `control_trace` + lease-takeover cancel path
      cover subsets today).
- [ ] OpenTelemetry exporter wired (depends on M9.2 verification).

#### M3.6 Shift-left workflow validation

- [ ] Audit every `/execute` and `/workflows/{id}/run` handler — call
      `validate_workflow` before engine handoff per the
      `crates/workflow/README.md` contract (current coverage map unknown).
- [ ] Encode "must validate before dispatch" as a typed boundary — e.g. a
      `ValidatedWorkflow` newtype the engine `Submit` API requires
      (impossible to call without validating).
- [ ] Map `WorkflowValidationError` → `400 Bad Request` with field-level
      detail in problem+json.
- [ ] Integration test: malformed workflow → 400 before any engine state
      mutation.
- [ ] Lint/CI gate catching a future handler that skips `validate_workflow`.

**Exit:** every shipping route reachable from `build_app`; every
state-changing endpoint replay-protected end-to-end; OpenAPI spec is the
authoritative contract; trace context flows API → engine → action →
resource; production-grade `AuthBackend` ships; no `/execute` path bypasses
validation.

### M4 — Plugin capability discovery enforcement

> Crate boundary (per [`CLAUDE.md`](../CLAUDE.md) layer map +
> `crates/sandbox/src/lib.rs` module docs): **discovery, `plugin.toml`
> parsing, the registry adapters, and the `SandboxError → ActionError`
> mapping live in `nebula-plugin`** (`crates/plugin/src/discovery.rs`,
> `plugin_toml.rs`, `discovered_plugin.rs`). `nebula-sandbox` is a leaf
> transport crate with **no per-plugin capability/scope model** — it only
> spawns the child and round-trips envelopes. The capability gate is a
> registration-time check in `nebula-plugin`, before `ProcessSandbox`
> spawns anything.

Closure criteria (on top of the global DoD):

- The capability-enforcement gap is closed in the `nebula-plugin`
  discovery path, not deprioritized; the canon §4.5 honesty notes in
  `crates/sandbox/src/lib.rs` + the sandbox/plugin READMEs are updated to
  describe the now-real gate (sandbox stays honestly transport-only).
- Capability mismatches reject registration **before** any
  `ProcessSandbox` spawn.
- Sandbox README Appendix TODO closed; canon §12.6 honesty preserved
  (in-process is correctness-only; child-process is the isolation boundary).
- OS-hardening primitives document what is / is not enforced per platform
  (the actual `os_sandbox` surface, not an aspirational matrix).

#### M4.1 Discovery validation (in `nebula-plugin`)

- [ ] Parse `plugin.toml` `[capabilities]` at registration in the
      `nebula-plugin` discovery path (`crates/plugin/src/discovery.rs` +
      `plugin_toml.rs` — parsing exists; the capability comparison does
      not run).
- [ ] Compare manifest declarations vs the runtime `PluginCapabilities`
      announced by the loaded binary.
- [ ] Typed capability-mismatch variant on the plugin-discovery error
      (`{ declared, runtime }`) + WARN-level `tracing` event with both
      sets; mapped through the existing `SandboxError → ActionError` seam
      that already lives in `nebula-plugin`.
- [ ] Mismatch rejects registration **before** any `ProcessSandbox`
      spawn; `discovered_plugin::DiscoveredPlugin` surfaces the error.
- [ ] Integration tests: malformed `plugin.toml` → reject; runtime claims
      a capability missing from the manifest → reject; runtime declares a
      subset of the manifest → allow.
- [ ] `nebula-plugin` README discovery section documents the gate;
      `nebula-sandbox` lib.rs + README "no capability model here" note
      updated to point at the plugin-side enforcement; §4.5 grep over
      **both** `crates/sandbox` and `crates/plugin` returns 0 stale
      "false capability" hits.

#### M4.2 OS-hardening honesty (1.0 honesty)

- [ ] Audit the actual `crates/sandbox/src/os_sandbox.rs` surface (today:
      Linux Landlock + rlimit child hardening, fixed system paths, **no
      per-plugin grant**) — document what is enforced vs best-effort, and
      mark non-Linux platforms as unsupported / no-op rather than implying
      a seccomp / macOS / Windows matrix that does not exist.
- [ ] Each primitive: typed enable/disable switch + tracing observation +
      Linux integration test (or an explicit "not supported on this
      platform" marker).
- [ ] Release-notes section: "what sandbox isolation actually guarantees
      (Linux Landlock + rlimit; non-Linux = transport isolation only)".

**Exit:** registration-time capability gate closed in `nebula-plugin`
before any `ProcessSandbox` spawn; OS-hardening honesty at 1.0 grade;
sandbox README Appendix TODO closed; §4.5 grep over `crates/sandbox` +
`crates/plugin` clean; integration tests cover the three reject/allow
paths.

### M5 — Plugin ABI + Engine-Plugin contract

Decision point first; coding task only after the ADR lands.

Closure criteria: the decision is in an accepted ADR (not a PR description);
the loader behaviour matches the ADR; plugin authors can answer "what
nebula versions does my plugin support" from `plugin.toml` + plugin-sdk
README alone; the in-tree first-party plugin (`crates/credential-builtin`,
future built-ins) migration story is documented.

#### M5.1 ABI-promise ADR

- [ ] Draft ADR: **A** (engine semver bound in manifest + deprecation
      policy) vs **B** (no ABI promise; community plugins rebuild per engine
      minor). Cost/benefit table (maintenance burden, author DX, engine
      refactor freedom — expert-level architecture, hard breaking changes
      acceptable when spec-correct).
- [ ] Maintainer review + acceptance; ADR linked from
      `crates/plugin-sdk/README.md`.

#### M5.2 Loader behaviour to match the ADR

- [ ] **If A:** parse `nebula_version` semver requirement at load time;
      typed `PluginLoadError::EngineVersionMismatch` + tracing event +
      integration test (compatible / patch-mismatch / major-mismatch);
      deprecation-window section in plugin-sdk README.
- [ ] **If B:** `plugin.toml` schema rejects `nebula_version` as an
      unrecognized field with a typed error pointing at the ADR; plugin-sdk
      README "1.0 ABI policy" section ("rebuild every minor" + rationale).

#### M5.3 In-tree plugin migration

- [ ] Audit `crates/credential-builtin` (+ other first-party plugins) for
      ABI assumptions; align with the chosen path; each plugin `Cargo.toml`
      references the engine version per ADR convention.

**Exit:** ADR accepted; plugin-sdk README + `plugin.toml` schema reflect the
choice; loader behaviour matches; version-mismatch integration tests cover
the chosen path.

### M6 — Resource layer finalization — ✅ DONE (core); M6.4 deferred

- [x] **M6.1** Action-integration (slot-binding `ResourceAction` wiring) —
      DONE (2026-04-29). `#[derive(Action)]` + `FromWorkflowNode` factory +
      typed `ctx.acquire_resource_by_id::<R>` / `ctx.resolve_credential_by_id::<C>`;
      engine factory-registry dispatch path; closest-ancestor scoped→global
      fallback (`crates/engine/src/scoped_resources.rs`).
- [x] **M6.2** Scoped-resources storage + lifecycle primitives — DONE
      (2026-04-29). `DashScopedResourceMap` per-branch storage with
      closest-ancestor walk + cycle defense; `CleanupOutcome` enum +
      `ExecutionEvent::ScopedResourceCleanupTimeout` + 30s default; 17
      integration tests. The originally-deferred engine frontier-loop
      per-branch wiring was unblocked by the ADR-0067 finalization (#688)
      + rotation fan-out (#690); the residual (production credential→slot
      bind-population resolver; engine frontier-loop branch-tree dominator
      analysis) is tracked under **M12.4**.
- [x] **M6.3** Runnable topology examples — DONE (2026-04-29). 3 examples in
      `examples/examples/` (Pool / Resident / Service cross-workflow);
      topology selection guidance distilled into
      `crates/resource/docs/topology-reference.md`. Run via
      `cargo run -p nebula-examples --example m6_*`.
- [ ] **M6.4** *(deferred — candidate)* `EventTrigger` DX wrapper around
      `nebula_engine::daemon::EventSource` + `TriggerAction` per ADR-0045.
      No commitment.

**Exit:** §M6.1–§M6.3 DONE; §M6.4 deferred per ADR-0045 with explicit
candidate marker. **M6 closed 2026-04-29.** Per-slot credential rotation
fan-out (originally part of §M6.2) shipped via #688/#690 (see M11.5);
bind-population is the remaining resource follow-up (M12.4).

### M7 — Storage operationalization

Closure criteria (on top of the global DoD):

- A production composition root exists and uses PG-backed repos by default;
  in-memory impls remain for dev/tests, never a quiet prod fallback.
- Multi-process deployment limits captured in 1.0 release notes.
- Loom probe runs nightly green; failures gate the 1.0 tag.
- The spec-16 Postgres adapter paths are verified against a real database
  (they are currently exercised only DATABASE_URL-gated; CI has no PG).

#### M7.1 PgControlQueueRepo as default composition root

- [ ] Identify or create the production composition root (today
      `crates/api/examples/simple_server.rs` explicitly defers this — it
      pulls in-memory only). Decide where it lives (`nebula-cli` vs a
      dedicated `crates/server` vs lifted into `nebula-api`).
- [ ] Composition root accepts a `ControlQueueRepo` impl from config and
      wires `PgControlQueueRepo` by default; `simple_server.rs` keeps the
      in-memory path behind an explicit `--queue=memory` opt-in + startup
      warning.
- [ ] Config schema surfaces queue backend selection + connection-string
      source.
- [ ] Restart-survival integration test: spawn → enqueue cancel → kill
      engine → respawn → cancel processes (currently lost on restart).
- [ ] Operator doc: "in-memory queue is dev-only" banner in
      `crates/storage/README.md` + release notes.
- [ ] `task db:migrate` includes the control-queue migration by default.

#### M7.2 Multi-process deployment limits documentation

- [ ] Release-notes section: "Nebula 1.0 multi-runner deployment with PG —
      what's enforced (Layer-1 lease) vs the spec-16 row-model adapter
      surface".
- [ ] Operator runbook: detecting + recovering from runner restart (lease
      expiry, takeover, in-flight command replay).
- [ ] Empirical scale-out numbers: max concurrent runners, throughput
      ceiling per PG instance, lease-renewal tradeoffs (reproducible
      benchmark scripts under `benches/`).
- [ ] `crates/storage/src/lib.rs` doc-comment points operators at the
      runbook + release notes.

#### M7.3 Loom probe nightly CI

- [ ] Add `.github/workflows/loom-nightly.yml` (cron + `workflow_dispatch`);
      run `cargo test -p nebula-storage-loom-probe --features loom` on the
      pinned toolchain.
- [ ] Failure surfaces as a required check (or escalates to issue
      auto-create after 3 consecutive red runs); CODEOWNERS notification on
      storage-path failures.
- [ ] Docs entry: reproduce locally + debug a loom red. Document the
      nightly contract inline in `lefthook.yml` / CI docs so the
      lefthook↔CI parity gate covers it.

**Exit:** prod deployments wired through `PgControlQueueRepo` by default;
restart-survival integration test green; nightly loom job runs and gates
the 1.0 tag; spec-16 Postgres adapter paths verified against a real PG;
operator runbook + release notes cover deferrals honestly.

### M8 — Engine concurrency verification

Closure criteria (on top of the global DoD):

- Engine runs under `cfg(loom)` exhaustive-schedule coverage for the three
  historically bug-prone seams (lease handoff, registry mutate, cancel-token
  handoff).
- DashMap loom-hostility resolved via `cfg(loom)` substitute or by
  extracting a lock-free struct loom can model.
- Property tests cover lease-fence + registration-nonce invariants beyond
  example-based tests.
- Nightly runs gate the 1.0 tag (shared with M7.3).

#### M8.1 Loom in nebula-engine

- [ ] Add a `loom` feature flag mirroring `nebula-storage-loom-probe`.
- [ ] Substitute `DashMap` under `cfg(loom)` (loom-friendly map or
      single-writer extraction loom can model exhaustively).
- [ ] Loom probes: lease renewal across simulated runner takeover;
      `running_registry` insert/remove during cancel; cancel-token handoff
      (parent → child under arbitrary scheduling).
- [ ] Each probe also runs in the M7.3 nightly CI.

#### M8.2 Property tests for invariants

- [ ] Lease fence: never two concurrent holders for the same execution.
- [ ] Registration nonce: duplicate-nonce rejected, different-nonce
      accepted.
- [ ] Third candidate (TBD on runtime audit): retry-policy decisions
      deterministic under shuffled wall-clock (M2.1 path).
- [ ] Tests run under `task dev:check` if fast enough; otherwise nightly
      with a clear escape-hatch flag.

**Exit:** loom suite green under nightly; property tests cover the three
invariants; multi-runner deployments have model-checked concurrency
contracts.

### M9 — Observability + DoD audit pass

Closure criteria (on top of the global DoD):

- Every hot-path boundary has the typed-error + tracing-span +
  invariant-check triple. Gaps are filed and closed before tag, not
  deferred.
- OpenTelemetry exporter end-to-end verified against a real OTLP collector
  (`task obs:up`) — no "config exists, never tested" state.
- The two known mutex hot-paths (#595 / #590) either have a recorded
  low-contention measurement or are refactored lock-free.

#### M9.1 Hot-path observability sweep

- [ ] Inventory hot-path boundaries: engine state transitions, control
      dispatch, sandbox spawn, storage CAS retries, retry-loop entry, lease
      acquire/renew, action dispatch, expression hot path, eventbus
      publish/subscribe.
- [ ] Per boundary verify: typed `thiserror` variant (extend the parent
      enum, not `String`); `tracing::Span` with structured fields (no
      string interpolation) or `tracing::event!` at the right level;
      invariant check (`debug_assert!` / typed-error early return).
- [ ] Per-crate gap report under `crates/<crate>/docs/observability.md` (or
      appended to the crate README observability section); each gap tracked
      as a sub-PR linked here — no "file an issue later".
- [ ] Verification gate: workspace `clippy::missing_errors_doc` + an audit
      checklist (grep heuristics for `String` error variants, missing
      `#[instrument]`, missing `debug_assert!` near lock acquisitions).

#### M9.2 OpenTelemetry bridge verification

- [ ] Read #598 + comments to capture the open question.
- [ ] Inventory current OTLP setup in `nebula-metrics` / `nebula-log`:
      implemented vs documented.
- [ ] If the bridge is missing: implement an OTLP exporter (metrics +
      traces) wired into the `MetricsRegistry` snapshot path.
- [ ] Integration test: trace + metrics flow into a local OTLP collector
      (`task obs:up`), verified via Jaeger UI probe or collector debug
      output.
- [ ] Cross-dep: M3.5 trace-context propagation provides the span tree this
      exporter ships.
- [ ] Operator doc: point `OTEL_EXPORTER_OTLP_ENDPOINT` at a real collector
      + what fields appear (worked example).

#### M9.3 Hot-path Mutex audit

- [ ] **#595 (metrics OTLP label allocation):** measure allocation cost on
      the export path; if hot, switch to a pre-interned `LabelKey` pool /
      arena; if cold, mark Out-of-Scope with the measurement attached.
- [ ] **#590 (expression regex_cache):** verify the moka migration (PR
      #625) closed the contention — stress test concurrent regex
      compilation; update the snapshot if a regression is found.
- [ ] Per #issue: comment with measurement + decision; close or move to 1.1
      with explicit rationale.

- [x] **M9.4** Metrics / telemetry crate boundary — DONE (2026-05-06,
      ADR-0046, PRs #652–#656). The unenforced `nebula-telemetry` ↔
      `nebula-metrics` boundary replaced with intra-crate module discipline;
      primitives absorbed (#653); `MetricsAdapter` deleted (#654); re-audit
      + cardinality/HELP quick wins (#655/#656).

**Exit:** observability gap report committed and gaps closed; OTLP exporter
verified end-to-end; the two mutex hot-paths each carry a measurement +
decision; spans/metrics/errors triple present at every new boundary.

### M10 — Documentation + DX + release process

Closure criteria (on top of the global DoD):

- A new contributor can build, test, and ship a plugin from `README.md` +
  `examples/` alone — no "ask the maintainer" step.
- The release procedure is documented end-to-end (tag → publish →
  announce) and is either fully manual with a runbook OR ships minimal
  automation; mixed states are out.
- `lefthook pre-push` mirrors every CI required job.
- `cargo doc --no-deps --workspace` is warning-free and broken intra-doc
  links forbidden in CI.

#### M10.1 Root `examples/` workspace member

- [ ] `examples/workflow_action/` — minimal end-to-end (define → register
      → run; output compared against expected).
- [ ] `examples/credential/` — declare + register a credential type;
      round-trip encryption + AAD binding + zeroize-on-Drop with a runnable
      assertion.
- [ ] `examples/plugin/` — third-party-style plugin built against
      `nebula-plugin-sdk` only (no internal imports); load + invoke from a
      host driver.
- [ ] `examples/resource_topology/` — promote/polish the M6.3
      `m6_postgres_pool` / `m6_resident_http` / `m6_telegram_multi_workflow`
      READMEs.
- [ ] Each example: top-level README with a runnable command + expected
      output snippet.
- [ ] CI: `task examples:check` builds + runs every example.

#### M10.2 Per-crate README quality pass

- [ ] Audit each `crates/*/README.md` against a shared template (Purpose /
      Public API / Usage / Status / Invariants / Related ADRs).
- [ ] Compile-checked doctests for every public-API crate (`nebula-sdk`,
      `nebula-plugin-sdk`, `nebula-credential`, `nebula-action`,
      `nebula-resilience`, `nebula-metrics`, `nebula-error`).
- [ ] Cross-link relevant ADRs + canon sections (no "see canon §X" without
      a link).
- [ ] `cargo test --workspace --doc` green.

#### M10.3 Release process resolution

- [ ] ADR: pick the path explicitly (manual + runbook vs minimal
      tag-triggered automation). `release.yml` was removed deliberately to
      cut Actions noise — re-adding it is a decision, not a default.
- [ ] If automation: minimal `release.yml` for tag → `cargo publish` per
      crate in dependency order, dry-run first; no Actions noise beyond the
      publish step.
- [ ] If manual: runbook (tag, version bump, changelog, publish order,
      post-publish verification).
- [ ] Post-1.0 versioning policy doc (semver scope: API vs internal,
      deprecation window).
- [ ] CHANGELOG strategy: git-cliff (#599) vs hand-curated; record in the
      same ADR.

#### M10.4 Lefthook == CI parity

- [ ] Compare `.github/workflows/ci.yml` required jobs vs `lefthook.yml`
      `pre-push` hooks; update to match with no silent drift.
- [ ] CI parity-check job that lints divergence between the two configs and
      fails the workflow on drift.
- [ ] Refresh procedure documented inline in `lefthook.yml`.

> Note: the lefthook pre-push gate does **not** currently mirror the CI
> "Documentation" job — a touched-crate `RUSTDOCFLAGS=-D warnings cargo doc
> -p <crate> --no-deps` step is the known parity gap to close here (see
> M10.5).

#### M10.5 cargo doc cleanliness

- [ ] `cargo doc --no-deps --workspace` returns 0 warnings on the pinned
      toolchain.
- [ ] `-D rustdoc::broken_intra_doc_links` enforced workspace-wide via
      `[workspace.lints]` (verify present; add if missing).
- [ ] CI required job: `cargo doc` cleanliness gate; lefthook parity (M10.4)
      includes the per-crate rustdoc step.
- [ ] Per-crate README + doc-comment links validated (no 404s; a cron-based
      external-link check counts). Dead-link sweeps must union **all** link
      forms (`docs/adr/NNNN`, `](adr/NNNN`, `](../adr/NNNN`) — an
      adr-dir-only grep has shipped dead links before.

**Exit:** root `examples/` ships 4+ runnable examples with output checks;
per-crate READMEs follow the template; release process captured by an ADR
+ runbook (or automation); lefthook == CI verified in CI; `cargo doc`
green with broken-intra-doc-links forbidden.

### M11 — Dependency redesign (action / resource / credential v4) — ✅ DONE

> Tracked separately from §M6 because the dependency-redesign cascade is
> bigger than M6 itself; M6 consumed the new APIs while this milestone
> delivered them.

- [x] **M11.1** Slot-binding pattern — DONE. `#[resource(key=…)]` /
      `#[credential(key=…)]` per-field attributes replace the
      `DeclaresDependencies` boilerplate + singular `Resource::Credential`;
      field-type matrix (bare guard, `Option<Guard>`, `Lazy<Guard>`,
      `Option<Lazy<Guard>>`).
- [x] **M11.2** `type Input`/`type Output` on base `Action` — DONE.
      Variant-A trait shape; sub-traits inherit `<Self as Action>::Input/Output`.
- [x] **M11.3** Supersede ADR-0036 — DONE via
      [`adr/0044-supersede-0036-resource-credential-singular.md`](adr/0044-supersede-0036-resource-credential-singular.md).
      `Resource::Credential` removed; per-slot
      `Resource::on_credential_refresh(&mut self, slot_name)` hook.
- [x] **M11.4** `FromWorkflowNode` async factory — DONE. Per-execution
      Action instances built by `from_workflow_node`; `#[derive(Action)]`
      emits the body; engine consults `ActionRegistry::get_factory()` first;
      legacy `Arc<dyn XxxHandler>` retained for 4 production paths (webhook
      routing, sandbox discovery, SDK runtime, EventSource adapter).
- [x] **M11.5** Per-slot credential rotation fan-out — **DONE** (orchestration
      / dispatch closed via ADR-0067 #688 + fan-out #690; #703 wired
      resource acquire through the `acquire_erased` lease pipeline). The
      `(CredentialId, ResourceKey, slot_name)` reverse-index + `&self`/
      `&Runtime` reentrant dispatch ship with order-sensitive epoch fold,
      revoke-dedupe, warmup-post-taint, single-owner drain, sealed trait.
      **Residual:** no production credential→slot **bind-population**
      resolver (`register_and_bind` has a quiesce contract, zero callers) —
      tracked under **M12.4**; `nebula-resource` stays `frontier` until
      that lands.
- [x] **M11.6** Derive macros + dispatch infrastructure — DONE. 5 macros
      (`#[derive(Action)]`/`Resource`/`Credential`/`Schema`, `#[action]`);
      `ActionFactory` + `ErasedAction` enum + generic factories.

**Exit:** §M11.1–§M11.6 DONE; the only residual is M11.5 bind-population,
folded into M12.4. **Closed 2026-04-29** (fan-out dispatch landed
2026-05 via #688/#690/#703). Verification gate: `cargo deny + clippy +
test + doc + build --examples` green; trybuild probes for all 5 macros
pass.

### M12 — Business-layer crate hardening (frontier → stable)

> M11 shipped the dependency-redesign primitives, but the business-layer
> crates consuming them stayed `frontier` (action, resource) / `partial`
> (plugin), and `nebula-credential-builtin` is still a scaffold. M12 closes
> those gaps so business-layer crates reach `status: stable` for 1.0.

Closure criteria (on top of the global DoD):

- Every business-layer crate flips `status: frontier|partial → stable`
  (action, credential, credential-builtin, resource, plugin), backed by a
  recorded row in [`MATURITY.md`](MATURITY.md).
- The `frontier`/`partial` markers in per-crate Maturity sections are
  removed because the gaps are closed, not silently relaxed.
- Cross-trait integration tests cover the action × credential × resource ×
  plugin matrix that today reads as "partial".

#### M12.1 nebula-action — frontier → stable

- [ ] Land `CheckpointPolicy` on `ActionMetadata` + propagate through
      engine consumption end-to-end (today "do not document as a current
      capability").
- [ ] Cross-trait integration tests for `PaginatedAction` / `BatchAction` /
      `WebhookAction` / `PollAction` × resource + credential slot bindings.
- [ ] Decide the `ActionHandler` enum + per-variant `XxxHandler`
      retirement: close the 4 legacy production paths (webhook routing,
      sandbox discovery, SDK runtime, EventSource adapter) to factory-only,
      OR document the dual surface as a 1.0 contract.
- [ ] Action README frontmatter: `status: frontier → stable`.

#### M12.2 nebula-credential — finish hardening

> **Strategic debt:** [`docs/plans/2026-05-25-001-refactor-credential-architecture-debt.md`](plans/2026-05-25-001-refactor-credential-architecture-debt.md) documents 8 architectural findings (capability model, ResolveResult gaps, store atomicity, bidirectional auth, registry namespacing, macro fragility, StaticProtocol vestige, handle invariance) with a phased resolution timeline. None block 1.0; all should be reviewed when credential scope expands beyond OAuth2/API-key.

- [ ] Audit ADR-0032 storage-layer composition (`EncryptionLayer`,
      `CacheLayer`, `AuditLayer`, `ScopeLayer`) for production wiring gaps —
      each layer has a typed-error surface, observability triple, and a
      PG-backed integration test.
- [ ] AAD-policy completeness check (SEC-11 hardening removed AAD-free
      `encrypt`; verify all call-sites carry AAD).
- [x] Capability sub-trait coverage matrix (`Interactive`, `Refreshable`,
      `Revocable`, `Testable`, `Dynamic`) — each capability exercised by ≥1
      in-tree concrete type. **Closed 2026-05-20:** sealed-capability lie
      rejection probe (Task 6) — each capability sub-trait enforced by a
      compile-fail probe; `Dynamic` exercised by `AnyCredential` dyn-compat
      probe (Task 21). Subtrait coverage recorded in `MATURITY.md`.
- [ ] Wire `nebula-api` onto the `nebula-credential-runtime`
      `CredentialService` facade (ADR-0066 #678) — **PARTIAL (2026-05-20)**:
      `AppState` now holds `CredentialService`; the OAuth domain code still
      consumes `scoped_store` via `CredentialScopeLayer`. Full OAuth path
      migration and `CredentialScopeLayer` deletion from `nebula-tenancy` are
      a follow-up PR (not gating credential crate stability).
- [x] Close the ADR-0052 residual: the `slot_bindings` confused-deputy
      Non-goal is still open (not closed by the cascade) — decide ship vs
      formally defer with rationale. **Closed 2026-05-20:** `ValidatedCredentialBinding`
      newtype with crate-private constructor (Task 12) — only
      `CredentialService::resolve_for_slot` may produce a binding; confused-deputy
      pattern structurally impossible at the type level.
- [x] Credential README frontmatter `frontier → stable`; subtrait coverage
      recorded in `MATURITY.md`. **Closed 2026-05-20** (this PR). Both
      `nebula-credential` and `nebula-credential-runtime` flipped to `stable`.

#### M12.3 nebula-credential-builtin — concrete types

- [ ] **ADR:** what concrete credential types ship in 1.0? Today the crate
      is a scaffold (`src/lib.rs` only). Decide a small generic core
      (`GenericOAuth2`, `GenericPat`, `GenericApiKey`, `GenericBasicAuth`,
      `AwsSigV4`) vs the original full vendor list.
- [ ] Implement the chosen set with the standard pattern
      (`#[plugin_credential(...)]` + sealed capability traits per ADR-0035
      §3 + integration tests + token refresh where applicable).
- [ ] Each shipped type: integration test, observability triple, doctest in
      the crate README.
- [ ] OAuth provider configs surfaced via operator-facing config schema
      (cross-dep with M3.1 production `AuthBackend`).
- [ ] Credential-builtin README rewritten (no "scaffold" disclaimer);
      frontmatter `status: scaffold → stable`. Reword the Out-of-Scope
      "Telegram / OAuth provider integrations" item (today it implies
      coverage that does not exist).

#### M12.4 nebula-resource — frontier → stable

- [ ] **Bind-population resolver (M11.5 residual):** ship the production
      credential→slot bind-population path so `register_and_bind` has real
      callers (currently a quiesce contract with zero callers — the reason
      `nebula-resource` is still `frontier`).
- [ ] Audit `crates/resource/plans/` for non-SUPERSEDED plans (`01-core`,
      `02-topology`, `03-infrastructure`, `04-recovery-resilience`,
      `05-manager`, `07-implementation`, `08-correctness`,
      `09-topology-guide`, `naming-audit`, `type-cross-reference`,
      `resource-author-contracts`, `resource-hld`) — for each: shipped,
      ship-for-1.0, or formally defer to 1.1.
- [ ] Pull "v2 deferred" items into 1.0 where they materially affect 1.0
      grade (`ConnectionAware` disconnect detection, `InfraProvider` nested
      lifecycle, `ResourceGroup`, `Authenticate<C>` design) — OR defer in
      Out-of-Scope with rationale.
- [x] Pre-expiry credential refresh decision — **deferred to 1.1** per [ADR-0084](adr/0084-pre-expiry-credential-refresh-deferred.md).
      Reactive path (L1 OnceCell + L2 RefreshClaimRepo) remains the contract for 1.0.
- [ ] Engine frontier-loop per-branch wiring of `ResourceAction::configure`
      / `cleanup` — needs branch-tree dominator analysis. Either ship or
      defer with an explicit "scoped resources require a manual driver"
      banner.
- [ ] Resource README frontmatter: `status: frontier → stable`.

#### M12.5 nebula-plugin — slice B (partial → stable)

- [ ] Land ADR-0027 (`ResolvedPlugin`, namespace invariant, registry
      accessors).
- [ ] Replace the legacy API with the slice-B replacement (`PluginManifest`
      canonical home in `nebula-metadata`, re-exported from `nebula-plugin`
      for source compat); remove the "if README and code disagree, trust
      the code" disclaimer.
- [ ] `ResolvedPlugin::from(impl Plugin)` validation hardening:
      duplicate-key + namespace-invariant integration tests.
- [ ] `PluginRegistry` accessors (`all_*`, `resolve_*`) covered across
      multiple registered plugins.
- [ ] Cross-dep with M5 (ABI promise) for loader validation behaviour.
- [ ] Plugin README frontmatter: `status: partial → stable`; disclaimer
      removed.

**Exit:** action / credential / credential-builtin / resource / plugin all
reach `status: stable`; 1.0 ships a defined credential-types set (not
"scaffold + planned"); M11.5 bind-population lands; M12.4 either ships or
formally defers each non-SUPERSEDED resource plan.

### M13 — Core-layer 1.0 polish (frontier → stable)

> Core-layer crates carry types every other crate imports (IDs, schemas,
> expressions, workflow definitions). The `frontier` markers on
> `nebula-core` (Role/Permission/Tenancy "may see breaking changes") and
> `nebula-metadata` mean any caller's 1.0 contract is fragile until these
> stabilize.

Closure criteria (on top of the global DoD):

- Core / metadata flip `status: frontier → stable`; the "may see breaking
  changes" disclaimers are removed.
- Schema / validator / expression have a coverage matrix in
  [`MATURITY.md`](MATURITY.md).
- Workflow / execution invariants documented + tested.

#### M13.1 nebula-core — Role/Permission/Tenancy stability lock-in

- [ ] Audit Role / Permission / Tenancy / slug modules for breaking changes
      still on the table.
- [ ] Land an ADR pinning the public surface; future changes go through the
      M5 / M10.3 deprecation policy.
- [ ] Compat fixture set covering identifier round-trips (prefixed-ULID
      parse + format + reject malformed).
- [ ] `SecretString` Debug-redaction integration test (forbid leaking
      through structured fields).
- [ ] Core README frontmatter: `status: frontier → stable`.

#### M13.2 nebula-metadata — finalize semantics

- [ ] `MaturityLevel` semantics doc: what each level means operationally +
      how engine/catalog consume it.
- [ ] Deprecation-flow integration test (registering a deprecated component
      warns, blocks new dependents).
- [ ] `BaseMetadata<K>` derive-emission audit (used by every catalog
      citizen — Action / Credential / Resource).
- [ ] Metadata README frontmatter: `status: frontier → stable`.

#### M13.3 nebula-schema — derive completeness coverage

- [ ] `#[derive(Schema)]` edge-case matrix: nested types, `Option<T>` /
      `Vec<T>` / `HashMap<K,V>`, optional-with-default, sensitive (redacted)
      fields, custom validators.
- [ ] Each edge case: trybuild probes (compile-fail + compile-pass).
- [ ] Validation cross-link with `nebula-validator` (schema-level validation
      invokes combinators correctly).
- [ ] Schema README stays `stable`; coverage row in `MATURITY.md`.

> Note: trybuild tests can false-TIMEOUT under the nextest `agent` profile
> on a cold cache — confirm with a warm plain `cargo test`; never
> `TRYBUILD=overwrite` a timeout.

#### M13.4 nebula-validator — combinator coverage

- [ ] Audit combinator coverage against
      `crates/validator/tests/fixtures/compat/error_registry_v1.json`
      (canonical stable error-code registry).
- [ ] Every combinator: ≥1 fixture row + unit test + doctest.
- [ ] Validator README stays `stable`; coverage row in `MATURITY.md`.

#### M13.5 nebula-expression — language extension decisions

- [ ] Decide the 1.0 expression-language surface: lock the current grammar
      OR ship a v2 grammar before tag.
- [ ] If lock: ADR pinning the grammar + a syntax-change deprecation
      policy. If v2: spec + v1→v2 migration guide for stored workflows.
- [ ] Verify the regex_cache moka migration (PR #625) under stress (M9.3
      #590 follow-up).
- [ ] Expression README stays `stable`; coverage row in `MATURITY.md`.

#### M13.6 nebula-workflow — DAG validation completeness

- [ ] Audit `validate_workflow` against the `WorkflowDefinition` shape:
      every activation-time constraint has a typed error variant +
      invariant check + test.
- [ ] Spec-28 §2.2 port-driven routing (M1.3 closed) verified by an
      integration test set exercising every routing pattern in
      `crates/workflow/docs/Architecture.md`.
- [ ] Workflow README stays `stable`; the `connection.rs`/`builder.rs`
      "historical context" block archived to `adr/HISTORICAL.md` if not
      already.

#### M13.7 nebula-execution — state machine invariants

- [ ] State-machine transition diagram (in
      `crates/execution/docs/state-machine.md` or the README) matching the
      actual `transition_node` allow-list.
- [ ] Property test: every legal transition reachable; every illegal one
      rejected with a typed error.
- [ ] Invariant audit: `let _ = transition_node(...)` silently swallows
      invalid-transition errors — reject the pattern via lint or grep gate;
      always use `transition_node`'s typed result.
- [ ] Execution README stays `stable`.

**Exit:** core / metadata flip frontier → stable; schema / validator /
expression / workflow / execution have coverage matrices in `MATURITY.md`;
state-machine invariants documented + tested.

### M14 — Cross-cutting maturation + Public API freeze

> Cross-cutting crates carry the seams every other crate uses (eventbus,
> log, metrics, resilience) plus the public surface plugin/integration
> authors consume (sdk, plugin-sdk). Mostly `stable` but with known gaps:
> `ExecutionEvent` still on raw mpsc, no Health trait abstraction,
> `ExecutionCommandService` still inline in the cancel handler.

Closure criteria (on top of the global DoD):

- `MATURITY.md` is the authoritative dashboard for every crate's status,
  coverage, and known gaps (the file now exists — verify completeness, not
  create).
- `ExecutionEvent` migrates from raw `mpsc` to `nebula-eventbus` so
  multi-subscriber consumers stop reinventing broadcast.
- `nebula-sdk` and `nebula-plugin-sdk` flip `status: partial → stable` with
  a frozen 1.0 public surface.
- Health trait extracted; command service extracted.

#### M14.1 docs/MATURITY.md — verify the dashboard

- [ ] `docs/MATURITY.md` exists (~20 KB). Verify the schema: row per crate
      × columns (`status`, `API stability`, `test coverage`, `loom
      coverage`, `chaos coverage`, `known gaps`, `1.0 readiness`) — fill
      missing rows; reconcile against post-spec-16 / post-ADR-0052 reality.
- [ ] Every per-crate README's "See `docs/MATURITY.md` row for `nebula-XXX`"
      link resolves.
- [ ] CI gate: file existence + link resolution from per-crate READMEs
      (catches future regressions).

#### M14.2 nebula-eventbus — ExecutionEvent migration

- [ ] Migrate `ExecutionEvent` from raw `mpsc` to `EventBus<ExecutionEvent>`
      — it is still on raw mpsc and multi-subscriber consumers reinvent the
      channel (`nebula-eventbus` is already stable, used by engine for
      `CredentialEvent`).
- [ ] Engine publishes via the eventbus; no direct `mpsc::send` from engine
      to event consumers.
- [ ] ≥2 subscribers in tests (e.g. metrics emitter + log tailer) verify
      broadcast semantics.
- [ ] Refine `Registry` / `Scope` patterns based on actual engine usage.

#### M14.3 nebula-sdk — partial → stable (1.0 surface freeze)

- [ ] Audit `prelude` exports — every type has a 1.0 stability commitment.
- [ ] `WorkflowBuilder` / `ActionBuilder` / `TestRuntime` API frozen via
      ADR.
- [ ] `testing` module + `TestRuntime` harness coverage filled in.
- [ ] `simple_action!` macro coverage decision: extend to
      stateful/trigger/resource-backed shapes OR document the constraint
      explicitly.
- [ ] `anyhow` re-export decision: keep (with a typed-error opt-in path) OR
      retire for 1.0 with migration guidance.
- [ ] SDK README frontmatter: `status: partial → stable`.

#### M14.4 nebula-plugin-sdk — partial → stable

- [ ] Land ADR-0006 slice 1d: `PluginCtx` broker RPC accessors +
      `PluginSupervisor` (`PluginCtx` is currently a placeholder with no
      methods).
- [ ] Capability negotiation in the handshake (cross-dep with M4.1).
- [ ] Protocol versioning becomes a tested contract.
- [ ] Retire the 1 flagged panic site to a typed error variant.
- [ ] Test-coverage lift: handshake + duplex envelope happy/error paths
      covered by integration tests.
- [ ] Plugin-sdk README frontmatter: `status: partial → stable`.

#### M14.5 nebula-resilience — feature gap closure

- [ ] Cross-dep with M2: action-internal retry stays here; verify no
      engine-internal retry regression on the Layer-1 path.
- [ ] Audit against the fix-pipeline-retry PR (#639) follow-ups for
      residual hardening.
- [ ] Resilience README stays `stable`; observability triple verified per
      M9.1.

#### M14.6 nebula-log — refinements

- [ ] File rolling + runtime reload audit under load.
- [ ] Log README stays `stable`.

#### M14.7 Health trait extraction

- [ ] Generalize `TriggerHealth` into a `Health` trait (when Resource /
      Agent need a uniform health surface).
- [ ] Resource health / Agent health surfaces consume the same trait.
- [ ] Engine exposes aggregate health for a `/healthz` endpoint surfacing
      per-resource / per-agent state.

#### M14.8 ExecutionCommandService extraction

- [ ] Extract the §12.2 orphan contract from the inline `cancel_execution`
      handler into an `ExecutionCommandService` — it is inline today, so any
      new command transport (queue, gRPC, CLI) would duplicate the contract.
- [ ] Cancel handler delegates to the service.
- [ ] Service has a typed error surface + observability triple +
      integration test covering the orphan contract that was inline.

**Exit:** every crate carries a real row in `MATURITY.md`; sdk + plugin-sdk
reach `status: stable`; `ExecutionEvent` flows through the eventbus; Health
trait + `ExecutionCommandService` extracted.

## Out of scope for 1.0

Explicit deferrals — must not silently slip into 1.0 scope:

- ADR-0013 compile-time modes (`build.rs` / `mode-*` features) — accepted
  but unimplemented; not a 1.0 blocker.
- Vendor-specific credential provider packs beyond the small generic-core
  set M12.3 ships in `crates/credential-builtin` (vendor packs are 1.1+;
  the 1.0 surface is the chosen generic types per the M12.3 ADR).
- WebSocket endpoint (`crates/api/.../websocket.rs` returns 501) — ship 1.0
  without a realtime API; document as 1.1.
- Performance regression testing harness (#600 loadgen-rs investigation).
- ADR-0024 dyn-trait migration (#601).
- Automated CHANGELOG generation via git-cliff (#599) unless M10.3 picks it.
- The ADR-0051 external-source bridge (`External = ExternalSourceNotWired`
  in `nebula-credential-runtime`) — unbuilt; not a 1.0 blocker unless an
  external credential source becomes a 1.0 requirement.

> **Note (was deferred, now landed):** the spec-16 storage port / adapter /
> tenancy redesign ("Sprint E / Layer 2") **shipped on `main`**
> (`refactor(storage)!: spec-16 port/adapter/tenancy redesign`; ADR-0072
> deleted the legacy `ExecutionRepo` / `WorkflowRepo` surface). It is no
> longer an Out-of-Scope deferral. The only residual is verifying the
> Postgres adapter paths against a real database (no `DATABASE_URL` in CI)
> — folded into **M7**.

## Sub-project ordering rationale

Not all parallelizable. Suggested ordering (M0–M2, M6, M11 closed
2026-04-29 → 2026-05; remaining in flight):

1. **M0 (durability)** — small, foundational, removes false claims. ✅ DONE.
2. **M3 (API)** in parallel with M0 — biggest user-facing gap; sliceable
   (M3.1 partial, M3.2 ✅, M3.3 ✅, M3.4 ✅, M3.5 ✅ engine-prop / test +
   OTLP open, M3.6 open).
3. **M1, M2 (engine correctness + retry)** after M0 — same `engine.rs`
   paths. ✅ DONE.
4. **M4, M5 (plugin capability gate + plugin ABI contract)** in parallel
   with M3.
5. **M6 (resource finalization)** independent. ✅ DONE (bind-population
   residual → M12.4).
6. **M7 (storage ops), M8 (loom)** — late, after engine work settles;
   M7 now also owns spec-16 PG verification.
7. **M9 (observability sweep)** — late; M9.4 closed via ADR-0046.
8. **M11 (dependency redesign)** — landed before M6 closure. ✅ DONE.
9. **M12 (business-layer hardening)** — parallel with M3 completion; needs
   M11 (DONE); M12.5 depends on M5 ABI ADR; M12.4 owns the M11.5
   bind-population residual.
10. **M13 (core-layer polish)** — independent of M3–M9; M13.1 gates anyone
    consuming Role/Permission/Tenancy.
11. **M14 (cross-cutting + public API freeze)** — gates the 1.0 tag;
    requires M12 + M13 substantially done so `MATURITY.md` (M14.1) and SDK
    / plugin-sdk freeze reflect a stable surface.
12. **M10 (docs/release)** — last; gate before tag, after M14 finalizes the
    public surface.

## Next step

Open a plan under `docs/plans/<milestone-or-branch>.md` for the chosen
sub-project (the roadmap entry's bullets become the tasks). Branch via
`scripts/worktree.sh new <slug> <type> <scope>` per the
[`CLAUDE.md`](../CLAUDE.md) Agent Git Workflow; squash-merge back to `main`.
