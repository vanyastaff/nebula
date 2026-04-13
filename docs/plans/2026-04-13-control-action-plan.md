# ControlAction — Implementation Plan

> Phased roadmap for shipping `ControlAction` DX family and 7 core control nodes.
> Companion to `2026-04-13-control-action-spec.md`.

**Date:** 2026-04-13
**Status:** Draft
**Depends on:** spec `2026-04-13-control-action-spec.md`, decision record in `.project/context/decisions.md`

---

## Phase Overview

| Phase | Name | Status | Effort | Impact | Dependencies |
|-------|------|--------|--------|--------|-------------|
| 0 | Correctness fixes in `ActionResult` / `ExecutionTerminationReason` / `ActionCategory` | 🔲 Next | S | Critical (fixes real bugs) | None — independent |
| 1 | `ControlAction` trait + adapter in `nebula-action` | 🔲 Planned | S–M | High (unblocks Phase 2, community extensibility) | Phase 0 |
| 2 | 7 core control nodes in a downstream crate (exact crate TBD) | 🔲 Planned | M | Medium (replaces hand-written `ActionResult::Branch`) | Phase 1, crate-placement decision |
| 3 | Engine integration smoke test + audit-log consumer | 🔲 Planned | S | Medium (validates end-to-end) | Phase 2 |
| post-v1 | Validation hook, extended `ControlInput` getters, error-as-artifact, `Emit` variant | 🔲 Deferred | — | — | v1 in use |

**Effort:** S = 1–2 days, M = 3–5 days, L = 1–2 weeks, XL = 2–4 weeks.

**Priority note:** none of these phases chase a user-visible feature. Current `If`/`Switch` (hand-written `StatelessAction` returning `ActionResult::Branch`) work. This plan fixes **correctness bugs** (Phase 0) and improves **DX + community extensibility** (Phases 1–2). `StopAction` and `FilterAction` cannot be written correctly without Phase 0 regardless of whether Phase 1 exists — that alone justifies the Phase 0 work independent of everything else.

Schedule relative to other in-progress work (per `.project/context/active-work.md`):

- **Phase 0 can run in parallel** with sandbox slice 1c/1d and credential bugs B1–B9. It touches `nebula-action::result`, `nebula-action::metadata`, `nebula-execution::status` — no conflict with sandbox transport, credential DI, or Postgres storage work.
- **Phases 1–3 should wait** until B6 (CRITICAL credential bug) lands and sandbox slice 1c is green. No engine surface involved, but reviewer bandwidth is the constraint.

---

## Phase 0: Correctness Fixes 🔲

**Goal:** Fix three correctness gaps in existing code so that Filter, Stop, and Fail semantics become expressible, and so that UI/audit can distinguish control nodes from data nodes.

**Why independent of Phase 1:** these fixes patch real holes in `ActionResult` and metadata even if `ControlAction` never ships. `FilterAction` desugared to `Skip` silently kills downstream subgraphs today. `StopAction` in parallel branches silently fails to terminate other branches today. Whether any author uses the new variants directly or through `ControlOutcome`, the underlying primitives must be correct.

### Deliverables

1. **`ActionResult::Drop { reason }`** in `crates/action/src/result.rs`
   - Variant on `ActionResult<T>` enum
   - Docstring: "Item dropped from this node's main output. Downstream dependents receive no data for this item, but the broader execution and other items continue."
   - Contrast documented against `Skip` (which skips the downstream subgraph entirely)
   - `From`-impl updates where necessary; no `Default` implication

2. **`ActionResult::Terminate { reason: TerminationReason }`** in `crates/action/src/result.rs`
   - Variant on `ActionResult<T>` enum
   - `TerminationReason` enum defined in same file:
     ```rust
     #[non_exhaustive]
     pub enum TerminationReason {
         Success { note: Option<String> },
         Failure { code: ErrorCode, message: String },
     }
     ```
   - Docstring: "Terminate the entire workflow execution. Unlike `Skip`, this transitions the `ExecutionState` to a terminal state regardless of other parallel branches."

3. **`ExecutionTerminationReason`** in `crates/execution/src/status.rs`
   - New enum:
     ```rust
     #[non_exhaustive]
     pub enum ExecutionTerminationReason {
         NaturalCompletion,
         ExplicitStop { by_node: NodeId, note: Option<String> },
         ExplicitFail { by_node: NodeId, code: ErrorCode, message: String },
         Cancelled,
         SystemError,
     }
     ```
   - Wire into `ExecutionState` or `ExecutionResult` struct — field name TBD based on current shape (need to read `status.rs` before landing)
   - Audit-log / execution-journal consumers updated to record `ExecutionTerminationReason` when execution ends

4. **`ActionCategory`** in `crates/action/src/metadata.rs`
   - New enum:
     ```rust
     #[non_exhaustive]
     pub enum ActionCategory {
         Data,
         Control,
         Trigger,
         Resource,
         Agent,
         Terminal,
     }
     ```
   - Field `category: ActionCategory` added to `ActionMetadata` struct
   - Default via `Default` impl on `ActionCategory` → `Data` (so existing actions don't need to opt in)
   - Serde: `#[serde(default)]` on the field so existing serialized metadata round-trips without breakage
   - Builder method `.category(ActionCategory::Control)` on `ActionMetadata` builder

5. **`MultiOutput` join semantics docstring** in `crates/action/src/result.rs`
   - No code change — only docstring update on `ActionResult::MultiOutput`
   - Documented rule: downstream with multiple upstream edges fires when all emitted ports carry data; absent output ports imply "not emitted" and do not block downstream (same as `all_success` trigger rule)

6. **Engine handling for new `ActionResult` variants**
   - `Drop`: engine frontier logic treats it as "no main output produced for this item." Downstream branch continues with next item in the iteration (if any). Audit log records the drop with reason.
   - `Terminate`: engine transitions `ExecutionState` to the terminal state corresponding to `TerminationReason` (Success → `Succeeded` with note, Failure → `Failed` with code/message). Other running nodes in parallel branches receive cancellation. Audit log records `ExecutionTerminationReason::ExplicitStop`/`::ExplicitFail`.

### Files touched

- `crates/action/src/result.rs` — two new variants, new `TerminationReason` enum, `MultiOutput` docstring update
- `crates/action/src/metadata.rs` — `ActionCategory` enum, `category` field, builder method
- `crates/execution/src/status.rs` — `ExecutionTerminationReason` enum, integration with `ExecutionState` or `ExecutionResult`
- `crates/engine/src/...` — dispatch handling for `Drop` and `Terminate` variants (specific files TBD; likely frontier/scheduler module)
- `crates/runtime/src/...` — same; handle `Drop`/`Terminate` in `execute_stateless` result processing
- `crates/action/tests/contracts.rs` — contract tests for serde round-trip of new variants
- `.project/context/crates/action.md` — note the new variants and category field
- `.project/context/crates/execution.md` — note the new `ExecutionTerminationReason`

### Acceptance tests

- Unit: `ActionResult::Drop` round-trips through serde
- Unit: `ActionResult::Terminate { Success }` round-trips with `TerminationReason`
- Unit: `ActionMetadata` with and without explicit `category` both deserialize (backward compat)
- Integration: minimal engine test — an action returning `Drop` in a two-item iteration leaves item 2 processing normally
- Integration: minimal engine test — an action returning `Terminate::Success` in one branch of a parallel split cancels the other branch cleanly
- Integration: audit log contains `ExplicitStop { by_node }` after `Terminate::Success`
- `cargo deny check` passes (no new dep violations)
- `cargo +nightly fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace` green

### Exit criteria

- All new variants and types documented, tested, and integrated
- Engine correctly handles both variants in parallel branch scenarios
- No existing test regresses

### Dependencies

None. Can be started immediately in a separate PR.

### Risks

- **Engine frontier logic is not trivially modifiable.** The dispatch points and where variant handling happens need to be located. Not blocker-level, but may expand scope if the frontier code is tangled. Mitigation: spike the engine change first on `Drop` alone, confirm pattern works, then add `Terminate`.
- **Backward compat on `ActionMetadata` serde.** Adding a field is safe with `#[serde(default)]`, but any existing contract tests that do strict round-trip comparison need to be updated. Grep `contracts.rs` before touching metadata.
- **`ErrorCode` dependency.** `TerminationReason::Failure` references `ErrorCode`. Action-v2 spec says `ErrorCode` is Phase 10 and not yet implemented. Workaround: use `Arc<str>` or `String` for v1 of `TerminationReason::Failure.code`; upgrade to `ErrorCode` when Phase 10 lands.

### Estimated size

S — 1–2 focused days for a single implementer. Could stretch to 3 if engine frontier logic requires untangling.

---

## Phase 1: `ControlAction` Trait + Adapter 🔲

**Goal:** Ship the public trait, types, and adapter described in the spec §5. Zero concrete node implementations — only contract and bridge.

### Deliverables

1. **`crates/action/src/control.rs`** — new file containing:
   - `pub trait ControlAction` — public, non-sealed, native async via RPITIT, one method (`evaluate`) plus `metadata` accessor
   - `pub enum ControlOutcome` — 5 variants, `#[non_exhaustive]`, documented per variant
   - `pub enum TerminationReason` — re-exported from `result.rs` (where it lives after Phase 0), or new alias in `control` module for DX locality
   - `pub struct ControlInput` — owned wrapper with 4+ typed getters (`get_bool`, `get_str`, `get_i64`, `get_f64`), `get` for raw sub-value, `into_value` for passthrough
   - `pub struct ControlActionAdapter<A: ControlAction>` — wraps typed action, caches metadata with category stamped, implements `StatelessHandler`
   - `impl From<ControlOutcome> for ActionResult<Value>` — desugar logic per spec §5.7
   - Private `derive_category` helper — infers `Control` vs `Terminal` based on declared output ports

2. **Module exposure** — `lib.rs` and `prelude.rs` re-exports:
   - `ControlAction`, `ControlOutcome`, `ControlInput`, `ControlActionAdapter`, `TerminationReason` added to `prelude`
   - Public at crate root so `use nebula_action::ControlAction;` works

3. **Tests in `crates/action/src/control.rs`** (inline `#[cfg(test)] mod tests`):
   - `ControlOutcome::Branch → ActionResult::Branch` round-trip
   - `ControlOutcome::Route → ActionResult::MultiOutput` round-trip
   - `ControlOutcome::Pass → ActionResult::Success` round-trip
   - `ControlOutcome::Drop → ActionResult::Drop` round-trip (depends on Phase 0)
   - `ControlOutcome::Terminate { Success } → ActionResult::Terminate { TerminationReason::Success }` round-trip
   - `ControlOutcome::Terminate { Failure } → ActionResult::Terminate { TerminationReason::Failure }` round-trip
   - `ControlInput::get_bool` valid path, missing path, wrong type
   - `ControlInput::get_str`, `get_i64`, `get_f64` — one happy-path, one error path each
   - Dummy `TestIf` implementing `ControlAction` — smoke test of the full adapter path: `ControlActionAdapter::new(TestIf) → StatelessHandler::execute → ActionResult::Branch`
   - `derive_category`: action with 2 outputs → `Control`; action with 0 outputs → `Terminal`
   - Metadata caching: `adapter.metadata().category == ActionCategory::Control` even if the original action's metadata didn't set it

4. **Documentation** — at top of `control.rs`:
   - Module-level doc comment with one complete "write your own control action" example
   - Link to spec and decisions.md
   - `cargo expand`-friendly structure (no macros yet, but so that when `#[derive(Action)]` lands, it composes cleanly)

5. **README / crate docs update** — add `ControlAction` to the DX family list in `crates/action/README.md` (if one exists; else create a minimal one)

### Files touched

- `crates/action/src/control.rs` — new file, ~400–500 LOC including tests
- `crates/action/src/lib.rs` — module declaration, re-exports
- `crates/action/src/prelude.rs` — add `ControlAction`, `ControlOutcome`, `ControlInput`, `ControlActionAdapter`
- `crates/action/Cargo.toml` — no new dependencies expected (`async-trait` already present)
- `crates/action/README.md` or `crates/action/src/control.rs` module doc — DX family reference
- `.project/context/crates/action.md` — note `ControlAction` DX family

### Acceptance tests

- All unit tests in `control.rs` pass
- `cargo check -p nebula-action` clean
- `cargo clippy -p nebula-action -- -D warnings` clean
- `cargo +nightly fmt` applied
- Adapter `metadata()` stamps `ActionCategory::Control` on an action whose original metadata was `ActionCategory::Data` (default)
- Smoke test: registering `Arc::new(ControlActionAdapter::new(TestIf))` into a mock `ActionRegistry` succeeds and the action can be looked up by key

### Exit criteria

- Trait, types, adapter merged into `nebula-action`
- No concrete node implementations in this PR — keep scope tight
- Contract test added for `ControlOutcome` variant set (catches accidental breakage when enum evolves)
- Phase 0 prerequisites confirmed landed (Phase 1 depends on `ActionResult::Drop`, `::Terminate`, `ActionCategory`)

### Dependencies

- **Phase 0 must be merged first.** `From<ControlOutcome> for ActionResult<Value>` references `ActionResult::Drop` and `ActionResult::Terminate`, which don't exist until Phase 0.
- No engine changes. No runtime changes. No `ActionHandler` enum changes.

### Risks

- **`async fn` in public trait RPITIT edge cases.** rust-senior review flagged that `async fn evaluate` sugar may not carry `Send` bound; must write `fn evaluate(...) -> impl Future<Output = ...> + Send` explicitly. Mitigation: follow the exact shape of `StatelessAction::execute` at `stateless.rs:81–85`.
- **Metadata clone in adapter.** `ControlActionAdapter::new` clones metadata to stamp category. If `ActionMetadata` is large or contains `Arc`'s that count references, clone cost may be non-trivial. Mitigation: if this becomes a bottleneck, cache `Arc<ActionMetadata>` with a separate "category override" layer instead of full clone. Defer unless benchmark shows it matters.
- **`non_exhaustive` match arms.** Any match on `ControlOutcome` must have wildcard arm. Test this compiles in a dummy external-crate test (or in-crate test simulating external).
- **`PortKey = String` allocation churn.** rust-senior noted `IfAction` emits `"true"`/`"false"` every call → heap alloc per tick. Not a blocker for Phase 1 (measurement required first), but flag for post-v1 optimization — possible path: `Cow<'static, str>` for known port keys.

### Estimated size

S–M — 2–4 focused days. One implementer, no engine involvement, trait + tests + documentation.

---

## Phase 2: 7 Core Control Nodes (downstream crate — exact placement TBD) 🔲

**Goal:** Ship reference implementations of If, Switch, Router, Filter, NoOp, Stop, Fail. Each node is a real, registerable, reviewable example that community plugin authors can read and copy.

**Blocker for Phase 2 start:** crate-placement decision. Must be resolved before Phase 2 deliverables can be scoped — which workspace member hosts the 7 nodes? Options to evaluate when this phase is greenlit:
- New dedicated crate (e.g. `nebula-plugin-core`, `nebula-control-nodes`)
- Module inside existing `nebula-plugin`
- Different downstream location surfaced later

The **contract** (trait + adapter) does not care about the answer. All of Phase 1 stays valid regardless. Phase 2 deliverables below are written crate-agnostic — file paths use `<downstream>/src/control/...` as a placeholder.

### Deliverables

1. **Crate scaffold in the chosen downstream location**
   - `Cargo.toml` with `nebula-action` dep, Business layer tag
   - `cargo deny` allow-list entry (Business layer → Business layer dep is permitted)
   - `src/lib.rs` — public exports, `register_core_control_nodes(registry: &mut ActionRegistry)` helper
   - Workspace `Cargo.toml` updated to include the new/existing member

2. **`<downstream>/src/control/if_action.rs`** — `IfAction`
   - Struct with `metadata` field and configurable predicate path
   - `impl ControlAction` with `evaluate` that reads the predicate from `ControlInput`, returns `ControlOutcome::Branch { "true" | "false", passthrough }`
   - Two output ports: `true`, `false`
   - Unit tests: truthy input routes to `true`, falsy input routes to `false`, missing/invalid condition returns `ActionError::validation`

3. **`<downstream>/src/control/switch_action.rs`** — `SwitchAction`
   - Struct with `metadata` field, a vector of `{ case: Value, output_port: PortKey }` rules, and a `default_port` fallback
   - `impl ControlAction` with `evaluate` that matches input value against rules, returns `Branch { selected, output }`
   - Static output ports declared from rules at metadata build time
   - Unit tests: match first rule, match middle rule, fallback to default, no rules and no default → `ActionError`

4. **`<downstream>/src/control/router_action.rs`** — `RouterAction`
   - Struct with `metadata` field, `OutputPort::Dynamic` for config-driven ports, `mode: RouterMode { FirstMatch, AllMatch }`
   - `impl ControlAction` with `evaluate` that:
     - `FirstMatch` → returns `ControlOutcome::Branch { first_matching_port, passthrough }`
     - `AllMatch` → returns `ControlOutcome::Route { Vec<(matching_port, passthrough)> }`
   - Unit tests: both modes, zero matches, all matches, mid-set matches

5. **`<downstream>/src/control/filter_action.rs`** — `FilterAction`
   - Struct with `metadata` field, predicate config
   - `impl ControlAction` with `evaluate` that reads predicate, returns `Pass { output }` or `Drop { reason }`
   - Unit tests: passing predicate, failing predicate (with reason), invalid predicate (error)

6. **`<downstream>/src/control/noop_action.rs`** — `NoOpAction`
   - Minimal struct, minimal `evaluate` that returns `Pass { output: input.into_value() }`
   - Unit tests: trivial passthrough

7. **`<downstream>/src/control/stop_action.rs`** — `StopAction`
   - Struct with optional `note` config
   - Zero output ports (metadata declares outputs as empty → adapter stamps `ActionCategory::Terminal`)
   - `impl ControlAction::evaluate` returns `ControlOutcome::Terminate { reason: TerminationReason::Success { note } }`
   - Unit tests: returns Terminate with Success, metadata category is Terminal

8. **`<downstream>/src/control/fail_action.rs`** — `FailAction`
   - Struct with `error_code` and `error_message_template` config
   - Zero output ports
   - `impl ControlAction::evaluate` returns `ControlOutcome::Terminate { reason: TerminationReason::Failure { code, message } }`
   - Unit tests: returns Terminate with Failure, custom code and message are passed through, metadata category is Terminal

9. **`<downstream>/src/control/mod.rs`** — module aggregator + `register_core_control_nodes` helper:
   ```rust
   pub fn register_core_control_nodes(registry: &mut ActionRegistry) {
       registry.register(Arc::new(ControlActionAdapter::new(IfAction::default())));
       registry.register(Arc::new(ControlActionAdapter::new(SwitchAction::default())));
       // ... 5 more
   }
   ```

10. **Integration test `<downstream>/tests/control_nodes_end_to_end.rs`**:
    - Build a `TestContextBuilder::minimal()` context (or manual construct if Phase 2b of action-v2 hasn't landed)
    - For each of 7 nodes: instantiate, adapt, execute against a canned input, assert resulting `ActionResult` shape
    - No engine involvement — just adapter-level correctness

11. **Crate-level docs** — `<downstream>/README.md`:
    - "Reference implementations of core control primitives for Nebula workflows"
    - Usage example: `register_core_control_nodes(&mut registry)`
    - Link to spec and `ControlAction` trait

### Files touched

- `<downstream>/Cargo.toml` — crate manifest (new or updated)
- `<downstream>/src/lib.rs` — public surface
- `<downstream>/src/control/{if_action,switch_action,router_action,filter_action,noop_action,stop_action,fail_action,mod}.rs` — new, ~50–150 LOC each
- `<downstream>/tests/control_nodes_end_to_end.rs` — new, ~200 LOC
- `<downstream>/README.md` — new or updated
- `Cargo.toml` (workspace) — add member if new crate
- `deny.toml` — layer declaration if new crate
- `.project/context/crates/<downstream>.md` — new or updated crate-context file (per project convention)
- `.project/context/ROOT.md` — add crate to Business layer list if new

### Acceptance tests

- All per-node unit tests pass
- Integration test `control_nodes_end_to_end.rs` passes all 7 scenarios
- `cargo clippy -p nebula-plugin-core -- -D warnings` clean
- `cargo deny check` passes (new crate is layer-compliant)
- `cargo nextest run -p nebula-plugin-core` green

### Exit criteria

- All 7 control nodes exist, tested, and registerable via single helper
- Each node's metadata correctly declares inputs/outputs and category-inference produces `Control` or `Terminal` as expected
- README documents the crate's purpose and usage
- Crate added to workspace context per convention

### Dependencies

- **Phase 1 merged.** `ControlAction` trait and adapter must be available.
- **Crate-placement decision made** — which workspace member hosts the 7 nodes. This is a packaging question, not a contract question, and is deliberately left open in the spec. Resolve before Phase 2 kicks off.
- **Phase 2a of action-v2 roadmap (`#[derive(Action)]`) is optional.** If Phase 2a has landed, use `#[derive(Action)]`; otherwise, manually construct `ActionMetadata` in each node's constructor. Either works — don't block this phase on action-v2 Phase 2a.

### Risks

- **`RouterAction` all-match mode interaction with `MultiOutput`.** The engine's handling of `MultiOutput` may not be fully exercised today. Verify with the Phase 0 docstring update and a targeted integration test before assuming all-match routing works.
- **`ControlInput::get_str` vs `PortKey`.** Config-driven `Switch`/`Router` rules store port keys in the node config. Authors need to read them as `&str` and clone to `PortKey = String`. Not a bug, just an ergonomics wart — post-v1 `Cow<'static, str>` optimization would help.
- **Metadata construction boilerplate.** Without `#[derive(Action)]`, each node's constructor manually builds `ActionMetadata`. 7 similar blocks. Mitigation: factor a small helper in `crates/plugin-core/src/control/mod.rs` that takes `(key, name, outputs)` and returns a pre-filled `ActionMetadata`. Not ideal but localized.

### Estimated size

M — 3–5 focused days. Seven small, similar, well-specified nodes with shared harness. Most time spent on `SwitchAction` and `RouterAction` (non-trivial `DynamicPort` wiring); the other five are each under 100 LOC.

---

## Phase 3: Engine Integration Smoke Test + Audit-Log Consumer 🔲

**Goal:** Validate end-to-end that a workflow using `ControlAction` nodes executes correctly in a real (or mock-realistic) engine and produces correct audit-log entries.

### Deliverables

1. **End-to-end workflow test** — `crates/engine/tests/control_action_smoke.rs` (or wherever engine tests live):
   - Build a small workflow: `Source → IfAction → { true: NoOpAction → Sink, false: StopAction }`
   - Feed truthy input → assert `Sink` receives it, execution state is `Succeeded`
   - Feed falsy input → assert `StopAction` fires, execution state is `Succeeded` with `ExecutionTerminationReason::ExplicitStop`, sink does not receive anything
   - Same setup but with `FailAction` instead of `StopAction` → execution state is `Failed` with `ExplicitFail { code, message }`

2. **Parallel-branch termination test:**
   - Workflow: `Source → { BranchA: NoOpAction → Sink, BranchB: StopAction }` (both branches run in parallel)
   - Assert `BranchA` is cancelled cleanly mid-execution when `BranchB` terminates
   - Audit log records termination reason

3. **Filter-drop test:**
   - Workflow: `Source (emits 3 items) → FilterAction (drops item 2) → Sink`
   - Assert `Sink` receives items 1 and 3 only
   - Audit log records one `Drop` with reason; execution state ends as `Succeeded`

4. **Audit log consumer update** — if there's an audit consumer module that formats execution outcomes for display, add formatting for:
   - `ExecutionTerminationReason::ExplicitStop { by_node, note }` → human-readable
   - `ExecutionTerminationReason::ExplicitFail { by_node, code, message }` → human-readable
   - `ActionResult::Drop { reason }` at node level → "dropped: {reason}"

### Files touched

- `crates/engine/tests/control_action_smoke.rs` — new
- `crates/engine/src/audit/...` or wherever audit formatting lives — update for new variants
- `.project/context/crates/engine.md` — note control-action integration test coverage

### Acceptance tests

- Three end-to-end tests pass
- Audit log output matches expected format for each termination path
- No regressions in existing engine tests

### Exit criteria

- Engine correctly handles all `ControlOutcome` variants through the full dispatch pipeline
- Audit log distinguishes explicit termination from crashes in its output
- `StopAction` in parallel branch correctly cancels siblings

### Dependencies

- Phases 0, 1, 2 all merged.
- Engine test infrastructure must exist (may need bootstrap if engine has no integration-test framework yet — verify before starting).

### Risks

- **Engine test harness may not exist.** If engine has no current integration-test setup, this phase expands to "build the harness first, then run the test." Scope creep. Mitigation: spike the harness question in Phase 1 review, before committing to Phase 3.
- **Parallel-branch cancellation semantics.** `StopAction` cancelling sibling branches requires engine cooperation. If engine's current cancellation propagation is incomplete (e.g. only responds to top-level `CancellationToken`, not to in-flight `ActionResult::Terminate`), this phase may uncover engine work. Mitigation: read engine frontier and scheduler code during Phase 0 to estimate whether `Terminate` handling is additive or requires restructuring.

### Estimated size

S — 1–2 days assuming engine test harness exists. Up to M if harness has to be built.

---

## Deferred to post-v1

1. **`ControlAction::validate()` hook.** Decide between (a) explicit method called by adapter at registration, (b) debug-assert in `execute`, (c) author-responsibility. Revisit after real bugs from community-written control actions surface.

2. **Extended `ControlInput` getters.** `get_datetime`, `get_uuid`, `get_path`, `get_array`, `get_object`. Add based on feedback.

3. **Error-as-artifact (Metaflow `@catch(var=)` pattern).** Upstream error becomes a variable in downstream's input. Requires `ControlInput::previous_error()` and scheduler cooperation.

4. **`ControlOutcome::Emit { events }` variant.** Fire events on `EventBus` as a side effect of the control decision. Add if a concrete use case materialises.

5. **`Cow<'static, str>` optimization for `PortKey`.** Eliminate heap alloc on every `IfAction` evaluate. Benchmark-driven; not speculation.

6. **Declarative macro sugar** — `control_action! { if MyIf { ... } }` as **opt-in** syntactic sugar on top of the trait (not a replacement). Would cut boilerplate in Phase 2 node constructors. Only if Phase 2 experience shows meaningful repetitive code that the trait alone can't eliminate.

---

## Open Coordination Questions (for reviewer)

1. **Phase 0 can start immediately.** Does anyone else have uncommitted work in `crates/action/src/result.rs`, `metadata.rs`, or `crates/execution/src/status.rs` that this would conflict with? If yes, resolve ordering before starting.

2. **`ErrorCode` dependency.** Phase 10 of action-v2 roadmap introduces `ErrorCode`. `TerminationReason::Failure` wants to use it. Two options:
   - Block Phase 0 on Phase 10
   - Use `String` / `Arc<str>` as placeholder in v1, upgrade to `ErrorCode` when Phase 10 lands
   - Decision: **use `Arc<str>` placeholder**. Phase 10 is not on the critical path.

3. **Crate placement for concrete nodes.** Deliberately deferred — not a spec or Phase 1 concern. Resolve before Phase 2 starts. Options surfaced for consideration at that point: new dedicated crate, module inside existing `nebula-plugin`, or something else.

4. **Phase 1 vs Phase 2a of action-v2 roadmap.** Phase 2a (`#[derive(Action)]`) would make Phase 2 of this plan significantly terser. Should we wait? **No — don't wait.** Phase 2 can use manual `ActionMetadata` construction; switch to derive when Phase 2a lands. Decoupling avoids critical-path dependency.

---

## References

- Spec: `docs/plans/2026-04-13-control-action-spec.md`
- Decision record: `.project/context/decisions.md` — "ControlAction — adapter pattern, not blanket impl, not macro"
- Prior-art adapter pattern: `crates/action/src/poll.rs` (`PollTriggerAdapter`), `crates/action/src/webhook.rs` (`WebhookTriggerAdapter`)
- Existing port model: `crates/action/src/port.rs`
- Action v2 roadmap: `docs/plans/2026-04-08-action-v2-roadmap.md`
- Workspace active work: `.project/context/active-work.md`
