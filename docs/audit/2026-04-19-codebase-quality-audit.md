---
title: Codebase Quality Audit (2026-04-19)
status: accepted
date: 2026-04-19
auditors: [tech-lead, rust-senior, dx-tester, security-lead]
verdict: library-first
related:
  - docs/PRODUCT_CANON.md
  - docs/MATURITY.md
  - docs/STYLE.md
  - docs/INTEGRATION_MODEL.md
  - docs/adr/0008-execution-control-queue-consumer.md
  - docs/adr/0013-compile-time-modes.md
  - docs/adr/0014-dynosaur-async-traits.md
---

# Codebase Quality Audit — 2026-04-19

## Executive summary

Four agents (tech-lead, rust-senior, dx-tester, security-lead) independently
audited the Nebula workspace. Unanimous strategic vote: **library-first
go-to-market**.

The codebase has the structural shape of a library (25 crates, layered
architecture, `deny.toml` mechanically enforces one-way deps, `nebula-sdk` is
the authored façade), but `apps/` carried mixed product-first ambitions
(`apps/web` placeholder, `apps/desktop` Tauri scaffold, `apps/server` slot
referenced from `deny.toml:62-66` but never built). rust-senior's verdict was
the sharpest: *codebase is library-first by structure, product-first by
audience signals — the worst-of-both combination*. Picking a direction now
lets us prune the other and stop spending budget on both.

This document records the verdict, the fix queue that follows from it, and the
load-bearing guard rails that apply regardless of direction.

## Strategic verdict: library-first

| Agent | Core argument |
|---|---|
| tech-lead | Audience signals (canon §3.5 integration model uniformity) already point library; product would fork the budget |
| rust-senior | Library-first cheaper long-term — single SemVer gate vs 25; the worst-of-both combo is what we have today |
| security-lead | Library = ~12 invariants in head; Product = infinite surface (TLS / RBAC / multi-tenant / KMS), needs 12-18mo fulltime security-engineer we don't have |
| dx-tester | Library layer is close to correct (`TestRuntime`, `stateless_fn` work); gap is one broken macro + missing example, both single-PR fixes. Product-first build-gap = months |

### Hybrid is allowed, narrowly

Library-first **now**, product-shaped `apps/server` **later**, only if the
future server is a thin composition root over the same library primitives —
not a parallel stack with its own auth / key / storage paths. ADR-0008's
`apps/server` follow-up survives under this constraint.

## Findings

### High-confidence (≥2 agents)

- **`crates/metadata` is speculative L4-style abstraction at L1.** No README,
  not in `nebula-sdk` re-exports, not in `MATURITY.md`. 650 LOC across 6 files
  for one trait + 4 helpers, used by exactly 3 consumers
  (`action`, `credential`, `resource`). [rust-senior, tech-lead, dx-tester]
- **`nebula-sdk` is broken at the entry point.** `simple_action!` macro
  expands to `impl nebula_action::ProcessAction`, but `ProcessAction` does not
  exist anywhere in `nebula-action` (it was renamed to `StatelessAction` in a
  prior refactor that didn't update the macro or the testing stub).
  `crates/sdk/src/lib.rs:235`. The first thing a newcomer copies fails to
  compile. [dx-tester confirmed by code; rust-senior found the
  `pub use anyhow; pub use async_trait::async_trait;` leak at lib.rs:48-49]
- **README drift in root and crate-level docs.** Root `README.md:43,69,84`
  lists ghost crates (`parameter`, `config`); `README.md:64-89` table omits
  real ones (`metadata`, `sandbox`, `plugin-sdk`, `schema`).
  `crates/workflow/docs/README.md` describes a phantom workflow API
  (`add_edge` with closures, `.execute()`) that does not exist in the real
  `WorkflowBuilder`. [tech-lead, dx-tester]
- **`plugin` / `sdk` / `plugin-sdk` name collision.** Three crates with
  similar names but distinct concepts (in-process plugin trait + registry /
  integration-author façade / out-of-process plugin protocol). Newcomer can't
  distinguish without reading three READMEs. [dx-tester, rust-senior]

### Per-agent specialty

**rust-senior — architectural debt:**

- 81 `#[async_trait]` attributes across 54 files vs **0 dynosaur usages**,
  despite ADR-0014 (accepted today). Workspace-wide migration debt.
- `crates/engine/src/engine.rs` is **7923 LOC, 187 functions, 7 types** — god
  file. Frontier loop, lease management, credential refresh, failure routing,
  status decision all in one. Split candidates: `frontier.rs`, `lease.rs`,
  `credential_refresh.rs`, `failure.rs`, `task.rs`.
- `crates/storage/src/lib.rs:105-129` — orphan generic `Storage` trait +
  `MemoryStorage*` re-exports with **zero workspace consumers**. Russian doc.
  Stale link to a removed architecture doc. Pure §12.7 orphan.
- `crates/resource/src/compat.rs` — deprecated `Scope`/`Context` re-exports
  shadowing the real `Context` in the same crate. Pre-v1 + `feedback_no_shims`
  applies. Two distinct `Context` types in one crate is exactly the §14
  anti-pattern.
- Other large files (split candidates, lower priority): credential
  `resolver.rs` 1326L, `oauth2.rs` 1129L; expression `eval.rs` 2545L; sandbox
  `process.rs` 1990L; runtime `runtime.rs` 2178L; storage `execution_repo.rs`
  1827L; resource `manager.rs` 2003L.

**tech-lead — strategic drift (revised):**

- *Withdrawn finding:* `telemetry` / `metrics` split is justified on re-read
  (telemetry owns lock-free primitives + label interning; metrics owns naming
  + Prometheus export). Originally flagged, now retracted.
- `apps/*` sprawl: `apps/web/` is one README; `apps/desktop/` is unfinished
  Tauri scaffold; `apps/server/` doesn't exist. Decision: delete `apps/web`,
  pin `apps/desktop` as `# Status: reference shell, not a release artifact`,
  defer `apps/server` to thin-composition-root form post-library-first.
- `docs/PLUGIN_MODEL.md` substantially overlaps `INTEGRATION_MODEL.md §7.1`
  (identity rules, three-layer model, discovery lifecycle). PRODUCT_CANON.md
  already flagged for merge. Lift unique paragraphs into §7.1 and delete.

**security-lead — secret/auth surface:**

- `crates/credential/src/layer/encryption.rs:62` accepts `Arc<EncryptionKey>`
  directly — there is **no `KeyProvider` seam**. Any composition path
  (library embedder, future `apps/server`, tests) must load the key into
  process memory with unknown provenance. Once `apps/server` ships with
  env-only key loading, that becomes de facto API forever (operators write
  systemd units, configs, runbooks). **Must land before any composition
  PR**.
- `crates/api/src/app.rs:51-60` — REST routes have no `DefaultBodyLimit`
  (webhook transport caps itself, but `/workflows`, `/credentials` POST do
  not). 1 MiB default before apps/server.
- `WebhookTrigger` lacks a `signature_policy()` contract — primitives exist
  in `crates/action/src/webhook.rs:972+` (constant-time tag compare), but
  enforcement is opt-in. Authors who forget the verify call ship unsigned
  webhooks behind discoverable URLs. Need `Required` default at the trait
  level.
- `apps/cli/src/config.rs:73` — `RemoteConfig.api_key: Option<String>`
  plaintext on disk; `default_toml()` example actively encourages it. Wrap in
  `SecretString` + add `keyring` backend before the TUI surface grows.

**dx-tester — newcomer experience:**

- `examples/` workspace member exists but contains only 5 examples, none at
  the "first 10-line stateless action" complexity level. New users land on
  `paginated_users` / `batch_products` / `poll_habr` — too advanced for first
  contact.
- `crates/sdk/src/prelude.rs` is a 60-item glob with no graduation. Stateful
  + Batch + Paginated + Webhook + Poll adapters, all schema field variants,
  workflow types, credential types — overwhelming for someone writing a
  stateless `hello.greet` action.
- `nebula-sdk::lib.rs` Quick start uses `simple_action!` (broken) and
  `metadata.base.name` (undocumented intermediate `.base` field on
  `ActionMetadata`). The correct minimal path is `stateless_fn(...)` + a 3-line
  manual `ActionMetadata::new(...)` — not advertised anywhere.
- `TestRuntime` (`crates/sdk/src/runtime.rs`) is real and well-shaped — once
  found. Acceptance for library-first announcement: `cargo run --example
  hello_action` green in <30s on cold install.

## Fix queue

### P0 — Pre-conditions for library-first announcement

| # | Action | Where | Size |
|---|---|---|---|
| 1 | Fix `simple_action!` macro: `ProcessAction` → `StatelessAction`, update output type to match real trait | [crates/sdk/src/lib.rs:235](../../crates/sdk/src/lib.rs:235) | S |
| 2 | Drop `pub use anyhow;` and `pub use async_trait::async_trait;` from SDK; remove from `Cargo.toml` | [crates/sdk/src/lib.rs:48-49](../../crates/sdk/src/lib.rs:48) | S |
| 3 | Remove or implement `ActionTester::execute` stub | [crates/sdk/src/testing.rs:110](../../crates/sdk/src/testing.rs:110) | S |
| 4 | Add `examples/hello_action.rs` — 10-line stateless action runnable via `TestRuntime`. Acceptance: `cargo run --example hello_action` green in <30s | [examples/](../../examples/) | S |
| 5 | Delete or rewrite `crates/workflow/docs/README.md` (phantom API) | workflow/docs | S |
| 6 | `KeyProvider` trait + `EnvKeyProvider` impl in `nebula-credential`. Open ADR. | [crates/credential/src/layer/encryption.rs](../../crates/credential/src/layer/encryption.rs) | M |
| 7 | Rewrite root `README.md` crate map from `Cargo.toml` + `MATURITY.md`; add CI check | [README.md](../../README.md) | S |
| 8 | Add `crates/metadata/README.md` + `MATURITY.md` row, OR fold into `nebula-core::metadata` (see P1 #11) | crates/metadata | S-M |

### P1 — Structural pruning (after P0)

| # | Action | Where | Size |
|---|---|---|---|
| 9 | Delete `apps/web/` placeholder | apps/web | S |
| 10 | Pin `apps/desktop/README.md` with `# Status: reference shell, not a release artifact` | apps/desktop | S |
| 11 | Fold `nebula-metadata` into `nebula-core::metadata` module (preserves `Metadata` trait + `BaseMetadata<K>` shape) | crates/metadata → crates/core | M |
| 12 | Delete orphan `Storage` trait + `MemoryStorage*` re-exports | [crates/storage/src/lib.rs:93,105-129](../../crates/storage/src/lib.rs:105) | S |
| 13 | Delete `crates/resource/src/compat.rs` deprecated `Scope`/`Context` shims | crates/resource/src/compat.rs | S |
| 14 | Fold `docs/PLUGIN_MODEL.md` into `INTEGRATION_MODEL.md §7.1`; delete; update inbound links | docs | S |
| 15 | Split `crates/sdk/src/prelude.rs` into starter (~15 items) + `prelude::full` | crates/sdk/src/prelude.rs | S |

### P2 — ADR-level changes

| # | Action | Notes |
|---|---|---|
| 16 | ADR: "Crate publication policy" — `publish=true` requires ≥3 external consumers OR ADR | rust-senior gate; auto-prunes orphans |
| 17 | ADR: "Library-first GTM, apps/server as thin composition root" | Closes the strategic question permanently per tech-lead |
| 18 | ADR-0014 dynosaur migration plan — 81 `#[async_trait]` → 0 workspace-wide | Sequenced: storage repos first (most consumers), then action/credential traits |
| 19 | Split `crates/engine/src/engine.rs` (7923 LOC) → frontier/lease/credential_refresh/failure/task | Boundaries TBD; tech-lead handoff per audit |

## Guard rails (apply regardless)

1. **`KeyProvider` trait must exist before any code merges that composes
   `EncryptionLayer` + auth + storage in one process.** This is the gate
   on P0 #6. Skipping it freezes env-only as de facto API.
2. **REST `DefaultBodyLimit` (1 MiB) before any new composition root binary.**
3. **Webhook signature `Required` by default** before the URL space is
   advertised in any deployment doc.
4. **Operational honesty taxonomy** (`partial`/`frontier`/`stable`/`planned`/
   `false capability`) survives every refactor. Hiding `planned` rows for
   cosmetics is a §11.6 violation.

## Do not break

- **§11.6 status vocabulary + feature-gated unstable surfaces.**
  `unstable-retry-scheduler` is the cleanest example — every new aspirational
  capability follows the same pattern.
- **§3.5 integration model uniformity** (`*Metadata + Schema` across
  Action/Credential/Resource/Trigger). This is *the* differentiator vs n8n
  (untyped JSON), Temporal (no integration model), Windmill (script-shaped).
  Defend even when individual crates feel duplicative.
- **L1→L4 layering with `deny.toml` enforcement + §0.2 canon revision
  triggers.** Falsifiable rules — the build fails when you break them. Most
  projects this size have neither.
- **§13 knife scenario as CI bar.** Single end-to-end create→activate→start→
  cancel scenario sourced from canon, not wishlist.

## Open ADRs needed

| ID | Title | Owner | Trigger |
|---|---|---|---|
| [0023](../adr/0023-keyprovider-trait.md) | KeyProvider trait between EncryptionLayer and key material source | security-lead | Before any composition root |
| [0021](../adr/0021-crate-publication-policy.md) | Crate publication policy (publish=true ≥3 consumers OR ADR) | rust-senior | Before next 1.0 release-train discussion |
| [0020](../adr/0020-library-first-gtm.md) | Library-first GTM + apps/server as thin composition root | tech-lead | Now (closes the strategic question) |
| [0022](../adr/0022-webhook-signature-policy.md) | Webhook signature policy (Required default) | security-lead | Before webhook-trigger v1 lock |

## References

- Per-agent full reports stored in agent memory:
  - `agent-memory-local/tech-lead/decision_2026_04_19_library_first.md`
  - `agent-memory-local/tech-lead/decision_2026_04_19_apps_server_priority.md`
  - `agent-memory-local/security-lead/debt_apps_server_guardrails.md`
  - `agent-memory-local/security-lead/debt_kms_key_provider.md`
  - `agent-memory-local/security-lead/debt_webhook_signature_enforcement.md`
- Audit context: `docs/PRODUCT_CANON.md`, `docs/MATURITY.md`, `docs/STYLE.md`,
  `docs/INTEGRATION_MODEL.md`.
