# nebula-core

Foundation crate. Provides the identifiers, scope system, base traits, common types, error
handling, and validated key types shared by every crate in the workspace.

**No other Nebula crate depends on it as a peer; everything else depends on this.**

---

## Role in the Workspace

```
nebula-core
  └─ depended on by ALL other crates
       ├─ nebula-log
       ├─ nebula-action
       ├─ nebula-expression
       ├─ nebula-parameter
       ├─ nebula-credential
       ├─ nebula-memory
       ├─ nebula-resource
       ├─ nebula-storage
       └─ … (every other crate)
```

Its purpose is to prevent cyclic dependencies by providing a shared vocabulary layer.
Crates communicate via the types defined here without importing each other.

---

## Module Map

| Module | File | What it provides |
|---|---|---|
| `id` | `id.rs` | 11 strongly-typed UUID wrappers |
| `scope` | `scope.rs` | `ScopeLevel` enum — resource lifecycle hierarchy |
| `traits` | `traits.rs` | Base traits: `Scoped`, `HasContext`, `Identifiable`, etc. |
| `types` | `types.rs` | `Version`, `Status`, `Priority`, `OperationResult`, etc. |
| `error` | `error.rs` | `CoreError` enum (26 variants), `CoreResult<T>` |
| `keys` | `keys.rs` | `PluginKey`, `ParameterKey`, `CredentialKey` |
| `constants` | `constants.rs` | System-wide defaults, limits, patterns |

---

## Topic Files

- [ids.md](ids.md) — Identifier system (`UserId`, `WorkflowId`, `ExecutionId`, …)
- [scope.md](scope.md) — `ScopeLevel` and resource lifecycle scoping
- [traits.md](traits.md) — Base traits used across the workspace
- [types.md](types.md) — `Version`, `Status`, `Priority`, `OperationResult`, etc.
- [error.md](error.md) — `CoreError` and error classification
- [keys.md](keys.md) — `PluginKey`, `ParameterKey`, `CredentialKey`
- [constants.md](constants.md) — All system-wide constants and limits

---

## Prelude

```rust
use nebula_core::prelude::*;
// CoreError, all IDs, InterfaceVersion, HasContext, Identifiable,
// ProjectType, Result, RoleId, RoleScope, ScopeLevel, Scoped,
// TenantId, UserId, UuidParseError, WorkflowId, PluginKey, PluginKeyError
```

---

## Design Goals

1. **Zero cyclic dependencies** — this crate has no Nebula dependencies.
2. **Shared vocabulary** — IDs, scopes, traits defined once, reused everywhere.
3. **Type safety** — distinct wrapper types prevent `WorkflowId` from being passed
   where a `NodeId` is expected.
4. **Serializable** — all public types implement `serde::Serialize/Deserialize`.
5. **Copy-friendly IDs** — UUID wrappers are `Copy` (16 bytes), zero heap allocation.
