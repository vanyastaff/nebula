# Engineering Playbook 2026

> Distilled from a July 2026 study of five reference-grade Rust codebases:
> **iroh** (n0-computer, p2p networking, 4 years → 1.0), **reth** (Paradigm,
> ~150-crate workspace), **omicron** (Oxide, rack control plane — the closest
> architectural relative of Nebula), **rust-analyzer** (matklad-era engineering
> culture), and **uv** (Astral, velocity-with-quality benchmark).
> Each rule cites where it comes from. "Status" tracks Nebula's adoption.

Companion docs: [`QUALITY_GATES.md`](QUALITY_GATES.md) (lint citations),
[`AGENTS.md`](../AGENTS.md) (agent rules), [`pitfalls.md`](pitfalls.md).

---

## 1. Convergent rules — practices all five projects share

These are the "rules of the game" in 2026: independently reached by every
project studied. Divergence from these needs a written reason.

| # | Rule | Seen in | Nebula status |
|---|------|---------|---------------|
| C1 | `[workspace.lints]` in root Cargo.toml; every crate `[lints] workspace = true`; **every allow/deny decision listed with a comment, never silent** | all five | ✅ done |
| C2 | Restriction lints promoted: `print_stdout`, `print_stderr`, `dbg_macro` warn/deny — all output flows through one sink (`tracing` / a `Printer`) | uv, rust-analyzer, iroh | ✅ done (2026-07-06) |
| C3 | `#[expect(..., reason)]` over `#[allow]`; stale suppressions fail the gate | uv (AGENTS.md), Nebula | ✅ done (2026-07-06) |
| C4 | **Lint the feature matrix, not just `--all-features`**: clippy on default + `--no-default-features` + all-features, or `cargo hack` | iroh (3 configs), reth (hack, sharded), omicron (`xtask check-features`) | ❌ gap #1 — proven by the 2026-07-06 `cfg_attr(not(feature))` incident |
| C5 | Rustdoc is a hard gate: `RUSTDOCFLAGS="-D warnings"` on the workspace | reth, omicron, iroh | ❌ gap |
| C6 | Unused-dependency control is mechanized: `unused_crate_dependencies` warn per crate + udeps/machete/shear in CI | reth (all three layers), uv (shear) | 🟡 partial (machete only) |
| C7 | Generated artifacts are committed and CI re-generates + `git diff --exit-code` | rust-analyzer (`codegen --check`), uv (`check-generated-files`), omicron (openapi), reth (CLI docs) | ❌ not yet needed at scale; adopt with schema export |
| C8 | A project-specific hermetic **test-context crate** (temp dirs, pinned env, frozen clock, redaction filters) | uv (`uv-test`, 2500 loc), omicron (`#[nexus_test]`), iroh (patchbay netsim) | 🟡 partial (`test-utils` exists, not formalized) |
| C9 | One aggregator required-check (`alls-green` / `required-checks-passed`); branch protection points at it, not at N jobs | reth, uv | ❌ gap |
| C10 | Conventional commits enforced mechanically (PR title check / convco) | iroh, reth, uv, Nebula | ✅ done |
| C11 | nextest as the runner, with per-test overrides (serial groups, slow-timeouts, retries) | reth, uv, omicron | 🟡 partial (no `.config/nextest.toml` tuning) |
| C12 | SHA-pinned GitHub Actions + workflow security audit (`zizmor`) | uv, reth | ❌ gap |

---

## 2. Per-project findings worth stealing

### 2.1 omicron (Oxide) — the control-plane playbook

Most relevant to Nebula: their saga engine (steno) is our workflow domain.

- **Saga rules** (`nexus/src/app/sagas/`): every forward action has a
  **mandatory idempotent undo**; actions declared via a DSL
  (`+ action - undo`), node outputs are named and `lookup()`-able; sub-sagas
  compose. Undo re-looks-up by ID, then deletes — never trusts cached state.
- **Auth context is serialized into saga params** so a crash-recovered saga
  resumes as the original actor, not as "system".
- Design note (verbatim): *"the more constrained this interface is, the easier
  it will be to test, version, and update in deployed systems"* — keep the
  execution-context surface deliberately narrow.
- **API-first**: endpoints are a `#[dropshot::api_description]` **trait**;
  OpenAPI is generated from the trait and committed; typed clients are
  progenitor-generated with a `replace` map onto shared hand-written types.
  Endpoint versions are declared as ranges (`versions = V1..V2`) in-tree.
- **Error split**: `MessagePair { external_message, internal_context }` —
  operator-facing text vs internals; one `From<Error> for HttpError` is the
  single domain→HTTP mapping point; **retryability is a first-class client
  predicate** (`is_retryable`). Nebula's `SecretFreeMessage` is the same
  instinct — extend it toward the full pair.
- **DB discipline**: numbered migration dirs, one SQL statement per file,
  schema version pinned in Rust; the service refuses to start against a
  mismatched schema version.

### 2.2 rust-analyzer — culture that is cheap to adopt

- **`Architecture Invariant:` callouts** in architecture docs, often stating
  what is *deliberately absent* ("`syntax` knows nothing about salsa").
  Crates tagged **API Boundary** vs internal. → Adopted into `AGENTS.md`.
- **`cov_mark`** (`hit!` in code / `check!` in test, 713 uses): ties a test to
  the exact branch it covers; a tidy test rejects unpaired marks.
- **expect-test** snapshots with inline fixtures + `UPDATE_EXPECT=1` bulk
  update; fixtures use a mini-language (`$0` cursor, `//- minicore:`).
- Style canon: push control flow to the caller; early `return Err(e)` over
  `Err(e)?` (dead-code detection); **never provide setters**; boring long
  local names; ban `#[should_panic]` and `#[ignore]` (assert the wrong
  behavior + FIXME instead); `stdx::never!`/`always!` for recoverable
  invariants instead of process-killing asserts.
- **xtask owns all bespoke automation**; `cargo test` is the single local
  gate that also runs tidy/codegen checks.

### 2.3 reth — big-workspace mechanics

- **`*-api` / `*-types` vs impl crate split** kills dependency cycles
  (Nebula's `storage-port` is this pattern; extend where cycles threaten).
- **zepter** enforces feature propagation across the crate graph (if A→B,
  A must re-expose B's `std`/`serde`/`test-utils` features).
- `#![cfg_attr(not(test), warn(unused_crate_dependencies))]` in every lib.rs;
  a CI job asserts test-only deps (`proptest`, `arbitrary`) never reach the
  release binary via `cargo tree | grep`.
- Curated **nursery** lint block with a written philosophy: *"nursery lints
  are allowed by default … enable them to prevent future problems"*.
- **Profile ladder**: `dev` = `line-tables-only` + `split-debuginfo`
  (fast edit cycle), `release` thin-LTO, `maxperf` fat-LTO/cu=1,
  `profiling`, `reproducible`. Nebula's ladder already matches ~80%.
- Docs gate: `--show-type-layout --generate-link-to-definition -D warnings`.

### 2.4 uv (Astral) — velocity with quality

- **insta at scale**: central `TestContext` hermetizes env (temp roots,
  `HOME`/`XDG` pinned, `COLUMNS=100`), **freezes the clock**
  (`UV_EXCLUDE_NEWER`) so snapshots never drift, and normalizes output via
  regex filters (`[TIME]`, `[SIZE]`, `[VERSION]`, `\`→`/`).
- **CI plan job**: diffs changed paths → ~16 boolean outputs; docs-only PRs
  skip Rust entirely; heavy suites run on labels (`test:integration`) or on
  `main` only. Snapshot failures upload as artifacts; a script pulls them
  down for local `cargo insta review`.
- **CodSpeed** two-mode benchmarking (walltime + instruction-count) per PR.
- Release automation: cargo-dist (18 targets) + changelog generation from
  merged PRs; `NEVER cargo update` wholesale — `--precise` only.
- Their 20-line `AGENTS.md` of ALWAYS/NEVER/PREFER imperatives is the
  densest agent-rules format seen. → Adopted as a layer in `AGENTS.md`.

### 2.5 iroh — application-level async canon (full study 2026-07-06)

- **Actor discipline**: struct owns `mpsc` inbox(es) + `async fn run(mut self)`;
  **priority inbox** for control messages separate from blocking work;
  supervisor holds children in a `JoinSet`, cancellation cascades through
  `CancellationToken::child_token()`; shutdown is bounded by `timeout(3s)`;
  fire-and-forget tasks are `AbortOnDropHandle`.
- **Cancel-safety as a documented API property**: hot `select!` loops are
  `biased;` with the cancel branch first; branches gated
  (`if fut.is_none()`) so no work is accepted mid-flight; methods document
  "Cancel-safe" explicitly.
- `n0-watcher` (Watchable/Watcher) for observable state instead of
  lock-and-poll; channels ≫ watch ≫ locks.
- Error style: **per-operation error enums** (`ConnectError` ⊃
  `ConnectWithOptsError`), `#[non_exhaustive]` on all public errors, and a
  documented `AnyError` escape hatch: wrap third-party errors as opaque with
  an explicit "not covered by semver" note instead of leaking their types.
- CI runs clippy on all three feature configs; `cargo-check-external-types`
  guards the public API surface.

---

## 3. Adoption plan

### Tier 1 — quick wins (CI/config only, no code churn)

1. **Feature-matrix clippy** (C4): add default + `--no-default-features`
   clippy jobs; `cargo hack --each-feature` for feature-heavy crates
   (`log`, `credential`, `resource`, `storage`).
2. **Rustdoc gate** (C5): `RUSTDOCFLAGS="-D warnings"` workspace job.
3. **Dep hygiene** (C6): `unused_crate_dependencies` in workspace lints
   (cfg-gated to non-test) + reth's no-test-deps-in-release `cargo tree` check.
4. **`cargo-check-external-types`** on `sdk` and `api` (the public crates).
5. **Aggregator check** (C9) + SHA-pin actions + `zizmor` (C12).
6. `.config/nextest.toml`: serial groups for DB tests, slow-timeout kill.

### Tier 2 — medium (a week of focused work)

7. **Architecture Invariants**: keep the root list in `AGENTS.md` curated;
   add an `## Invariants` section to each crate's `AGENTS.md` stating what
   the crate deliberately does NOT do; tag API-boundary crates.
8. **Snapshot testing**: adopt `insta` for engine outputs / API responses /
   error renderings with uv-style redaction filters and a frozen clock in
   the test context; adopt `cov_mark` for branch-tied tests in `engine`
   and `resilience`.
9. **Formalize the test-context crate** (C8): one `nebula-test-context` with
   hermetic env, DB bootstrap, and filter presets, replacing ad-hoc helpers.

### Tier 3 — strategic (design work, separate ADRs)

10. **Saga-grade engine review** (omicron): audit workflow actions for
    idempotent undo coverage; serialize auth context into durable workflow
    params; narrow the execution-context trait surface.
11. **API-first flip** (omicron): define `nebula-api` endpoints as a trait,
    commit generated OpenAPI, generate the SDK client from it (progenitor or
    equivalent), with explicit endpoint version ranges.
12. **Schema-version guard** (omicron): pin the DB schema version in Rust and
    refuse startup on mismatch (extend existing sqlx migrations).

---

## 4. What Nebula already does at or above the 2026 bar

- Curated `[workspace.lints]` with per-decision comments (C1) — matches reth.
- `print_*`/`dbg_macro` gates + `#[expect]` discipline (C2, C3) — stricter
  than uv (reasons required at the hook level).
- Layered dependency map **mechanically enforced** via cargo-deny wrappers —
  none of the five studied projects enforce layering this directly.
- `AGENTS.md` + per-crate agent maps — richer than reth's, far richer than
  uv's; the study's additions are invariants and the terse-rules layer.
- Conventional commits + convco validation (C10), nextest, cargo-deny +
  audit, profile ladder with `line-tables-only`, secret-free error messages
  (`SecretFreeMessage` ≈ omicron's `MessagePair`).
