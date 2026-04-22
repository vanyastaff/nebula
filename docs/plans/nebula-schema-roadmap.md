---
title: nebula-schema — execution roadmap
status: active
last-reviewed: 2026-04-22
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

## Status snapshot (2026-04-22)

| Track | State | Notes |
|-------|--------|--------|
| **Phase 1 — Foundation** | **Shipped** in tree | Proof tokens, `nebula-parameter` removed, callers migrated. See design spec acceptance; bench partiality documented in `crates/schema/benches/RESULTS.md` if present. |
| **Phase 2 — DX** | **Mostly shipped** | `HasSchema` / `HasSelectOptions`, typed `Schema::builder()` DSL, `#[derive(Schema)]`, `#[derive(EnumSelect)]`, `field_key!`, trybuild suite, `#[param(enum_select)]` for enum-backed `SelectField`, `SelectField::extend_options`. |
| **Phase 3 — Security** | **Complete on branch** (PR-2; merge + security review) | `SecretValue`, `KdfParams` (Argon2id), `FieldValue::SecretLiteral`, `get_secret`, `LoaderContext::with_secrets_redacted`, ADR-0034 + configuration pipeline diagram (ADR + `INTEGRATION_MODEL.md`). Credential wiring for `SecretWire` at call sites remains follow-up per ADR-0034. |
| **Phase 4 — Advanced** | **In progress** (PR-3 C1 implemented on branch) | JSON Schema export (`schemars`) is implemented on this branch with tests; C2–C6 remain intentionally deferred follow-ups per PR-3 scope cut. |

## Remaining work — Phase 2 (close the gap)

Use the phase 2 **design** spec for semantics; this list is the execution queue.

- [x] **Struct-level `#[schema(...)]`** — `#[schema(custom = "...")]` → `Rule::custom`; builder `root_rule` for predicate/value rules at validate time.  
- [x] **Serde default alignment** — documented + integration pattern: callers add `#[serde(default)]` / `#[serde(default = ...)]` on the struct; `#[derive(Schema)]` does not inject serde attributes.  
- [x] **Integration roundtrip** — `crates/schema/tests/flow/derive_roundtrip.rs`: derive + `validate` (+ serde-default example).  
- [x] **`Vec<Enum>` + select** — derive still rejects `Vec<...>` with `enum_select`; `docs/GLOSSARY.md` + trybuild `derive_schema_enum_select_vec` document the manual list path.  
- [x] **Doctest / lib.rs** — crate-level doctest for `Schema::builder` + `root_rule` (predicate).  
- [x] **CHANGELOG** — one entry when the above close enough for a release note.

**Verification (when touching schema):** `cargo test -p nebula-schema`, `cargo clippy -p nebula-schema -p nebula-schema-macros -- -D warnings`, and if compile-fail changes: `cargo test -p nebula-schema --test compile_fail` with `TRYBUILD=overwrite` only when updating `.stderr` intentionally.

## Backlog — Phase 3 (Security)

See `docs/superpowers/specs/2026-04-16-nebula-schema-phase3-security-design.md` and archived skeleton tasks in `docs/plans/archive/2026-04-16-nebula-schema-implementation-plans/2026-04-16-nebula-schema-phase3-security.md`. **Do not** execute that file as canonical; use this roadmap + the design spec.

## Backlog — Phase 4 (Advanced)

See `docs/superpowers/specs/2026-04-16-nebula-schema-phase4-advanced-design.md` and archived `...-phase4-advanced.md` in the same archive folder.

## Active queue — PR-3 (Phase 4 C1 + docs)

Execution source: `docs/plans/nebula-schema-pr3-phase4-json-schema-plus-docs.md`.

- [x] **C1.1 `schemars` feature wiring** — add optional JSON Schema export path in `nebula-schema` behind the existing `schemars` feature.
- [x] **C1.2 Type mapping pass** — cover field/value variants touched in this PR with deterministic JSON Schema output; keep unsupported cases explicit.
- [x] **C1.3 Test matrix** — verify `cargo test -p nebula-schema` and `cargo test -p nebula-schema --features schemars`; add targeted tests for exported schema shape.
- [x] **P3-D1 CHANGELOG** — add release-note entry for the PR stack closeout (including the remaining Phase 2 checkbox in this file).
- [x] **P3-D2 MATURITY** — no maturity row change required; `nebula-schema` remains `frontier` with stable integration semantics.
- [x] **P3-D3 Roadmap refresh** — after merge, update this roadmap (`status`, Phase 4 row, remaining checkboxes, `last-reviewed`).
- [x] **P3-D4 Schema README** — update `crates/schema/README.md` with feature flag and JSON Schema usage examples if public API changed.
- [ ] **Follow-up issue handoff (C2-C6)** — create owner-tagged issues for deferred Phase 4 scope (expression AST coupling, inference, diff, i18n crate, async validation).

## Agent instructions

- **Do not** use the archived 2026-04-16 implementation plan files as a task list; they are historical and mix completed Phase 1 steps with stale checkboxes.  
- **Do** use this roadmap + the phase design specs for decisions.  
- **Renaming `enum_select`:** not planned — tech-lead call is to keep the shipped name.
