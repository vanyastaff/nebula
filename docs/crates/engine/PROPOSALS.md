# Proposals

## P-001: Trigger Lifecycle in Engine

**Type:** Non-breaking (additive)

**Motivation:** Trigger-based workflows need register/unregister and start/stop; someone must own trigger lifecycle.

**Proposal:** Engine (or dedicated service) owns trigger lifecycle; action crate defines TriggerContext and trigger types; engine or runtime executes triggers like any action.

**Expected benefits:** Clear ownership; webhook/schedule triggers can start workflows.

**Costs:** New engine API; possible new crate for trigger registry.

**Risks:** Coordination complexity between engine, runtime, and action.

**Compatibility impact:** Additive; existing execute_workflow unchanged.

**Status:** Review (see CONSTITUTION)

---

## P-002: Backpressure and Admission

**Type:** Non-breaking (additive)

**Motivation:** Under load, engine should reject or queue new executions based on system/memory pressure.

**Proposal:** Integrate with nebula-system pressure events; optional admission controller that gates execute_workflow (reject or queue).

**Expected benefits:** Protect process from overload; predictable degradation.

**Costs:** Engine subscribes to pressure; policy (reject vs queue) to define.

**Risks:** False positives or wrong policy hurting availability.

**Compatibility impact:** Additive; optional integration.

**Status:** Draft
