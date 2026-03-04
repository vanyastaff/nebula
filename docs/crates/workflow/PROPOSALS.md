# Proposals

## P-001: Schema Snapshot Tests

**Type:** Non-breaking

**Motivation:** Lock JSON shape of WorkflowDefinition and related types for API and storage compatibility; catch accidental breaking changes in CI.

**Proposal:** Add JSON fixtures (or schema snapshot) in repo; CI asserts roundtrip and optional schema diff. Document in ROADMAP Phase 2.

**Expected benefits:** No accidental breaking change to serialized form in patch/minor.

**Costs:** Maintain fixtures when schema evolves (minor additive changes).

**Risks:** Fixtures drift if not updated on schema change.

**Compatibility impact:** None.

**Status:** Draft

---

## P-002: Version Field for Workflow Definitions

**Type:** Non-breaking

**Motivation:** Support multiple schema versions for stored workflows; allow future migration path.

**Proposal:** Formalize version field semantics (e.g. definition schema version vs workflow revision); document in API.md. Optional: version in WorkflowDefinition used by storage/API for compatibility checks.

**Expected benefits:** Clear upgrade path when definition schema changes.

**Costs:** Need to define version semantics and migration policy.

**Risks:** Over-engineering if single version is enough for now.

**Compatibility impact:** Additive only if version is optional.

**Status:** Draft

---

## P-003: Optional nebula-validator Integration

**Type:** Non-breaking

**Motivation:** Reuse validator combinators for parameter or field-level rules without duplicating logic.

**Proposal:** Optional dependency on nebula-validator; validate_workflow or node validation can delegate to composable validators where useful.

**Expected benefits:** Consistent validation patterns across platform; less custom code in workflow.

**Costs:** New dependency; integration surface to maintain.

**Risks:** Validator crate API changes could affect workflow.

**Compatibility impact:** None if behind feature or optional path.

**Status:** Defer
