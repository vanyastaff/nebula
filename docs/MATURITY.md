---
name: Nebula crate maturity dashboard
description: Manual per-crate state dashboard. Edited in PRs that change a crate's API stability, test coverage, doc state, engine integration, or SLI-readiness.
status: accepted
last-reviewed: 2026-04-23
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
| nebula-action        | frontier | stable  | stable | partial (webhook sig `Required` by default at trait surface — ADR-0022; CheckpointPolicy planned; `ActionResult::Retry` gated behind `unstable-retry-scheduler`, #290) | n/a |
| nebula-api           | frontier | stable  | stable | partial (knife steps 3+5: Start/Cancel producers stable, #332/#330; engine-side Start/Resume/Restart dispatch wired via EngineControlDispatch — ADR-0008 A2; Cancel/Terminate dispatch wired via engine cancel registry — ADR-0008 A3 / ADR-0016; API routing infrastructure per spec 05: tenant-scoped routes, RBAC/tenancy/CSRF middleware, cursor pagination, port traits `OrgResolver`/`WorkspaceResolver`/`SessionStore`/`MembershipStore`, extended `ApiConfig` with TLS/Cookie/CORS/Versioning/Pagination sub-configs, 10 new `ApiError` variants) | partial |
| nebula-core          | frontier | stable  | stable | stable (6 public modules: `role`, `permission`, `tenancy`, `slug`, `auth`, `guard` — `TenantContext`, `ResolvedIds`, `OrgRole`, `WorkspaceRole`, `Permission`, `PermissionDenied`, `Slug`, `SlugKind`, `SlugError`, `is_prefixed_ulid()`, `AuthScheme`, `AuthPattern`, `Guard`, `TypedGuard`, `BaseContext`, `Context`) | n/a |
| nebula-credential    | frontier (**П1 trait scaffolding landed 2026-04-26** per Tech Spec §15.4-§15.8 — capability sub-trait split, `AuthScheme` sensitivity dichotomy, fatal duplicate-KEY registration, `SchemeGuard`/`SchemeFactory` refresh hook, capability-from-type authority shift; 10 landing-gate compile-fail probes + 1 runtime probe green; ADR-0035 phantom-shim canonical form; P6-P11 architecture cleanup remains — accessor/metadata flattened, `AuthScheme`/`AuthPattern` canonical in `nebula-core`, dep diet к core/metadata/schema/resilience/error, `ExternalProvider` abstraction; **pruned 2026-04-24**: `FederatedAssertion`/`OtpSeed`/`ChallengeSecret` schemes + corresponding `AuthPattern` variants — Plane-A / integration-internal territory) | stable  | stable | stable (**П2 refresh coordination L2 landed 2026-04-26** per PR [#583](https://github.com/vanyastaff/nebula/pull/583) — two-tier coordinator: in-process L1 (`L1RefreshCoalescer`) + durable L2 (`RefreshClaimRepo` per ADR-0041) with TTL/heartbeat/reclaim sweep; sentinel N=3-in-1h threshold + `ReauthRequired` escalation; 5 metrics + 3 spans + 3 audit events; nightly chaos test (3 replicas × 100 creds × 10 min) — closes n8n #13088 cross-replica `refresh_token_v1`-invalidates-`v2` race; engine `iter_compatible` slot-picker consumer wiring + manager-side `OnCredentialRefresh<C>` fan-out tracked as post-П1 follow-ups in credential concerns register; **security hardening 2026-04-27 SEC-cluster landed** per `docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md` — SEC-01/02 IdP boundary (bounded reader + sanitize_error_uri), SEC-05/06 plaintext-lifecycle invariants (CredentialGuard !Clone, SchemeGuard !Send + !Sync), SEC-09/10 Zeroizing discipline (bearer_header buffer, refresh_oauth2_state scope-tighten), SEC-11 bare crypto::encrypt removed from public surface, SEC-13 redact_sensitive_fields + ADR-0030 §4 redaction CI gate) | n/a |
| nebula-credential-builtin | frontier (preview — П1 trait scaffolding landed 2026-04-26; `mod sealed_caps` convention surface present; concrete capability sub-trait impls land in П3) | n/a | partial (README + crate-level rustdoc only — no concrete API surface yet beyond the П1 trait scaffolding re-exports) | n/a | n/a |
| nebula-engine        | partial  | stable  | stable | partial (ControlConsumer skeleton lands §12.2; all five control commands dispatched via EngineControlDispatch — ADR-0008 A2 (Start/Resume/Restart) + A3 (Cancel/Terminate) + ADR-0016 cancel registry; ADR-0008 B1 reclaim sweep implemented via ControlQueueRepo::reclaim_stuck + ADR-0017; engine-owned `credential` runtime surface landed in P8 slice) | n/a |
| nebula-error         | stable   | stable  | stable | n/a | n/a |
| nebula-eventbus      | stable   | stable  | stable | n/a | n/a |
| nebula-execution     | stable   | stable  | stable | stable | partial |
| nebula-expression    | stable   | stable  | stable | stable | n/a |
| nebula-log           | stable   | stable  | stable | n/a | n/a |
| nebula-metadata      | frontier | stable  | stable | n/a | n/a |
| nebula-metrics       | stable   | stable  | stable | n/a | n/a |
| nebula-plugin        | stable   | stable  | stable | stable | n/a |
| nebula-plugin-sdk    | partial  | stable  | stable | n/a | n/a |
| nebula-resilience    | stable   | stable  | stable | n/a | n/a |
| nebula-resource      | frontier | stable  | stable | partial (lifecycle visible; CAS guards partial) | n/a |
| nebula-runtime       | partial  | stable  | stable | stable | partial |
| nebula-sandbox       | partial  | stable  | stable | partial (process isolation; signing planned) | n/a |
| nebula-schema        | frontier | stable  | stable | stable | n/a |
| nebula-sdk           | partial  | stable  | stable | n/a | n/a |
| nebula-storage       | partial  | stable  | stable | stable | partial |
| nebula-system        | partial  | partial | stable | n/a | n/a |
| nebula-telemetry     | stable   | stable  | stable | n/a | n/a |
| nebula-testing       | planned  | n/a     | n/a    | n/a    | n/a |
| nebula-validator     | frontier | stable  | stable | stable | n/a |
| nebula-workflow      | stable   | stable  | stable | stable | n/a |

---

## Review cadence

This file is a living dashboard. Reviewers check truthfulness on every PR that touches a crate's public surface, test suite, or docs. Canon §17 DoD includes "MATURITY.md row updated if the PR changes crate state."

Last full sweep: 2026-04-17 (Pass 4 of docs architecture redesign).
Last targeted revision: 2026-04-26 — **credential П2 refresh coordination L2 landed:**
worktree `worktree-credential-p2` lands cross-replica refresh coordination per sub-spec
`docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md` and ADR-0041 in
6 stages — Stage 1 storage infrastructure (`RefreshClaimRepo` trait + 3 impls + migrations
0022/0023 + loom CAS probe); Stage 2 engine refactor (`L1RefreshCoalescer` private + new
outer two-tier `RefreshCoordinator` composing L1 + L2 claim repo + sentinel set before
IdP POST); Stage 3 sentinel N=3-in-1h threshold + `ReauthRequired` escalation + reclaim
sweep with `RefreshOutcome::CoalescedByOtherReplica` + `reauth_required` persisted on
`StoredCredential` (sub-spec §3.6); Stage 4 observability (5 metrics + 3 spans + 3
audit events) + nightly chaos test (3 replicas × 100 creds × 10 min); Stage 5 doc sync.
`nebula-credential` Engine integration upgraded `partial → stable` honestly — multi-replica
mid-refresh race handled via durable claim repo. Closes n8n #13088 class production race
where rotated `refresh_token_v2` invalidates `refresh_token_v1` on a parallel replica.
Default chaos test gated behind `--features chaos-full`; nightly workflow runs it. Public
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
`nebula-credential-builtin` scaffold present (concrete types land in П3).
Tech Spec frontmatter status flipped to «П1 in-implementation 2026-04-26»; §16.5.1
implementation tracker entry added; 5 architectural register rows flipped to
`in-implementation`; 5 stage5-followup process rows resolved.
Prior: 2026-04-24 — **credential scheme pruning + honest multi-replica status:**
removed `FederatedAssertion`/`OtpSeed`/`ChallengeSecret` schemes (Plane A / integration-internal);
removed corresponding `AuthPattern` variants (`FederatedIdentity`/`OneTimePasscode`/`ChallengeResponse`);
`nebula-credential` Engine integration downgraded `stable → partial` honestly — single-process
refresh coordination only, multi-replica race handling tracked in Spec H0. Archived 8-file
`docs/superpowers/drafts/2026-04-24-credential-redesign/` exploratory set to
`docs/superpowers/archive/2026-04-24-credential-redesign-exploratory/` with STATUS.md
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
store impl gating (`test-util`), rotation orchestration → engine, OAuth HTTP →
api/engine, `Cargo.toml` dep diet.
Prior: 2026-04-22 — **Test stack:** workspace `dev-dependencies` for
`insta`, `pretty_assertions`, `rstest`, `wiremock`, `mockall`, `assert_cmd`, `predicates`,
`assert_fs`; `docs/TESTING.md` + cross-links from `QUALITY_GATES` / `PRODUCT_CANON` §15;
example tests in `nebula-api`, `nebula-credential`, `nebula-storage`, `nebula-cli`.
Prior: 2026-04-21 — **P6–P11 credential cleanup:** add
`docs/superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md` (ADR-0032 pointer, spec §12
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
Prior: 2026-04-21 — [ADR-0033](adr/0033-integration-credentials-plane-b.md)
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
`docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md`.
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
