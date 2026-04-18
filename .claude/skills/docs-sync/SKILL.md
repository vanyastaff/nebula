---
name: docs-sync
description: Use after a non-trivial change, before commit. Walks PRODUCT_CANON §17 definition of done — MATURITY.md row, ADR if L2 moved, crate README, CLAUDE.md toolchain, INTEGRATION_MODEL, STYLE, GLOSSARY, plan/spec archival. Prevents the "code moved, docs lied" failure that rots canon truthfulness.
---

# docs-sync

## When to invoke

- Immediately after implementation, before `git commit` on non-trivial work.
- Always before a PR that changes public API, crate boundary, or any L1/L2 invariant.
- When you moved files between crates or renamed public items.

## Why this exists

Canon §17 defines DoD as "relevant tests/commands green **and** alignment with `docs/PRODUCT_CANON.md`". A PR that ships code with stale docs is, by definition, incomplete. This skill is the mechanical walk-through so nothing is quietly dropped.

## Checklist

### 1. Crate maturity — `docs/MATURITY.md`

For each crate the PR touched, review its row along five axes:

- [ ] **API stability** — `stable` / `frontier` / `partial`. Did this PR move it?
- [ ] **Test coverage** — did this PR add new behavior without covering it?
- [ ] **Doc completeness** — did new public items get `///` docs and a `lib.rs //!` mention?
- [ ] **Engine integration** — if the crate plumbs into engine/runtime, did this PR change that?
- [ ] **SLI readiness** — if the crate emits metrics/events, did this PR change that?

If any axis moved: update the row. If unchanged: confirm the row is still truthful.

### 2. Invariant changes — `docs/PRODUCT_CANON.md` + `docs/adr/`

If this PR touches any L2 invariant (execution lifecycle, storage CAS, durable outbox, structural contract, plugin packaging, schema proof-token pipeline, credential stored-state split):

- [ ] An ADR exists at `docs/adr/NNNN-<slug>.md` describing the decision.
- [ ] The canon section is either updated OR confirmed unchanged because the ADR preserves it.
- [ ] The seam test covering the invariant is updated **in the same PR** (§17 DoD requirement).

If touching L1 (pillars, principles) — that is a product-level revision. Hand off to `tech-lead` and open an ADR per `docs/PRODUCT_CANON.md §0.2`.

### 3. Crate README + `lib.rs //!`

If the PR changed a crate's public surface (added / removed / re-shaped items):

- [ ] `crates/<name>/README.md` reflects the new surface.
- [ ] `canon-invariants:` YAML frontmatter (if present in the README) is still truthful.
- [ ] `lib.rs //!` doc comment is current — no dangling references to removed items.

### 4. Toolchain / workflow — root `CLAUDE.md`

If this PR changes:

- MSRV, edition, rustfmt config, clippy config
- CI required jobs
- Canonical commands
- Layer boundaries (`deny.toml` update)

…update root `CLAUDE.md` accordingly. Canonical commands must never drift from CI reality.

### 5. Integration / style / glossary

- [ ] `docs/INTEGRATION_MODEL.md` — updated if Resource / Credential / Action / Schema / Plugin mechanics shifted.
- [ ] `docs/STYLE.md` — updated if a new idiom, antipattern, naming convention, or type-design bet was introduced.
- [ ] `docs/GLOSSARY.md` — updated if a new architectural term was introduced.

### 6. Plan / spec archival

If this PR implements a plan or spec:

- [ ] Plan file under `docs/plans/` is marked implemented or moved to `archive/`.
- [ ] Spec file under `docs/superpowers/specs/` is linked from the ADR or PR body.

### 7. Observability contracts

If the PR added / removed / changed:

- Lifecycle events on `execution_journal`
- SLI / SLO definitions
- Metric names or labels

…confirm `docs/OBSERVABILITY.md` is in sync. Operators read that doc first when a run misbehaves.

## Output format

```
## docs-sync: [change]

MATURITY.md:   [rows updated / no change needed]
Canon / ADR:   [ADR NNNN drafted / canon §X updated / none]
Crate READMEs: [list / none]
lib.rs //!:    [list / none]
Root CLAUDE.md: [updated / no change]
INTEGRATION / STYLE / GLOSSARY: [list / none]
OBSERVABILITY: [updated / none]
Plans / specs: [archived <path> / linked <path> / none]

### Residual drift
[items the PR should have updated but didn't, with reasoning]
```

If any residual drift is listed — fix it before committing. Canon §17 makes this non-optional.
