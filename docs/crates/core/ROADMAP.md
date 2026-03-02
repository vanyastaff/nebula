# Roadmap

This roadmap is focused on making `nebula-core` a production-grade foundation for a Rust-first workflow automation platform.
Detailed breaking/non-breaking initiatives are tracked in `PROPOSALS.md`.

## Phase 1: API Cleanup (Short Term)

- Align docs with exact source behavior and examples.
- Remove outdated API narratives from old docs (archived).
- Audit naming consistency (`id.rs` vs `ids`, trait method names, scope terminology).
- Add missing module-level examples for `keys` and `scope`.

Definition of done:
- All public docs compile conceptually against current APIs.
- No stale references to non-existing methods.

## Phase 2: Compatibility Contracts

- Introduce explicit compatibility policy for:
  - `InterfaceVersion`
  - serialized enums (`Status`, `RoleScope`, `ProjectType`)
  - `CoreError` code stability
- Add snapshot-style tests for serialized forms.

Definition of done:
- Breaking-change rules documented and test-enforced.

## Phase 3: Scope Semantics Hardening

- Improve `ScopeLevel::is_contained_in` semantics where containment is currently simplified.
- Add deterministic mapping hooks for workflow/execution ownership validation.
- Document canonical scope transitions for runtime/engine integration.

Definition of done:
- Containment rules are explicit, test-covered, and unambiguous.

## Phase 4: Constants Governance

- Split broad constants into tiers:
  - truly global defaults (keep in core)
  - domain-owned defaults (move to owning crate over time)
- Mark deprecated constants and provide migration notes.

Definition of done:
- `constants.rs` contains only stable foundation defaults.

## Phase 5: Rust Baseline Upgrade Plan

- Current workspace baseline is Rust `1.93`.
- Prepare bump path to Rust `1.93+`:
  - CI matrix update
  - clippy/rustdoc policy checks
  - documentation refresh for language/library changes

Definition of done:
- Workspace baseline updated with green CI and updated docs.
