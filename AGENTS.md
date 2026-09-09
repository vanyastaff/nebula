# AGENTS.md

> **Canonical agent guide for Nebula.** `CLAUDE.md` is a thin pointer here.
> Product and architecture prose lives in `README.md` and `docs/` — this file
> carries what an agent can't infer from the code: layer rules, invariants,
> repo-specific gotchas, and the commands that aren't guessable.

Read this, then `crates/<crate>/AGENTS.md` for the crate you're touching. Each follows the
same shape — **Purpose / Layer** header, **Key files**, **Conventions & never-do** (that
crate's real traps and what it deliberately does *not* do), **See also**, plus
**Commands** only where the crate needs something beyond `cargo check -p <name>` /
`cargo nextest run -p <name>`. **Change checks** maps affected contracts to relevant tests;
it is a starting point, not an exhaustive gate or a claim that those tests passed.
Run package commands from the workspace root unless stated otherwise. These guides add
only crate-specific rules; everything in this file applies everywhere.

`orchestrator`, `worker`, `plugin-core`, `apps/server`, and `apps/worker` have no
`AGENTS.md` yet — read their `README.md` where present, otherwise the crate-root docs in
`src/lib.rs` or `src/main.rs`, plus the invariants below.

---

## Orientation

| You're doing | Read |
|---|---|
| Anything in a crate | `crates/<crate>/AGENTS.md`, then its `README.md` |
| Adding a cross-crate dependency | §Layered Dependency Map below, then `deny.toml` |
| Connecting crates (Resource, Credential, Action, Schema, Plugin) | `docs/INTEGRATION_MODEL.md` |
| Anything touching durability or credentials | `docs/PRODUCT_CANON.md` (binding invariants) |
| Touching a hot path | `docs/pitfalls.md` |
| Looking for a doc | `docs/README.md` (doc map) |

Design records — ADRs, roadmap, specs, research — are **not tracked in this repo**.
They live in the maintainers' private Obsidian vault, reachable via the `obsidian` MCP
under `projects/nebula/`. Run `/recall <area>` before working a known area so prior
decisions carry forward, and `/remember` after a non-obvious decision, gotcha, or fix.
If the MCP is absent, skip vault access. If the host provides local project memory,
recall relevant notes there and verify them against current files before relying on them.
Local memory is not a substitute for the private design vault or binding canon.

**Tooling.** Serena is configured (`.mcp.json`) — prefer its symbolic tools for
definition lookup, reference search, and renames; `rename_symbol` over grep+replace.
Also wired: `rust-analyzer-mcp` (hover, diagnostics, code actions), `rust-mcp-server`
(cargo check/clippy/deny/test runners), `rust-docs`, `cratesio`. Modern CLI equivalents
(`rg`, `fd`, `bat`, `eza`, `sd`, `jq`, `yq`, `delta`, `dust`, `procs`) are installed.
Install cargo tools with `cargo binstall`, not `cargo install`.
Configuration is not proof of availability: use tools exposed in the current session.
If an MCP is unavailable, use scoped `rg` and CLI checks; report any evidence you could
not retrieve instead of inventing tool results.

## Agent Work Loop

- **Start with the outcome.** For non-trivial work, state the observable result,
  constraints, and how it will be verified. Infer routine details from the request and
  code; preserve later user corrections alongside the original requirements. Ask only
  when a missing decision materially changes scope or correctness.
- **Inspect before editing.** Check the current branch and `git status --short`, read
  the applicable guides and surrounding implementation, and preserve existing edits.
  Keep changes scoped, but finish required cross-crate consequences.
- **Continue within authorization.** Make reversible implementation choices and state
  consequential assumptions. Before an unapproved destructive operation, publication,
  deployment, paid service, or access change, finish the safe preparation and request
  approval for the concrete action. Existing user authorization still applies.
- **Protect context and secrets.** Do not dump environment variables, real `.env` files,
  credentials, or keys into tool output or notes. Use public templates and redacted
  diagnostics. Source modules named `secrets` or `credentials` are ordinary code.
- **Parallel work needs ownership.** When delegation is requested or required by an
  applicable skill, assign independent responsibilities and explicit file ownership.
  Workers must preserve others' edits. Follow the Git Workflow for persistent branches;
  the coordinator owns integration and verification of the combined diff.
- **Done requires evidence.** Compare the result with the user's request and corrections,
  not only the implementation plan. Review the actual diff, run the applicable checks
  below, and report results for the final revision. Distinguish passed, failed, and not run;
  explain missing prerequisites. A plan, a started command, or a worker's summary is
  not a passing check. Do not weaken gates to obtain a green result.

For long tasks or a handoff, use [Agent workflow templates](docs/AGENT_WORKFLOW.md).
Keep task state in the conversation or local ignored notes, not in this guide.

---

## Tech Stack

Rust 1.97+, edition 2024, resolver 3 · Tokio · `thiserror` in libs / `anyhow` in bins ·
PostgreSQL + SQLite (`crates/storage/migrations/`) · `cargo nextest` + doctests ·
`Taskfile.yml` for tasks (`task --list`) · `lefthook.yml` mirrors CI required jobs.

---

## Commands

| Command | Purpose |
|---|---|
| `cargo check -p nebula-<name>` | Fastest feedback after an edit |
| `cargo nextest run -p nebula-<name> [<test>]` | Tests for one crate, or one test |
| `task dev:check` | **Pre-PR gate:** fmt + clippy + nextest + doctests + deny |
| `task quality` | Quick gate: fmt:check + clippy |
| `task deny` | Layer wrappers + advisories + licenses |
| `task ci` | Full CI pipeline locally |
| `task bench:crate CRATE=<name>` | Benchmarks |
| `task db:up`, then `task db:migrate` | Local Postgres + admitted migration operator |
| `task obs:up` / `obs:down` | Jaeger + OTEL collector |
| `cargo xtask ci-plan full` | Versioned full CI package plan |
| `cargo xtask ci-plan diff --base <sha> --head <sha> --comparison merge-base` | Metadata-driven diff plan |

Run chained shell steps separately, not `&&`-joined — one clear pass/fail per step.
Preserve the check's exit status when shortening output: a pipeline ending in `tail`
or `tee` needs `pipefail`; otherwise a failed check can report success.

---

## Workspace Layout

```text
nebula/
├── Cargo.toml          # workspace members + pinned deps + [workspace.lints]
├── Taskfile.yml        # task runner; `task dev:check` = pre-PR gate
├── deny.toml           # cargo-deny: layer wrappers, licenses, advisories (CI gate)
├── lefthook.yml        # pre-commit / pre-push, mirrors CI
├── clippy.toml         # lint thresholds (msrv 1.97)
├── rustfmt.toml        # stable-only, pinned toolchain
├── .mcp.json           # MCP servers
├── crates/             # workspace members
├── apps/               # first-party deployment composition roots
├── examples/           # all runnable examples (workspace member)
├── tools/xtask/        # metadata-driven repo automation + contract tests
├── scripts/            # worktree.sh + lefthook helpers
└── .github/workflows/  # CI required jobs (ci.yml)
```

Per crate: `Cargo.toml`, `README.md`, `AGENTS.md`; some carry a sibling derive crate
(`<name>/macros`) and/or a `docs/` folder.

---

## Layered Dependency Map

**Mechanically enforced** by `cargo deny check` against `deny.toml` `[bans].deny`
wrappers. Each layer depends only on layers below. Direct imports of lower-layer domain
types and ports are normal; upward dependencies and undeclared lateral coupling are CI
failures.

| Layer | Crates |
|-------|--------|
| **API / Surfaces** | `api`, `sdk` |
| **Exec** | `engine`, `orchestrator`, `worker`, `storage`, `storage-loom-probe` |
| **Business** | `resource`, `action`, `plugin`, `plugin-core`, `tenancy` |
| **Core / shared-infra** | `core`, `validator`, `expression`, `workflow`, `execution`, `schema`, `metadata`, `storage-port`, `credential` |
| **Cross-cutting** | `crypto`, `log`, `eventbus`, `metrics`, `resilience`, `error`, `env` |

`nebula-xtask` is repository tooling, not a product crate or architectural layer. It may
depend on general-purpose tooling libraries but never on a `nebula-*` product package.

### Architecture Invariants

Each states what holds — or is *deliberately absent* — everywhere. Violating one is an
architecture change, not a refactor.

- **Durable commands and business facts cross crate boundaries through persisted state
  or explicit outbox/inbox ports.** Direct downward dependencies on domain types and
  ports are normal. `nebula-eventbus` carries only ephemeral observations (telemetry,
  cache/UI invalidation, wake hints); consumers must tolerate loss, duplication, and
  reordering, and it is never a source of truth — never use it to bypass the layer map.
- **`nebula-storage-port` (Core) is the object-safe storage seam** — it contains no
  backend code and never will.
- **`nebula-storage` (Exec) is the sole persistence-backend implementation.** SQLite and
  Postgres are deployment backends; InMemory is an internal test/reference/conformance
  adapter, not a supported deployment backend. Policy decorators such as `nebula-tenancy`
  may wrap the port, but no other crate implements a persistence backend.
- **`nebula-credential` is shared infra** importable from Exec, Business, and API tiers;
  secrets never appear in error messages (`SecretFreeMessage`) or `Debug` output.
- **Durable write authority is aggregate-scoped.** Runtime control owns the execution
  aggregate, execution journal and queues, execution outbox/inbox, and operation ledger;
  credential runtime owns credential/refresh/lease state; resource lifecycle owns
  resource/binding/fan-out state. Cross-aggregate commands and facts use durable
  persisted seams; `nebula-eventbus` may only wake or observe their owners.
- **Every first-party deployment composition root lives under `apps/`.** `nebula-worker`
  (Exec) is reusable assembly for exact-flavor control consumption and accepted-turn
  recovery; `apps/worker` selects concrete adapters, configuration, and process lifecycle.
  A downstream host becomes a supported composition root only through the
  curated `nebula_sdk::embedded::RuntimeBuilder`; until that façade ships, downstream
  embedding is not a supported deployment surface. It cannot replace or bypass aggregate
  ownership, admission, or tenant authority.
- **Plugins are statically linked, trusted in-process adapters** (ADR-0091); WASM/process
  isolation is a non-goal (canon §12.6). `nebula-plugin-core` (Business) is the
  first-party `core` plugin built on `action`/`plugin`.
- **Each `+macros` companion lives at the same layer as its parent** and ships derives
  only — no runtime code.
- **CI package selection comes only from Cargo metadata** through `cargo xtask ci-plan`;
  workflow and hook scripts consume its versioned JSON and do not maintain
  package-selection name lists or path-to-crate inference. The pre-push names
  `nebula-resilience`, `nebula-log`, `nebula-expression`, `nebula-credential`,
  `nebula-resource`, and `nebula-storage` form an independent no-default-feature
  gate-policy list applied only after selection; they never decide matrix membership.
- **`sdk` is the sole supported and branded Rust surface**, organized by persona:
  workflow/authoring, integration, schema, testing, client, and embedded façades. The
  curated client submits versioned transport requests; the curated embedded façade
  submits typed runtime commands. Neither exposes raw stores, mutation/admission
  capabilities, claim tokens, or tenant proofs. The HTTP API contract and all
  implementation crates remain technical boundaries, not separately supported Rust
  products. Required internal packages may be published as exact-version, lockstep
  dependencies of `nebula-sdk`, but direct use is unsupported (private ADR-0117).

---

## Conventions & Gotchas

Repo-specific things that bite. General Rust taste — naming, comment density, module
shape — should match the surrounding code.

- **`#[expect(lint, reason = "...")]`, never bare `#[allow]`** — the `allow_attributes`
  lint enforces it. If the lint fires only under some feature/cfg, gate the expectation:
  `#[cfg_attr(not(feature = "x"), expect(...))]`. The one sanctioned `#[allow]` is inside
  exported `macro_rules!` bodies, where expansions land in downstream crates that can't
  fulfil an expectation — there use the self-suppressing form
  `#[allow(<lint>, clippy::allow_attributes, reason = "...")]`.
- **No `unwrap()`/`expect()`/`panic!()` in library code.** Tests, `const`, and binaries
  are exempt. Propagate with `?` and a typed `thiserror` variant.
- **Verify the feature matrix for crates with features** (`log`, `credential`,
  `resource`, `storage`): run clippy on default *and* `--no-default-features`, not just
  `--all-features` — `cfg_attr(not(feature = ...))` code is invisible to an all-features
  pass.
- **Ship observability with every new state, error, or hot path** — typed error variant +
  tracing span + invariant check, in the same change.
- **No TODO/FIXME/HACK in committed code.** If something must land temporarily, mark it
  `// guard-justified: <reason>`.
- **Don't weaken a test while changing the implementation it covers**, in the same turn.
- **Runnable examples go in the root `examples/` workspace member**, not per-crate dirs.
- **Pin lockfile changes**: `cargo update -p <crate> --precise <ver>`, never a wholesale
  `cargo update`.
- **Don't read `target/`, `.worktrees/`, or `.claude/worktrees/`** — denied in
  `.claude/settings.json`.

---

## Git Workflow

All persistent branches go through `scripts/worktree.sh` (or `task wt:*` wrappers).

| Step | Command |
|---|---|
| New branch | `bash scripts/worktree.sh new <slug> <type> <scope>` |
| List | `bash scripts/worktree.sh list` |
| Commit | `bash scripts/worktree.sh commit <type> <scope> <summary>` |
| Finish | `bash scripts/worktree.sh finish <slug>` |

Conventional Commits, validated by `convco`. **Types:** `build`, `chore`, `ci`, `docs`,
`feat`, `fix`, `perf`, `refactor`, `revert`, `style`, `test`. **Scope:** crate name
without the `nebula-` prefix (`resilience`, `engine`, `api`) or a top-level area
(`docs`, `ci`).

Branch from `main`, squash-merge to `main`, never force-push shared history. Don't use
`git commit --no-verify` or `git push --force` without explicit user confirmation.

---

## Enforced Discipline

`lefthook.yml` runs locally and mirrors CI required jobs.

- **pre-commit:** `fmt-check` (per-crate rustfmt) · `clippy` (per-crate) · `typos` ·
  `taplo` (TOML fmt) · `cargo-deny` (layer wrappers + advisories)
- **commit-msg:** `convco`
- **pre-push:** `clippy-full` (workspace `-D warnings`, skipped when the push range has
  no `.rs`) · `crate-diff-gate` (nextest for changed crates)

Not hook-enforced — CI catches these, so check them yourself: no `unwrap()`/`expect()`/
`panic!()` in libs, no TODO/FIXME/HACK, no test weakening. (`.claude/hooks/` contains
guard scripts; verify their registration in the active host and their execution before
relying on them. A script on disk is not an enforced gate.)

---

## Error Triage

1. **Layer violation (cargo-deny)** → the crate you're importing is in a higher layer or
   is undeclared lateral coupling. Depend on a lower-layer domain/port crate, move the
   shared contract down, or route the durable command through its owning persisted port.
   Don't route around it with `nebula-eventbus`.
2. **`unfulfilled_lint_expectations`** → the `#[expect]`ed lint no longer fires. Either
   the suppression is stale (delete it) or it's config-dependent (gate with `cfg_attr` to
   the configs where it fires — remember lib and lib-test compile the same source twice,
   so test-only items need `cfg_attr(not(test), ...)`).
3. **`clippy::allow_attributes`** → convert `#[allow]` to `#[expect(..., reason)]`; see
   the `macro_rules!` exception in §Conventions.
4. **`convco` rejection** → commit message isn't `type(scope): summary`.
5. **Clippy warning** → `task clippy` for the workspace view. Fix it; don't suppress it.
