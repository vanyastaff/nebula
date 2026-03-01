# nebula-tenant

Planned tenant isolation and quota-management layer for Nebula.

## Scope

- In scope:
  - tenant identity and context resolution
  - isolation strategy selection (shared/dedicated/isolated)
  - quota policy and enforcement contracts
  - tenant-level governance hooks for runtime/resource/storage
- Out of scope:
  - low-level storage engine implementation details
  - workflow execution orchestration internals
  - credential protocol implementation details

## Current State

- maturity: planned only; `crates/tenant` is not implemented yet.
- key strengths:
  - archived design seeds already define isolation strategies and quota intent.
  - strong adjacent crates (`core`, `resource`, `credential`, `storage`) provide integration anchors.
- key risks:
  - tenant semantics are currently scattered across crates without a single owner.
  - cross-crate policies may diverge before a dedicated crate lands.

## Target State

- production criteria:
  - single authoritative tenant context contract for runtime stack
  - deterministic quota enforcement and isolation boundaries
  - auditable tenant policy decisions and failure paths
- compatibility guarantees:
  - additive policy/config expansion in minor releases
  - isolation/ownership semantics change only in major releases

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
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
