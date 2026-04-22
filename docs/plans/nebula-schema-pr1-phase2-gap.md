---
title: nebula-schema — PR-1 (Phase 2 gap)
status: done
created: 2026-04-22
depends_on: []
blocks: [nebula-schema-pr2-phase3-security.md]
spec: ../superpowers/specs/2026-04-16-nebula-schema-phase2-dx-layer-design.md
roadmap: nebula-schema-roadmap.md
---

# PR-1 — Phase 2 gap (`nebula-schema` + macros only)

**Goal:** Close remaining DX items from `nebula-schema-roadmap.md` without touching Phase 3/4 runtime security or new crates.

**Blast radius:** `crates/schema/`, `crates/schema/macros/`, plus a **one-line re-export** in `crates/validator` (`validate_rules_with_ctx` at crate root) so `nebula-schema` does not depend on `nebula_validator::engine::` paths.

## Tasks

| ID | Task | Primary paths | Tests |
|----|------|---------------|--------|
| P1-A1 | Struct-level `#[schema(...)]` on `#[derive(Schema)]` — `custom = "..."` + `SchemaBuilder::root_rule` / validate | `macros/src/attrs.rs`, `macros/src/derive_schema.rs`, `macros/src/lib.rs`, `src/schema.rs`, `src/validated.rs` | `compile_fail/` + `flow_derive_roundtrip` + unit tests |
| P1-A2 | Serde default alignment — document + integration: pair `#[param(default)]` with `#[serde(default = "...")]` | `tests/flow/derive_roundtrip.rs`, `lib.rs` | integration |
| P1-A3 | Flow integration `derive_roundtrip` | `crates/schema/tests/flow/derive_roundtrip.rs` | `cargo test -p nebula-schema --test flow_derive_roundtrip` |
| P1-A4 | `Vec<Enum>` + `enum_select` — **no derive extension**; **clear compile error** + `docs/GLOSSARY.md` row for `enum_select` | `macros` (unchanged error text), `docs/GLOSSARY.md`, `compile_fail/derive_schema_enum_select_vec.*` | trybuild |
| P1-A5 | Doctest: builder (`root_rule`) + derive (`Schema` + `#[schema]`) in `lib.rs` | `crates/schema/src/lib.rs` | `cargo test -p nebula-schema --doc` |

## API lock (aligned with `nebula-validator`)

Struct-level attribute shipped in PR-1:

```text
#[schema(custom = "expression_wire_string")]
```

This maps to [`nebula_validator::Rule::custom`] (deferred wire hook). Predicate / value rules that run at schema-validate time are attached via `SchemaBuilder::root_rule` in Rust code, not via the derive string form.

## Merge gate (PR-1)

```bash
cargo +nightly fmt --all --check
cargo test -p nebula-schema
cargo test -p nebula-schema --test compile_fail
cargo clippy -p nebula-schema -p nebula-schema-macros --all-targets -- -D warnings
cargo bench -p nebula-schema --no-run
```

**Perf:** `cargo bench -p nebula-schema --no-run` in CI; a before/after **≤5%** check on `bench_validate` / `bench_resolve` is optional for this DX PR (run if you touch the per-field validate hot path).

## After merge

- Land PR-2 from `nebula-schema-pr2-phase3-security.md`.
- Update `docs/plans/nebula-schema-roadmap.md` checkboxes for completed Phase-2 items.
