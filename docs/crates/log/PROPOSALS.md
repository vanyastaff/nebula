# Proposals

## P001: Hook Execution Isolation with Budget

**Type:** Breaking (runtime behavior)

**Motivation:** Hooks execute inline during `emit_event`; slow hooks increase tail latency.

**Proposal:** Introduce optional hook budget policy:
- v1 (implemented): max execution time budget per hook with over-budget diagnostics
- v2 (deferred): async offload queue with explicit drop/shed strategy and counters

**Expected benefits:** Better visibility of slow hooks now; bounded-latency controls later.

**Costs:** New policy surface; deferred async queue adds future complexity.

**Risks:** Hook order/timing changes; possible event drops.

**Compatibility impact:** Default remains `Inline`; current opt-in `Bounded` mode is non-breaking.

**Status:** Partially Implemented (v1 in place, async offload deferred)

---

## P002: Typed Event Names Instead of Free Strings

**Type:** Breaking (API)

**Motivation:** String event names are typo-prone and hard to refactor.

**Proposal:** Introduce typed event identifiers (`EventKind` enum or interned key type).

**Expected benefits:** Type safety; easier refactoring.

**Costs:** Custom hook/event implementations need migration.

**Risks:** Dual API maintenance during deprecation.

**Compatibility impact:** Add dual API; deprecate string-only trait method; remove in next major.

**Status:** Draft

---

## P003: Context ID Types from nebula-core

**Type:** Potential Breaking

**Motivation:** Execution/node/workflow IDs in observability contexts are plain `String`.

**Proposal:** Migrate to typed IDs from `nebula-core` where feasible.

**Expected benefits:** Type safety; consistency with core.

**Costs:** Signature changes; possible new dependency on core.

**Risks:** Log crate may need to depend on core (currently it does not).

**Compatibility impact:** Add typed constructor variants; deprecate string-only constructors.

**Status:** Draft

---

## P004: Writer Multi-Fanout with Explicit Failure Policy

**Type:** Non-breaking

**Motivation:** Needed deterministic multi-destination delivery behavior.

**Proposal:** Implement true fanout with policy: `FailFast`, `BestEffort`, `PrimaryWithFallback`.

**Expected benefits:** Multi-destination logging; configurable failure behavior.

**Costs:** Implementation complexity.

**Risks:** Low; additive config.

**Compatibility impact:** None if behind enriched config; default preserves current behavior.

**Status:** Implemented

---

## P005: Config Schema Stability Contract

**Type:** Non-breaking

**Motivation:** Deployment configs depend on serde schema stability.

**Proposal:** Version config schema; add snapshot tests for config serialization/deserialization.

**Expected benefits:** Prevents accidental breaking config changes.

**Costs:** Maintenance of snapshot files.

**Risks:** Low.

**Compatibility impact:** None.

**Status:** Review
