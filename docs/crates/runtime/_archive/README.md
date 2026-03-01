# nebula-runtime — Archive

Pre-SPEC and legacy notes for `nebula-runtime`. **Mine for insights; do not delete.** See root `docs/SPEC.md` and `docs/docs-deep-pass-checklist.md`.

**Runtime** is the action execution layer (ActionRuntime, ActionRegistry, DataPassingPolicy). **Depends on:** core, action, plugin, ports, telemetry. **Used by:** engine.

**Contents:**
- `archive-node-execution.md` — ActionRuntime, ResourceRuntime, isolation levels
- `archive-crates-dependencies.md` — Cargo dependencies
- `archive-layers-interaction.md` — sandbox ↔ runtime ↔ action; capability checks
- `archive-crates-architecture.md` — WorkflowEngine, Executor, ExecutionContext (legacy layout)
- `archive-nebula-complete.md` — Week 7 plan: Runtime Core, Trigger Management, Event Processing
- `from-archive/nebula-complete-docs-part3/nebula-runtime.md` — target architecture (TriggerManager, EventProcessor)
- `from-core-full/archive-nebula-complete.full.sections.md` — section index

Rules: do not delete; keep filenames descriptive; reference from active docs when needed.
