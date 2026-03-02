# Roadmap

This roadmap is focused on making `nebula-core` a production-grade foundation for a Rust-first workflow automation platform.
Detailed breaking/non-breaking initiatives are tracked in `PROPOSALS.md`, `CONSTITUTION.md`, and `MIGRATION.md`.

---

## Phase 1: API & Docs Cleanup (Short Term)

**Status:** Done (module-level examples added)

- Align public docs (`API.md`, `DOCS.md`, `CONSTITUTION.md`) with exact source behavior and examples.
- Remove outdated API narratives from archived docs.
- Audit naming consistency (`id.rs` vs `ids`, trait method names, scope terminology).
- Add missing module-level examples for `keys`, `scope`, and `CoreError` usage. ✓

Definition of done:
- All public docs conceptually match current APIs (`lib.rs` re-exports and module maps).
- No stale references to non-existing methods or types.

---

## Phase 2: Compatibility Contracts

**Status:** In progress (schema tests + policy doc added)

- Introduce explicit compatibility policy for:
  - `InterfaceVersion`
  - serialized enums (`Status`, `RoleScope`, `ProjectType`)
  - `CoreError` code and `user_message` stability
- Add snapshot-style tests for serialized forms and error codes.

Definition of done:
- Breaking-change rules documented and test-enforced for IDs, enums, and core types used across crate/API/storage boundaries.

---

## Phase 3: Scope Semantics Hardening

**Status:** In progress (`ScopeResolver` + `is_contained_in_strict` added)

- Improve `ScopeLevel::is_contained_in` semantics where containment is currently simplified (no ID verification for some variants).
- Add deterministic mapping hooks / resolver for workflow–execution–action ownership validation.
- Document canonical scope transitions for runtime/engine integration.

Definition of done:
- Containment rules are explicit, test-covered, and unambiguous (including strict/ID-aware APIs).

---

## Phase 4: Constants Governance

**Status:** Planned (see P-003)

- Split broad constants into tiers:
  - truly global / cross-cutting defaults (keep in core)
  - domain-owned defaults (move to owning crate over time)
- Mark deprecated constants and provide migration notes and aliases.

Definition of done:
- `constants.rs` contains only stable foundation defaults; domain constants live in owning crates with documented migration.

---

## Phase 5: Rust Baseline Strategy

**Status:** Planned

- Current workspace MSRV is Rust `1.93` (edition `2024`).
- Prepare bump path beyond Rust `1.93` while keeping an explicit MSRV policy:
  - CI matrix update
  - clippy/rustdoc policy checks
  - documentation refresh for language/library changes

Definition of done:
- Workspace baseline updated with green CI, updated docs, and a documented MSRV strategy for future upgrades.
