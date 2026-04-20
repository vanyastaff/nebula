---
id: 0010
title: rust-2024-edition
status: superseded
date: 2026-04-19
supersedes: []
superseded_by: [0019]
tags: [toolchain, msrv, edition, workspace]
related:
  - Cargo.toml
  - rust-toolchain.toml
  - CLAUDE.md
  - docs/PRODUCT_CANON.md
linear:
  - NEB-148
---

# 0010. Rust 2024 edition + MSRV 1.94

## Context

Nebula is a long-horizon workflow engine aiming for v1.0 by Q2 2027. The
toolchain choice has to survive two-plus years of active development without
locking us out of language features we already rely on, while still being
reachable by users who track stable.

Two decisions are intertwined and therefore recorded together:

1. **Language edition.** Rust 2024 is the current stable edition. It tightens
   disjoint captures in closures, hardens `unsafe_op_in_unsafe_fn`, and makes
   several lints warn-by-default that Nebula already enforces. Staying on 2021
   would require us to suppress these in code we actively want strict.
2. **MSRV (`rust-version`).** Nebula uses stable language features that landed
   across 2024 and the 2024 edition itself (stabilized in 1.85). Several
   crates in the workspace depend on `async fn` in traits, `let ... else`,
   `gen` blocks (where applicable), and `#[diagnostic::on_unimplemented]` —
   features with MSRV ≥ 1.75..=1.90. We pick **1.94** (current stable at the
   time of writing) so all workspace crates share a single floor that is easy
   to reason about, mirrors what CI runs, and gives first-party plugin authors
   a predictable baseline.

Contributor tooling (`rust-toolchain.toml`, `rustfmt.toml` with nightly-only
options) already assumes 2024 and a recent rustc. This ADR documents the
decision that is de-facto in force.

## Decision

1. Every workspace member uses `edition = "2024"` via
   `[workspace.package]` in the root `Cargo.toml`. Per-crate overrides are
   **not allowed** — the workspace is single-edition to keep cross-crate
   macros and trait impls uniform.
2. Workspace MSRV is **`rust-version = "1.94"`**, pinned once in
   `[workspace.package]`.
3. CI enforces the MSRV on every PR (`rust-toolchain: 1.94` matrix entry plus
   a stable channel entry). The local `lefthook` pre-push mirrors the CI MSRV
   job so divergence is caught before CI.
4. A new dependency that effectively raises the compiler floor above 1.94
   must fail the dedicated 1.94 CI/MSRV job (and the matching local
   `lefthook` pre-push check) loudly rather than being accepted silently.
   (`deny.toml` does not currently encode an MSRV gate; if one is added
   later, it becomes the primary enforcement point.)
5. **Bumping the MSRV is a breaking change.** Any raise requires:
   - an update to this ADR (supersede, not edit in place);
   - a CHANGELOG entry flagged `breaking`;
   - CI matrix update in the same PR.

## Consequences

**Positive**

- One edition, one MSRV, one story. Plugin authors can read a single line in
  `Cargo.toml` and know what they are targeting.
- Rust 2024's disjoint closure captures and `unsafe_op_in_unsafe_fn`
  let us delete a pile of older `#[allow(...)]` workarounds (see
  `crates/resilience`, `crates/engine`).
- Keeping MSRV on the current stable lets us use language features as they
  land instead of waiting a year, which matters on a two-year roadmap.

**Negative**

- Users on long-term distro rustc (e.g. Debian stable) cannot `cargo install
  nebula` directly — they need `rustup`. Documented in
  `docs/dev-setup.md`.
- Any crate that wants to depend on `nebula-sdk` inherits the 1.94 floor;
  external plugin authors cannot be on older stable.

**Neutral**

- Nightly is still allowed for tooling (`cargo +nightly fmt`) because
  `rustfmt.toml` uses unstable options. This is tooling-only and does not
  affect MSRV of the compiled crates.

## Alternatives considered

- **Rust 2021 + MSRV = N-3.** Reject. Too many clippy lints fire under the
  older edition, and we would keep bumping the floor anyway as we adopt new
  features.
- **Pin nightly for the whole workspace.** Reject. Breaks `cargo-deny`
  version gates, breaks `docs.rs`, breaks our goal of being usable from
  stable Rust.
- **Per-crate MSRV.** Reject. Creates a matrix where `cargo-deny` cannot
  give a single "MSRV clean" answer and where the CI matrix balloons.

## Follow-ups

- `NEB-137` — `ci.yml` update to pin Rust 1.94 across all jobs.
- `docs/dev-setup.md` — link to this ADR from the toolchain section.
- Future: when Rust 1.94 leaves stable by ≥ 6 months, open a new ADR raising
  the floor (never edit this one).
