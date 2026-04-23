---
name: nebula-core
role: Shared Vocabulary (identifiers, keys, auth primitives, scope system, context contracts)
status: frontier
last-reviewed: 2026-04-23
canon-invariants: []
related: [nebula-error, nebula-schema, nebula-action, nebula-resource, nebula-credential]
---

# nebula-core

## Purpose

Every crate in the Nebula workspace needs stable, opaque handles for executions, workflows, users,
and resources. Without a single home for these identifiers and keys, each crate invents its own
ULID newtype, scope concept, or auth enum — and they diverge. `nebula-core` is the one crate every
other crate can safely depend on for shared vocabulary: typed identifiers, normalized keys, scope
levels, auth scheme enums, context contracts, and lifecycle signals.

## Role

**Shared Vocabulary** — the vocabulary layer at the bottom of every other crate in the workspace
(cross-cutting infrastructure per `CLAUDE.md` layer direction). Pattern: *Layered Architecture with
cross-cutting infrastructure* (Fundamentals of SW Architecture). This crate sits below Core in the
stack; nothing here depends upward. Changing any of these identifiers or keys cascades across the
workspace — extend `nebula-core` deliberately (canon §3.10).

## Public API

- `ExecutionId`, `WorkflowId`, `NodeId`, `UserId`, `TenantId`, `ProjectId`, `OrganizationId`, `ResourceId`, `RoleId` — prefixed ULID typed identifiers. (`CredentialId` lives in `nebula-credential`.)
- `PluginKey`, `ActionKey`, `CredentialKey`, `ParameterKey`, `ResourceKey`, `NodeKey` — normalized string keys with validation.
- `ScopeLevel`, `Scope`, `Principal`, `ScopeResolver` — hierarchical scope system (Global → Organization → Project → Workflow → Execution → Action).
- `Context` trait, `BaseContext`, `BaseContextBuilder` — base context with capability traits (`HasCredentials`, `HasResources`, `HasMetrics`, `HasEventBus`, `HasLogger`).
- `ResourceAccessor`, `CredentialAccessor`, `Logger`, `MetricsEmitter`, `EventEmitter`, `Clock` — capability accessor traits injected through context.
- `Guard`, `TypedGuard` — RAII guard traits for scoped resource/credential wrappers (module `guard`). Debug helpers: `debug_redacted`, `debug_typed`.
- `AuthScheme`, `AuthPattern` — open auth scheme trait and credential classification enum (module `auth`). Canonical home; re-exported by `nebula-credential` for discoverability.
- `LayerLifecycle`, `ShutdownOutcome` — lifecycle signal types.
- `TraceId`, `SpanId` — observability identity types.
- `CoreError` — typed error for this crate (thiserror, no anyhow).
- `OrgRole`, `WorkspaceRole`, `effective_workspace_role` — organization and workspace role enums for RBAC (module `role`).
- `Permission`, `PermissionDenied` — granular permission definitions (module `permission`).
- `TenantContext`, `ResolvedIds` — multi-tenant context and resolved organization/workspace IDs (module `tenancy`).
- `Slug`, `SlugKind`, `SlugError`, `is_prefixed_ulid()` — validated slug strings for human-readable identifiers (module `slug`).

Credential-specific vocabulary (`CredentialEvent`, `CredentialId`) lives in `nebula-credential`. The `AuthScheme` trait and `AuthPattern` enum are canonical in this crate (module `auth`); `nebula-credential` re-exports them for discoverability.

## Contract

- **[L1-§3.10]** Identifiers and keys in this crate are the stable, opaque handles shared by every other crate. Changing their representation cascades across the workspace.
- **[L2-§12.5]** `SecretString` (credential material wrapper) must keep its `Debug` implementation redacted. No secret material may appear in log output or error strings. Seam: credential-related key types in `crates/core/src/keys.rs`. Test coverage: see `docs/MATURITY.md`.

## Non-goals

- Not a validation system — see `nebula-schema` for schema and `nebula-validator` for rules.
- Not an error taxonomy — see `nebula-error` for `NebulaError`, `Classify`, and `ErrorCategory`.
- Not a resilience pipeline — see `nebula-resilience`.
- Not a storage or persistence layer — types here are vocabulary; persistence lives in `nebula-storage`.

## Maturity

See `docs/MATURITY.md` row for `nebula-core`.

- API stability: `frontier` — identifiers and keys are stable and in active use; context and accessor traits are still evolving as the integration model solidifies. Role, permission, tenancy, and slug modules are new and may see breaking changes.
- Identifier types are load-bearing and unlikely to change; context capability traits may gain new associated methods.
- New modules (`role`, `permission`, `tenancy`, `slug`) are actively used by `nebula-api` routing infrastructure.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.10 (shared vocabulary), §12.5 (secrets / redaction).
- Integration model: `docs/INTEGRATION_MODEL.md` §1 (identifier usage across concepts).
- Glossary: `docs/GLOSSARY.md` §1 (identifiers and keys).
- Siblings: `nebula-error` (error taxonomy used by this crate), `nebula-schema` (configuration schema).

## Appendix: Identifier Conventions

All ID types use prefixed ULIDs via the `domain-key` crate (no direct `uuid` dependency).
All ID types are `Copy`, support `new()`, `nil()`, `parse(&str)`, serde, and re-export
`UuidParseError` for parse error handling.

Prefix examples: `ExecutionId` → `exe_01J9…`, `WorkflowId` → `wf_01J9…`.

### Prelude

```rust
use nebula_core::prelude::*;

let execution_id = ExecutionId::new();
```
