# Proposals

## P-001: Formal Prelude Stability Policy

**Type:** Non-breaking

**Motivation:** Authors depend on prelude; need clear rules for what can change in minor vs major.

**Proposal:** Document in API.md: list of re-exports; minor = additive only; removal or signature change = major. Add CI check that prelude content is documented.

**Expected benefits:** Predictable upgrades for authors.

**Costs:** Maintain doc and possibly tooling.

**Status:** Draft

---

## P-002: Macro/Output Compatibility Tests

**Type:** Non-breaking

**Motivation:** Ensure derive and builder output always work with current action/runtime contract.

**Proposal:** Contract test: build node with sdk (derive or builder), pass to engine or runtime test harness; assert execution succeeds. Run in CI when action or sdk changes.

**Expected benefits:** No silent breakage between sdk and action.

**Costs:** CI dependency between sdk and engine/runtime.

**Status:** Draft

---

## P-003: Optional Codegen and Dev-Server

**Type:** Non-breaking (additive)

**Motivation:** Phase 4 DX: OpenAPI spec generation, hot-reload dev server.

**Proposal:** Feature flags codegen, dev-server; implement behind flags; document as optional. Keep default features minimal.

**Expected benefits:** Richer DX without bloating default dependency.

**Costs:** Maintain optional features.

**Status:** Defer (ROADMAP Phase 3)
