# nebula-core Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula is a workflow automation platform (n8n-class) with many crates: engine, runtime, storage, credential, action, api, and more. Without a shared foundation, crates would depend on each other for primitives, leading to cyclic dependencies and fragile evolution.

**nebula-core is the canonical vocabulary layer for the whole platform.**

It answers: *What types and traits does every other crate use for IDs, scope, context, and errors — without core depending on any of them?*

```
All nebula-* domain crates
    ↓ depend on
nebula-core (id, scope, traits, types, keys, constants, error)
    ↓
nebula-core → only nebula-log (cross-cutting); no engine/storage/workflow/action etc.
```

Data and control flow: IDs and context flow outward from core into consumers. No data flows into core from other Nebula crates. Core is passive; consumers call into core types and traits. This is the core contract.

---

## User Stories

### Story 1 — Engine Author Uses Typed IDs and Context (P1)

An engineer implementing the workflow engine needs to pass execution ID, workflow ID, and node ID through the pipeline without mixing them up. They use `ExecutionId`, `WorkflowId`, `NodeId` from core so that wrong-ID bugs are caught at compile time.

**Acceptance**:
- `OperationContext::new("op").with_execution_id(exec_id).with_workflow_id(wf_id)` builds a typed context
- Passing `NodeId` where `WorkflowId` is expected is a compile error
- IDs serialize to/from UUID strings for API and storage boundaries

### Story 2 — Resource Manager Checks Scope (P1)

The resource crate needs to know whether a handle belongs to a workflow, execution, or action scope so it can enforce lifecycle and cleanup. It uses `ScopeLevel` and `Scoped` from core.

**Acceptance**:
- `scope.is_contained_in(other)` answers containment for cleanup and access checks
- Scope hierarchy: Global → Organization → Project → Workflow → Execution → Action
- New scope levels can be added without breaking existing consumers

### Story 3 — API/Storage Relies on Stable Serialization (P2)

The API and storage layers serialize workflow and execution identifiers to JSON and store them. They need a stable, documented schema so that clients and migrations do not break.

**Acceptance**:
- All public ID and context types implement `Serialize`/`Deserialize`
- Patch/minor releases do not change serialized form; breaking changes go through major + MIGRATION.md

### Story 4 — Plugin Author Uses Normalized Keys (P2)

A plugin author registers an action under a key like `"My-Plugin"`. The platform normalizes it so that `my_plugin` and `My_Plugin` resolve to the same plugin.

**Acceptance**:
- `PluginKey`, `ParameterKey`, `CredentialKey` normalize on deserialize (trim, lowercase, collapse)
- Max length and character rules are documented and enforced

### Story 5 — Auditor Traces Operations by Context (P3)

An operator or auditor needs to correlate logs and metrics with execution and workflow. Every operation carries an `OperationContext` with optional execution/workflow/node IDs and priority.

**Acceptance**:
- `HasContext` and `OperationContext` provide optional IDs and metadata
- No business logic in core — only vocabulary and traits

---

## Core Principles

### I. Core Has Minimal Nebula Dependencies

**Core must not depend on domain Nebula crates (engine, storage, workflow, action, etc.). The only allowed Nebula dependency is nebula-log (cross-cutting).**

**Rationale**: If core depended on engine or storage, the dependency graph would either cycle or force all domain crates to pull in unrelated code. Keeping core free of domain crates allows every layer to depend on a single, stable vocabulary.

**Rules**:
- `nebula-core`'s `Cargo.toml` may list only `nebula-log` among nebula-* crates
- New types that require domain logic belong in owning crates, not core

### II. Typed IDs Over Raw Strings

**All primary identifiers are strongly typed (ExecutionId, WorkflowId, NodeId, etc.), not plain strings.**

**Rationale**: Passing a node ID where a workflow ID is expected causes subtle bugs. Typed IDs make misuse a compile error and document domain semantics at the type level.

**Rules**:
- Use `domain_key::define_uuid!` or equivalent for ID types
- IDs are `Copy`, 16 bytes; serialize to UUID strings at boundaries
- No generic `Id<T>` in public API — explicit names per domain

### III. Serde-First Public Types

**Public types that cross crate boundaries must implement Serialize and Deserialize.**

**Rationale**: Workflow definitions, API payloads, and storage schemas all cross boundaries. Without serde, every consumer would reinvent serialization and compatibility would break.

**Rules**:
- New public structs/enums in core must derive or implement serde where appropriate
- Breaking serialized form is a major-version change with MIGRATION.md

### IV. Explicit Scope Model

**Scope and lifecycle are first-class: ScopeLevel and Scoped trait.**

**Rationale**: n8n-class platforms need clear boundaries for resource cleanup, credential scope, and multi-tenancy. Implicit or string-based scope leads to leaks and security gaps.

**Rules**:
- ScopeLevel encodes hierarchy; parent/containment semantics documented
- Extensions (e.g. strict ID-verified containment) go through proposals and migration

### V. Shared Error Vocabulary Without Domain Coupling

**Core provides CoreError for foundation concerns; domain crates define their own errors.**

**Rationale**: Validation, serialization, and config failures are foundation-level. Workflow or cluster errors belong in engine/cluster. Mixing them in core would create unwanted dependencies or a bloated core.

**Rules**:
- CoreError covers validation, serialization, keys, constants, infra-style failures
- Domain variants (e.g. WorkflowExecution, Cluster) are candidates to move to owning crates (P-001)

### VI. Stable External Schema

**Serialized representations of core types used at API/storage boundaries are treated as stable.**

**Rationale**: Clients and migrations depend on JSON shape. Silent changes cause production incidents.

**Rules**:
- Patch/minor: no breaking changes to serialized form
- Major: document in MIGRATION.md and provide compatibility path where possible

---

## Production Vision

### The vocabulary layer in an n8n-class fleet

In a production Nebula deployment, every service (engine, worker, API, credential store) shares the same IDs, scope model, and context types. There is no “core service” — core is a library embedded in each binary.

```
┌─────────────────────────────────────────────────────────┐
│  Presentation (api, ui, cli, hub)                        │
├─────────────────────────────────────────────────────────┤
│  Multi-Tenancy & Clustering (cluster, tenant)            │
├─────────────────────────────────────────────────────────┤
│  Business Logic (resource, registry)                      │
├─────────────────────────────────────────────────────────┤
│  Execution (engine, runtime, worker, sandbox)            │
├─────────────────────────────────────────────────────────┤
│  Node (action, parameter, credential)                    │
├─────────────────────────────────────────────────────────┤
│  Core (core, workflow, execution, memory, expression,     │
│        eventbus, idempotency)                             │
├─────────────────────────────────────────────────────────┤
│  Cross-Cutting (config, log, metrics, resilience,         │
│                 system, validator, locale)               │
├─────────────────────────────────────────────────────────┤
│  Infrastructure (storage, binary)                         │
└─────────────────────────────────────────────────────────┘
         ↑ all layers depend on nebula-core (IDs, scope, traits, types)
```

**Module map:** `id` — UserId, TenantId, ExecutionId, WorkflowId, NodeId, ActionId, ResourceId, CredentialId, ProjectId, RoleId, OrganizationId; `scope` — ScopeLevel (Global, Organization, Project, Workflow, Execution, Action); `keys` — ParameterKey, CredentialKey, PluginKey; `traits` — Scoped, HasContext; `types` — Version, InterfaceVersion, ProjectType, RoleScope, OperationContext, OperationResult, Priority; `constants` — SYSTEM_*, DEFAULT_*; `error` — CoreError, Result&lt;T&gt;. Core depends only on nebula-log and vendor. No network, storage, or async runtime of its own.

### From the archives: layer interaction and shared types

The archive `_archive/archive-overview.md` describes the full layer stack and states that core holds “базовые идентификаторы” and “концепция Scope для resource management”. The archive `_archive/archive-layers-interaction.md` states: “Shared types — общие типы только в nebula-core” and “взаимодействие через трейты из nebula-core”. Production vision is consistent: core remains the single source of truth for identifiers and scope; no duplicate ID or scope definitions in other crates.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Strict scope containment (ID-verified) | High | P-002; security and lifecycle correctness |
| CoreError split (domain variants out) | Medium | P-001; cleaner boundaries |
| Constants ownership (domain constants out of core) | Medium | P-003; reduce coupling |
| Frozen serialized schema + snapshot tests | High | P-004; prevent accidental breakage |
| ScopeLevel parent() complete for all variants | Low | Improves tooling and debugging |

---

## Key Decisions

### D-001: Typed IDs Everywhere

**Decision**: Keep domain-specific typed IDs (WorkflowId, NodeId, etc.) instead of plain strings.

**Rationale**: Compile-time protection from ID mix-ups; clear domain semantics in signatures and logs.

**Rejected**: Generic `Id<T>` as single type — explicit names improve readability and docs.

### D-002: Core Has No Domain Dependencies

**Decision**: nebula-core must not depend on domain Nebula crates; only nebula-log is allowed.

**Rationale**: Prevents cycles; keeps foundation reusable by all layers.

**Rejected**: Core depending on a “common” or “kernel” engine, storage, workflow, or any domain crate.

### D-003: Explicit Scope Model

**Decision**: Use ScopeLevel as first-class model for lifecycle and ownership.

**Rationale**: Enables consistent resource cleanup and access checks; aligns with n8n-class lifecycle needs.

**Rejected**: Implicit scope via thread-local or string keys.

### D-004: Unified Core Error Vocabulary

**Decision**: Keep CoreError broad enough for foundational operations; domain crates have their own errors.

**Rationale**: Shared behavior (serialization, context propagation) needs common error categories without pulling domain semantics into core.

**Rejected**: One global Nebula error type in core — would force core to know all domains.

### D-005: Serde-First Public Types

**Decision**: Public types that cross boundaries implement Serialize/Deserialize.

**Rationale**: API, storage, and workflow definitions depend on stable serialization.

**Rejected**: Keeping some types “internal only” without serde — would block future API/storage use.

---

## Open Proposals

### P-001: Split CoreError into Foundation + Domain Layers (Breaking)

**Problem**: CoreError mixes foundation and domain variants.

**Proposal**: Keep in core only foundation concerns; move domain variants to owning crates.

**Impact**: Breaking for callers that match on moved variants; migration path in PROPOSALS.md.

### P-002: Make Scope Containment Strict and Verifiable (Breaking)

**Problem**: Some containment checks are simplified and do not verify ID ownership.

**Proposal**: Introduce strict containment API with resolver; keep legacy wrapper during migration.

**Impact**: Engine/runtime must provide resolver; breaking for code relying on current semantics.

### P-003: Reduce Core Constant Bloat

**Problem**: constants.rs contains domain-specific defaults.

**Proposal**: Move domain-owned constants to owning crates; keep only cross-cutting constants in core.

**Impact**: Import paths change; re-exports for one cycle.

### P-004: Stable External Schema Policy

**Problem**: Accidental changes to serialized form break clients and migrations.

**Proposal**: Freeze schema for boundary types; add snapshot tests for JSON representation.

**Impact**: Non-breaking if adopted early; prevents future silent breakage.

---

## Non-Negotiables

1. **No domain Nebula dependencies in core** — core may depend only on nebula-log; no engine, storage, workflow, action, etc.
2. **Typed IDs for primary identifiers** — no raw strings for ExecutionId, WorkflowId, NodeId in public API.
3. **Serde for boundary types** — public types that cross crate boundaries must serialize/deserialize.
4. **ScopeLevel is the scope model** — lifecycle and containment go through ScopeLevel and Scoped.
5. **Patch/minor do not break serialized form** — breaking schema changes require major + MIGRATION.md.
6. **CoreError is foundation-only** — domain errors live in owning crates; no new domain variants in core without proposal.
7. **No business logic in core** — core is vocabulary and traits only; no workflow execution, storage, or transport.

---

## Governance

- **PATCH**: Bug fixes, docs, internal refactors. No API or serialization change.
- **MINOR**: Additive only (new types, new optional fields, new trait methods with default). No removal or change of existing contracts.
- **MAJOR**: Breaking changes (removed or changed public API or serialized form). Requires MIGRATION.md and, where applicable, deprecation cycle.

Every PR must verify: no new nebula-* dependency (only nebula-log allowed); new public types have serde where they cross boundaries; no change to existing serialized form in patch/minor.
