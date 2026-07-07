# Engineering Playbook 2026

> Distilled from a July 2026 study of ten reference-grade Rust codebases:
> **iroh** (n0-computer, p2p networking, 4 years → 1.0), **reth** (Paradigm,
> ~150-crate workspace), **omicron** (Oxide, rack control plane — the closest
> architectural relative of Nebula), **rust-analyzer** (matklad-era engineering
> culture), **uv** (Astral, velocity-with-quality benchmark), **restate**
> (durable execution — Nebula's direct domain), **vector** (Datadog, data
> pipelines — sources/transforms/sinks ≈ nodes), **wasmtime** (Bytecode
> Alliance, fuzzing/security discipline), **bevy** (plugin architecture at
> community scale), and **turso** (deterministic simulation testing).
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
| C4 | **Lint the feature matrix, not just `--all-features`**: clippy on default + `--no-default-features` + all-features, or `cargo hack` | iroh (3 configs), reth (hack, sharded), omicron (`xtask check-features`) | ✅ done (2026-07-07): workspace clippy on default + no-default in `check`, cargo-hack each-feature |
| C5 | Rustdoc is a hard gate: `RUSTDOCFLAGS="-D warnings"` on the workspace | reth, omicron, iroh | ✅ already in place (`doc` job runs `RUSTDOCFLAGS=-D warnings`) |
| C6 | Unused-dependency control is mechanized: `unused_crate_dependencies` warn per crate + udeps/machete/shear in CI | reth (all three layers), uv (shear) | ✅ done (2026-07-07): per-crate `unused_crate_dependencies` + udeps weekly + machete; 15 dead deps purged on adoption |
| C7 | Generated artifacts are committed and CI re-generates + `git diff --exit-code` | rust-analyzer (`codegen --check`), uv (`check-generated-files`), omicron (openapi), reth (CLI docs) | ❌ not yet needed at scale; adopt with schema export |
| C8 | A project-specific hermetic **test-context crate** (temp dirs, pinned env, frozen clock, redaction filters) | uv (`uv-test`, 2500 loc), omicron (`#[nexus_test]`), iroh (patchbay netsim) | 🟡 partial (`test-utils` exists, not formalized) |
| C9 | One aggregator required-check (`alls-green` / `required-checks-passed`); branch protection points at it, not at N jobs | reth, uv | ✅ already in place (`required` aggregator job) |
| C10 | Conventional commits enforced mechanically (PR title check / convco) | iroh, reth, uv, Nebula | ✅ done |
| C11 | nextest as the runner, with per-test overrides (serial groups, slow-timeouts, retries) | reth, uv, omicron | ✅ already in place (trybuild groups, ci/agent/chaos profiles) |
| C12 | SHA-pinned GitHub Actions + workflow security audit (`zizmor`) | uv, reth | ✅ done (2026-07-07): actions were already SHA-pinned; zizmor gate added (high severity) |
| C13 | Path-/label-driven CI matrix: a plan job diffs changed files and toggles expensive suites; opt-in labels or commit magic (`prtest:full`) for the rest | uv (plan job), wasmtime (`determine` + `prtest:`), turso (diff→targeted Antithesis coverage) | ❌ gap |
| C14 | Async-perf footgun lints **denied**: `await_holding_lock`, `redundant_clone`, `or_fun_call`, `assigning_clones`, `large_stack_frames` | restate + turso (deny), vector (`await_holding_lock` warn) | ✅ done (2026-07-07): all four now warn; 13 sites fixed |
| C15 | Every `unsafe` block carries a `SAFETY:` comment, lint-enforced (`undocumented_unsafe_blocks`) | wasmtime (~341 in one crate + policy doc), bevy (lint = warn) | ✅ clippy.toml configures it; `unsafe_code = "warn"` workspace-wide |
| C16 | Backend conformance suite: one shared test suite run against every implementation of a port/trait | restate (`loglet_tests.rs` over memory/local/replicated), vector (component compliance) | ✅ `crates/storage/tests/conformance.rs` — same instinct, keep extending |

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

### 2.6 restate — durable execution done right (Nebula's direct domain)

- **Fencing tokens**: every effect an invocation attempt emits is stamped with
  a `FencingToken` so the partition processor can reject stale effects from a
  superseded attempt (`invoker-impl/src/invocation_state_machine.rs`). A
  `JournalTracker` records which effects are stored+acked — retry is allowed
  only past that watermark.
- **Side effects are journaled** (`RunCommand` + completion): replay re-reads
  the journaled result, never re-executes the side effect.
- **Error codes as a system** (`codederror`): thiserror extended with
  `#[code(RT0012)]`; every code has a markdown doc under `error_codes/`;
  transient-vs-terminal is an explicit method (`is_transient()`), and
  user-code errors (`InvocationError`) are a separate type from system errors.
- **Durable-state versioning**: every stored record starts with a
  `StorageCodecKind` byte; retired discriminants are *reserved, never reused*;
  schema evolution via parallel table modules (`journal_table` /
  `journal_table_v2`) — in-flight invocations stay on their pinned protocol
  version while a `VersionBarrierCommand` gates cluster-wide transitions.
- **Conformance suite** (`loglet_tests.rs`) runs against every log backend.
- Lint stance: **denies** `or_fun_call`, `redundant_clone`,
  `assigning_clones`, `large_stack_frames`, `mutex_atomic`; warns
  `large_futures`.

### 2.7 vector — component model + mandatory telemetry (nodes ≈ sources/transforms/sinks)

- **Config trait ≠ runtime object**: components are serde-tagged config types
  (`#[typetag::serde(tag = "type")]`) with `async fn build(ctx)`; the runtime
  object is a thin future/sink. A proc-macro (`configurable_component`)
  derives serde + JSON-schema feeding docs and config validation.
- **`resources()` declares exclusive dependencies** (ports, files) so
  topology hot-reload can shut down / free / reassign in the right order.
- **Instrumentation spec is RFC-2119 law** (`docs/specs/instrumentation.md`):
  every component MUST emit `component_received_events_total`,
  `component_sent_events_total`, errors with bounded `error_type` +
  `stage ∈ {receiving, processing, sending}`. **Compliance is tested**: a
  harness (`test_util/components.rs`) runs a real component and asserts the
  required events and tags fired.
- **End-to-end acks**: `EventFinalizers` + `EventStatus` monotonic lattice
  (Rejected > Errored > Delivered) decouple delivery status from buffers.
- Buffers: `WhenFull { Block | DropNewest | Overflow }` per sink; disk buffer
  deletion is driven by finalization.

### 2.8 wasmtime — fuzzing and security discipline

- **Fuzzer-agnostic oracle library**: generators/oracles live in a reusable
  crate (`crates/fuzzing`); `fuzz/fuzz_targets/*` are 2-line glue. Differential
  fuzzing compares engines/configs; fuzz findings become committed regression
  tests annotated with the issue URL.
- **The model `unsafe` policy** (`contributing-coding-guidelines.md`): public
  API sound without `unsafe`; every `unsafe fn` documents its contract; every
  block has a locally-verifiable comment; prefer safer designs at minor perf
  cost. Miri runs a nextest matrix in CI.
- **Vulnerability classification cheat-sheet**: a table of what is/isn't a
  CVE, tied to platform support tiers; a numbered incident runbook.
- CI: one `ci-status` join gate; a `determine` job builds dynamic matrices
  from changed paths and `prtest:` commit tags.

### 2.9 bevy — plugin architecture at community scale

- `Plugin` trait with one required method + blanket impl so **a plain
  `fn(&mut App)` is a plugin**; lifecycle (`build → ready → finish → cleanup`)
  lets plugins await dependencies without hard ordering; uniqueness by
  `type_name` with opt-out.
- `PluginGroupBuilder`: `add_before/add_after/disable/enable/set` — ordering,
  overrides, and per-plugin feature gates, auto-documented via macro.
- **Migration-guide culture**: every breaking PR must add a
  `_release-content/migration-guides/*.md` entry (frontmatter + terse
  what/why/how); CI validates. `#[deprecated]` is explicitly *not* a
  substitute.
- Examples discipline: 426 examples with metadata, run headless in CI with
  screenshot comparison; CI fails if an example lacks docs metadata.
- Compile-fail tests live in separate non-workspace crates.

### 2.10 turso — deterministic simulation testing (DST)

The blueprint for making durable execution deterministically testable:

- **One `u64` seed** forks `ChaCha8Rng` streams for generation, IO, clock;
  `--seed N` reproduces any run bit-for-bit.
- **Completion-based IO trait** (`trait IO { step(); … }`, `trait File`) —
  the engine pumps `step()`; a deterministic in-memory backend replaces real
  IO; RNG and clock are routed *through the IO layer*.
- **Properties as data** (`Property` enum with documented invariants) executed
  as generated interaction plans; failures are **shrunk** to minimal
  reproducers and persisted in a **bug base** (seed + plan + commit + opts)
  for one-command replay.
- **Selective fault injection** (per-file: e.g. WAL only; error/latency/
  partial-write) with integrity checks asserted after every fault.
- **Tiered schedule**: seeded sim loops per-PR (35 min), long fault runs +
  **Antithesis** nightly (24h on Saturdays), differential runs vs real SQLite,
  Elle consistency checks, TLA+ spec for transactions, Miri on the simulator.
- Same invariant macros (`turso_assert!`) compile to Antithesis assertions
  under `cfg(antithesis)` and plain asserts otherwise.

---

## 3. Adoption plan

### Tier 1 — quick wins (CI/config only, no code churn) — **SHIPPED 2026-07-07**

1. ✅ **Feature-matrix clippy** (C4): `check` job now runs workspace clippy on
   default and `--no-default-features` after the all-features pass;
   `feature-hygiene` (cargo-hack `--each-feature`, 108 configs) was already
   in place for type errors.
2. ✅ **Rustdoc gate** (C5): was already in place (`doc` job,
   `RUSTDOCFLAGS=-D warnings`, `--document-private-items`).
3. ✅ **Dep hygiene** (C6): `#![cfg_attr(not(test), warn(unused_crate_dependencies))]`
   in all 36 lib.rs. Adoption immediately exposed **15 dead dependencies**
   (incl. the whole never-implemented `redis`/`s3` storage features, which
   machete had been told to ignore) — all purged. Feature-conditional deps
   (`loom`, `smallvec`, `uuid`) became properly optional or cfg-anchored.
   Plus reth's no-test-deps-in-release `cargo tree` gate (`test-dep-leak` job).
4. ✅ **`cargo-check-external-types`** on `sdk` and `api` with per-crate
   allowlists (`external-types` job, nightly-2026-03-20 ↔ tool 0.5.0).
5. ✅ **Aggregator + workflow security**: aggregator (`required` job) and
   SHA-pinned actions were already in place; **zizmor** gate added at
   high severity — its 4 pre-existing high findings (workflow-level
   `actions:/issues:/id-token: write` permissions, template injection in
   `test-matrix.yml`) fixed. 24 medium findings (persist-credentials on
   checkouts) tracked as follow-up.
6. ✅ `.config/nextest.toml`: was already in place (trybuild serial group,
   ci/agent/chaos profiles).
7. ✅ **Async-footgun lints** (C14): `or_fun_call`, `assigning_clones`,
   `large_futures`, `await_holding_lock` now warn; 13 call sites fixed;
   `large_futures`/`await_holding_lock` had zero hits — the architecture
   was already clean.

### Tier 2 — medium (a week of focused work each)

8. **Architecture Invariants**: keep the root list in `AGENTS.md` curated;
   add an `## Invariants` section to each crate's `AGENTS.md` stating what
   the crate deliberately does NOT do; tag API-boundary crates.
9. **Snapshot testing**: adopt `insta` for engine outputs / API responses /
   error renderings with uv-style redaction filters and a frozen clock in
   the test context; adopt `cov_mark` for branch-tied tests in `engine`
   and `resilience`.
10. **Formalize the test-context crate** (C8): one `nebula-test-context` with
    hermetic env, DB bootstrap, and filter presets, replacing ad-hoc helpers.
11. **Node telemetry spec + compliance harness** (vector): write an RFC-2119
    spec of events/metrics every action/node MUST emit
    (`node_received_items_total`, errors with bounded `error_type` + stage);
    build the harness that runs a real node and asserts the required
    telemetry fired. Extends the existing QUALITY_GATES instinct to runtime
    behavior.
12. **Error codes** (restate `codederror`): stable public error codes
    (`NEB0042`) on `nebula-error` variants + a markdown doc per code;
    transient-vs-terminal as an explicit method, user-code errors as a
    distinct type from system errors.
13. **Fuzz the expression language** (wasmtime layout): a fuzzer-agnostic
    generator/oracle crate + thin libFuzzer targets for
    `nebula-expression` (lexer/parser/eval); minimized findings land as
    committed regression tests with issue URLs.
14. **Migration-guide gate** (bevy): every breaking PR adds a
    `docs/migration-guides/` entry (frontmatter + what/why/how), validated
    in CI — pairs with the ongoing public-surface curation.

### Tier 3 — strategic (design work, separate ADRs)

15. **Saga-grade engine review** (omicron + restate): audit workflow actions
    for idempotent undo coverage; serialize auth context into durable
    workflow params; narrow the execution-context trait surface; add
    **fencing tokens** to worker attempts so a superseded attempt's effects
    are rejected; journal side-effect results and replay from the journal.
16. **API-first flip** (omicron): define `nebula-api` endpoints as a trait,
    commit generated OpenAPI, generate the SDK client from it (progenitor or
    equivalent), with explicit endpoint version ranges.
17. **Schema-version guard** (omicron + restate): pin the DB schema version
    in Rust and refuse startup on mismatch; tag every durable record with a
    codec-kind byte; never reuse retired discriminants; evolve via parallel
    `_v2` modules while in-flight workflows stay on their pinned version.
18. **Deterministic simulation testing for the engine** (turso): route the
    engine's IO/clock/RNG through a pump-able trait so a seeded in-memory
    backend makes runs reproducible; express workflow correctness as a
    `Property` enum (no lost steps, idempotent resume, undo completeness);
    shrink + persist failures in a bug base; seeded sim loop per-PR, faults +
    (eventually) Antithesis nightly. Builds on the existing
    `storage-loom-probe` instinct.

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
