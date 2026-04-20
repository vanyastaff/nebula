---
id: 0021
title: crate-publication-policy
status: proposed
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [workspace, packaging, release, semver, crates-io]
related:
  - docs/audit/2026-04-19-codebase-quality-audit.md
  - Cargo.toml
  - docs/MATURITY.md
  - docs/adr/0013-compile-time-modes.md
  - docs/adr/0020-library-first-gtm.md
  - deny.toml
linear: []
---

# 0021. Crate publication policy

## Context

The Nebula workspace has **25 production crates** plus **8 proc-macro crates**
(see `Cargo.toml [workspace] members`), all sharing the workspace-inherited
`version = "0.1.0"`. The current workspace default for `publish` is
`true` — only `apps/cli`, `examples`, and `crates/sandbox` are explicitly
opted out today. A naive `cargo publish` sweep at 1.0 would push the entire
perimeter to crates.io under one coordinated release.

The rust-senior audit verdict in
[`docs/audit/2026-04-19-codebase-quality-audit.md`](../audit/2026-04-19-codebase-quality-audit.md)
called this out directly:

> **Library-first cheaper long-term — single SemVer gate vs 25; the
> worst-of-both combo is what we have today.**

A 1.0 with `publish = true` on every crate means:

- 25 separate SemVer contracts, each a docs.rs surface that must stay
  buildable at MSRV, free of broken intra-doc links, and consistent with
  `docs/MATURITY.md`.
- 25 separate bug-report inboxes, any of which can block a coordinated
  release-train.
- Every internal refactor (splitting a god-file, renaming a trait, folding
  `nebula-metadata` into `nebula-core` per P1 #11 of the audit) becomes a
  potential breaking change for external consumers we cannot name.

Only a handful of crates have genuine third-party consumer pressure today:

- `nebula-sdk` — the integration-author façade; `deny.toml` already carves
  it out as the only crate `nebula-examples` may depend on. Its entire
  purpose is external consumption.
- `nebula-core` — primitive types re-exported through `nebula-sdk`.
- `nebula-error` — shared error taxonomy (`STYLE.md §6`); every public
  `Result` surfaces it.
- `nebula-resilience` — standalone retry / bulkhead primitives with no
  upward deps; useful outside Nebula.
- `nebula-credential` — security-primitives surface flagged in the audit's
  `security-lead` section (the `KeyProvider` seam is an intentional public
  contract).

Everything else (`nebula-engine`, `nebula-runtime`, `nebula-storage`,
`nebula-sandbox`, `nebula-plugin-sdk`, the individual integration crates,
and their macros) currently has **zero external consumers** and reaches
the outside world only via `nebula-sdk` or via Nebula-owned binaries.
Publishing them advertises a public API we neither intend nor have the
budget to support.

The workspace default is the wrong default for our situation. This ADR
flips it.

## Decision

1. **`publish = false` is the workspace default.** Every crate's
   `Cargo.toml` must explicitly carry `publish = false` unless it meets
   the opt-in conditions below. Opt-in, not opt-out.

2. **Opt-in conditions.** A crate may set `publish = true` only when
   **one** of the following is documented:

   - **(a) ≥ 3 external consumers** outside the Nebula workspace, already
     shipping or committed within 6 months of the opt-in. "External"
     means *not* `nebula-*` and *not* an in-tree binary. Record the count
     and named consumers in a `# publish-justification:` comment block
     immediately above `publish = true` in that crate's `Cargo.toml`.
   - **(b) Dedicated ADR** justifying the public surface, the SemVer
     commitment, and the long-term maintenance plan. The ADR ID must be
     referenced in the same `# publish-justification:` comment
     (e.g. `# publish-justification: ADR-00NN`).

3. **Initial published set (this ADR serves as condition (b) for them).**

   | Crate | Justification |
   |---|---|
   | `nebula-sdk` | Integration-author façade; already carved out in `deny.toml` as the only crate `nebula-examples` may depend on. Its entire role is external consumption. |
   | `nebula-core` | Primitive types re-exported via `nebula-sdk`; external consumers touch them through sdk. |
   | `nebula-error` | Shared error taxonomy (`STYLE.md §6`); surfaces through every public result type. |
   | `nebula-resilience` | Standalone retry / bulkhead primitives with no upward workspace deps. |
   | `nebula-credential` | Security-primitives surface; the `KeyProvider` seam from the 2026-04-19 audit's `security-lead` section is an intentional public contract. |

   Their proc-macro companions (`nebula-error-macros`,
   `nebula-credential-macros`, and any macro crates the five above
   directly depend on) carry `publish = true` on condition (b) — a proc
   macro is useless without its host crate.

   **Every other workspace member starts `publish = false`.** This ADR
   does not pre-decide their fate; future flips go through the gate.

4. **Review cadence.** The published list is reviewed at every minor
   release-train discussion (alongside `docs/MATURITY.md`). Adding a
   crate goes through the same gate — the burden is on the proposer to
   document (a) or (b), not on the reviewer to disprove it.

5. **Enforcement.** A CI check fails the build if any workspace crate
   has `publish = true` without either:

   - a `# publish-justification:` comment listing consumer count and
     named consumers (condition (a)), or
   - a referenced ADR ID in that same comment (condition (b)).

   The mechanical shape of the check — matrix job, a step alongside
   `cargo deny`, or a small `xtask/` helper — is implementation detail
   for the follow-up PR. The requirement itself is normative: it must
   be in CI, not only in reviewer discipline.

6. **Explicitly out of scope.**

   - **Directory layout.** Whether `publish = false` crates should move
     under `crates/internal/` vs. stay under `crates/` is aesthetic and
     deferred.
   - **Layered-architecture enforcement.** `deny.toml` `wrappers` rules
     govern *who may depend on what*; this ADR governs *who may be
     depended on by the world*. They are complementary and unchanged by
     this ADR.
   - **Per-crate flip roadmap.** This ADR names only the initial set.
     Future crates earn publish via the gate; this document is not a
     roadmap.

## Consequences

**Positive**

- One SemVer gate at 1.0, not 25. Internal refactors stop blocking
  external consumers because most crates are not externally visible.
- Each `publish = true` flip becomes a deliberate, reviewed act — with
  a documented consumer list or an ADR — instead of the accidental
  default.
- docs.rs hosts the crates we want external authors to read, not every
  internal helper.

**Negative**

- `nebula-sdk`'s current re-exports of `nebula-action`, `nebula-credential`,
  `nebula-resource`, `nebula-schema`, `nebula-workflow`, `nebula-plugin`,
  `nebula-validator` (see `crates/sdk/Cargo.toml`) would force transitive
  publication of those crates — crates.io rejects publishing a crate
  with unpublished path deps whose types appear in the publisher's
  public API. Resolving this is a follow-up: either tighten sdk's
  re-exports to what it directly owns, or add those crates to the
  published set via a dedicated ADR before the first crates.io push.
- Adds one CI check to maintain; a new crate cannot silently inherit
  `publish = true`.

**Neutral**

- `deny.toml` is unchanged. Layered dep rules and publication rules are
  distinct contracts.
- `docs/MATURITY.md` status columns (`frontier`/`partial`/`stable`) remain
  orthogonal to publication. A `stable` + `publish = false` crate is
  fine; a `frontier` + `publish = true` crate is a red flag — public
  SemVer is incompatible with instability-by-contract.

## Alternatives considered

- **Keep `publish = true` as the workspace default; prune after 1.0.**
  Reject. The blast radius — 25 crates, 25 inboxes, 25 docs surfaces —
  shows up *before* we have the capacity to back any of them. Cheaper
  to start closed.
- **Publish everything but mark most crates as "experimental" in README.**
  Reject. crates.io has no experimental tier; docs.rs renders every
  crate equally. Readers take publication as endorsement.
- **`publish = false` for everything except `nebula-sdk`.** Reject —
  too tight. `nebula-error`, `nebula-resilience`, and `nebula-core`
  have independent utility; excluding them would pressure us to smuggle
  them through sdk's re-exports, inflating sdk's surface for orthogonal
  reasons.
- **No workspace-wide policy; decide per crate at opt-in time.** Reject.
  That is the current state. It produces silent drift — the audit's
  "worst-of-both" situation where most crates are accidentally-public
  because nobody paused to decide.

## Follow-ups

- Implementation PR: flip `publish = false` on every crate not named in
  Decision §3. Non-trivial because of the `nebula-sdk` re-export
  transitive-closure issue; may require tightening sdk's public surface
  or a companion ADR first.
- CI check (see Decision §5). Natural home is alongside the existing
  `cargo deny` job; scan workspace `Cargo.toml` files for the
  `# publish-justification:` comment convention.
- Release-train agenda hook: first item every minor cut is
  "published-list delta since last release, with justifications".
- Follow-up ADR if any crate in the initial published set proves wrong
  — e.g. `nebula-credential`'s `KeyProvider` seam lands differently and
  the crate ends up below the public waterline.
