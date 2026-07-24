---
name: Nebula crate maturity dashboard
description: Manual per-crate state dashboard. Edited in PRs that change a crate's API stability, test coverage, doc state, engine integration, or SLI-readiness.
status: accepted
last-reviewed: 2026-07-23
related: [PRODUCT_CANON.md, STYLE.md]
---

# Crate maturity dashboard

Updated manually in PRs that touch a crate's public surface, test bar, docs, engine integration, or observability. Reviewers check that this file stays truthful.

Legend:
- `stable` — end-to-end works, tested, safe to depend on.
- `frontier` — actively moving; breakage expected; do not add consumers without coordinating.
- `partial` — works for declared happy path; known gaps documented in the crate README.
- `n/a` — dimension does not apply to this crate.

| Crate | API stability | Test coverage | Doc completeness | Engine integration | SLI ready |
|---|---|---|---|---|---|
| nebula-action        | frontier | stable  | stable | partial (webhook sig `Required` by default at trait surface — ADR-0022; `checkpoint_policy` field present, default `Inherit`, non-`Inherit` enforcement not yet wired; no `ActionResult::Retry` surface — engine retry is driven by operator-declared `retry_policy` plus retryable errors) | n/a |
| nebula-api           | frontier | stable  | stable | partial (pure library — no binaries; composition root is `apps/server` (`nebula-server` crate); per-domain module structure under `domain/` (§12.7); god-files split: `config/`, `error/`, `middleware/idempotency/`; `services/` renamed `transport/`; behavior-neutral Phase 0 refactor — 239 total tests green (236 `nebula-api` + 3 `nebula-server`), OpenAPI drift byte-identical, knife §13 green; knife steps 3+5: Start/Cancel producers stable #332/#330; EngineControlDispatch ADR-0008 A2/A3; ADR-0016 cancel registry; Phases 1–4 stub completion still pending) | partial |
| nebula-server        | frontier | stable  | partial | partial (thin composition root; selects transport via `--transport=api\|webhook\|realtime\|all`; verbatim-ported transport runtime from pre-refactor `nebula-api` bins; 3 nextest tests green; Postgres idempotency path gated behind `postgres` feature) | n/a |
| nebula-core          | frontier | stable  | stable | stable (6 public modules: `role`, `permission`, `tenancy`, `slug`, `auth`, `guard` — `TenantContext`, `ResolvedIds`, `OrgRole`, `WorkspaceRole`, `Permission`, `PermissionDenied`, `Slug`, `SlugKind`, `SlugError`, `is_prefixed_ulid()`, `AuthScheme`, `AuthPattern`, `Guard`, `TypedGuard`, `BaseContext`, `Context`) | n/a |
| nebula-credential    | partial (**M12.2 hardening closed 2026-05-20** — error taxonomy reshape per Smithy RFC-0022 with per-variant context structs + boxed payloads + 32-byte size cap (closes #588), `SecretString` as thin wrapper over `secrecy::SecretBox<String>` with `ExposeSecret` trait surface for grep-able audits, `ValidatedCredentialBinding` newtype closes the `slot_bindings` confused-deputy non-goal from the ADR-0052 cascade, `CredentialService::resolve_for_slot` is the production bind-population seam, fallback-on-interrupt protects in-flight executions from transient provider failures, three-registry sync invariant probe + composite `register_credential_complete` close the silent-drift vector, dyn-compat probe locks the plugin registry against Rust 1.95 next-gen solver regressions. `AuthStyle` moved to `scheme::oauth2`. ADR-0084 defers proactive pre-expiry refresh to 1.1. **ADR-0092 consolidation (2026-06-10): the runtime (resolver/refresh/lease/rotation-state, ex-`nebula-engine`) + the `CredentialService` facade (ex-`nebula-credential-runtime`) + the builtin types (ex-`nebula-credential-builtin`) were all folded into this crate; `nebula-credential-testutil` was deleted; crypto extracted to `nebula-crypto`; heavy I/O inverted to `RefreshTransport`/`Cipher`/`Kdf` ports.** Follow-ups (see `crates/credential/docs/DESIGN.md`): finish the single `CredentialRuntime` pipeline (ADR-0088 D1/D2/D6 unfinished), design any future Plane-B browser interaction through universal acquisition, keep production credential adapters in `apps/server` (authority, persistence, catalog, and refresh composition landed 2026-07-22). The raw API OAuth ceremony is parked; `resolve` / `resolve/continue` is the supported HTTP acquisition surface.) | stable | stable | partial (**П2 refresh coordination L2 landed 2026-04-26** per PR [#583](https://github.com/vanyastaff/nebula/pull/583) — two-tier coordinator: in-process L1 (`L1RefreshCoalescer`) + durable L2 (`RefreshClaimRepo` per ADR-0041) with TTL/heartbeat/reclaim sweep; sentinel N=3-in-1h threshold emits a lossy `ReauthRequired` observation, while the owner-qualified durable sentinel transition remains a K3/1.0 blocker; 5 metrics + 3 spans + 3 audit events; nightly chaos test (3 replicas × 100 creds × 10 min) — closes n8n #13088 cross-replica `refresh_token_v1`-invalidates-`v2` race; engine `iter_compatible` slot-picker consumer wiring + manager-side `OnCredentialRefresh<C>` fan-out tracked as post-П1 follow-ups in credential concerns register; **security hardening 2026-04-27 SEC-cluster landed** per archived credential security hardening spec (the maintainers' private design vault) — SEC-01/02 IdP boundary (bounded reader + sanitize_error_uri), SEC-05/06 plaintext-lifecycle invariants (CredentialGuard !Clone, SchemeGuard !Send + !Sync), SEC-09/10 Zeroizing discipline (bearer_header buffer plus split prepare/dispatch/interpret refresh phases), SEC-11 bare crypto::encrypt removed from public surface, SEC-13 redact_sensitive_fields + ADR-0030 §4 redaction CI gate) | n/a |
| nebula-crypto        | stable   | stable  | stable | n/a (cross-cutting leaf — AES-256-GCM + Argon2id + `Cipher`/`Kdf` ports + `EncryptedData`/`key_id` envelope; extracted from `nebula-credential` per ADR-0088/0092; consumed by `nebula-credential` + `nebula-storage`) | n/a |
| nebula-env           | stable   | stable  | stable | n/a (cross-cutting typed environment reader — ADR-0086) | n/a |
| nebula-engine        | partial  | stable  | stable | partial (ControlConsumer skeleton lands §12.2; all five control commands dispatched via EngineControlDispatch — ADR-0008 A2 (Start/Resume/Restart) + A3 (Cancel/Terminate) + ADR-0016 cancel registry; ADR-0008 B1 reclaim sweep implemented via ControlQueueRepo::reclaim_stuck + ADR-0017; operator-declared retry is implemented via `retry_policy` → `WaitingRetry` / `next_attempt_at` / re-dispatch with `ExecutionBudget.max_total_retries`, while result-driven `ActionResult::Retry` is not a public surface; **the engine `credential` module is now bridge + test-harness only** — the runtime surface was relocated into `nebula-credential::runtime` per ADR-0092, engine keeps the credential/resource accessor bridges + `default_in_memory_coordinator`) | n/a |
| nebula-error         | stable   | stable  | stable | n/a | n/a |
| nebula-eventbus      | stable   | stable  | stable | n/a | n/a |
| nebula-execution     | stable   | stable  | stable | stable | partial |
| nebula-expression    | stable   | stable  | stable | stable | n/a |
| nebula-log           | stable   | stable  | stable | n/a | n/a |
| nebula-metadata      | frontier | stable  | stable | n/a | n/a |
| nebula-metrics       | stable   | stable  | stable | n/a | n/a |
| nebula-plugin        | partial  | stable  | stable | stable | n/a |
| nebula-resilience    | stable   | stable  | stable | n/a | n/a |
| nebula-storage-port  | stable   | stable  | stable | stable (ADR-0072 — object-safe row-model seam; every storage consumer depends on this and not on `nebula-storage`) | n/a |
| nebula-storage-loom-probe | partial | stable | partial | partial (`loom`-checked concurrency probes for storage critical sections; library, no SLI) | n/a |
| nebula-tenancy       | stable   | stable  | stable | stable (ADR-0072 policy plus decorators for the enumerated general Scope-taking ports; credential persistence is the separate owner-bound exception) | n/a |
| nebula-resource      | frontier | stable  | stable | stable for the operational substrate (ADR-0067): lock-free `SlotCell` slot store + `&self`+`&Runtime` refresh/revoke hook, narrow `Manager::{refresh_slot,revoke_slot}(_for)` port, structural slot-identity dedup key closing the cross-tenant `fingerprint()==0` bleed, config CRUD + CAS update + read-only runtime-status API with no HTTP lifecycle (INTEGRATION_MODEL §13.1), ADR-0028 §7 redaction gate green. The per-slot rotation fan-out **dispatch path is wired and e2e-proven** (2026-05-17; **relocated from `nebula-engine` into this crate's `credential_fanout/` per ADR-0092**): a `ResourceFanoutDriver` subscribes the `CredentialEvent::{Refreshed,Revoked}` + `LeaseEvent::LeaseRevoked` buses and drives `dispatch_refresh`/`dispatch_revoke` with per-resource drain + post-taint re-check (#679), a two-phase cancellation-safe revoke port (#681), and rotation-vs-first-acquire epoch reconcile (#680). **Still gating full §M11.5/§M12.4 closure:** *bind-population* — populating the reverse index when a credential resolves into a `#[credential]` slot in production — has no production caller (depends on the still-deferred resource-activation path: plugin-auto-population / a production `ResourceRepo`; rotation-outcome eventbus emission also still deferred per ADR-0067). A real rotation/revoke event now fans out correctly to every *bound* row, but production binds none until activation lands — so this stays `frontier`, not flipped to stable. | n/a |
| nebula-schema        | frontier | stable  | stable | stable | n/a |
| nebula-sdk           | partial (manual/builder one-dependency subset is supported; client, embedded, and all current Nebula procedural-derive families remain gaps) | partial (external fixture proves `ActionBuilder`, `WorkflowBuilder`, and credential `TestResult`; derive compile-pass fixtures are pending) | stable (gaps and unsupported direct-leaf workarounds documented) | n/a | n/a |
| nebula-storage       | partial  | partial (SQLite/internal reference suites are local evidence; live PostgreSQL remains a release gate) | stable | stable | partial |
| nebula-validator     | frontier | stable  | stable | stable | n/a |
| nebula-workflow      | stable   | stable  | stable | stable | n/a |

Credential authority/persistence delivery is intentionally staged:

- **K1 (delivered):** supported authenticated HTTP management authority, owner-bound technical
  persistence, and the verified manual/builder SDK subset.
- **K2 (current, implemented):** paired SQLite/PostgreSQL migration `0039` makes owner and
  live/tombstoned structure database invariants; ready-store admission prevents raw-pool bypass;
  statement `RETURNING` plus bounded versions provide linearizable secret-free commit receipts;
  and live PostgreSQL owner/concurrency suites are a required gate. Refresh/revoke now run behind
  one cancel-safe L1/L2 boundary: ambiguous provider or commit outcomes retain UUID-identified
  durable poison and never gain replay authority from elapsed TTL.
- **K3 (debt):** make the controller plus semantic idempotency/operation ledger the sole management
  writer; add owner-qualified poison reconciliation and the durable sentinel-to-reauth command;
  replace trace-only audit with transactional audit/outbox evidence.
- **K4 (debt):** ship supported apps-owned membership/deployment composition and complete SDK
  client/embedded/procedural-derive paths. Concrete credential adapters now live in `apps/server`,
  but the default server still leaves workspace-directory and membership policy unwired, so tenant
  routes return 503.

---

## Review cadence

This file is a living dashboard. Reviewers check truthfulness on every PR that touches a crate's public surface, test suite, or docs. Canon §17 DoD includes "MATURITY.md row updated if the PR changes crate state."

Last full sweep: 2026-04-17 (Pass 4 of docs architecture redesign).
Last targeted revision: 2026-07-22 — **K1 credential authority/persistence and SDK perimeter truth pass.**
Removed the raw credential OAuth authorization/callback routes and their API-owned
pending authority. The supported Plane-B HTTP acquisition contract is now only
`resolve` / `resolve/continue`; Plane-A identity OAuth remains mounted and public.
OpenAPI absence, exact runtime 404s, and source-structure regressions pin the boundary.
Prior targeted revision: 2026-07-09 — **retry scheduler canon reconciliation.**
Corrected PRODUCT_CANON §11.2 and crate docs/status to distinguish implemented
operator-declared engine retry (`retry_policy` → `WaitingRetry` / `next_attempt_at`
/ re-dispatch) from non-existent result-driven `ActionResult::Retry`; removed stale
mentions of the `unstable-retry-scheduler` feature, which is not present in the
current action/engine manifests.
Prior targeted revision: 2026-06-12 — **credential-subsystem inventory truth pass
(ADR-0092 as-built).** Removed dead rows `nebula-credential-runtime` and
`nebula-credential-builtin` (both crates deleted; contents folded into
`nebula-credential`). Added missing rows `nebula-crypto` and `nebula-env`.
Corrected `nebula-engine` (credential module now bridge + test-harness only) and
`nebula-resource` (per-slot rotation fan-out relocated into this crate). Also fixed
"engine owns orchestration" drift across PRODUCT_CANON §3.5/§13.2, STRATEGY,
root README + AGENTS layer maps, and the credential/engine/resource crate docs.
Prior targeted revision: 2026-06-10 — **`nebula-credential-testutil` deleted.**
Its two in-memory store shims (`InMemoryStore`, `InMemoryPendingStore`) were
redundant copies of the canonical `nebula_storage::credential` backends.
`nebula-tenancy` (Business-tier, cannot depend on the Exec storage adapter)
formerly kept a colocated credential-store double. That legacy credential scope/store seam was
removed in the 2026-07-22 authority and object-safe-persistence pass; current credential test
doubles implement the port-local `CredentialPersistence` contract.
Prior targeted revision: 2026-05-26 — **AI Factory retired entirely.**
The `.ai-factory/` directory, `.ai-factory.json` install manifest,
`.claude/skills/aif-*` (Claude variant) and `.github/skills/aif-*`
(Copilot variant) skill packs, and the `.claude/agents/` subagent fleet
(loop-*, *-sidecar, plan-*, implement-*, commit-preparer, docs-auditor)
were removed from the repository. Coding workflow now runs through
`task` + `worktree.sh` + guard hooks; AI tooling rules live in
`AGENTS.md`. External references in AGENTS.md, `crates/action/README.md`,
`crates/engine/src/lib.rs`, `crates/engine/tests/scoped_resources.rs`,
the maintainers' private design vault, ADR-0046, ADR-0050, this file, and the recon
working notes were rewritten in the same change. Earlier AI Factory
artifacts survive in `git log` only.
Prior: 2026-05-26 — **Crate inventory truth pass:** removed ghost rows for crates that no longer exist (`nebula-runtime`, `nebula-telemetry` — merged into `nebula-metrics` per ADR-0046; `nebula-testing` — never landed); added missing rows for crates that do exist (`nebula-storage-port`, `nebula-storage-loom-probe`, `nebula-tenancy`, `nebula-credential-vault`, `nebula-credential-testutil`). README crate map and `.github/copilot-instructions.md` synced in the same change.
Prior: 2026-05-20 — **M12.2 nebula-credential + nebula-credential-runtime stabilize sweep:**
`nebula-credential` API stability `frontier → stable` (error taxonomy reshape, SecretBox migration, ValidatedCredentialBinding, resolve_for_slot seam, fallback-on-interrupt, three-registry probe, dyn-compat probe, AuthStyle moved to scheme::oauth2, testutil extracted, ADR-0084 defers proactive refresh to 1.1). `nebula-credential-runtime` row added at `stable` (ADR-0066 facade extracted; all M12.2 hardening items in production path).
Prior: 2026-05-15 — **nebula-api Phase 0 structural refactor (behavior-neutral):**
worktree `restructure` (branch `refactor/api-restructure`) converts `nebula-api` from a
mixed lib+bin crate into a **pure library**; a new `apps/server` workspace member
(`nebula-server` crate) becomes the composition root with a single binary selecting
transport via `--transport=api|webhook|realtime|all`. Internal reorganization: god-files
split (`config.rs` → `config/`, `errors.rs` → `error/`, `middleware/idempotency.rs` →
`middleware/idempotency/`); `services/` renamed `transport/`; per-domain module structure
introduced under `domain/` (§12.7 knife seam); `simple_server.rs` example relocated to
root `examples/` member. Behavior-neutral: 239 total nextest tests green (236 `nebula-api`
+ 3 `nebula-server`), OpenAPI 3.1 spec and operationIds byte-identical (ADR-0047 drift
tests green), knife §13 green. The 501 honest stubs (`me/*`, `org/*`,
`execution/{terminate,restart}`, `resource/*`) are unchanged — Phases 1–4 will implement
the engine-honored subset. `nebula-system` crate row removed (crate deleted, orphan —
commit `6f5e72e9`). MATURITY rows: `nebula-api` note updated; `nebula-server` row added.
Prior: 2026-04-26 — **credential П2 refresh coordination L2 landed:**
worktree `worktree-credential-p2` lands cross-replica refresh coordination per sub-spec
ADR-0041 (refresh coordination; design archived — see the maintainers' private design vault) in
6 stages — Stage 1 storage infrastructure (`RefreshClaimRepo` trait + 3 impls + migrations
0022/0023 + loom CAS probe); Stage 2 engine refactor (`L1RefreshCoalescer` private + new
outer two-tier `RefreshCoordinator` composing L1 + L2 claim repo + sentinel set before
IdP POST); Stage 3 sentinel N=3-in-1h threshold + lossy `ReauthRequired` observation +
reclaim sweep with framework-owned coalesced re-read handling; the
`reauth_required` field exists on `StoredCredential` and provider rejection persists it,
but the sentinel-driven owner-qualified durable command remains K3; Stage 4 observability (5 metrics + 3 spans + 3
audit events) + nightly chaos test (3 replicas × 100 creds × 10 min); Stage 5 doc sync.
`nebula-credential` Engine integration handles the multi-replica mid-refresh race through
the durable claim repo but remains `partial` until the K3 sentinel command lands. Closes n8n #13088 class production race
where rotated `refresh_token_v2` invalidates `refresh_token_v1` on a parallel replica.
The concrete shared-L2 chaos harness lives in `nebula-storage`, is always ignored by ordinary
test gates, and nightly CI opts into its 10-minute plane with `--features chaos-full
--run-ignored only`. Public
API breaking change: `AuditOperation` lost `Copy` derive. PRODUCT_CANON anchor
`#132-rotation-refresh-seam` fixed in this PR (out-of-band cleanup).
Tracked under PR [#583](https://github.com/vanyastaff/nebula/pull/583) (squash-merge to `main`).
Prior: 2026-04-26 — **credential П1 trait scaffolding landed:**
worktree `worktree-credential-p1` lands the validated CP5/CP6 trait shape per
Tech Spec §15.4-§15.8 in 8 stages — capability sub-trait split (Tech Spec §15.4
— `Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic` replace 4 capability bools);
`AuthScheme` sensitivity dichotomy (§15.5 — `SensitiveScheme: AuthScheme + ZeroizeOnDrop`
vs `PublicScheme: AuthScheme`); fatal duplicate-KEY registration (§15.6 —
`Result<(), RegisterError>`); `SchemeGuard`/`SchemeFactory` refresh hook (§15.7);
capability-from-type authority shift (§15.8 — `iter_compatible(required: Capabilities)`).
ADR-0035 phantom-shim canonical form via `#[capability]` proc-macro and `#[action]`
attribute. 10 mandatory landing-gate compile-fail probes + 1 runtime probe green.
`nebula-credential-builtin` scaffold present (concrete catalog types land in flagship milestone 1).
Tech Spec frontmatter status flipped to «П1 in-implementation 2026-04-26»; §16.5.1
implementation tracker entry added; 5 architectural register rows flipped to
`in-implementation`; 5 stage5-followup process rows resolved.
Prior: 2026-04-24 — **credential scheme pruning + honest multi-replica status:**
removed `FederatedAssertion`/`OtpSeed`/`ChallengeSecret` schemes (Plane A / integration-internal);
removed corresponding `AuthPattern` variants (`FederatedIdentity`/`OneTimePasscode`/`ChallengeResponse`);
`nebula-credential` Engine integration downgraded `stable → partial` honestly — single-process
refresh coordination only, multi-replica race handling tracked in Spec H0. Archived 8-file
credential redesign exploratory drafts (archived — see the maintainers' private design vault) with STATUS.md
(not adopted: Q1 compile test + 4-agent review converged).
Prior: 2026-04-23 — **API routing infrastructure (spec 05):**
`nebula-core` gains 4 public modules (`role`, `permission`, `tenancy`, `slug`);
`nebula-api` gains tenant-scoped routes, RBAC/tenancy/CSRF middleware, cursor
pagination, new port traits, extended `ApiConfig`, 10 new `ApiError` variants,
6 new route files, 7 new handler files with TODO stubs.
Prior: 2026-04-23 — **nebula-credential architecture cleanup:**
retry dedup (→ `nebula-resilience`), `oauth2/` flattening, `accessor/`+`metadata/`
flattened to root modules, `AuthScheme`/`AuthPattern` moved to `nebula-core`,
eventbus removed, `ExternalProvider`/`CredentialMetrics`/prelude added,
`Guard`/`TypedGuard` on `CredentialGuard`, DYNAMIC credential support,
historical store-impl gating (then named `test-util`; concrete stores now live
in `nebula-storage`), rotation orchestration → engine, OAuth HTTP →
api/engine, `Cargo.toml` dep diet.
Prior: 2026-04-22 — **Test stack:** workspace `dev-dependencies` for
`insta`, `pretty_assertions`, `rstest`, `wiremock`, `mockall`, `assert_cmd`, `predicates`,
`assert_fs`; cross-links from `QUALITY_GATES` / `PRODUCT_CANON` §15;
example tests in `nebula-api`, `nebula-credential`, `nebula-storage`, `nebula-cli`.
Prior: 2026-04-21 — **P6–P11 credential cleanup:** add
credential cleanup P6–P11 plan (archived — see the maintainers' private design vault; ADR-0032 pointer, spec §12
rolled-up status: storage/engine/API phases landed); P1–P5 plan links to it; `credential/Cargo.toml`
tokio comment corrected (resolver/executor in engine).
Prior: 2026-04-21 — `nebula-credential`: trim `rotation/mod.rs` rustdoc noise;
fix `store_memory` / utility-module comments in `lib.rs`; README **Architecture cleanup status**
(ADR-0032 trait home, remaining `oauth2/flow.rs` follow-up). No API removal — cleanup was
already landed in P1–P8; deleting contract/runtime files would break the crate.
Prior: 2026-04-21 — ADR-0033 **implementation start**: `crate`-level `//!` docs
for Plane A vs Plane B (`nebula-api` `middleware::auth` vs `credential` / `routes::credential`;
`nebula-credential` `Credential` trait; `nebula-engine::credential`).
Prior: 2026-04-21 — `docs/INTEGRATION_MODEL.md` **correctness**: `AuthScheme` / `AuthPattern`
now canonical in **`nebula-core::auth`**, re-exported by `nebula-credential`;
`SecretString` / `CredentialEvent` remain in **`nebula-credential`**. `MATURITY.md` Plane B
wording includes **`AuthPattern`** alongside **`AuthScheme`**.
Prior: 2026-04-21 — `docs/INTEGRATION_MODEL.md` adds an **industry reference**
subsection (n8n credential taxonomy vs Nebula Plane B axes: acquisition /
`AuthScheme` / `AuthPattern` / persistence). Illustrative bucket counts from a public codebase;
not a Nebula API surface.
Prior: 2026-04-21 — ADR-0033 (historical)
names **Plane B (integration credentials)** vs future Plane A / `nebula-auth`, and
documents acquisition vs `AuthScheme` / `AuthPattern` vs persistence. Cross-links in
`docs/INTEGRATION_MODEL.md` and `crates/credential/README.md`.
Prior: 2026-04-21 — OAuth2 HTTP transport split: `nebula-credential`
gains Cargo feature `oauth2-http` (default on) with optional `reqwest`;
authorization URL construction lives in `oauth2/authorize_url.rs` without HTTP.
CI checks `cargo check -p nebula-credential --no-default-features`. Aligns with
ADR-0031 incremental relocation of token exchange out of the contract crate.
Prior: 2026-04-21 — P10 slice of credential cleanup completed:
feature-gated API OAuth controller landed (`/credentials/:id/oauth2/auth`,
GET/POST callback), callback path now persists exchanged OAuth2 state into
`oauth_credential_store`, and callback tests cover both create and overwrite
paths (`callback_persists_oauth_state_in_credential_store`,
`callback_overwrites_existing_oauth_state`). With P8/P9/P10 landed, the
`nebula-credential` row now marks Engine integration as `stable`; `nebula-api`
remains `partial` while OAuth remains optional behind `credential-oauth`.
Prior: 2026-04-21 — P8 slice of credential cleanup: engine-owned
`credential` runtime surface landed (`CredentialResolver`, `CredentialRegistry`,
`execute_resolve`/`execute_continue` and pending-lifecycle coverage under
`crates/engine/tests/credential_pending_lifecycle_tests.rs`). Legacy runtime
modules (`resolver.rs`, `registry.rs`, `executor.rs`) were removed from
`nebula-credential`; crate now keeps contract/types/primitives while runtime
orchestration lives in `nebula-engine`.
Prior: 2026-04-20 — P4 of credential cleanup: `AuthPattern`,
`AuthScheme`, `CredentialEvent`, and `CredentialId` migrated from
`nebula-core` into `nebula-credential` (credential-specific types no longer
pollute the cross-cutting base). Consumers updated: `nebula-action`,
`nebula-plugin` (tests), `nebula-resource` (now depends on `nebula-credential`
for `AuthScheme`); `nebula-sandbox`, `nebula-engine`, `nebula-runtime`,
`nebula-sdk` carried no direct references. The `#[derive(AuthScheme)]` and
`#[derive(Credential)]` proc-macros emit `::nebula_credential::...` paths,
resolved inside the credential crate itself via `extern crate self as
nebula_credential`. Spec:
credential architecture cleanup design (archived — see the maintainers' private design vault).
Both crate rows (`nebula-core`, `nebula-credential`) remain `frontier` per
canon decision to hold off API-stability bumps.
Prior: 2026-04-20 — Plugin load-path stabilization
slice B landed: `Plugin` trait returns runnable `Arc<dyn Action|Credential|Resource>`,
`PluginManifest` moved to `nebula-metadata` with `ManifestError` companion,
`PluginMeta` deleted from the SDK, `ResolvedPlugin` per-plugin wrapper
enforces the namespace invariant at construction, multi-version runtime
registry (`PluginType` / `PluginVersions`) dropped (YAGNI — zero
production consumers), `PluginRegistry` simplified + gains
`all_*` / `resolve_*` aggregate accessors, wire protocol bumped to v3
with full manifest + per-action `ValidSchema` per action, `plugin.toml`
parsed at discovery with `[nebula].sdk` semver-constraint check,
`RemoteAction` wraps `ProcessSandboxHandler` as `impl Action`,
`DiscoveredPlugin: impl Plugin` is the host-side adapter.
Workflow-config-sourced `PluginCapabilities` enforcement at the broker
remains open under ADR-0025 slice 1d. See ADR-0027.
Prior: 2026-04-19 — Phase 1 of Rust 1.75–1.95 adoption complete: `once_cell` workspace dependency dropped (`LazyLock`/`OnceLock` fully adopted); ~60 `#[allow]` attrs flipped to `#[expect]` across 18 crates (Phase 1b free-lunch sweep), reducing total `#[allow]` from 116 to 56. Phases 2–5 (inherent AFIT, dynosaur, precise-capture, late polish) remain. Prior: 2026-04-19 (nebula-metadata row added; `compat.rs` extracted to BaseCompatError + validate_base_compat; action / credential / resource wired to the shared check). Prior: 2026-04-19 (ADR-0008 B1 / ADR-0017 follow-up: `pg::PgControlQueueRepo` landed — Postgres now honors the durable control plane via `FOR UPDATE SKIP LOCKED` and a concurrent-safe `reclaim_stuck` CAS; in-memory + Postgres share one behavioral parity test suite). Prior: 2026-04-19 (ADR-0008 A3 landed: engine cancel registry + dispatch_cancel / dispatch_terminate wired end-to-end; ADR-0016 documents the cooperative-cancel contract and the forced-shutdown gap).
