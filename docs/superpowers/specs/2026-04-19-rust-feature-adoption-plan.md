---
title: Rust 1.75-1.95 feature adoption plan
date: 2026-04-19
status: draft
related:
  - docs/adr/0014-dynosaur-macro.md
  - docs/adr/0019-msrv-1.95.md
  - docs/audit/2026-04-19-codebase-quality-audit.md
  - docs/STYLE.md
  - docs/MATURITY.md
---

# Rust 1.75-1.95 feature adoption plan

## Executive summary

- **Scope at HEAD.** 88 `#[async_trait]` attributes across 49 `.rs` files in 7
  crates; 0 `dynosaur` usages. ADR-0014 compliance is 0 %. (Audit quoted
  81/54 — the number has drifted +7 attrs, −5 files since 2026-04-19 morning
  because markdown design docs were included in the original count.)
- **Hardest slice.** `TriggerHandler` (18 dyn sites across `action` +
  `api` + `sdk`) and `CredentialAccessor` (16 dyn sites, cross-crate). Each
  is a single-ADR, multi-crate PR — cannot be sharded per-crate without
  breaking the workspace for a cycle.
- **Free lunch.** `once_cell` is down to **one** surviving call site
  (`crates/expression/src/maybe.rs:7` — a `use once_cell::sync::OnceCell;`),
  `lazy_static!` is already zero, and `LazyLock` / `OnceLock` are used 84
  times across 31 files. Removing `once_cell` from `[workspace.dependencies]`
  is a one-line-touch chip.
- **Recommended sequencing.** Phase 1 free-lunch (`once_cell` removal +
  `#[expect]` conversions, single PR per crate, parallelisable). Phase 2
  inherent AFIT for the 18 traits with zero `dyn` sites (one PR per crate,
  no cross-cutting). Phase 3 `dynosaur` migration for high-fanout traits
  (five multi-crate PRs, ADR-gated — see Hazards). Phases 4–5 are polish.
- **Biggest gotcha.** Two legacy storage traits (`WorkflowRepo`,
  `ExecutionRepo`) have duplicated definitions under
  `crates/storage/src/*_repo.rs` (actively used as `Arc<dyn …>` from the
  engine) **and** `crates/storage/src/repos/*.rs` (newer, not yet wired).
  The migration must not merge the two — the `repos/*.rs` layer already
  avoids `dyn`, so only the legacy pair needs dynosaur.

## Inventory

### `#[async_trait]` usage (per crate)

Counts come from `rg --count-matches '#\[async_trait\]' --glob '*.rs'`
(filtering out markdown design docs included in the audit). `dyn` column
reflects cross-workspace `\bdyn\s+<Trait>\b` hits at HEAD.

| Crate | Attrs | Defs | Impls | Dyn traits (fanout) | Notes |
|---|---:|---:|---:|---|---|
| `nebula-action` | 33 | 8 | 25 | `TriggerHandler` (18), `StatelessHandler` (14), `ResourceHandler` (9), `ResourceAccessor` (7), `ExecutionEmitter` (7), `StatefulHandler` (6), `TriggerScheduler` (5), `AgentHandler` (2) | Highest concentration. 8 trait families; every `dyn`-consumed integration seam lives here. |
| `nebula-storage` | 27 | 18 | 9 | `ControlQueueRepo` (10), `ExecutionRepo` (6, legacy at `execution_repo.rs:121`), `WorkflowRepo` (4, legacy at `workflow_repo.rs:78`). **17 others with 0 dyn sites** (`WorkflowVersionRepo`, `AuditRepo`, `WorkspaceRepo`, `JournalRepo`, `CredentialRepo`, `ExecutionNodeRepo`, `BlobRepo`, `UserRepo`, `SessionRepo`, `PatRepo`, `OrgRepo`, `ResourceRepo`, `QuotaRepo`, `TriggerRepo`, plus `repos/workflow.rs`-side `WorkflowRepo` & `repos/execution.rs`-side `ExecutionRepo` duplicates). | Split personality: legacy `*_repo.rs` files (3 traits, wired to engine via `Arc<dyn>`) vs newer `repos/*.rs` layer (17 traits, not yet dyn-consumed). Migration must keep the split. |
| `nebula-credential` | 13 | 4 | 9 | `CredentialAccessor` (16, cross-crate). `NotificationSender`/`TestableCredential`/`RotatableCredential` have 0 dyn sites. | Rotation traits are pure generic bounds — inherent AFIT fine. |
| `nebula-engine` | 5 | 0 | 5 | n/a (consumer only) | Impls of traits defined in `storage`, `credential`, `action`. Migrations here are follow-on to upstream trait surgery. |
| `nebula-runtime` | 4 | 3 | 1 | `StatefulCheckpointSink` (6), `BlobStorage` (2). `TaskQueue` has 0 dyn sites. | Small but feeds the engine's hot path — needs careful `'static` analysis after AFIT flip. |
| `nebula-sandbox` | 4 | 1 | 3 | `SandboxRunner` (2) | Low fanout. Single-PR migration. |
| `nebula-api` | 2 | 0 | 2 | n/a (consumer) | Both impls are in `handlers/health.rs`. |
| **Total** | **88** | **34** | **54** | — | 49 `.rs` files; audit 2026-04-19 quoted 81/54 (markdown-inclusive). |

Discrepancy vs audit explained: the audit counted any `.rs` **or**
markdown occurrence (`crates/action/docs/*.md` has 25, `crates/resource/
plans/*.md` has 3). Restricting to compiled code gives 88/49.

#### Duplicate trait definitions (known hazard)

| Trait | Legacy (in-use) | Newer (staged) | Recommendation |
|---|---|---|---|
| `WorkflowRepo` | `crates/storage/src/workflow_repo.rs:78` — consumed as `Arc<dyn WorkflowRepo>` in `crates/engine/src/engine.rs:141,497` and `crates/engine/tests/control_dispatch.rs:148`. | `crates/storage/src/repos/workflow.rs:16` — 0 dyn sites. | Phase 3 migrates the legacy one via `dynosaur`; Phase 2 moves the `repos/*.rs` sibling to inherent AFIT. Do not merge the two under this plan — ADR-0008 refactor owns that decision. |
| `ExecutionRepo` | `crates/storage/src/execution_repo.rs:121` — consumed as `Arc<dyn ExecutionRepo>` in `engine.rs:139,486,2728,3512,7500,7594`. | `crates/storage/src/repos/execution.rs:14` — 0 dyn sites. | Same split; same recommendation. |

### Other migration targets

Counts from `rg --count-matches` at HEAD unless stated.

#### `once_cell` / `lazy_static!` → `LazyLock` / `OnceLock` (stable 1.80)

| Pattern | Count | Notes |
|---|---:|---|
| `once_cell::sync::Lazy` | 0 | — |
| `once_cell::sync::OnceCell` | 1 | Single site: `crates/expression/src/maybe.rs:7`. |
| `once_cell::race::*` | 0 | — |
| `lazy_static!` | 0 | Already fully migrated. |
| `LazyLock` / `OnceLock` / `std::sync::Once` | 84 (31 files) | Pattern is already the workspace default. |
| `once_cell` in a crate `Cargo.toml` | 1 | `crates/expression/Cargo.toml:30`. |

**Workspace-dep removal estimate.** Deleting `once_cell = "1.21"` from
`Cargo.toml:79` and `crates/expression/Cargo.toml:30` plus flipping the
single `OnceCell` at `crates/expression/src/maybe.rs:7` is one small PR.
`OnceLock::get_or_try_init` shipped stable in 1.70 and is a drop-in for
the `OnceCell::get_or_try_init` shape used here (verify on read — the
file is 5 lines of use; no exotic feature).

#### `parking_lot` const-init → `std::sync::Mutex::new` (stable 1.83)

| Pattern | Count | Notes |
|---|---:|---|
| `parking_lot` in `Cargo.toml` | 8 crates | Kept for non-poisoning + uncontended-fast-path. |
| `parking_lot` mentions in `.rs` | 41 | Most are `parking_lot::RwLock` / `Mutex` in hot structs. |
| `static … : parking_lot::(Mutex\|RwLock)<…>` | **0** | No static declarations exist. |
| `const_new` feature requested in any `Cargo.toml` | 0 | — |

**Recommendation: no change.** `parking_lot` earns its place on
uncontended fast paths (no poisoning, smaller stable size). There is
nothing in the workspace using it **purely** for `const_new`, so there is
no "std now has const fn, swap it" win. Leave this class alone.

#### Nested `if let` → let-chains (stable 1.88)

| Pattern | Count | Notes |
|---|---:|---|
| `if let Some(…) = … { if … }` (sampled multiline) | 16 | Direct let-chain candidates. |
| `if let … = … { if … }` (any pattern, sampled) | 23 | Wider pool; not all will read better as chains. |
| `let … else { … }` already in use | 147 | Workspace is already idiomatic about single-escape let-else. |

Heuristic for a PR: convert only where at least three levels of nesting
share one body (i.e. eliminate an `else { return … }` mirror too). Avoid
flattening two-level `if let` / `if` pairs where the inner arm has more
than two statements — the nested form is still easier to read there.
Safe to defer — no dep removal, no compile-time impact.

#### `#[allow(...)]` → `#[expect(...)]` (stable 1.81)

| Pattern | Count | Notes |
|---|---:|---|
| `#[allow(dead_code)]` | 42 | Largest bucket. Many have a "used only when feature X enabled" story — good `#[expect]` candidates. |
| `#[allow(unused*)]` | 4 | Usually local; should be `expect`. |
| `#[allow(deprecated)]` | 5 | Must verify each still fires — `expect` gives us a regression guard. |
| `#[allow(clippy::*)]` | 38 | Rule-specific; safe to flip. |
| All other `#[allow(...)]` | 27 | Scan individually — some are legitimate forward-compat. |
| `#[expect(...)]` already | 21 | Migration started; no blocker. |
| **Total `#[allow]` in tree** | **116** | Upper bound on the chip. |

**Plan:** do *not* flip a blanket `s/allow/expect/`. Flip on a
crate-by-crate pass, confirming the lint still fires (build in verbose
mode, then `grep` for the `unfulfilled_lint_expectations` warning that
exposes a stale expect). The `forward-compat for future lints`
`#[allow]`s — typically the ones with no explanatory comment — stay as
`allow`. A sane target is ~80–90 conversions out of 116.

#### `core::error::Error` (stable 1.81)

| Pattern | Count |
|---|---:|
| `std::error::Error` refs | 60 |
| `core::error::Error` refs | 0 |

Most of Nebula is firmly std-bound (tokio, reqwest, sqlx). Only
candidates for `core::error::Error` would be `nebula-error` itself and
possibly `nebula-expression` / `nebula-validator` — crates that *could*
become `no_std`-adjacent later. Not worth a chip until someone files a
`no_std`-use story. Classified **P3 polish**, not a migration blocker.

#### Async closures (stable 1.85)

| Pattern | Count | Notes |
|---|---:|---|
| `Box::pin(async move …)` | 54 | Pool of potential `async ||` workarounds — must inspect each for shape (closures that capture shared state and are called multiple times are the real target; one-shot `Box::pin(async move)` in a `spawn` is not). |
| Existing `async fn` / `async \|\|` closures | n/a | Nothing to convert to. |

Not every `Box::pin(async move)` is an async-closure candidate; many are
inside `spawn` calls where the pinning is incidental. Real targets are
stored `FnMut`-shaped futures (e.g. retry predicates, observer
callbacks). A useful chip does a one-hour pass, converts ~5–10 real
cases, and stops.

#### `precise capturing use<…>` (stable 1.82) — AFIT migration blocker

| Pattern | Count | Notes |
|---|---:|---|
| `tokio::spawn(…)` total | 80 | Overall spawn surface. |
| `tokio::spawn(async move { adapter.<method>(…).await })` | ~22 | These call a method on a trait object that is currently `#[async_trait]` — after naive AFIT the returned future captures `'self` and cannot cross thread boundaries. |
| `use<…>` uses at HEAD | 0 | No adoption yet. |

The 22 at-risk spawn sites are concentrated in:
`crates/action/tests/dx_poll.rs` (15 sites — `adapter.start(...)` on
`TriggerHandler`), `crates/action/tests/dx_webhook.rs` (3 sites —
`adapter.handle_event(...)` / `stop(...)`), `apps/cli/src/commands/
actions.rs:294` (`handler.start(...)`), `crates/sdk/src/runtime.rs:231`
(`handler.start(...)`), `crates/engine/src/control_consumer.rs:308`
(`self.run(...)`), and `crates/storage/src/pg/control_queue.rs:1034-1035`
(integration-test worker spawns on `ControlQueueRepo`).

After the Phase 3 dynosaur flip these keep working (the `Dyn*` sibling
returns `Pin<Box<dyn Future + Send>>` just like `async-trait` did). The
`use<>` precise-capture chip kicks in for Phase 2 — the 18 inherent-AFIT
traits' return futures. Without `use<>`, spawning an inherent AFIT call
on a non-`'static` borrow breaks compile. Budget 1–2 per-call adjustments
per crate.

#### Small wins (sweep)

| Pattern | Count | Notes |
|---|---:|---|
| `inline const { … }` blocks | 2 | Already a few; low-volume target. |
| `Option::as_slice` (rough) | 80 occurrences of `as_slice` workspace-wide — hard to attribute cleanly without per-call inspection | Most are already the slice variant; no blanket migration. |
| `Cell::update` | 0 | No use sites — hand-written `let v = c.get(); c.set(f(v));` audits would be the chip. Low-value. |
| `[T]::as_chunks` (1.88) | 0 hits for `.chunks(` | Unused. |
| Atomic `update` / `try_update` candidates (`fetch_update` / `compare_exchange`) | 5 | Small enough to inline in Phase 5 polish. |
| `cfg_if!` invocations | 0 | `cfg_select!` has nothing to replace. |
| Match-arm `if let` guards (stable 1.95) | 0 nested `=> if` hits in sample | Future use, not a migration. |

## Migration sequencing

Five phases, ordered by reviewability and blast radius. Each phase
produces an independent, revertable PR family.

### Phase 1 — Free-lunch sweep (1 PR)

**Scope.** Remove `once_cell` and switch a small batch of `#[allow]`s
with explanatory comments to `#[expect]`.

- Flip `crates/expression/src/maybe.rs:7` from `once_cell::sync::OnceCell`
  to `std::sync::OnceLock` (`get_or_try_init` is API-compatible).
- Delete `once_cell = "1.21"` from `Cargo.toml:79` and
  `crates/expression/Cargo.toml:30`.
- Open a separate per-crate chip converting `#[allow(dead_code)]` →
  `#[expect(dead_code)]` for the ones that explain themselves in a
  trailing comment. Skip bare `#[allow]` with no rationale.

**Risk.** Zero blast radius; one workspace dep disappears.

**Verify.**

```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

**Acceptance.** `cargo deny check` still green; `rg 'once_cell' crates/`
and `rg 'lazy_static!' crates/` both return nothing; workspace
dependency count drops by **1** entry.

### Phase 2 — Inherent AFIT for zero-dyn traits (~7 PRs, one per crate)

**Scope.** 18 traits that have **zero** `dyn Trait` use sites. Drop
`#[async_trait]`, leave the trait as plain `async fn` (stable 1.75 AFIT),
delete the `async_trait` macro imports from the crate.

Trait list by crate:

| Crate | Traits with 0 `dyn` sites | File anchors |
|---|---|---|
| `nebula-storage` (new `repos/*.rs` layer) | `WorkflowVersionRepo`, `AuditRepo`, `WorkspaceRepo`, `JournalRepo`, `CredentialRepo`, `ExecutionNodeRepo`, `BlobRepo`, `UserRepo`, `SessionRepo`, `PatRepo`, `OrgRepo`, `ResourceRepo`, `QuotaRepo`, `TriggerRepo`, newer `WorkflowRepo` (repos/workflow.rs:16), newer `ExecutionRepo` (repos/execution.rs:14) | `crates/storage/src/repos/*.rs` |
| `nebula-credential` | `NotificationSender`, `TestableCredential`, `RotatableCredential` | `crates/credential/src/rotation/{events.rs,validation.rs}` |
| `nebula-runtime` | `TaskQueue` | `crates/runtime/src/queue.rs:50` |

**Per-crate PR shape.**

1. Remove `#[async_trait]` attribute on each trait definition and each
   impl in the crate.
2. Delete the `async_trait` macro import in each `.rs` file touched.
3. Delete the `async-trait` dependency from the crate's `Cargo.toml` if
   no other consumer remains.
4. For any `spawn` site that breaks under inherent AFIT because the
   returned future captures a non-`'static` borrow, add an
   `impl Future<Output = …> + Send + use<>` (or the explicit generic
   whitelist) to the method signature. Inventory says no such sites
   exist for these traits today (they are all either consumed via
   generics or not spawned).
5. Update the crate's README if it mentions `async-trait`.
6. Run the canonical quickgate. Knife scenario must still pass on
   `storage` changes (see `crates/api/tests/knife.rs`).

**Risk.** Per-PR risk is small. No cross-crate coordination. Main gotcha
is impl blocks in downstream crates that were themselves decorated with
`#[async_trait]` — those also need the attribute off. Grep for
`impl <Trait> for` before merging.

**Verify.** Same as Phase 1 plus `cargo test --workspace --doc`.

**Acceptance.** `rg '#\[async_trait\]' crates/<touched-crate>/src/`
returns nothing; `async-trait` Cargo.toml entries drop to the crates
that still need it for Phase 3.

### Phase 3 — `dynosaur` migration for cross-crate `dyn` traits (5 PRs, ADR-tracked)

**Scope.** The 14 traits that *are* consumed as `dyn` somewhere. One
coordinated PR per family; the ADR gate in Hazards below applies.

Ordered by fanout (highest first — where the most call sites change):

| # | Trait family | Owner crate | Dyn sites | Consumer crates | Notes |
|---:|---|---|---:|---|---|
| 1 | `TriggerHandler` | `nebula-action` | 18 | `action`, `api`, `sdk`, `apps/cli`, `apps/desktop` | Highest fanout. Action's integration seam. |
| 2 | `CredentialAccessor` | `nebula-credential` | 16 | `credential`, `engine`, `action`, `resource` | Hot path — preserving static dispatch is the point. |
| 3 | `StatelessHandler` | `nebula-action` | 14 | `action`, `sdk`, `apps/cli` | Internal action-crate seam mostly. |
| 4 | `ControlQueueRepo` | `nebula-storage` | 10 | `storage`, `engine`, `api` | Canon §12.2 durable control plane — extra care. |
| 5 | Storage legacy dyn pair | `nebula-storage` | `ExecutionRepo` (6), `WorkflowRepo` (4) | `storage`, `engine`, `api` | Keep split from `repos/*.rs` siblings. Batch into one PR. |
| 6 | Remaining action traits | `nebula-action` | `ResourceHandler` (9), `ResourceAccessor` (7), `ExecutionEmitter` (7), `StatefulHandler` (6), `TriggerScheduler` (5), `AgentHandler` (2) | `action`, `engine`, `sdk` | Bundle because same crate, same review context. |
| 7 | Remaining runtime/sandbox | `nebula-runtime`, `nebula-sandbox` | `StatefulCheckpointSink` (6), `BlobStorage` (2), `SandboxRunner` (2) | `runtime`, `sandbox`, `engine` | Smallest family. Last PR. |

Rows 6 and 7 can bundle (same-crate consolidations); rows 1–5 each
want their own PR.

**Per-PR shape (ADR-0014 alignment).**

1. Add `#[dynosaur::dynosaur(DynFoo)]` to the trait definition (AFIT form).
2. Drop `#[async_trait]` from the trait and every impl in the workspace.
3. At storage/registry sites, replace `Arc<dyn Foo>` with
   `Arc<dyn DynFoo>`. At static-dispatch sites, keep `impl Foo`.
4. Pin `dynosaur = "<exact>"` in `[workspace.dependencies]` (ADR-0014
   follow-up note — use exact pin so semver bumps are intentional).
5. Update crate README and, where relevant, the `*Metadata`/`*Schema`
   types' public docs.
6. Run the knife scenario (`crates/api/tests/knife.rs`) — this hits the
   engine's dispatch dyn-path for `TriggerHandler`, `ExecutionRepo`,
   `ControlQueueRepo` end-to-end.

**Risk.** Dynosaur is a young crate (ADR-0014 explicitly flags this).
Mitigations:

- Never downgrade via `cargo update` — exact pin.
- Every PR runs the full workspace test suite plus the knife scenario.
- If a trait has `Self: Sized` bounds, generic methods, or returns
  referencing `Self`, dynosaur refuses — move those methods to a
  sealed static-dispatch sibling trait before flipping. **None of the
  34 trait defs today use these shapes** (spot-checked the 14 dyn ones
  during inventory); if a future refactor adds one, revisit.

**Verify.**

```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo +1.95 check --workspace        # MSRV gate
cargo nextest run -p nebula-api --test knife  # full knife scenario
```

**Acceptance.** `rg '#\[async_trait\]' crates/ apps/` returns at most
the markdown design docs; `rg 'dynosaur::dynosaur' crates/` equals 14
(one per dyn-consumed trait); `async-trait` dep removed from every
crate's `Cargo.toml`; `[workspace.dependencies]` loses the
`async-trait = "0.1.89"` line.

### Phase 4 — `use<…>` precise-capture cleanup (inline, no standalone PR)

Happens inside Phase 2 and Phase 3 PRs when a compile error points at
the future returned from an AFIT method. No separate chip unless the
compiler finds something we missed — in which case one tidy-up PR
covers the stragglers.

Budget: ~22 touches across the 18 spawn-through-trait sites enumerated
under "precise capturing use<...>" above.

### Phase 5 — Late polish (parallel, each ~1 PR)

| Pattern | Shape |
|---|---|
| let-chains | One chip per crate, touch the 16 `if let Some = _ { if … }` nesting sites. Defer indefinitely if nobody cares. |
| Atomic `update` / `try_update` | Replace 5 `fetch_update` / `compare_exchange` loops in `telemetry`/`metrics` where the shape matches. |
| `inline const { … }` | Opportunistic only; no dedicated chip warranted. |
| `core::error::Error` | Only if a `no_std`-adjacent goal emerges (not today). |
| Async closures | One-hour pass over the 54 `Box::pin(async move)` sites; convert the stored `FnMut`-like cases, leave the incidental spawn-wrappers alone. |

## Hazards / things that need an ADR before code moves

| Hazard | Where | Action |
|---|---|---|
| `ExecutionRepo`, `WorkflowRepo`, `ControlQueueRepo` are part of `nebula-storage`'s workspace-internal surface. ADR-0021 (crate publication policy, PR #501) should decide whether these crates are `publish = true` before Phase 3 makes `dyn DynExecutionRepo` a rename. If `storage` stays `publish = false` this is a CHANGELOG note; if it flips to `publish = true` the rename is a SemVer breaking event that needs its own ADR. | `crates/storage/Cargo.toml` | Verify publication status with tech-lead before merging Phase 3 PR #4/#5. |
| `TriggerHandler` is re-exported through `nebula-sdk::prelude` (see audit finding about the 60-item glob prelude). A rename `dyn TriggerHandler` → `dyn DynTriggerHandler` in sdk consumer code is a visible API change, even if sdk is `publish = false`. Examples that depend on the current form exist in `examples/` workspace member. | `crates/sdk/src/prelude.rs`, `examples/**/*.rs` | Update `examples/` in the same PR; note that ADR-0014 §Style already prescribes the `Dyn*` naming, so this is intended behaviour — just not silent. |
| `CredentialAccessor` dyn migration touches `EncryptionLayer` composition path. ADR-0023 (KeyProvider, just landed PR #502) introduced a new seam right next to it; sequencing dynosaur here after ADR-0023 has stabilised avoids re-review of overlapping diffs. | `crates/credential/src/accessor.rs`, `layer/encryption.rs` | Phase 3 row #2 waits until ADR-0023 follow-ups close. |
| Dynosaur version selection. ADR-0014 §Follow-ups calls for exact version pinning; the workspace has no entry yet. First Phase 3 PR adds `dynosaur = "=<exact>"` to `[workspace.dependencies]`. | `Cargo.toml` | Include in Phase 3 PR #1 (`TriggerHandler`), not earlier. |
| Two-definition storage trait hazard. The PR that touches `crates/storage/src/workflow_repo.rs` and the PR that touches `crates/storage/src/repos/workflow.rs` must not run concurrently on different branches — they will merge-conflict over `lib.rs` re-exports. Keep them in the same PR (Phase 3 row #5). | `crates/storage/src/lib.rs:101` | Single PR, not parallel chips. |

## Out of scope

- **Edition 2024 migration beyond what 1.95 already implies.** ADR-0010
  already committed edition 2024; this plan doesn't revisit it.
- **GATs-on-futures / async stream traits.** Not a stable story yet
  workspace-wide; ADR-0014 explicitly calls out `trait-variant` as a
  future re-evaluation when the picture changes.
- **`no_std` support.** Nothing in this plan flips any crate to `no_std`.
  `core::error::Error` is mentioned only to *inventory* the surface.
- **`#[unstable(feature = …)]` gating.** Canon §11.6 feature-gating is
  orthogonal to toolchain feature adoption.
- **Internal refactors enabled by the migration.** E.g. merging
  legacy/new storage repo trait pairs, splitting
  `crates/engine/src/engine.rs` (7923 LOC) — both are independent
  audit action items owned by tech-lead (P1 #19 / ADR backlog) and are
  **not** coupled to this rollup.
- **`async-trait` in examples / docs.** Markdown design documents under
  `crates/*/docs/` and `crates/*/plans/` contain 28 `#[async_trait]`
  mentions that are illustrative, not compiled. Leave them; update on
  the next doc pass touching each file.

## Methodology notes

Counts at HEAD (2026-04-19, commit `62754680`) using the canonical
commands below so a reviewer can reproduce:

```bash
# async_trait in compiled code
rg --count-matches '#\[async_trait\]' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 88

# async_trait files
rg --files-with-matches '#\[async_trait\]' --glob '*.rs' crates/ apps/ examples/ | wc -l
# → 49

# dyn fanout per trait (example for TriggerHandler)
rg --count-matches '\bdyn\s+TriggerHandler\b' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 18

# once_cell surface
rg --count-matches 'once_cell' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 1

# #[allow] surface
rg --count-matches '#\[allow\(' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 116

# dynosaur today
rg --count-matches 'dynosaur' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 0
```

Re-run these before opening any Phase PR — the drift from audit-time
(+7 attrs, −5 files over ≈12 hours) is a reminder that these numbers
move fast.
