# nebula-sdk Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Action and plugin authors need one place to get core types, macros, builders, and test utilities without depending on every crate individually. The SDK is the developer-facing facade: prelude, re-exports, NodeBuilder, TriggerBuilder, TestContext, MockExecution, and optional proc-macros (via nebula-macros).

**nebula-sdk is the all-in-one developer toolkit for building custom nodes.**

It answers: *What does an action author import and use to define a node, parameters, and execution — with minimal boilerplate and stable prelude?*

```
Action author → use nebula_sdk::prelude::*
    ↓
Core types (ids, traits), serde_json::Value, macros (node, action), builders, TestContext/MockExecution
    ↓
Single dependency; stable re-export surface
```

This is the SDK contract: prelude and re-exports are stable; new crates are added to prelude only when they become part of the authoring contract; breaking prelude = major.

---

## User Stories

### Story 1 — Action Author Uses Prelude (P1)

Author adds nebula-sdk; uses prelude to get core, Value, and common types. They do not need to depend on nebula-core, nebula-action, etc. directly for basic authoring.

**Acceptance**: Prelude re-exports documented; one line brings in ids, traits, Value, optional macros and builders. Minor = additive re-exports only.

### Story 2 — Author Uses Builders or Macros (P1)

Author uses NodeBuilder/TriggerBuilder or derive(Action, Parameters) to define node. SDK (and nebula-macros) provide the API. Same node runs in engine/runtime.

**Acceptance**: Builder and macro path documented; output is compatible with nebula-action contract. Breaking builder or macro output = major.

### Story 3 — Author Tests With TestContext/MockExecution (P2)

Author writes unit tests with TestContext and MockExecution from SDK. No need to spin up engine. Contract: test utilities stay in sync with action/runtime expectations.

**Acceptance**: TestContext and MockExecution documented; minor = additive helpers only.

---

## Core Principles

### I. SDK Is a Facade, Not New Domain Logic

**SDK re-exports and composes; it does not implement workflow engine, storage, or credential logic.**

**Rationale**: Single place for authoring deps; domain stays in core, action, runtime.

### II. Prelude and Re-Exports Are Versioned

**Prelude content is stable. Additive re-exports in minor; removal or signature change in major.**

**Rationale**: Authors depend on prelude; breaking it breaks all plugins.

### III. Optional Macros and Builders

**Macros (nebula-macros) and builders are optional so that minimal authors can depend only on what they need.**

**Rationale**: Some authors want only types; others want full DX. Feature flags or optional deps keep baseline small.

### IV. No Orchestration or Runtime in SDK

**SDK does not run workflows or schedule nodes. It provides types and authoring tools.**

**Rationale**: Engine and runtime own execution; SDK owns authoring DX.

---

## Production Vision

In production, plugin and action authors depend on nebula-sdk. Prelude brings in core, action types, Value, and optional macros/builders. TestContext and MockExecution allow testing without engine. SDK version tracks platform compatibility; prelude and re-export surface are stable. From archives: developer toolkit, prelude, NodeBuilder, TriggerBuilder, testing utilities. Gaps: formal prelude stability policy; macro/output compatibility tests.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Prelude stability and compatibility tests | High | Lock re-exports for minor |
| Macro/output compatibility with action contract | Medium | Ensure derive output works with engine |
| Document minimal vs full SDK deps | Low | Feature flags or optional deps |

---

## Key Decisions

### D-001: Single Prelude

**Decision**: One prelude re-exports core, Value, and common authoring types/macros.

**Rationale**: One import for authors; fewer version conflicts.

**Rejected**: Multiple preludes per layer — would confuse authors.

### D-002: Test Utilities in SDK

**Decision**: TestContext and MockExecution live in SDK so authors can test without engine dependency.

**Rationale**: Lowers barrier to testing; engine stays optional for unit tests.

**Rejected**: Test utilities only in engine — would force authors to depend on engine for tests.

---

## Non-Negotiables

1. **SDK is a facade** — re-exports and authoring tools only; no domain logic.
2. **Prelude is stable** — additive in minor; breaking in major.
3. **No orchestration in SDK** — engine and runtime run; SDK only authoring.
4. **Breaking prelude or authoring contract = major + MIGRATION.md**.

---

## Governance

- **PATCH**: Bug fixes, docs. No prelude or re-export change.
- **MINOR**: Additive re-exports and helpers. No removal.
- **MAJOR**: Breaking prelude or builder/macro contract. Requires MIGRATION.md.
