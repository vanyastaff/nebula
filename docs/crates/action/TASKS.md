# Tasks: nebula-action

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix ACT

---

## Phase 1: Contract freeze and cleanup

**Goal**: Lock stable API surface, remove stale terminology.

- [x] ACT-T001 [P] Lock current stable surface (Action, metadata, components, result/output/error/ports)
- [x] ACT-T002 [P] Write contract tests in `crates/action/tests/contracts.rs` for ActionOutput and FlowKind
- [x] ACT-T003 [P] Add compatibility policy in `COMPATIBILITY.md`
- [x] ACT-T004 Audit all docs and examples for stale terminology (StatelessAction vs ProcessAction) and update
- [x] ACT-T005 Verify contract tests cover serialization roundtrip for all ActionResult variants

**Checkpoint**: No aspirational types remain in docs or code comments. Contract tests green. `COMPATIBILITY.md` published.

---

## Phase 2: Context and capability model

**Goal**: Replace NodeContext with ActionContext/TriggerContext; establish capability model.

- [x] ACT-T006 Add `ActionContext` and `TriggerContext` concrete structs
- [x] ACT-T007 ~~Deprecate `NodeContext`~~ **Removed `NodeContext` entirely** (breaking change — nothing uses it externally)
- [x] ACT-T008 [P] Define core execution traits: StatelessAction, StatefulAction, TriggerAction, ResourceAction
- [x] ACT-T008b Implement `StatelessActionAdapter<A>` bridging `StatelessAction` → `dyn InternalHandler` (was missing — P1 fix)
- [x] ACT-T008c Add `ActionRegistry::register_stateless()` helper in nebula-runtime
- [x] ACT-T009 Context trait provides `execution_id()`, `node_id()`, `workflow_id()`, `cancellation()` methods ✅ already done
- [x] ACT-T010 [P] Define capability module interfaces: ResourceAccessor, CredentialAccessor, ActionLogger
- [x] ACT-T011 [P] Add capability fields to ActionContext (resources, credentials, logger)
- [x] ACT-T012 Add capability fields to TriggerContext (scheduler, emitter, credentials, logger)
- [x] ACT-T013 Write tests verifying engine/sandbox/runtime can construct and use both context types

**Checkpoint**: NodeContext removed. ActionContext and TriggerContext carry all capability modules. Context trait provides identity and cancellation methods.

> **Remaining gap**: ActionContext has no resource/credential injection. Engine engine.rs has TODOs at the call sites (lines ~427, ~536). Capability modules (ACT-T010–T013) must be completed before engine wiring.

---

## Phase 3: Deferred and streaming hardening

**Goal**: Lock deferred/streaming resolution behavior.

- [x] ACT-T014 [P] Document deferred output resolution contract (who resolves, when, persistence)
- [x] ACT-T015 [P] Document streaming output backpressure semantics and consumer contract
- [ ] ACT-T016 Define compatibility matrix: which ActionOutput variants downstream nodes can consume
- [ ] ACT-T017 Add resume/recovery scenario tests for deferred ActionOutput
- [ ] ACT-T018 Add streaming backpressure integration test (bounded channel, slow consumer)

**Checkpoint**: Deferred and streaming outputs have fully specified resolution behavior. Backpressure semantics tested.

---

## Phase 4: Port and metadata governance

**Goal**: Freeze port schema, add validation tooling.

- [ ] ACT-T019 [P] Freeze dynamic port schema semantics in code and docs
- [ ] ACT-T020 [P] Freeze support port schema semantics in code and docs
- [ ] ACT-T021 Add metadata version compatibility check (breaking change detection)
- [ ] ACT-T022 Build validation function for action packages (validate metadata + ports + components)
- [ ] ACT-T023 Write migration guide template for action version bumps

**Checkpoint**: Port schemas are frozen. CI can validate action packages. Migration guide exists.

---

## Phase 5: Ecosystem and DX rollout

**Goal**: External authors can build action nodes with predictable behavior.

- [ ] ACT-T024 [P] Publish end-to-end example: stateless action with runtime
- [ ] ACT-T025 [P] Publish end-to-end example: stateful action with state management
- [ ] ACT-T026 [P] Publish end-to-end example: trigger action (webhook)
- [ ] ACT-T027 Define error-to-retry mapping patterns and document recommended conventions
- [ ] ACT-T028 Implement ergonomic authoring layer (dx/authoring module) with common action patterns
- [ ] ACT-T029 Document runtime and sandbox integration end-to-end

**Checkpoint**: Three working examples. Authoring DX module available. Error-to-retry conventions documented.

---

## Dependencies & Execution Order

Phases are sequential: Phase 1 must complete before Phase 2 (API must be frozen before building context model). Phase 3 can begin after Phase 2 context types are stable. Phase 4 is independent of Phase 3 and could run in parallel. Phase 5 depends on Phase 2 (contexts) and Phase 4 (port governance).

Within each phase, tasks marked `[P]` can run in parallel.
