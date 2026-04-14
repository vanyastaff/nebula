# nebula-action v2 — Roadmap

> Phased implementation plan with priorities, dependencies, and exit criteria.
> Companion to `2026-04-08-action-v2-spec.md`.

**Date:** 2026-04-08
**Status:** Draft

---

## Phase Overview

| Phase | Name | Status | Effort | Impact | Dependencies |
|-------|------|--------|--------|--------|-------------|
| 0 | Contract Freeze & Cleanup | ✅ Done | S | High | None |
| 1 | Context & Capability Model | ✅ Done | M | Critical | Phase 0 |
| 2a | Derive Macro — Action + Dependencies | 🔲 Next | M | Critical | Phase 1 |
| 2b | Derive Macro — Parameters & Testing DX | 🔲 Next | L | High | Phase 2a |
| 3 | Handler Adapters (all core traits) | 🔲 Planned | M | High | Phase 2a |
| 4 | ActionRegistry v2 (version coexistence) | 🔲 Deferred | M | High | Phase 3 |
| 5 | Testing Infrastructure v2 | 🔲 Planned | M | High | Phase 2a |
| 6 | DX Types — StatefulAction family | 🔲 Planned | M | Medium | Phase 3 |
| 7 | DX Types — TriggerAction family | 🔲 Planned | L | Medium | Phase 3 |
| 8 | DataTag Registry | 🔲 Deferred | M | Medium | Phase 0 |
| 9 | AgentAction (5th core trait) | 🔲 Planned | XL | High | Phase 3 |
| 10 | ErrorCode & ResultActionExt | 🔲 Next | S | Medium | Phase 1 |
| 11 | Port::Provide & Tool System | 🔲 Planned | L | High | Phase 9 |
| 13 | Dynamic Properties | 🔲 Planned | M | Medium | Phase 2b (nebula-parameter) |
| 14 | CostMetrics & OutputMeta | 🔲 Planned | S | Low | Phase 1 |
| 15 | Idempotency & Durable Execution | 🔲 Planned | L | Critical | Phase 3, nebula-storage |

**Effort:** S = 1-2 days, M = 3-5 days, L = 1-2 weeks, XL = 2-4 weeks

---

## Phase 0: Contract Freeze & Cleanup ✅

**Goal:** Lock the stable API surface. Remove ambiguity between current code and aspirational docs.

**Delivered:**
- Core surface locked: `Action`, `ActionMetadata`, `ActionResult`, `ActionOutput`, `ActionError`, ports
- Contract tests in `crates/action/tests/contracts.rs` for serialization stability
- Compatibility policy documented
- Stale terminology cleaned (ProcessAction → StatelessAction)

**Exit criteria:** ✅ All met
- No ambiguity between current API and aspirational design
- Contract tests pass in CI

---

## Phase 1: Context & Capability Model ✅

**Goal:** Production-ready context types with capability injection.

**Delivered:**
- `ActionContext` and `TriggerContext` — concrete structs, not trait objects
- `Context` base trait — `execution_id()`, `node_id()`, `workflow_id()`, `cancellation()`
- 4 core execution traits: `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`
- 5 capability traits: `ResourceAccessor`, `CredentialAccessor`, `ActionLogger`, `TriggerScheduler`, `ExecutionEmitter`
- No-op implementations for all capabilities
- Typed credential access: `credential_typed::<S>(key)` with `AuthScheme` projection
- `NodeContext` removed — clean break

**Exit criteria:** ✅ All met
- Engine/sandbox/runtime can implement the same context contract
- Capability checks map to deterministic `ActionError` variants

---

## Phase 2a: Derive Macro — Action + Dependencies 🔲

**Goal:** `#[derive(Action)]` generates `Action` + `ActionDependencies` impls. Credentials and resources resolved by TYPE, not string. Unblocks Phase 3.

**Deliverables:**
1. **`#[derive(Action)]` proc macro** — generates `Action` trait impl (metadata from attributes) + `ActionDependencies` impl (credentials/resources from attributes)
2. **`#[action(...)]` attribute syntax** — key, name, description, version, credential, resource, isolation
3. **Credential by type** — `#[action(credential = TelegramBotKey)]` and `ctx.credential::<TelegramBotKey>()` returning `CredentialGuard<S>` (requires nebula-credential `AuthScheme` types)
4. **`cargo expand` reference output** — publish expanded output for canonical `#[derive(Action)]` examples so plugin authors can see what the macro generates (C6 — Figma DX feedback)

**Exit criteria:**
- Canonical HTTP request action compiles with `#[derive(Action)]` + `impl StatelessAction`
- `ctx.credential::<TelegramBotKey>()` compiles and returns typed `CredentialGuard<S>`
- Manual registration path documented and tested (no macro alternative)
- `cargo expand` output committed for at least one canonical example
- Existing tests don't break

**Dependencies:** Phase 1 (context types exist), nebula-credential (AuthScheme types)

---

## Phase 2b: Derive Macro — Parameters & Testing DX 🔲

**Goal:** Parameters integration and testing DX improvements. Can run in parallel with Phase 3.

**Deliverables:**
1. **Combined `#[derive(Action, Parameters, Deserialize)]`** — struct IS the input, parameters generated from field attributes
2. **`stateless_fn()` enhancement** — accept `impl Fn` with auto-deduced Input/Output types
3. **`TestContextBuilder` basics** — `with_credential::<T>()`, `minimal()` for zero-config context in simple tests (Phase 2b owns these two methods only)

**Exit criteria:**
- `#[derive(Action, Parameters, Deserialize)]` compiles with correct derive ordering
- `stateless_fn()` deduces Input/Output types from closure signature
- `TestContextBuilder::minimal()` produces a working context with no configuration
- `TestContextBuilder::with_credential::<T>()` injects typed credential

**Dependencies:** Phase 2a

**Risks:**
- Proc macro interaction with `#[derive(Parameters)]` from nebula-parameter — need to ensure derive order doesn't matter
- `type Input = Self` pattern requires struct to be `DeserializeOwned` + `Clone`

---

## Phase 3: Handler Adapters (all core traits) 🔲

**Goal:** Type-erased adapters for all 5 core traits, not just StatelessAction.

**Deliverables:**
1. **`ActionHandler` enum** — 5 variant-specific handler traits (`StatelessHandler`, `StatefulHandler`, `TriggerHandler`, `ResourceHandler`, `AgentHandler` stub)
2. **Type-erased dispatch** — each handler variant manages its own serialization/deserialization
3. **`StatefulHandler`** — manages state init, serialization, Continue/Break loop
4. **`TriggerHandler`** — bridges start/stop lifecycle
5. **`ResourceHandler`** — bridges configure/cleanup
6. **`AgentHandler`** (stub) — placeholder for Phase 9
7. **Adapter validates schema conformance** (serde `from_value`), delegates depth/size limits to API ingress layer per spec section 9.3

**Exit criteria:**
- All 4 implemented core traits can be registered in `ActionRegistry` via `ActionHandler`
- Adapter rejects schema-nonconformant payloads with `ActionError::Validation`
- Round-trip test: typed action → handler → JSON execute → typed result

**Dependencies:** Phase 2a (derive macro for test actions)

---

## Phase 4: ActionRegistry v2 (version coexistence) 🔲 — Deferred to Post-MVP

**Goal:** Multiple versions of the same action coexist. Engine pins version per node.

**Deliverables:**
1. **`VersionedActionKey`** — `(ActionKey, InterfaceVersion)` compound key
2. **`get_versioned(key, version)`** — exact version lookup
3. **`get_latest(key)`** — latest version (for editor/UI only)
4. **Version replacement** — same major.minor overwrites previous registration
5. **Node pinning** — `NodeDefinition.action_version: Option<InterfaceVersion>` — engine uses pinned version during execution, `get_latest()` only in editor

**Exit criteria:**
- Register v1.0 and v2.0 of same action → `get_versioned` returns correct one
- `get_latest` returns v2.0
- Re-registering v1.0 overwrites previous v1.0

**Dependencies:** Phase 3 (all adapters registered)

---

## Phase 5: Testing Infrastructure v2 🔲

**Goal:** Action authors can test any action type with minimal ceremony.

**Deliverables:**
1. **`StatefulTestHarness<A>`** — step through iterations, inspect state, run to completion
2. **`TriggerTestHarness<A>`** — test start/stop lifecycle, capture emitted executions and scheduled delays
3. **`WebhookTestRequest`** builder — construct HTTP requests with HMAC signing for webhook trigger tests
4. **Assertion macros** — `assert_success!`, `assert_branch!`, `assert_continue!`, `assert_break!`, `assert_skip!`, `assert_retryable!`, `assert_fatal!`, `assert_validation_error!`
5. **`TestContextBuilder` extensions** — `.trigger()`, `.build_trigger()`, `.with_resource()`, `.with_input()` (Phase 2b owns `minimal()` and `with_credential::<T>()`)
6. **Feature gate** — `test-support` feature to keep test utilities out of production builds

**Note:** `HttpResource` (C4 — n8n feedback) is a nebula-resource concern, but test harnesses should provide a mock/stub `HttpResource` so action tests can exercise HTTP-dependent actions without real network calls.

**Exit criteria:**
- Each core trait type has a corresponding test harness
- All assertion macros compile and work
- Example tests in docs compile against real API

**Dependencies:** Phase 2a (derive macro for test action definitions)

---

## Phase 6: DX Types — StatefulAction Family 🔲

**Goal:** Convenience traits for common StatefulAction patterns.

**Deliverables:**
1. **`PaginatedAction`** — cursor-driven pagination with auto-progress tracking
2. **`BatchAction`** — process items in fixed-size chunks with per-item error handling
3. **`TransactionalAction`** — saga pattern: `execute_tx()` + `compensate()` with compensation data
4. **`StatefulAction::migrate_state()`** — default method for state schema evolution across action versions (C3 — Airflow feedback). Engine: try deserialize -> on fail, call `migrate_state(raw_json)` -> on `None`, propagate error.

Each has a blanket `impl StatefulAction for T where T: DxTrait` so engine sees only core types.

**Exit criteria:**
- `PaginatedAction` with 3-page test passes
- `BatchAction` processes 100 items in chunks of 10
- `TransactionalAction` stores and retrieves compensation data
- `migrate_state()` round-trip test: v1 state JSON -> v2 deserialization fails -> migration succeeds
- All 3 DX types compile down to core traits (engine never imports DX module)

**Dependencies:** Phase 3 (StatefulHandler), Phase 5 (StatefulTestHarness)

**Risks:**
- Blanket impls may conflict with manual `impl StatefulAction` — need orphan rule analysis

---

## Phase 7: DX Types — TriggerAction Family 🔲

**Goal:** Convenience traits for common TriggerAction patterns.

**Deliverables:**
1. **`WebhookAction`** — register/verify/handle/unregister lifecycle with state persistence. Default impls: `on_activate` → no-op, `on_deactivate` → no-op, `verify_signature` → `Ok(true)`
2. **`PollAction`** — periodic polling with cursor persistence and configurable interval
3. **`EventTrigger`** — event source subscription with auto-reconnect and error policy
4. **`ScheduledTrigger`** — cron/interval scheduling (thin wrapper)

Each has blanket `impl TriggerAction` via framework-generated start/stop methods.

**Exit criteria:**
- `WebhookAction` test: activate → handle request → verify signature → deactivate
- `WebhookAction` with default impls compiles (no-op activate/deactivate, auto-accept signature)
- `PollAction` test: 3 poll cycles with cursor advancement
- `EventTrigger` test: connect → receive 3 events → disconnect on error → reconnect
- All DX types compile down to `TriggerAction` trait

**Dependencies:** Phase 3 (TriggerHandler), Phase 5 (TriggerTestHarness)

---

## Phase 8: DataTag Registry 🔲 — Deferred (designed, not prioritized)

**Goal:** Semantic port type tags for compatibility checking.

**Deliverables:**
1. **`DataTag` type** — string-based tag with hierarchy support
2. **`DataTagRegistry`** — registration, lookup, compatibility check
3. **Core tags** — 9 tags: json, text, number, boolean, array, object, binary, file, stream
4. **Hierarchy rules** — child compatible with parent, `json` accepts all
5. **`ConnectionFilter` integration** — ports can filter connections by tag
6. **Tag validation** — reject invalid tag format at registration

**Exit criteria:**
- `image.svg` is compatible with `image` port
- `json` accepts any tag
- Custom tags register without conflict
- Invalid tag format rejected

**Dependencies:** Phase 0 (port system stable)

---

## Phase 9: AgentAction (5th Core Trait) 🔲

**Goal:** First-class support for autonomous LLM agents.

**Deliverables:**
1. **`AgentAction` trait** — `execute(input, &AgentContext) → AgentOutcome<Output>`
2. **`AgentContext`** — extends ActionContext with budget, usage tracking, available tools
3. **`AgentOutcome<T>`** — `Complete(T)` | `Park { reason, partial }`
4. **`AgentBudget`** — max iterations, tokens, tool calls, duration, cost
5. **`AgentUsage`** — atomic counters for all budget dimensions
6. **`AgentActionAdapter`** — type-erased handler for registry
7. **DX types** (stubs) — `ReActAgent`, `PlanExecuteAgent`, `RouterAgent`, `SupervisorAgent`

**Exit criteria:**
- AgentAction with budget limit: stops when budget exceeded
- Park/resume cycle: park → external input → resume → complete
- DX `ReActAgent` compiles and runs 3-iteration tool loop
- Budget enforcement: atomic counters accurate under concurrent access

**Dependencies:** Phase 3 (adapter infrastructure). DataTag dependency is soft — tool ports can use untyped strings initially; typed DataTag integration added when Phase 8 lands.

**Risks:**
- Tool invocation mechanism depends on engine support for SupportPort resolution

---

## Phase 10: ErrorCode & ResultActionExt 🔲

**Goal:** Semantic error codes and fluent error conversion.

**Deliverables:**
1. **`ErrorCode` enum** — `RateLimited`, `AuthExpired`, `UpstreamTimeout`, `QuotaExhausted`, etc.
2. **Add `code: Option<ErrorCode>` to `ActionError::Retryable` and `Fatal`**
3. **`ResultActionExt` trait** — `.retryable()`, `.fatal()`, `.retryable_with_code()`
4. **`ensure!` macro** — `ensure!(condition, "message")` → `ActionError::Validation`

**Exit criteria:**
- `client.get(url).await.retryable()?` compiles and produces correct error
- Engine can match on `ErrorCode::RateLimited` for smarter retry
- `ensure!` macro produces `ActionError::Validation`

**Dependencies:** Phase 1 (ActionError exists)

---

## Phase 11: Port::Provide & Tool System 🔲

**Goal:** Actions can provide tools/data/resources to agent support ports.

**Deliverables:**
1. **`OutputPort::Provide(ProvidePort)`** variant
2. **`ProvideKind`** — Data, Tool, Resource
3. **`ToolSpec`** — name, description, parameters (JSON Schema), hints
4. **`ToolHints`** — idempotent, read_only, estimated_latency
5. **Engine resolution** — collect Provide(Tool) from connected nodes → inject into `AgentContext.tools`
6. **`#[provide(tool(...))]` attribute** on action struct fields or methods

**Exit criteria:**
- Calculator tool action connects Provide(Tool) port → Agent's Support port
- Agent receives tool in `ctx.available_tools()`
- Agent invokes tool → result flows back
- Multiple tools from multiple providers aggregate correctly

**Dependencies:** Phase 9 (AgentAction + AgentContext)

**Risks:**
- Tool invocation is cross-node — needs engine support for "call back into another node"
- Latency of tool invocation depends on engine scheduling

---

## Phase 13: Dynamic Properties 🔲

**Goal:** UI-driven workflow builders get conditional visibility and dynamic dropdowns.

**Deliverables:**
1. **`VisibilityCondition`** on `ParameterDefinition` — show/hide based on other field values
2. **`OptionsLoader`** — static or dynamic (with refreshers list)
3. **`#[param(visible_when = "field == value")]`** attribute syntax
4. **`#[param(options_loader = dynamic, refreshers = ["auth"])]`** attribute syntax
5. **Runtime API** for option loading — engine calls action with partial form data → returns options list

**Exit criteria:**
- Parameter hidden when condition not met (serialization omits it)
- Dynamic dropdown triggers reload when refresher field changes
- Options loader receives current form values and returns valid options

**Dependencies:** Phase 2b (attribute syntax), primarily a nebula-parameter concern

**Note:** This is cross-crate — main work is in nebula-parameter, but action derives need to support the attributes.

---

## Phase 14: CostMetrics & OutputMeta 🔲

**Goal:** Track AI costs and output metadata per action execution.

**Deliverables:**
1. **`CostMetrics`** struct — input_tokens, output_tokens, model_id, estimated_cost_usd
2. **Optional `cost` field on `ActionResult`** — actions can report cost alongside result
3. **Engine aggregation** — sum costs per execution, per workflow
4. **`OutputMeta`** — origin, timing, cost, cache info on `ActionOutput`

**Exit criteria:**
- AI action returns `ActionResult::success(output).with_cost(metrics)`
- Engine aggregates cost across execution nodes
- Cost visible in execution history

**Dependencies:** Phase 1 (ActionResult exists)

---

## Phase 15: Idempotency & Durable Execution 🔲

**Goal:** Financial-grade idempotency for actions that must not double-execute.

**Deliverables:**
1. **`IdempotencyManager` trait** — `check(key)`, `record(key, result, ttl)`
2. **`IdempotencyKey`** — deterministic from `{execution_id}:{node_id}:{attempt}`
3. **Postgres-backed implementation** — durable, survives restarts
4. **Integration with adapter layer** — automatic check-before-execute for actions that declare `#[action(idempotent)]`
5. **TTL-based cleanup** — expired keys removed automatically

**Exit criteria:**
- Same idempotency key returns cached result without re-executing
- Key survives process restart (Postgres persistence)
- TTL expiration removes stale keys
- Action without `#[action(idempotent)]` skips the check (no overhead)

**Dependencies:** Phase 3 (adapter layer), nebula-storage (Postgres)

---

## Priority Matrix

```
                    HIGH IMPACT
                        │
     Phase 15           │         Phase 2a
     (Idempotency)      │         (Derive — Action)
                        │
     Phase 9            │         Phase 3
     (AgentAction)      │         (Handlers)
                        │
                        │         Phase 2b
                        │         (Derive — Params)
  ──────────────────────┼──────────────────────
     Phase 11           │         Phase 5
     (Provide+Tools)    │         (Testing v2)
                        │
     Phase 4            │         Phase 10
     (Registry v2)      │         (ErrorCode)
                        │
     Phase 13           │         Phase 14
     (Dynamic Props)    │         (CostMetrics)
                        │
     Phase 8            │
     (DataTags—deferred)│
                        │
                    LOW IMPACT
        HIGH EFFORT              LOW EFFORT
```

**MVP = Phase 2a + Phase 3 + Phase 5 + Phase 10**

**Recommended execution order:**
1. Phase 2a + Phase 10 (parallel — two independent tracks)
2. Phase 3 + Phase 2b + Phase 5 (parallel — after Phase 2a)
3. Phase 4 — deferred to post-MVP
4. Phase 6 + Phase 7 (after Phase 3 + Phase 5)
5. Phase 9 → Phase 11 (agent chain — after Phase 3)
6. Phase 15 (when Postgres ready)
7. Phase 13 + Phase 14 (whenever)
8. Phase 8 (DataTags — deferred, not prioritized)

---

## Dependency Graph

```
Phase 0 (done) ──→ Phase 1 (done) ──┬──→ Phase 2a ──┬──→ Phase 3 ──→ Phase 4 (post-MVP)
                                     │               │     │
                                     │               │     ├──→ Phase 6 (DX Stateful)
                                     │               │     ├──→ Phase 7 (DX Trigger)
                                     │               │     ├──→ Phase 15 (Idempotency)
                                     │               │     │
                                     │               │     └──→ Phase 9 (Agent) ──→ Phase 11 (Tools)
                                     │               │          (soft dep on Phase 8 for typed DataTags)
                                     │               │
                                     │               ├──→ Phase 2b ──→ Phase 13 (Dynamic Props)
                                     │               │
                                     │               └──→ Phase 5 (Testing v2)
                                     │
                                     ├──→ Phase 10 (ErrorCode, independent)
                                     └──→ Phase 14 (CostMetrics, independent)

Phase 0 ──→ Phase 8 (DataTags — deferred, not prioritized)
```

---

## Cross-Crate Dependencies

| Phase | Requires from other crates |
|-------|---------------------------|
| Phase 2a | nebula-credential for `AuthScheme` types |
| Phase 2b | nebula-parameter `HasParameters` trait |
| Phase 7 | nebula-webhook for `WebhookAction` integration |
| Phase 9 | nebula-engine support for SupportPort resolution |
| Phase 11 | nebula-engine cross-node tool invocation |
| Phase 13 | nebula-parameter `ParameterDefinition` extensions |
| Phase 15 | nebula-storage Postgres implementation |

---

## Risk Register

| Risk | Impact | Likelihood | Mitigation |
|------|--------|-----------|------------|
| Derive macro interaction with Parameters derive | M | Medium | Test combined derives early in Phase 2b |
| Derive macro ordering sensitivity (`#[derive(Action, Parameters, Deserialize)]`) | M | High | Test all derive orderings in Phase 2b; document canonical order |
| Blanket impl coherence across DX types | H | Medium | Try sealed marker traits first, fallback to newtypes |
| Phase 2 as single-point-of-failure blocking all downstream | H | Low | Mitigated by 2a/2b split — Phase 3 unblocked by 2a alone |
| AgentContext delegation burden | M | Medium | Mitigated by `Deref` to `ActionContext` |
| AgentAction budget enforcement edge cases | M | Low | Hard-stop via CancellationToken decided; edge cases handled by partial output preservation |
| Dynamic properties requires nebula-parameter breaking changes | M | Medium | Feature-gate new parameter attributes |
| Postgres storage not ready for Phase 15 | H | Medium | Phase 15 waits for nebula-storage; MemoryStorage for testing |
