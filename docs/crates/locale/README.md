# nebula-locale

Planned localization and internationalization layer for Nebula.

## Scope

- In scope:
  - locale negotiation and fallback resolution
  - translation bundle management
  - localized formatting for user-visible messages and errors
  - cross-crate localization contracts for API/runtime/action surfaces
- Out of scope:
  - business logic unrelated to language/region handling
  - authentication/authorization policy
  - workflow scheduling/orchestration

## Current State

- maturity: planned; `crates/locale` is not implemented yet.
- key strengths:
  - clear legacy intent around fluent-style translations and localized error rendering.
  - strong integration demand from API/action/runtime layers.
- key risks:
  - localization concerns are currently scattered and can drift without central ownership.

## Target State

- production criteria:
  - centralized locale negotiation and translation runtime
  - deterministic fallback behavior and stable message keys
  - measurable coverage for localized user-facing surfaces
- compatibility guarantees:
  - additive locale/catalog support in minor releases
  - message-key or interpolation semantic breaks only in major releases

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
