---
title: nebula-schema — PR-1 (Phase 2 gap)
status: draft
created: 2026-04-22
depends_on: []
blocks: [nebula-schema-pr2-phase3-security.md]
spec: ../superpowers/specs/2026-04-16-nebula-schema-phase2-dx-layer-design.md
roadmap: nebula-schema-roadmap.md
---

# PR-1 — Phase 2 gap (`nebula-schema` + macros only)

**Goal:** Close remaining DX items from `nebula-schema-roadmap.md` without touching Phase 3/4 runtime security or new crates.

**Blast radius:** `crates/schema/`, `crates/schema/macros/` only (no `nebula-credential` in this PR unless a trivial re-export is unavoidable — prefer not).

## Tasks

| ID | Task | Primary paths | Tests |
|----|------|---------------|--------|
| P1-A1 | Struct-level `#[schema(...)]` on `#[derive(Schema)]` — agreed shape below | `macros/src/attrs.rs`, `macros/src/derive_schema.rs`, `macros/src/lib.rs` | `compile_fail/` + positive derive |
| P1-A2 | Serde default alignment for `#[param(default = ...)]` — compile-time `#[serde(default = ...)]` or const fn helper | `derive_schema.rs`, optional `crates/schema/src/` | integration: `{}` → defaults |
| P1-A3 | Flow integration `derive_roundtrip` (validate → resolve as API allows) | `crates/schema/tests/flow/derive_roundtrip.rs` | `cargo test -p nebula-schema` |
| P1-A4 | `Vec<Enum>` + `enum_select` — **no derive extension** unless spike proves trivial; prefer **clear compile error** + doc/example using `Field::select(..).extend_options(..)` | macros + `lib.rs` / README | trybuild |
| P1-A5 | Doctest: builder path + derive path in `lib.rs` | `crates/schema/src/lib.rs` | `cargo test -p nebula-schema --doc` |

## API lock (sync with `nebula-validator` before coding)

**Proposed** struct-level attribute (verify against actual `Rule` / deferred validation hooks in tree):

```text
#[schema(custom(validate = "path::to::validate_fn"))]
```

Implementers must **confirm** the callback shape matches an existing dispatch path (e.g. deferred / custom rule wiring) — **do not invent a parallel validation trait** without tech-lead re-approval.

## Merge gate (PR-1)

```bash
cargo +nightly fmt --all --check
cargo test -p nebula-schema
cargo test -p nebula-schema --test compile_fail
cargo clippy -p nebula-schema -p nebula-schema-macros --all-targets -- -D warnings
cargo bench -p nebula-schema --no-run
```

**Perf (merge-blocking for PR-1):** run `cargo bench -p nebula-schema` before/after on representative targets (`bench_validate`, `bench_resolve`); report **≤5%** regression delta in the PR body (or justify).

## After merge

- Land PR-2 from `nebula-schema-pr2-phase3-security.md`.
- Update `docs/plans/nebula-schema-roadmap.md` checkboxes for completed Phase-2 items.
