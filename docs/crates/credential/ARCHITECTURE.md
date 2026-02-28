# Architecture

## Positioning

`nebula-credential` is a security-critical infrastructure crate.

Dependency direction:
- runtime/action/api layers -> `nebula-credential`
- `nebula-credential` should not depend on workflow business logic

## Layered Structure

- `core`
  - identity, scope, context, metadata, errors, credential references
- `traits`
  - contracts for storage, distributed locks, rotatable/testable credentials
- `providers`
  - concrete persistence backends selected via feature flags
- `manager`
  - main orchestration API with cache + validation + CRUD
- `protocols`
  - protocol-specific state/config models and reusable protocol implementations
- `rotation`
  - policy-driven credential rotation with safety and failure handling
- `utils`
  - crypto + secret handling + time/retry utilities

## Security Boundaries

- encrypted credential payloads are first-class values
- context + scope is used for tenant isolation
- secret value handling is centralized in dedicated utility types
- unsafe code is forbidden at crate root

## Operational Properties

- async-first API surface
- provider abstraction allows environment-specific backend choice
- cache layer is optional and configurable
- rotation subsystem supports periodic/scheduled/manual/before-expiry patterns

## Known Complexity Hotspots

- wide feature matrix for providers and protocols
- large rotation subsystem with many safety components
- extensive internal docs in `crates/credential/docs/` require synchronization discipline
