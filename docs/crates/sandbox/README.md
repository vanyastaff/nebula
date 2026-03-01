# nebula-sandbox

Sandbox execution contracts for action isolation in Nebula.

## Scope

- In scope:
  - sandbox runner port contract (`nebula-ports::sandbox`)
  - in-process sandbox driver behavior and integration boundaries
  - capability/cancellation enforcement responsibilities at sandbox boundary
- Out of scope:
  - full WASM/process isolation runtime (planned)
  - action implementation business logic
  - generic runtime scheduling policy

## Current State

- maturity: partially implemented through port + in-process driver; no standalone `crates/sandbox` crate yet.
- key strengths:
  - clean port abstraction (`SandboxRunner`) decouples runtime from concrete sandbox backend.
  - working in-process driver with cancellation checks and tracing.
  - architecture supports future pluggable backends.
- key risks:
  - capability model is not yet fully implemented in `SandboxedContext` (current wrapper mostly forwards `NodeContext`).
  - no full-isolation backend (WASM/process) for untrusted/community actions.

## Target State

- production criteria:
  - stable sandbox port contract with at least two backends (`inprocess`, `wasm/process`).
  - explicit capability enforcement for resource/credential/network/filesystem access.
  - auditable sandbox violation and policy decision flow.
- compatibility guarantees:
  - additive backend options in minor releases
  - breaking execution/capability semantics only in major releases

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
