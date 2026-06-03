# Nebula — Onboarding / Handoff

> For a new human or AI collaborator taking over Nebula. This is a
> **pointer map**, not a spec — it tells you *where* the truth lives, not what
> it says. Read the linked canon; do not trust a summary over the source.
>
> **Reading order:** [`CLAUDE.md`](CLAUDE.md) (rules + layer map) →
> [`README.md`](README.md) (product) → [`STRATEGY.md`](STRATEGY.md) (direction)
> → [`docs/README.md`](docs/README.md) (the doc map — read it before opening
> anything else under `docs/`).

## 1. What Nebula is

Nebula is a modular, type-safe **workflow automation engine** in Rust
(edition 2024) — same space as n8n / Temporal / Restate, but shipped as a
composable **library** (`nebula-sdk` is the headline surface), not a monolithic
platform. Credentials, typing between nodes, durability, and resilience are
first-class engine concerns rather than bolt-ons.

- Product overview, design principles, data flow → [`README.md`](README.md)
- Direction, the 2026 standard bar, flagship, tracks → [`STRATEGY.md`](STRATEGY.md)
- Binding invariants (durability / credentials / operational honesty) →
  [`docs/PRODUCT_CANON.md`](docs/PRODUCT_CANON.md)

## 2. Current state & milestone

**Active alpha.** Cross-cutting + Core layers are **stable**; Exec (`engine`,
`storage`) and the API layer are being wired toward production. Not 1.0 yet.

The 1.0 plan is a **capability/dependency checklist** (not a calendar) in
[`docs/ROADMAP.md`](docs/ROADMAP.md), milestones M0–M14:

- **DONE:** M0 (engine durability debts), M1 (engine correctness), M2 (retry
  semantics + leases), M6/M11 (resource + action/credential/resource v4
  dependency redesign), M9.2 (OTLP exporter end-to-end), most of M3 (API
  surface — auth backend, OpenAPI 3.1, webhooks, idempotency, trace
  propagation, OAuth-from-secrets).
- **Open / frontier:** M3.6 (shift-left workflow validation), M4 (plugin
  capability discovery enforcement), M5 (plugin ABI ADR), M7 (storage
  operationalization — PG composition root, nightly loom), M8 (engine
  concurrency loom), M9.1/M9.3 (observability sweep + mutex audit), M10
  (docs/DX/release), **M12 (business-layer crates frontier → stable)**, M13/M14
  (core polish + final hardening).

**Stable vs frontier (status lives in [`docs/MATURITY.md`](docs/MATURITY.md)):**

- **Stable:** all Cross-cutting (`error`, `log`, `eventbus`, `metrics`,
  `resilience`, `env`), all Core (`core`, `validator`, `expression`,
  `workflow`, `execution`, `schema`, `metadata`, `storage-port`), `storage`,
  `nebula-credential` + `nebula-credential-runtime` (flipped 2026-05-20).
- **Frontier / partial:** `engine` (~85%), `action`, `nebula-resource`
  (blocked on the **bind-population** gap — see §7), `plugin` (partial),
  `credential-builtin` (scaffold → concrete types TBD in M12.3), `sandbox`
  (capability gate not yet enforced — M4).

The ROADMAP "Status snapshot" section is the freshest narrative; trust
`#PR` / `file:line` evidence over prose. Note: squash-merge often misses
auto-close, so an "open" GitHub issue may already be fixed — verify with
`git log --grep="#N"` before planning.

## 3. Architecture in 30 seconds

Six layers, each depends **only on layers below it**. Cross-cutting crates are
importable at any level. Boundaries are **mechanically enforced** by
`cargo deny check` against `deny.toml [wrappers]` — a missing edge fails CI
before review. Cross-crate signalling goes through `nebula-eventbus`, never
direct sibling imports. Full map + nuances → [`CLAUDE.md`](CLAUDE.md)
"Layered Dependency Map".

| Layer | Crates |
|-------|--------|
| API / Public | `api`, `sdk` |
| Exec | `engine`, `storage`, `storage-loom-probe` |
| Business | `credential-builtin`, `resource`, `action`, `plugin`, `tenancy` |
| Plugin-Proto | `plugin-sdk`, `sandbox` |
| Core | `core`, `validator`, `expression`, `workflow`, `execution`, `schema`, `metadata`, `storage-port` |
| Cross-cutting | `log`, `eventbus`, `metrics`, `resilience`, `error`, `env` |

Three placements that are *not* a simple single-tier reading:

- **`nebula-credential` is shared infra**, not Business-only — Exec (`engine`,
  `storage`) and API consume the credential contract directly alongside
  Business; the `deny.toml [wrappers]` allowlist locks the exact consumer set.
- **Storage is a port/adapter/tenancy split:** `nebula-storage-port` (Core) is
  the object-safe spec-16 row-model **contract** (no backend code);
  `nebula-storage` (Exec) is the **sole adapter** (InMemory + SQLite +
  Postgres); `nebula-tenancy` (Business) is the **scope-enforcing decorator**
  that substitutes a tenant scope on every call before it reaches a handler.
  The legacy `ExecutionRepo`/`WorkflowRepo` surface was deleted (ADR-0072).
- **Plugin-Proto** is a leaf tier between Core and Business: `plugin-sdk`
  (out-of-process protocol) + `sandbox` (duplex transport). The discovery path
  and the `SandboxError → ActionError` seam live in `nebula-plugin`, not in
  `sandbox`.

**Per-crate quick-maps:** the convention is that every `crates/<x>/` carries a
`CLAUDE.md` agent quick-map (purpose, layer, commands, key files, crate rules)
alongside the human-facing `README.md` — see the "AI Context Files" table in
[`CLAUDE.md`](CLAUDE.md). When editing a crate, open its `CLAUDE.md` /
`README.md` first.

## 4. Where to start, by task type

Pick the crate by intent. One-liners are condensed; the crate's own
`README.md` (and `CLAUDE.md` quick-map) is authoritative.

| I want to… | Start in (crate) | Layer | What it owns |
|------------|------------------|-------|--------------|
| **Touch credentials** (contract / secret primitives) | `nebula-credential` | Shared-infra | Typed Credential Contract: stored-State vs projected auth-Scheme split + secret primitives. |
| Add a concrete credential type | `nebula-credential-builtin` | Business | First-party impls (`bearer_token`, `shared_key`, `signing_key`) + `register_builtins()` + `sealed_caps`. |
| Wire the credential **lifecycle** (resolve/refresh/rotate/revoke/bind) | `nebula-credential-runtime` | Exec | `CredentialService<B,PS>` facade — the owner-isolated lifecycle behind one typed entry point. |
| Add a Vault-backed secret source | `nebula-credential-vault` | Business | HashiCorp Vault backend (KV v2 + dynamic secrets + lease renew/revoke). |
| Crypto primitives (AEAD / KDF) | `nebula-crypto` | Cross-cutting | AES-256-GCM (mandatory AAD) + Argon2id, `EncryptedData` envelope, `CryptoError`. |
| **Storage / persistence** — define a port | `nebula-storage-port` | Core | Object-safe repo traits, port-local DTO rows, `Scope`, `StorageError`, `TransitionBatch`. No backend code. |
| Storage — implement a backend (PG/SQLite/mem) | `nebula-storage` | Exec | Sole spec-16 adapter: execution CAS, journal, control-queue outbox, idempotency, leases, refresh claims. |
| Multi-tenancy / scope enforcement | `nebula-tenancy` | Business | Resolves `Principal → Scope`; wraps each store in a scope-substituting decorator. |
| Model-check a CAS critical section | `nebula-storage-loom-probe` | Exec | Loom probes mirroring refresh-claim + lease-handoff single-owner invariants. |
| **Errors / classification** | `nebula-error` | Cross-cutting | `Classify` trait + `NebulaError<E>` (typed details/context chain + `RetryHint`). |
| **Validation** (rules / schema-field constraints) | `nebula-validator` | Core | Composable validators + JSON-serializable `Rule` enum. |
| **Config schema** for Action/Credential/Resource | `nebula-schema` | Core | Typed config schema with lint→validate→resolve proof-token pipeline. |
| Expressions / `{{ }}` templates | `nebula-expression` | Core | n8n-compatible evaluator; backend for `nebula-schema`'s resolve step. |
| **Add an action** (integration logic) | `nebula-action` | Business | Action trait family (Stateless/Stateful/Trigger/Resource) + `ActionMetadata` for discovery/validate/dispatch. |
| Manage a long-lived resource (pool/client/bot) | `nebula-resource` | Business | acquire/health/hot-reload/scope-bounded release; hands a drop-releasing `ResourceGuard` to actions. |
| **Engine / execution** (scheduling, leases, frontier loop) | `nebula-engine` | Exec | Composition root wiring runtime/storage/plugin/credential/resource; drives a DAG to terminal state. |
| Execution model (status machine / journal / plan) | `nebula-execution` | Core | 8-state status machine, journal (WAL), idempotency-key shape, DAG-derived plan. |
| Workflow definition / DAG / activation validation | `nebula-workflow` | Core | serde-round-trippable `WorkflowDefinition` + petgraph DAG + `validate_workflow`. |
| **HTTP / REST / webhooks / OAuth transport** | `nebula-api` | API/Public | Thin axum gateway → typed port-trait calls; hosts webhook + OAuth transports. |
| Build a plugin (author side) | `nebula-plugin-sdk` | Plugin-Proto | Implement `PluginHandler` + `run_duplex` (line-delimited JSON envelope). |
| Host-side plugin distribution / discovery / registry | `nebula-plugin` | Business | `Plugin` trait, `ResolvedPlugin`, `PluginRegistry`, out-of-process discovery path. |
| Host-side sandbox transport / OS hardening | `nebula-sandbox` | Plugin-Proto | `ProcessSandbox` duplex child-process transport + credential-scope identity + Linux hardening. |
| Outbound-call resilience (retry/CB/bulkhead/timeout) | `nebula-resilience` | Cross-cutting | The **only** retry surface in the stack; composed at outbound action call sites. |
| Logging / tracing init | `nebula-log` | Cross-cutting | Single tracing subscriber-init pipeline (format, writers, reload, OTLP/Sentry). |
| Metrics / Prometheus / OTLP export | `nebula-metrics` | Cross-cutting | In-memory primitives, `nebula_*` naming, cardinality safety, Prometheus + OTLP. |
| Cross-crate pub/sub | `nebula-eventbus` | Cross-cutting | Transport-only generic `EventBus<E>`; defines no domain event types. |
| Read env vars (never panics) | `nebula-env` | Cross-cutting | Typed reader (`var`/`parse`/`flag`/`list`). |
| The one-import integrator façade | `nebula-sdk` | API/Public | Re-export façade + `WorkflowBuilder` + `TestRuntime` test harness. |
| Catalog/metadata leaf types | `nebula-metadata` | Core | `BaseMetadata<K>` + `Metadata` trait + Icon/Maturity/Deprecation + compat rules. |
| Shared IDs / keys / scopes / auth enums | `nebula-core` | Core | Prefixed-ULID ids, normalized keys, scope system, auth enums, context/accessor contracts. |

**Skills / slash commands** (advisory, in `.claude/skills/` + `.claude/commands/`):

- `rust-intel` — load **before** writing Rust; defends against the known
  taxonomy of LLM-Rust failure modes. `/rust-cc-audit`, `/rust-cc-fix`,
  `/rust-cc-plan`.
- `clippy-configuration` — for Clippy/TOML lint config work.

## 5. Build / test / PR workflow

Daily commands go through **`task`** (see [`CLAUDE.md`](CLAUDE.md) "Common
Commands"; `task --list` for the full catalog). Don't call raw `cargo` for
fmt/lint.

- **Pre-PR gate:** `task dev:check` (fmt + clippy `-D warnings` + nextest +
  doctests + deny). Fast single-crate loop: `cargo check -p <crate>` /
  `cargo nextest run -p <crate>`.
- **Branches:** branch from `origin/main`, squash-merge back, never force-push
  shared history. Persistent task branches go through
  `bash scripts/worktree.sh new <slug> <type> <scope>` (creates
  `.worktrees/<slug>` + branch `<type>/<scope>-<slug>`); clean up with
  `bash scripts/worktree.sh finish <slug>`.
- **Commits:** Conventional Commits, validated by `convco`. Scope = crate name
  without the `nebula-` prefix, or a top-level area (`docs`, `ci`).
- **Gates:** `lefthook` (`pre-commit` per-crate fmt+clippy; `pre-push`
  full-workspace clippy + crate-diff nextest) mirrors the CI required jobs in
  `.github/workflows/ci.yml` (fmt, clippy, nextest, doctests, MSRV 1.95, deny).
  Keep lefthook ↔ CI in sync if you touch either.
- **Coding rules (enforced, not advisory):** no `unwrap()`/`expect()`/`panic!()`
  in library code (tests/const/bins exempt); no `TODO`/`FIXME`/plan-ids in
  committed code; **observability is Definition of Done** — every new state /
  error / hot path ships a typed `thiserror` variant + `tracing` span +
  invariant check. `.claude/hooks/*.sh` enforce these per-turn for Claude Code
  (`task hooks:test` proves each guard).
- **Local infra:** `task db:up && task db:migrate` (Postgres + sqlx);
  `task obs:up` (Jaeger + OTEL collector).

> Windows note: `cargo fmt --all` / `task dev:check` fmt:check can break with
> OS error 206 in deep worktree paths — verify fmt per-crate; don't report
> `dev:check` green from a deep worktree.

## 6. Code intelligence (symbol navigation)

For an LLM/IDE working this codebase, set up Rust symbol intelligence rather
than grepping blind:

- **Claude Code:** install a Rust LSP plugin
  (`/plugin install <rust-lsp>@claude-plugins-official`) or wire the **serena**
  MCP for symbol-level find/edit (find-symbol, find-referencing-symbols,
  rename) — far cheaper than re-reading whole files.
- **Pi:** `.pi/lsp.json` already maps `rust-analyzer` for `.rs` (clippy on
  `check`, all features, `target/` excluded) and `taplo` for `.toml`; the
  `lsp_diagnostics` / `lsp_fix` tools use it.
- `.claude/settings.json` now **denies `Read` of `target/**`** (and
  `*.generated.rs`) so generated/build artifacts don't pollute context — rely
  on the LSP / Grep tools for navigation instead.

## 7. Active frontiers / moats

- **The moat: active credential lifecycle.** Per [`STRATEGY.md`](STRATEGY.md)
  + the competitive analysis, Nebula's empty-niche bet is OAuth-refresh +
  rotation + per-tenant isolation **as an engine primitive**. The contract +
  facade exist (`nebula-credential`, `nebula-credential-runtime`,
  ADR-0066/0088); for 1.0 the path is **reactive-only** (pre-expiry/proactive
  refresh deferred to 1.1 per ADR-0084).
- **The bind-population gap (the live frontier).** `nebula-resource` stays
  `frontier` because there is **no production credential→slot bind-population
  resolver** — `register_and_bind` has a quiesce contract and **zero callers**.
  Rotation fan-out dispatch is wired (#688/#690/#703); bind-population is the
  remaining piece, tracked under ROADMAP **M11.5 residual / M12.4**. This is the
  reason `nebula-resource` is the active frontier crate.
- **In-flight credential rework:** ADR-0088 is landing on `main` right now
  (nebula-crypto extract, policy-as-data lifecycle, `#[credential]` attribute
  macro, 4-registries→1 collapse, canonical `owner_id` derivation — see recent
  `git log`). Check the latest commits before assuming registry/owner-id
  shapes.
- Decisions live in [`docs/adr/`](docs/adr/) (live = 0046+ standalone +
  contract ADRs 0080–0082; index for 0001–0041 in
  [`docs/adr/HISTORICAL.md`](docs/adr/HISTORICAL.md)). **Conflict order:**
  PRODUCT_CANON → INTEGRATION_MODEL → accepted ADR → STRATEGY → crate README
  (per [`docs/README.md`](docs/README.md)). Plans under `docs/plans/` are
  **non-normative**.

## 8. Gotchas

- Recurring trap classes (the ones that repeated ≥2× across crates) →
  [`docs/pitfalls.md`](docs/pitfalls.md). Read it before touching a hot path.
  (e.g. `nebula-expression`: builtins must not re-enter the evaluator — the
  `BuiltinView` boundary makes it a compile error, not a discipline rule.)
- **Don't bulk-read `docs/adr/0*` or `glob docs/**`.** Use
  [`docs/README.md`](docs/README.md) (the doc map) + one crate README + the
  single cited ADR. Tier-0 docs first.
- "Open" GitHub issues are frequently already fixed (squash-merge auto-close
  misses) — `git log --grep="#N"` before planning work against one.
- ADRs are point-in-time. If following one forces workarounds, supersede it
  (don't patch around it) — that includes past-wave decisions.
