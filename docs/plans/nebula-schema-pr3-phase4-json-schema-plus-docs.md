---
title: nebula-schema — PR-3 (scoped Phase 4 + docs)
status: draft
created: 2026-04-22
depends_on: [nebula-schema-pr2-phase3-security.md]
blocks: []
spec: ../superpowers/specs/2026-04-16-nebula-schema-phase4-advanced-design.md
roadmap: nebula-schema-roadmap.md
---

# PR-3 — Scoped Phase 4 + documentation closeout

**Goal:** Land **only** the tech-lead-approved Phase 4 slice for this series, then update docs/maturity/changelog.

## Phase 4 scope (hard cap)

**In scope for PR-3:**

- **C1 — JSON Schema export** behind existing optional feature `schemars` on `nebula-schema` (wiring + mapping for field types you touch; expand iteratively with tests).

**Explicitly out of scope** (open follow-up issues with owners — do not sneak into this PR):

- C2 real `nebula_expression` parse coupling (unless already trivially true — still prefer separate PR).
- C3 build-time expression type inference.
- C4 `SchemaDiff`.
- C5 `nebula-schema-i18n` new crate.
- C6 `validate_async` and loader semantics.

## Docs / meta (same PR as C1)

| ID | Task | Paths |
|----|------|-------|
| P3-D1 | `CHANGELOG.md` entry for the three-PR series (or per-PR entries if preferred) | root |
| P3-D2 | `docs/MATURITY.md` row for `nebula-schema` if stability/integration moved | `docs/MATURITY.md` |
| P3-D3 | Refresh `docs/plans/nebula-schema-roadmap.md` (`last-reviewed`, checkboxes) | `docs/plans/` |
| P3-D4 | `crates/schema/README.md` if public surface / features changed | `crates/schema/README.md` |

## Merge gate (PR-3)

```bash
cargo +nightly fmt --all --check
cargo test -p nebula-schema
cargo test -p nebula-schema --features schemars
cargo clippy -p nebula-schema --all-targets --all-features -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo deny check
```

Adjust `--features` if the workspace uses a different flag name — **must match CI**.

## Reviews

- **rust-senior** if macro-heavy JSON schema glue leaks complexity into `field.rs`.
- **api-contract-reviewer** if generated JSON shape is considered a stable external contract.

## After merge

- Close tracking issues; file follow-ups for deferred C2–C6.
- Optional: run `docs-sync` skill checklist before tagging a release.
