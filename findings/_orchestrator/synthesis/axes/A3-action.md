# A3 — Action / Node Abstraction: Compact Cross-Project Analysis

**Strategic verdict for Nebula**: Nebula's 5 sealed action kinds (Process / Supply / Trigger / Event / Schedule) with associated `Input`/`Output`/`Error` types is **architecturally unique** — no competitor has a kinded sealed taxonomy. Industry default is **single open trait + `serde_json::Value` I/O**.

## Trait shape distribution

| Pattern | Projects | Count |
|---------|----------|------:|
| Single open trait + `serde_json::Value` (or equivalent type-erased) I/O | acts, acts-next, runner_q, dataflow-rs, runtara, fluxus, flowlang, rust-rule-engine, dag_exec, orchestral, rayclaw, aofctl, cloudllm, deltaflow, runner_q | ~15/27 |
| Two-trait split (workflow vs activity) | temporalio-sdk (Workflow + Activity), duroxide (OrchestrationHandler + ActivityHandler), durable-lambda-core (closures + ops trait) | 3/27 |
| Closure-based (no trait) | dagx, dag_exec, raftoral (`WorkflowFunction<I,O>`), tianshu (coroutine-replay) | 4/27 |
| Variant enum | ebi_bpmn (22-variant `BPMNElement`), rust-rule-engine (10-variant `ActionType`), kotoba | 3/27 |
| Pipeline/Stage trait | orka (Pipeline<TData,Err>), treadle (Stage), aqueducts-utils (Stage) | 3/27 |
| **Sealed kinded taxonomy with assoc types** | **Nebula (5 kinds)** | **1/27** |

## Type erasure prevalence

| I/O typing | Projects |
|------------|----------|
| Type-erased (`serde_json::Value` / `DataObject` / `Vars`) | majority — about 18/27 |
| Generic associated types | dagx (TaskHandle<T>), durable-lambda-core (RPITIT), deltaflow (Step assoc Input/Output), orka (Pipeline<TData,Err>), tianshu (sort-of via coroutine context) |
| **Per-kind sealed assoc types** | **Nebula** — unique |

## Versioning

| Pattern | Projects | Comment |
|---------|----------|---------|
| `(name, version)` dispatch | duroxide (BTreeMap<semver::Version, Arc<dyn H>>), raftoral ((workflow_type: String, version: u32)) | versioning at runtime registry level |
| Type identity (no version field) | Nebula | trait identity = version |
| No versioning | most others | newer version = code change |

## Lifecycle hooks

| Hooks | Projects | Comment |
|-------|----------|---------|
| pre/execute/post/cleanup | acts (`prepare`/`process`/`complete`), partial in others | rare full coverage |
| execute only | most projects | minimal contract |
| Cancellation point | implicit via async cancel | rarely explicit |
| Idempotency key | runner_q (`OnDuplicate` enum: AllowReuse/ReturnExisting/AllowReuseOnFailure/NoReuse) | best in class |

## Resource & credential dependencies

| Mechanism | Projects | Comment |
|-----------|----------|---------|
| **Action declares deps via assoc types** | **Nebula** | unique — compile-time check |
| Constructor inject | most projects | runtime-only |
| Closure capture | dagx, raftoral, tianshu | runtime-only |
| No mechanism | many | user manages everything |

## Retry / resilience attachment

| Pattern | Projects | Comment |
|---------|----------|---------|
| Per-action policy via attribute / config | runner_q (`#[derive(...)]` config), aofctl (yaml config) | most flexible |
| Workflow-level only | most projects | coarse-grained |
| Server-owned policy | temporalio-sdk (RetryPolicy proto), durable-lambda-core (AWS-side) | distributed-style |
| Per-class `Retryable`/`Permanent` enum return | runner_q (`ActivityError`), deltaflow (`StepError`), rayclaw (5-cat LLM error) | clean DX |
| Nebula | per-action via metadata + workflow-level override | combination |

## Authoring DX

| "Hello world" line count (approximate) | Projects | Mechanism |
|---|---|---|
| ~3-5 lines | dagx (typestate builder), runtara (`#[resilient]` macro) | macro / builder |
| ~5-8 lines | acts (`inventory::submit!`), Nebula (`#[derive(Action)]` + impl) | derive macro |
| ~8-15 lines | runner_q, aofctl, most others | manual `#[async_trait]` impl |
| ~15-25 lines | rayclaw (Tool trait with metadata + safety + execute) | manual with metadata |

## Notable competitor patterns worth borrowing

1. **`OnDuplicate` enum (runner_q)** — explicit idempotency semantics: `AllowReuse` / `ReturnExisting` / `AllowReuseOnFailure` / `NoReuse`. Cleaner than ad-hoc `idempotency_key: Option<String>`. **Borrow effort**: 1 week. **Strategic value**: ⭐⭐ — closes API ergonomics gap for trigger-action idempotency.

2. **`(name, version)` dispatch (duroxide, raftoral)** — runtime versioned registry with capability-filtered dispatch via SemverRange. Useful for live workflow migration. **Borrow effort**: 2-3 weeks. **Strategic value**: ⭐⭐⭐ — enables zero-downtime action migration. Worth ADR before adopting.

3. **`inventory::submit!` for built-in registration (acts, runtara)** — eliminates manual registration boilerplate at engine init. Compatible with Nebula's plugin-v2 spec for third-party plugins (built-ins use inventory; third-party use WASM). **Borrow effort**: 1 week. **Strategic value**: ⭐ — boilerplate reduction.

4. **Two-trait split for AI activities (duroxide proposal)** — separate `LlmActivity` trait that records `LlmRequested`/`LlmCompleted` history events for replay-safe LLM. Doesn't require a 6th sealed kind in Nebula; can be a marker trait extending `ProcessAction`. **Borrow effort**: see A21 axis file. **Strategic value**: ⭐⭐⭐.

5. **Stage / QualityGate / ReviewPolicy split (treadle v2 design)** — separates "does work" from "judges quality" from "decides next action". Specifically designed for LLM pipeline patterns. Could be exposed as a higher-level pattern on top of Nebula's existing actions, not a new action kind. **Borrow effort**: 2-3 weeks if implemented. **Strategic value**: ⭐⭐ for AI pipeline use cases.

## What NOT to borrow

- **Type-erased I/O** (16+ projects use `serde_json::Value`). Tempting for plugin SDK simplicity; would erode Nebula's TypeDAG L1 compile-time port safety.
- **Single open trait** (most projects). Tempting for plugin SDK ergonomics; would erode the Process/Supply/Trigger/Event/Schedule kinded taxonomy that maps to user mental models.
- **Closure-only API** (dagx, raftoral). Compact for libraries; loses the metadata surface (display name, icon, description) that Nebula needs for visual editor.

## Verdict

Nebula's 5-kinded sealed taxonomy + assoc types is the **deepest** A3 design observed. The trade-off (more verbose authoring vs runner_q/acts simplicity) is justified by:
- Visual editor metadata requirements (no-code/low-code mode)
- Compile-time port type safety (TypeDAG L1)
- Plugin Fund commercial requirements (typed contracts for paid plugins)

**Recommendation**: hold the line on the kinded taxonomy. Adopt selective patterns from competitors (idempotency enum, versioned registry, inventory pattern for built-ins) without diluting the core design.
