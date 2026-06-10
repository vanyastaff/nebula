---
name: nebula-layer-boundaries
description: Use when adding a crate dependency, hitting a cargo-deny wrappers error, deciding where code lives, or crossing layers / calling a sibling crate.
---

# Nebula layer boundaries

The layer map in `CLAUDE.md` is mechanically enforced by `cargo deny check`
against `deny.toml`'s `[bans]` `deny = [...]` `wrappers` allowlists. A new
`use nebula_*` edge that violates a layer is a **CI failure**, not a warning.
Load this before introducing any cross-crate edge or moving code between crates.

## The 6 layers — depend DOWNWARD only

From `CLAUDE.md` "Layered Dependency Map". Each layer may depend only on layers
*below* it. An upward edge (e.g. Core depending on Business) is a violation.

| Layer | Crates |
|-------|--------|
| API / Public | `api`, `sdk` |
| Exec | `engine`, `storage`, `storage-loom-probe` |
| Business | `credential-builtin`, `resource`, `action`, `plugin`, `tenancy` |
| Plugin-Proto | `plugin-sdk`, `sandbox` |
| Core | `core`, `validator`, `expression`, `workflow`, `execution`, `schema`, `metadata`, `storage-port` |
| Cross-cutting | `log`, `eventbus`, `metrics`, `resilience`, `error`, `env` |

- **Plugin-Proto** is a leaf tier between Core and Business. `plugin-sdk` /
  `sandbox` depend only on Core (+ tokio/serde); `plugin` (Business) and
  `engine` (Exec) depend on them *downward*. The `SandboxRunner` runner
  abstraction lives in `engine`; discovery + the `SandboxError`→`ActionError`
  seam live in `plugin`.
- Each `+macros` companion (`action/macros`, `credential/macros`,
  `error/macros`, `plugin/macros`, `resource/macros`, `schema/macros`,
  `validator/macros`, `sdk/macros-support`) sits at the **same layer as its
  parent** and ships derives only.
- The full member list is `Cargo.toml` `[workspace.members]`; every crate opts
  into the shared lint policy with `[lints] workspace = true`.

## Cross-cutting crates are importable at ANY level

`log`, `eventbus`, `metrics`, `resilience`, `error`, `env` are NOT a bottom
layer you must stay above — they are importable from any tier. A Core crate
depending on `nebula-error` or `nebula-eventbus` is **not** a layer violation
and these crates carry no `wrappers` entry in `deny.toml`.

## Shared-infra exceptions (NOT single-tier) — each has a locked allowlist

Four crates are deliberately consumed across multiple tiers. They are not
"Business-row" crates; each has an explicit `{ crate = ..., wrappers = [...] }`
allowlist in `deny.toml` that **locks the exact consumer set** so a future PR
cannot quietly add an upward edge.

- **`nebula-credential`** (credential *contract*) — `deny.toml` lines ~186-213.
  Consumed by Business (`action`, `plugin`, `resource`, `tenancy`), Exec
  (`engine`, `storage`, `credential-runtime`), API (`api`), and the first-party
  backends (`credential-builtin`, `credential-vault`).
  Plugin authors depend on this contract crate, **not** on
  `nebula-credential-builtin` (whose wrappers list is intentionally narrow:
  itself + `credential-runtime`).
- **`nebula-storage-port`** (Core storage seam, ADR-0072 §2.2) — `deny.toml`
  lines ~125-133. Object-safe repository traits, port-local DTO rows, the
  plain-data `Scope { workspace_id, org_id }`, `StorageError`,
  `TransitionBatch`. **No sqlx, no upward deps.** Broadly importable: the
  adapter (`storage`), loom probe, tenancy decorator, `engine`/`api`,
  `credential-runtime`, plus a `credential-vault` dev-dep.
  Distinguish from **`nebula-storage`** (Exec) — the sole adapter impl
  (InMemory + SQLite + Postgres, sqlx, migrations); its wrappers list is the
  composition seam (`engine`, `api`, `server`, + dev-deps). The legacy
  `ExecutionRepo`/`WorkflowRepo` surface was deleted per ADR-0072.
- **`nebula-resource`** (shared runtime framework, ADR-0081 SF-1) — `deny.toml`
  lines ~170-176. Consumers locked to `action`, `engine`, `examples`,
  `plugin`, `sdk` — no upward deps from API or core/*.
- **`nebula-expression`** (Core evaluator, ADR-0043 §9) — `deny.toml`
  lines ~222-231. Consumers locked to `schema`, `resource`, `engine`,
  `examples`, + an `action` dev-dep. (`nebula-credential-runtime` is similarly
  Exec-tier shared infra with its own allowlist; see lines ~283-287.)

## Reading & editing a `{ crate, wrappers = [...] }` entry

A `deny.toml` `[bans]` `deny` entry of the form
`{ crate = "nebula-X", wrappers = [ ... ], reason = "..." }` means: **only the
crates named in `wrappers` may have a direct dependency on `nebula-X`.** Any
other crate that adds `nebula-X` to its `Cargo.toml` fails `cargo deny check`.

To legally add a new consumer:

1. Add the consumer crate name to the **target's** `wrappers` array (e.g. to
   depend on `nebula-storage-port` from a new crate, add it to the
   `nebula-storage-port` entry — NOT to a `nebula-storage` entry).
2. Add an inline `#` comment on the new line explaining **why** the edge is
   legal and which direction it points (every existing entry does this — see
   the `credential-runtime` / knife / conformance dev-dep comments). Never widen
   silently.
3. **Dev-dep carve-outs are normal and expected** — integration tests and
   conformance matrices legitimately reach across boundaries (e.g.
   `nebula-tenancy` as a dev-dep of `nebula-storage` for the scoped conformance
   matrix; `nebula-engine` as a dev-dep of `nebula-api` for the §13 knife test).
   Mark them as dev-only in the comment. cargo-deny has **no feature-aware /
   dev-dep-aware wrappers**, so the allowlist is the only structural gate —
   `test-util`-gated deps still need an explicit entry.
4. If an edge would be **upward** (lower layer → higher layer), do not add it.
   Restructure instead: move the shared code down, or route through eventbus.

## Sibling-to-sibling goes through eventbus, never a direct import

`CLAUDE.md` Agent Rules: "Cross-crate communication goes through
`nebula-eventbus` — never reach across layer boundaries with direct imports."
Two crates at the **same** layer (e.g. two Business crates) must not import each
other directly. Publish/subscribe through `nebula-eventbus` (cross-cutting, so
importable from anywhere). `nebula-eventbus` is stable and already used by
`engine` for `CredentialEvent`.

## Decision recipe: "where does this helper / type go?"

Placement is a **boundary decision, not a convenience** ("one small helper in
the wrong crate" compounds).

1. Enumerate every crate that will consume the new code.
2. Find the **lowest layer all consumers can reach downward**. Put the code
   there. A type used by both `engine` (Exec) and `action` (Business) belongs in
   Core or a shared-infra crate, never duplicated in each.
3. If consumers span tiers and no existing crate fits, it is shared infra —
   add a new `wrappers` allowlist entry (see above), do not smear it upward.
4. If it is a pure error/event/metric/config concern, it belongs in the
   matching cross-cutting crate (`error` / `eventbus` / `metrics` / `env`).
5. Never add an upward edge to make a helper reachable. Push the helper down.

## Verify before committing

1. Run `cargo deny check` (or `task deny`) — it checks layer wrappers +
   advisories + licenses. This is a CI required job; a wrapper failure blocks
   the merge.
2. `task deny` is also part of `task dev:check` (the pre-PR gate) and the
   lefthook pre-commit `deny` step.
3. On **any** dep add/change, stage the **root `Cargo.lock`** too
   (`git add Cargo.lock`) — a narrow `git add crates/<name>` alone breaks the
   per-commit `--locked` build.
4. If a wrapper edit and a code edit land together, both go in the same commit
   so each commit stays green.
