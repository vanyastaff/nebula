---
id: 0013
title: compile-time-modes
status: accepted
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [build, cargo-features, deployment, workspace, packaging]
related:
  - deploy/STACKS.md
  - Cargo.toml
  - docs/PRODUCT_CANON.md#123-local-path
linear:
  - NEB-151
---

# 0013. Compile-time deployment modes

## Context

Nebula ships **one codebase, three deployment shapes** (per `deploy/STACKS.md`):

- **Local / desktop** — single process, SQLite by default, no external brokers.
- **Self-hosted** — `nebula-api` + embedded worker loop, Postgres + optional
  Redis via Docker compose.
- **Cloud / SaaS** — multi-tenant, Postgres, Redis, long-lived workers.

Each mode has different assumptions about storage, brokers, background
workers, and tenant boundaries. Some choices (e.g. SQLite vs Postgres
driver, in-process dispatch vs `LISTEN/NOTIFY`, tenancy middleware) must
be resolved **at build time** because:

1. Pulling SQLite into a cloud binary is dead code.
2. Pulling Postgres into a desktop binary blocks the "no Docker required"
   local path (§12.3).
3. A runtime `DeploymentMode` enum forces every call site to handle three
   branches forever; the canon (§0 — Non-goals) rules it out.

Three existing constraints make this decision urgent enough to record:

- **§12.3 Local path** mandates that the default developer experience runs
  without Docker or external brokers — the desktop shape must not transitively
  depend on `tokio-postgres`/`deadpool-postgres`.
- **§12.2 Durable control plane** requires **one** consumer wiring per
  deployment mode, documented in code. That wiring cannot be chosen
  "dynamically at startup" without proliferating branches across crates.
- **Non-goal** (project scope): `DeploymentMode` as a runtime enum.

This ADR records the decision to select mode at compile time via cargo
features, with a `build.rs` mutual-exclusivity gate, before any crate
starts coding against the choice.

## Decision

1. **Deployment mode is a build-time selection, exposed as cargo features
   on the top-level binary crates** (`apps/cli`, `apps/desktop`,
   `crates/api`):
   - `mode-desktop` — SQLite storage, in-process control-queue consumer,
     single-tenant, no Redis.
   - `mode-self-hosted` — Postgres storage, `LISTEN/NOTIFY` consumer,
     single-tenant, optional Redis.
   - `mode-cloud` — Postgres storage, `LISTEN/NOTIFY` consumer,
     multi-tenant middleware, Redis required.

2. **Exactly one mode feature must be active.** A `build.rs` in each
   binary crate asserts mutual exclusivity:

   ```rust
   // apps/<binary>/build.rs
   fn main() {
       let modes = [
           cfg!(feature = "mode-desktop"),
           cfg!(feature = "mode-self-hosted"),
           cfg!(feature = "mode-cloud"),
       ];
       let active = modes.iter().filter(|x| **x).count();
       assert!(
           active == 1,
           "exactly one of mode-desktop/mode-self-hosted/mode-cloud \
            must be selected (got {active})"
       );
   }
   ```

   A zero-mode or two-mode build fails **at compile time**, not at runtime.

3. **Library crates do not carry mode features.** Crates below the binary
   layer (`core`, `engine`, `storage`, `credential`, …) expose capability
   features (`postgres`, `sqlite`, `redis`, `multi-tenant`) and let the
   binary compose them. A library that depends on "which mode am I in"
   is an architectural smell — it should depend on the capability instead.

4. **No runtime `DeploymentMode` enum.** The decision is encoded in the
   dependency graph and in `#[cfg(feature = "...")]` gates. Call sites
   do not match on a mode at runtime; they either have a Postgres pool
   in scope or they have a SQLite one.

5. **`deploy/STACKS.md` is the human-facing description of the three
   shapes** (what they include, how to run them). This ADR is the
   normative build-system contract. The two must stay in sync; any
   change to the list of modes lands as a new ADR.

## Consequences

**Positive**

- Unused drivers and middleware never reach the final binary — desktop
  builds stay slim, cloud builds do not carry SQLite.
- Call-site complexity does not balloon over time; there is no
  `if mode == Desktop` pattern to sprawl across crates.
- Mode-specific behavior lives at the dependency boundary and is
  greppable by feature name.

**Negative**

- CI must build all three modes to catch feature-gating bugs. Add a
  `build-modes` matrix job (desktop × self-hosted × cloud × OS).
  See "Follow-ups".
- Cross-mode code sharing still requires careful feature composition;
  rushed `#[cfg(feature = ...)]` without a capability abstraction can
  produce "Frankenstein" builds that compile only under specific
  combinations.

**Neutral**

- `deploy/STACKS.md` continues to describe runtime deployment (how to
  start a stack); this ADR covers how the binary is *built*.

## Alternatives considered

- **Single universal binary with runtime `DeploymentMode` enum.**
  Reject. Forces every dependency into every build, violates §12.3
  (local path stays lean), and spreads branch logic across crates
  indefinitely.
- **Separate git repos per mode.** Reject. Defeats the
  "one codebase, three shapes" positioning and doubles the maintenance
  cost of shared crates.
- **`default-run = "..."`-style switch without feature gates.** Reject.
  Doesn't stop the wrong drivers from being compiled in; not a build-time
  decision in any meaningful sense.
- **One binary per mode with fully duplicated `main.rs`.** Reject.
  Deduplication of glue code produces reusable modules that end up
  needing the same feature gates — this decision just records them.

## Follow-ups

- Add `build.rs` with the assertion above to `apps/cli`, `apps/desktop/src-tauri`,
  and `crates/api` (binary targets only).
- Extend the CI test matrix in `.github/workflows/test-matrix.yml` with
  a `build-modes` dimension (`mode-desktop | mode-self-hosted | mode-cloud`)
  so feature-gating regressions are caught before merge.
- When the first cross-cutting code path needs to vary per mode, open a
  follow-up ADR (or capability sub-ADR) that specifies *the boundary
  type* — do not scatter `#[cfg(feature = ...)]` in high-traffic
  functions without design review.
- `deploy/STACKS.md` link note: each mode section should name the cargo
  feature it expects.
