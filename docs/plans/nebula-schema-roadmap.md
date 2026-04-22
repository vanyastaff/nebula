---
title: nebula-schema — execution roadmap
status: active
last-reviewed: 2026-04-21
supersedes: archive/2026-04-16-nebula-schema-implementation-plans/
related:
  - superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md
  - superpowers/specs/2026-04-16-nebula-schema-phase2-dx-layer-design.md
  - superpowers/specs/2026-04-16-nebula-schema-phase3-security-design.md
  - superpowers/specs/2026-04-16-nebula-schema-phase4-advanced-design.md
---

# nebula-schema — execution roadmap

This file is the **single place** for *what to do next* for `nebula-schema` (`crates/schema/`). Design rationale stays in `docs/superpowers/specs/2026-04-16-nebula-schema-phase*-design.md`. Historical checkbox plans from 2026-04-16 live in `docs/plans/archive/2026-04-16-nebula-schema-implementation-plans/` for archaeology only.

**Execution plans (Ph2–Ph4 closeout):**

- **Preferred — three-PR stack (tech-lead 2026-04-22):** [PR-1 Phase 2 gap](nebula-schema-pr1-phase2-gap.md) → [PR-2 Phase 3 security](nebula-schema-pr2-phase3-security.md) → [PR-3 scoped Ph4 + docs](nebula-schema-pr3-phase4-json-schema-plus-docs.md).
- **Master document** (tech-lead override + mega-PR fallback): [nebula-schema-one-pr-final-plan.md](nebula-schema-one-pr-final-plan.md).

**Tech-lead (2026-04-21):** Keep the attribute name `#[param(enum_select)]` as shipped; do not rename. Archive old implementation plan files; keep design specs in `superpowers/specs/`.

## Design specs (normative reference)

| Phase | Design document |
|-------|-----------------|
| 1 Foundation | `docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md` |
| 2 DX | `docs/superpowers/specs/2026-04-16-nebula-schema-phase2-dx-layer-design.md` |
| 3 Security | `docs/superpowers/specs/2026-04-16-nebula-schema-phase3-security-design.md` |
| 4 Advanced | `docs/superpowers/specs/2026-04-16-nebula-schema-phase4-advanced-design.md` |

## Status snapshot (2026-04-21)

| Track | State | Notes |
|-------|--------|--------|
| **Phase 1 — Foundation** | **Shipped** in tree | Proof tokens, `nebula-parameter` removed, callers migrated. See design spec acceptance; bench partiality documented in `crates/schema/benches/RESULTS.md` if present. |
| **Phase 2 — DX** | **Mostly shipped** | `HasSchema` / `HasSelectOptions`, typed `Schema::builder()` DSL, `#[derive(Schema)]`, `#[derive(EnumSelect)]`, `field_key!`, trybuild suite, `#[param(enum_select)]` for enum-backed `SelectField`, `SelectField::extend_options`. |
| **Phase 3 — Security** | **Not started** (skeleton) | `SecretValue`, zeroize, KDF, redaction — follow phase 3 design spec. |
| **Phase 4 — Advanced** | **Not started** (skeleton) | JSON Schema export, expression inference, diff, i18n — follow phase 4 design spec. |

## Remaining work — Phase 2 (close the gap)

Use the phase 2 **design** spec for semantics; this list is the execution queue.

- [ ] **Struct-level `#[schema(...)]`** — parse `#[schema(custom(my_fn))]` (or agreed surface) and emit a top-level custom rule. *Blocked on API design; align with `nebula_validator::Rule`.*  
- [ ] **Serde default alignment** — when `#[param(default = ...)]` is set on a derived field, ensure serde deserialization defaults match (emit `#[serde(default = ...)]` or shared helper). Integration test: empty JSON uses schema defaults.  
- [ ] **Integration roundtrip** — optional `crates/schema/tests/flow/derive_roundtrip.rs`: derive → `validate` → `resolve` → typed decode (as far as the public API allows).  
- [ ] **`Vec<Enum>` + select** — today `#[param(enum_select)]` is rejected on `Vec<...>`; either document “manual list + item field” or extend the derive.  
- [ ] **Doctest / lib.rs** — short example showing both builder path and `#[derive(Schema)]` (if not already satisfied).  
- [ ] **CHANGELOG** — one entry when the above close enough for a release note.

**Verification (when touching schema):** `cargo test -p nebula-schema`, `cargo clippy -p nebula-schema -p nebula-schema-macros -- -D warnings`, and if compile-fail changes: `cargo test -p nebula-schema --test compile_fail` with `TRYBUILD=overwrite` only when updating `.stderr` intentionally.

## Backlog — Phase 3 (Security)

See `docs/superpowers/specs/2026-04-16-nebula-schema-phase3-security-design.md` and archived skeleton tasks in `docs/plans/archive/2026-04-16-nebula-schema-implementation-plans/2026-04-16-nebula-schema-phase3-security.md`. **Do not** execute that file as canonical; use this roadmap + the design spec.

## Backlog — Phase 4 (Advanced)

See `docs/superpowers/specs/2026-04-16-nebula-schema-phase4-advanced-design.md` and archived `...-phase4-advanced.md` in the same archive folder.

## Agent instructions

- **Do not** use the archived 2026-04-16 implementation plan files as a task list; they are historical and mix completed Phase 1 steps with stale checkboxes.  
- **Do** use this roadmap + the phase design specs for decisions.  
- **Renaming `enum_select`:** not planned — tech-lead call is to keep the shipped name.
