# nebula-tenant Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula may serve multiple tenants (organizations, workspaces). Each tenant has isolated resources, credentials, and quotas. Runtime, resource, and storage need a single authoritative tenant context so that scope and quota enforcement are consistent.

**nebula-tenant is the tenant isolation and quota-management layer for Nebula.**

It answers: *What is tenant identity, how is tenant context resolved (e.g. from request or execution), and how are isolation and quota enforced across runtime, resource, and storage?*

```
Request or execution carries tenant_id (or resolution key)
    ↓
Tenant context resolved: identity, isolation strategy (shared/dedicated/isolated), quota policy
    ↓
Runtime/resource/storage enforce scope and quota using tenant context
```

This is the tenant contract: single authoritative tenant context; deterministic isolation and quota; auditable policy decisions. Crate is planned; not yet implemented.

---

## User Stories

### Story 1 — API Resolves Tenant from Request (P1)

API receives request (e.g. JWT or API key). It resolves tenant_id and attaches TenantContext to execution or request scope. Engine and runtime use this context for resource and credential scope.

**Acceptance**:
- TenantContext (or equivalent) is the contract type
- Resolution (from JWT, header, or config) is documented; tenant crate may own resolution or delegate to api
- No tenant_id in execution without going through tenant context

### Story 2 — Resource and Credential Are Scoped by Tenant (P1)

Resource manager and credential manager receive tenant context. They only return resources/credentials that belong to that tenant. Cross-tenant access is denied and auditable.

**Acceptance**:
- Scope includes tenant (e.g. ScopeLevel::Tenant(tenant_id))
- Resource and credential crates enforce scope; tenant crate defines context and policy
- Violation is explicit error and logged

### Story 3 — Quota Is Enforced Before Execution (P2)

Before starting a workflow run, quota (e.g. max concurrent executions per tenant) is checked. If over quota, request is rejected with clear error. Policy is configurable per tenant.

**Acceptance**:
- Quota policy (concurrent runs, storage, etc.) is data; enforcement is deterministic
- Check is before execution start; no partial start then fail
- Auditable: quota check result and reason logged

### Story 4 — Isolation Strategy Is Explicit (P2)

Tenant can be shared (same process, logical isolation), dedicated (dedicated pool or process), or isolated (hard isolation). Strategy is part of tenant config; runtime and resource respect it.

**Acceptance**:
- Isolation strategy is enum or config; documented semantics
- Runtime/resource use strategy to decide pooling and cleanup
- No cross-tenant leakage by default

---

## Core Principles

### I. Single Authoritative Tenant Context

**TenantContext (or equivalent) is the one contract that runtime, resource, storage, and credential use. No ad-hoc tenant_id passing.**

**Rationale**: Scattered tenant semantics cause leaks and inconsistent enforcement. One context type ensures consistent scope.

**Rules**:
- TenantContext carries tenant_id and optional policy (quota, isolation)
- All scoped operations receive TenantContext from resolution layer
- Tenant crate owns or defines this type; others consume it

### II. Deterministic Quota and Isolation

**Quota enforcement and isolation boundaries are deterministic for the same context and config.**

**Rationale**: Operators need predictable behavior. Non-determinism causes "sometimes over quota" bugs.

**Rules**:
- Quota check is a function of (tenant_id, policy, current usage)
- Isolation strategy has defined behavior (shared vs dedicated vs isolated)
- No implicit or environment-dependent behavior

### III. Auditable Policy Decisions

**Tenant resolution, quota check, and scope violation are auditable (log or event) so that security and ops can trace decisions.**

**Rationale**: Multi-tenant systems need audit trails for compliance and debugging.

**Rules**:
- Resolution result (tenant_id, strategy) is loggable
- Quota pass/reject and reason are loggable
- Scope violation is explicit error and logged
- No secret material in audit log

### IV. No Storage or Credential Implementation in Tenant Crate

**Tenant crate defines context and policy. It does not implement storage or credential backends.**

**Rationale**: Storage and credential crates own their backends. Tenant only provides context and policy contract.

**Rules**:
- Tenant crate may depend on core; not on storage/credential for implementation
- Resource and credential crates enforce scope using tenant context
- Quota enforcement may call into storage or a dedicated quota store (trait)

### V. Additive Policy in Minor, Semantics Change in Major

**New quota types, new isolation options, and new context fields are additive in minor. Changing isolation or ownership semantics is major.**

**Rationale**: Operators add new policies without breakage. Semantic changes require migration.

**Rules**:
- Minor: new policy fields, new strategy options
- Major: change in scope semantics or quota semantics; MIGRATION.md

---

## Production Vision

### The tenant layer in an n8n-class fleet

In production, every request and execution has a TenantContext. API resolves tenant from JWT or API key; engine and runtime pass context to resource and credential managers. Quota is checked before execution start. Isolation strategy (shared/dedicated/isolated) determines pooling and cleanup. All policy decisions are auditable.

```
TenantContext
    ├── tenant_id
    ├── isolation: Shared | Dedicated | Isolated
    ├── quota_policy (concurrent, storage, ...)
    └── resolution_metadata (for audit)

Runtime/Resource/Credential use TenantContext for scope and quota
```

From the archives: tenant isolation and quota intent; single authoritative contract. Production vision: deterministic enforcement, auditable decisions, no cross-tenant leakage.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Implement crates/tenant | Critical | Currently planned only |
| Tenant context type and resolution API | High | Define and adopt across api, engine, runtime |
| Quota store and enforcement hook | High | Where is usage stored? Trait or storage crate |
| Isolation strategy implementation in resource/runtime | High | Resource/runtime must respect strategy |
| Cross-crate policy alignment | Medium | Ensure resource/credential/storage use same context |

---

## Key Decisions

### D-001: Tenant Crate Owns Context Type

**Decision**: TenantContext (or equivalent) is defined in tenant crate; api, engine, resource, credential consume it.

**Rationale**: Single source of truth. No duplicate definitions.

**Rejected**: Each crate defining its own tenant_id type — would fragment.

### D-002: Resolution Outside or Inside Tenant Crate

**Decision**: Tenant resolution (request → tenant_id) can live in api crate or tenant crate; contract (TenantContext) is in tenant crate.

**Rationale**: API has request; tenant has identity and policy. Clear handoff.

**Rejected**: Resolution in storage crate — would mix transport and identity.

### D-003: Quota Enforcement Before Execution

**Decision**: Quota is checked before engine starts execution; reject with clear error if over quota.

**Rationale**: Prevents over-commit. No partial start.

**Rejected**: Check after start — would allow burst over quota.

### D-004: Isolation as Strategy, Not Implementation

**Decision**: Tenant crate defines isolation strategy (shared/dedicated/isolated) as policy; runtime and resource implement behavior (e.g. dedicated pool per tenant).

**Rationale**: Tenant owns policy; runtime/resource own mechanism.

**Rejected**: Tenant crate implementing pools — would duplicate resource crate.

---

## Non-Negotiables

1. **Single authoritative tenant context** — one type used by runtime, resource, storage, credential.
2. **Deterministic quota and isolation** — same context and config ⇒ same outcome.
3. **Auditable policy decisions** — resolution, quota, violation logged; no secrets in logs.
4. **No storage/credential implementation in tenant crate** — context and policy only.
5. **Breaking isolation or quota semantics = major + MIGRATION.md** — operators depend on behavior.

---

## Governance

- **PATCH**: Bug fixes, docs. No context or policy change.
- **MINOR**: Additive (new policy fields, new strategy). No semantics change.
- **MAJOR**: Breaking context or enforcement semantics. Requires MIGRATION.md.
