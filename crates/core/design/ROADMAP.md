# Roadmap

This roadmap is focused on making `nebula-core` a production-grade foundation for a Rust-first workflow automation platform.
Detailed breaking/non-breaking initiatives are tracked in `PROPOSALS.md`, `CONSTITUTION.md`, and `MIGRATION.md`.

---

## Phase 1: API & Docs Cleanup (Short Term)

**Status:** Done

- Align public docs (`API.md`, `CONSTITUTION.md`, crate `README.md`) with exact source behavior and examples.
- Remove outdated API narratives from archived docs.
- Audit naming consistency (`id.rs` vs `ids`, trait method names, scope terminology).
- Add missing module-level examples for `keys`, `scope`, and `CoreError` usage. ✓

Definition of done:
- All public docs conceptually match current APIs (`lib.rs` re-exports and module maps).
- No stale references to non-existing methods or types.

---

## Phase 2: Compatibility Contracts

**Status:** Done

- Explicit compatibility policy in `COMPATIBILITY.md` for:
  - `InterfaceVersion`, serialized enums (`Status`, `RoleScope`, `ProjectType`), `ScopeLevel`, ID types
  - `CoreError::error_code()` stability
- Schema contract tests in `crates/core/tests/schema_contracts.rs` assert JSON and error codes; CI enforces.

Definition of done:
- Breaking-change rules documented and test-enforced for IDs, enums, and core types used across crate/API/storage boundaries. ✓

---

## Phase 3: Scope Semantics Hardening

**Status:** Done

- `ScopeLevel::is_contained_in` retained as simplified level-only check (backward compatible).
- `ScopeResolver` trait and `is_contained_in_strict(scope, other, resolver)` added for ID-verified containment; engine/runtime implement the resolver.
- Canonical scope hierarchy and transitions documented in `ARCHITECTURE.md` (see "Canonical scope transitions").

Definition of done:
- Containment rules are explicit, test-covered, and unambiguous (including strict/ID-aware APIs). ✓

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
