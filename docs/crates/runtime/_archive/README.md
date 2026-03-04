# nebula-runtime — Archive

Pre-SPEC and legacy notes for `nebula-runtime`. **Mine for insights; do not delete.** See root `docs/SPEC.md` and `docs/docs-deep-pass-checklist.md`.

**Runtime** is the action execution layer (ActionRuntime, ActionRegistry, DataPassingPolicy). **Depends on:** core, action, plugin, telemetry. **Used by:** engine. Sandbox and queue are in this crate.

## Mapping to current docs and boundaries

| Archive concept | Current home | Notes |
|-----------------|--------------|--------|
| **execute_action(action_id, context)** | This crate (runtime) | Context is **NodeContext** today; target **ActionContext** (see INTERACTIONS, CONSTITUTION P-001). Archive pseudocode often uses ActionContext — that is the target. |
| **ActionRuntime + Sandbox + isolation** | This crate | resolve_isolation_level, SandboxedContext — ROADMAP Phase 1; sandbox in nebula-runtime. |
| **WorkflowEngine, Scheduler, Executor** | **nebula-engine** + **nebula-execution** | Split: engine = orchestration/scheduling; execution = state machine, plan, journal. Not in runtime. |
| **TriggerManager, TriggerAction lifecycle** | ROADMAP Phase 2; trigger *types* in **nebula-action** | TriggerAction (start/stop) in action crate; runtime or engine runs lifecycle. Archive `from-archive/.../nebula-runtime.md` = broad “runtime” (triggers + coordination); current crate = action execution only. |
| **WorkflowCoordinator, multi-runtime** | ROADMAP Phase 3 | Coordination and runtime discovery; optional. |
| **ResourceRuntime, MemoryManager** | nebula-resource, nebula-memory | Not in current runtime deps; resource injection is PROPOSALS P003. |
| **Dependencies (sandbox, resource, memory, resilience)** | **nebula-plugin** (InternalHandler); sandbox in this crate | Current deps: action, plugin, telemetry, metrics. Sandbox (SandboxRunner, InProcessSandbox) and queue (TaskQueue, MemoryQueue) in nebula-runtime. |

**Contents:**
- `archive-node-execution.md` — ActionRuntime, ResourceRuntime, isolation levels
- `archive-crates-dependencies.md` — Cargo dependencies (legacy list)
- `archive-layers-interaction.md` — sandbox ↔ runtime ↔ action; capability checks; **ActionContext** in signature (target)
- `archive-crates-architecture.md` — WorkflowEngine, Executor, ExecutionContext (legacy; now engine + execution)
- `archive-nebula-complete.md` — Week 7 plan: Runtime Core, Trigger Management, Event Processing
- `from-archive/nebula-complete-docs-part3/nebula-runtime.md` — target architecture (TriggerManager, EventProcessor, coordination)
- `from-core-full/archive-nebula-complete.full.sections.md` — section index

Rules: do not delete; keep filenames descriptive; reference from active docs when needed.
