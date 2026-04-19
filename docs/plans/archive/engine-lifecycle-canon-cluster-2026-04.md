# Engine Lifecycle Canon Cluster — Planning Document

> **Status:** COMPLETED (archived 2026-04-19). All 15 P1 issues from §0 closed in `main`; cross-reference below.
>
> | # | Fix commit |
> |---|---|
> | 290 | batch landed via `b2b0c6fc` (`ActionResult::Retry` gated behind `unstable-retry-scheduler`) |
> | 297 | `15092823` (persist OnError payload) + `54d416b5` (persist before edge routing) |
> | 298 | batch 1 `a130ea93` (run_frontier fixes) |
> | 299 | batch 2 `646d11e6` (execution-state correctness) |
> | 308 | `54d416b5` (persist before edge routing) |
> | 311 | batch 2 `646d11e6` + `7dd3d00d` (ExecutionBudget restore) |
> | 321 | batch 2 `646d11e6` + `58740664` (setup-failure checkpoint symmetry) |
> | 324 | `54d416b5` (persist before edge routing) |
> | 325 | `3d7db131` (acquire and renew execution lease) — ADR-0015 (ex-0008 lease-lifecycle) promoted to accepted |
> | 327 | `926a2d5f` (persist canonical state on start) |
> | 330 | `9134fb45` (control-queue consumer skeleton, ADR-0008) |
> | 332 | `df1c996a` (dispatch execution to engine on start) |
> | 333 | `d51f9c5c` (surface CAS conflicts) |
> | 336 | `54d416b5` (persist before edge routing) |
> | 341 | `8c47623f` (gate Completed on all_nodes_terminal invariant) |
>
> Residual follow-ups flagged during closure: engine-side **real** dispatch for Start/Resume/Restart (ADR-0008 A2) and Cancel/Terminate (ADR-0008 A3) remain `planned` — producer side is done, engine consumer is currently a skeleton with no-op dispatch defaults. Out of scope for this cluster; lives as ongoing work under ADR-0008.
>
> **Date:** 2026-04-18
> **Scope:** 15 P1 issues against `vanyastaff/nebula` clustered around execution lifecycle (canon §11–§12). Output: grouping, root-cause hypothesis per group, canon impact, ADR-needed flag, recommended PR sequencing.
> **Authority:** Subordinate to [`docs/PRODUCT_CANON.md`](../PRODUCT_CANON.md). All groups below are framed against §11 (core contracts) and §12 (non-negotiable invariants).
> **Hand-off:** This document is to be reviewed by the **`tech-lead`** agent before any implementation chip is spun up. Comments on this file capture sign-off and priority calls.

---

## 0. Cluster verification (2026-04-18)

All 15 issues confirmed `OPEN` against `vanyastaff/nebula` at session start:

| # | Title (truncated) | Group |
|---|---|---|
| 290 | Engine treats `ActionResult::Retry` as terminal | E |
| 297 | Engine checkpoint ordering: emit/idempotency around persist | D |
| 298 | NodeTask rate limiter acquire error logged-and-ignored | F (mitigated, see note) |
| 299 | `check_and_apply_idempotency` reconstructs result as `Success` | B |
| 308 | Runtime `execute_stateful` state lives only on stack | D |
| 311 | `resume_execution` drops original workflow input | B |
| 321 | Engine setup-failure path skips checkpoint | D |
| 324 | `resume_execution` loses historical OnError edge activations | B |
| 325 | Execution leases implemented but unused | C |
| 327 | API persists non-canonical `pending` status | A |
| 330 | API cancel does not signal running engine task | A |
| 332 | API start endpoints do not dispatch to engine | A |
| 333 | Engine CAS conflict handling is write-blind | C |
| 336 | `resume_execution` unconditionally activates all outgoing edges | B |
| 341 | Engine reports `Completed` without all-nodes-terminal invariant | C |

None already fixed in `main`. Issue **#298** is **partially mitigated** in current `main` (engine.rs:1775-1795 now fails the node on limiter error rather than logging-and-falling-through), but the surfaced error is a `retryable` action error that depends on **Group E (#290)** for actual retry — so #298 stays in the cluster as a §12.4 honesty fix, scope shrunk.

---

## 1. Smoking-gun finding (do not skip)

`crates/engine/src/lib.rs:11-13` documents:

> *"This crate is the **single real consumer** of `execution_control_queue` in production deployment modes (canon §12.2). A handler that only logs and discards control-queue rows does not satisfy the canon."*

`grep -rn "ControlQueueRepo\|ControlCommand::" crates/engine/src/` returns **zero** non-test hits. The engine **never imports**, **never instantiates**, and **never drains** the control queue. Production references all live on the API side: `crates/api/src/handlers/execution.rs:338` (enqueue on cancel), `crates/api/src/state.rs:10,44` (`AppState` holds an `Arc<dyn ControlQueueRepo>`), and `crates/api/examples/simple_server.rs` (sets up the in-memory repo). No engine-side consumer or dispatcher implementation exists.

Worse — there are **three** doc-truth sites that all claim the consumer exists:

- `crates/engine/src/lib.rs:11-13` — *"This crate is the **single real consumer** of `execution_control_queue` in production deployment modes."*
- `crates/api/src/state.rs:39-43` — *"The engine dispatcher drains this queue to deliver signals to running executions."*
- `crates/storage/src/repos/mod.rs:7` — *"Consumed by the API cancel handler"* (also wrong on direction — the API is the producer).

This is simultaneously:

- A **canon §11.6 docs-truth violation** at all three sites above — the crate / module / state docs advertise a capability the code does not deliver.
- A **canon §14 anti-pattern** — "Discard-and-log workers": rows are produced but no consumer exists. (Worse than discard-and-log: there isn't even a discarding loop.)
- A **canon §12.7 orphan-module violation** — queue produced but never consumed.
- The **root cause of #330**, and the missing peer of **#332** (no enqueue on start, no consumer for either).

**Implication for grouping:** Group A is not "two API bugs that share a theme." It is one architectural gap (the consumer half of `execution_control_queue`) with three symptoms. Solving it requires building the consumer **and** wiring start-side enqueue **and** correcting all three doc-truth sites in the same PR.

---

## 2. Groups

### Group A — API ↔ Engine control plane (durable outbox)

**Issues:** [#332](https://github.com/vanyastaff/nebula/issues/332), [#330](https://github.com/vanyastaff/nebula/issues/330), [#327](https://github.com/vanyastaff/nebula/issues/327)

**Root-cause hypothesis:** `execution_control_queue` exists as a **producer-only** outbox. The cancel path enqueues `ControlCommand::Cancel` but no consumer drains; the start path does not enqueue at all. API writes execution rows directly with non-canonical `"pending"` JSON because there is no canonical dispatch path that would force `ExecutionState::Created`. All three are symptoms of one missing component: an engine-side `ControlConsumer` that drains the queue and dispatches to `WorkflowEngine::execute_workflow` / cancel-token paths.

**Canon impact:**
- §12.2 (durable control plane) — currently violated end-to-end.
- §13 knife step 3 (start) and step 5 (engine-visible cancel) — both currently fail.
- §11.6 (docs truth) — `crates/engine/src/lib.rs //!`, `crates/api/src/state.rs:39-43`, and `crates/storage/src/repos/mod.rs:7` all lie about consumer status.
- §14 anti-patterns — discard-and-log workers, orphan modules.

**ADR needed:** **YES.** Producer/consumer wiring choice (in-process direct dispatch via shared `Arc<WorkflowEngine>` vs polling loop vs notify channel + outbox) is an L2 design decision that future deployment modes (cloud / multi-worker) will inherit. Suggested ADR title: *"`execution_control_queue` consumer wiring and start-side enqueue contract."*

**Architectural-fit verdict (per skill):**
- Decision gate: directional answers go right way; current state itself is the §11.6 / §12.2 / §14 violation; the fix removes the violations.
- Bounded context: API (producer) + Exec (new consumer module) + Storage (existing trait). No upward dep. Two contexts → not two concepts; one concept (control plane) crossing layers as canon §12.2 already mandates.
- Concept promotion: 🟠 — new module `crates/engine/src/control_consumer.rs` (or similar). No new crate, no L2 invariant change, but enough surface area for an ADR.
- Quick-Win traps to avoid: shipping consumer half without start-side enqueue (leaves #332 open while pretending to fix Group A); shipping start enqueue without consumer (deepens the orphan).

**Smallest correct fix shape (for ADR to refine):**
1. New `crates/engine/src/control_consumer.rs` that holds an `Arc<dyn ControlQueueRepo>` + an `Arc<WorkflowEngine>` (or equivalent dispatch handle), runs as a Tokio task spawned from the composition root.
2. API `start_execution` rewrites: build canonical `ExecutionState` with `ExecutionStatus::Created`, persist via `ExecutionRepo::create`, enqueue `ControlCommand::Start { execution_id }` in the same logical operation (per §12.2 atomicity rule — share a transaction or document the orphan window with explicit reconciliation).
3. API `cancel_execution` keeps existing CAS + enqueue, but the comment at `crates/api/src/handlers/execution.rs:311-315` (acknowledging orphan window) becomes a TODO retired by the consumer wiring.
4. `crates/engine/src/lib.rs` `//!`, `crates/api/src/state.rs:39-43` doc comment, and `crates/storage/src/repos/mod.rs:7` truth strings all updated to match reality in same PR.
5. Integration test extending the §13 knife: real engine + real consumer; cancel actually stops a running task; start actually causes node execution.

**Recommended PR sequencing within Group A:**
- **A1:** ADR + `ControlConsumer` skeleton + composition-root wiring. No behavior change yet.
- **A2:** Start-side enqueue + canonical `ExecutionState::Created` (kills #332 + #327). Consumer dispatches start.
- **A3:** Consumer dispatches cancel (kills #330). Same-PR documentation truth fix.
- **A4:** Knife integration test extending step 3 + step 5. Mark `simple_server.rs` either `// DEMO ONLY` per §12.2 or migrate it to use the real consumer.

**Acceptance:** §13 knife steps 3 + 5 pass with **no** stub or DEMO ONLY caveat for the production deployment mode.

---

### Group B — Resume correctness (state reconstruction)

**Issues:** [#311](https://github.com/vanyastaff/nebula/issues/311), [#324](https://github.com/vanyastaff/nebula/issues/324), [#336](https://github.com/vanyastaff/nebula/issues/336), [#299](https://github.com/vanyastaff/nebula/issues/299)

**Root-cause hypothesis:** Resume reconstructs runtime decisions (workflow input, edge activations, `ActionResult` variant shape) from a persistence record that was never designed to support replay. The four issues are four distinct slices of the same missing data:

| Issue | What is lost on resume | Why |
|---|---|---|
| #311 | Original workflow trigger input | Not persisted; `engine.rs:876-882` TODO; resume passes `Value::Null` |
| #324 | OnError edge activations from `Failed` predecessors | Reconstruction at `engine.rs:829` only marks `Completed\|Skipped` sources active |
| #336 | Per-edge condition (branch_key, port) | Reconstruction unconditionally activates **all** outgoing edges of `Completed` nodes |
| #299 | `ActionResult` variant (Branch/Route/MultiOutput/Skip/Wait) | `check_and_apply_idempotency` synthesizes `ActionResult::success(output)` (`engine.rs:1546`) |

**Canon impact:**
- §11.5 (persistence story) — extends what is durable; new schema rows.
- §11.1 (execution authority) — resumed execution must be byte-equivalent in dispatch behavior to non-crashed run; today it is not.
- §10 golden path step 7 — "persistence story is explicit": currently the persistence story is silently **wrong** for resume.

**ADR needed:** **YES.** Two viable persistence schemas (from #299 issue body):
- (1) Persist full `ActionResult<Value>` per node (smallest delta; `evaluate_edge` stays single source of truth).
- (3) Persist edge-activation decisions per edge (cleanest long-term; bypasses re-evaluation; requires schema change to edge-tracking store).

ADR must pick one and explain the trade-off. Suggested title: *"Resume correctness: persisted edge-activation + workflow input + ActionResult variant."*

**Architectural-fit verdict (per skill):**
- Decision gate: Q3 hazard — adds an L2 contract on what is durable. Needs ADR.
- Bounded context: Exec (engine — resume path + checkpoint path) + Storage (schema + migration). No upward dep.
- Concept promotion: 🔴 — new L2 contract on persisted state shape; schema migration; ADR + seam test in same PR per §0.1.
- Quick-Win traps to avoid: fixing #311 alone (workflow input) without addressing the broader pattern would land four PRs that each move one row of persisted data and inevitably duplicate migration work; "`unwrap_or_default()` to Null on missing input" would be a §4.5 false-capability — must surface as explicit `ResumeError`.

**Smallest correct fix shape:**
1. ADR picks schema (recommend option 1 + persisted workflow input as separate `executions.input_blob` column).
2. Migration: SQLite + Postgres schema update (parity per `crates/storage/migrations/{sqlite,postgres}/README.md`).
3. `ExecutionRepo::create` persists workflow input alongside row.
4. `checkpoint_node` persists serialized `ActionResult<Value>` (or selected variant metadata) — extend existing `save_node_output` to `save_node_result` carrying full variant.
5. `resume_execution` loads workflow input and rebuilds activated_edges by deserializing each terminal node's `ActionResult` and running the existing `evaluate_edge` against the real result (kills #324, #336, #299 in one stroke).
6. Regression tests per issue (entry-node-with-input restart; OnError mid-flight restart; Branch/Route/MultiOutput restart).

**Recommended PR sequencing within Group B:**
- **B1:** ADR + schema design + migration (no behavior change).
- **B2:** Persist workflow input on start; resume restores it (kills #311).
- **B3:** Persist full `ActionResult<Value>` on completion; resume reconstructs from it (kills #299).
- **B4:** Resume uses real `evaluate_edge` over reconstructed results (kills #324, #336).

**Acceptance:** Property test "resumed execution dispatch trace ≡ uninterrupted execution dispatch trace" for graphs containing each of: `Branch`, `Route`, `MultiOutput`, `OnError`, non-null trigger input.

**Cross-group dependency:** Group B touches `checkpoint_node` (persist site) and `resume_execution` (load site). It must land **after** Group D's checkpoint-ordering fix (otherwise the new persisted shape inherits the same crash-window divergence).

---

### Group C — Execution authority enforcement

**Issues:** [#325](https://github.com/vanyastaff/nebula/issues/325), [#333](https://github.com/vanyastaff/nebula/issues/333), [#341](https://github.com/vanyastaff/nebula/issues/341)

**Root-cause hypothesis:** §11.1 declares the engine the single source of truth via CAS. Today the engine declares it but does not enforce it: leases are defined but uncalled, CAS conflicts are recovered by re-reading version (without reloading state), and final completion does not verify the all-nodes-terminal invariant. Three faces of one authority gap.

**Canon impact:**
- §11.1 (execution authority) — direct violation in all three.
- §10 step 5 (state transitions are visible and attributable) — silently violated when CAS races are absorbed.
- §14 (anti-pattern: green tests, wrong product) — current tests don't cover concurrent runners.

**ADR needed:** **NO** for #325 + #333 (these are implementations of an existing canon section). **MAYBE** for #341 if the cleanup behavior on inconsistent terminal state is not obvious — but likely a one-line guard plus a typed `EngineError::FrontierIntegrity` is sufficient and lives in the implementation PR rationale.

**Architectural-fit verdict:**
- Decision gate: all green (strengthens §11.1; no new public surface; no new L2; no upward dep).
- Bounded context: Exec (engine) + Storage (lease trait already exists).
- Concept promotion: 🟢 — uses existing `acquire_lease` / `renew_lease` / `release_lease` methods and existing CAS interface. No new abstraction.
- Quick-Win trap risk: low; the temptation here is to "log and continue" on CAS mismatch (current behavior) — explicitly forbidden by §11.1.

**Smallest correct fix shape:**
- **#325:** Wrap `WorkflowEngine::execute_workflow` and `resume_execution` with `acquire_lease` → renew loop → `release_lease`. Backoff or fail on `LeaseUnavailable`.
- **#333:** On CAS mismatch in `checkpoint_node`, reload full state, classify the conflict (cancel from API → honor; foreign mutation → propagate `EngineError::ConflictReconciliationFailed`), retry once, then abort.
- **#341:** Gate `determine_final_status` on `exec_state.all_nodes_terminal()`; non-terminal exit returns `EngineError::FrontierIntegrity` and emits a diagnostic event.

**Recommended PR sequencing within Group C:**
- **C1:** #341 invariant guard (smallest; lands first as scaffolding for tests in C2/C3).
- **C2:** #325 lease lifecycle around execute + resume.
- **C3:** #333 CAS reconcile with conflict classification.

**Acceptance:** Concurrency test (two engine instances, same execution_id) — exactly one runner makes progress; the other backs off with a typed error. Concurrent API cancel during run — engine sees the cancel, no overwrite. Bookkeeping fault-injection — engine fails loudly, no false success.

**Cross-group dependency:** C2 depends on Group A (cancel via control queue) reaching the engine — without it, C3's "honor cancel on CAS mismatch" cannot be tested.

---

### Group D — Checkpoint ordering and stateful state

**Issues:** [#297](https://github.com/vanyastaff/nebula/issues/297), [#321](https://github.com/vanyastaff/nebula/issues/321), [#308](https://github.com/vanyastaff/nebula/issues/308)

**Root-cause hypothesis:** Two related but distinct gaps:

- **#297 + #321** — checkpoint discipline. Engine emits events / activates edges / runs error-routing **before or without** persisting. Symptom of one principle violation: *persist before any externally observable side effect*. Note: explorer found the success path at `engine.rs:1219-1262` already in the order persist → idempotency → event → edges, while the issue body cites lines 1064-1098 with the wrong order. **Verify pre-implementation** which branch is current; either way #321's setup-failure-without-checkpoint asymmetry is real.
- **#308** — stateful handler state. `StatefulCheckpointSink` infrastructure exists in `crates/runtime/src/runtime.rs:74` and is wired into `execute_action_with_checkpoint`, but `NodeTask::run` always calls `execute_action_versioned` which passes `checkpoint: None`. Mid-iteration state never reaches the sink.

**Canon impact:**
- §11.5 (checkpoint policy + best-effort failure mode) — #297 and #321 currently violate the implicit ordering this section assumes.
- §11.5 + §11.1 — #308 implements a "post-MVP" gap that is documented in code but not reflected in operator-facing capability claims.
- §13 integration bar #5 (non-idempotent side effects under retry/restart pressure) — #297 directly enables the failure mode this bar exists to prevent.

**ADR needed:**
- **#297 + #321:** **NO.** Implementation rationale in PR body; verify ordering claim against current code first.
- **#308:** **YES.** Wiring `StatefulCheckpointSink` end-to-end requires answering: handler state serializability contract; resume entry point shape; `non_checkpointable` opt-out; `NodeAttempt`/iteration-record schema. Suggested ADR title: *"Stateful handler state durability contract."*

**Architectural-fit verdict (compact — these were not the two skill-required groups, but checked):**
- #297/#321 — 🟢/🟡, no new abstractions; ordering correction within `run_frontier`.
- #308 — 🔴, new L2 contract on `StatefulHandler` (state must be `Serialize + Deserialize + Default` or explicitly opt-out). ADR required.

**Smallest correct fix shape:**
- **#297:** Verify branch ordering against `engine.rs:1064-1098` and `1219-1262`. Move all `emit_event` / `process_outgoing_edges` / `record_idempotency` calls to **after** `checkpoint_node` succeeds.
- **#321:** Add `checkpoint_node` call in setup-failure branch (`spawned == false`) before `handle_node_failure` returns control.
- **#308:** ADR; plumb `StatefulCheckpointSink` from `NodeTask` into `execute_action_with_checkpoint`; extend `NodeExecutionState` with iteration record; resume hydrates last checkpoint instead of `init_state()`.

**Recommended PR sequencing within Group D:**
- **D1:** #321 (smallest; no design choice).
- **D2:** #297 ordering correction + crash-window regression test (uses fault injection).
- **D3:** ADR + #308 stateful checkpoint contract.

**Acceptance:** Crash-injection test: kill engine between any two adjacent operations in `run_frontier` and verify no externally-observable state escapes the persisted state. Stateful handler resume test: 10-iteration handler crashes at iteration 5, resumes at iteration 5 (not 0).

**Cross-group dependency:** D1 + D2 should land **before** Group B begins; B inherits the persist-then-announce ordering when extending what is persisted.

---

### Group E — Retry honesty (false capability)

**Issues:** [#290](https://github.com/vanyastaff/nebula/issues/290)

**Root-cause hypothesis:** `ActionResult::Retry` is a public variant the engine does not honor end-to-end. Current handling at `engine.rs:1173-1217` synthesizes `ActionError::retryable("Action retry is not supported by the engine")` and routes through failure path. Comment in code confirms: "ActionResult::Retry has no scheduler yet." This is the canonical example of canon §11.2 false-capability + §14 phantom-types anti-pattern.

**Canon impact:**
- §11.2 (retry honesty) — explicitly named as canon debt; status table marks engine-level retry as `planned`.
- §4.5 (operational honesty — no false capabilities) — §11.2 cites this exact variant as the example.
- §14 (anti-pattern: phantom types) — exact match.

**ADR needed:** Depends on the chosen direction:
- **Removal path:** Hide variant under `unstable-retry-scheduler` feature gate or delete entirely. **NO ADR** needed (executes existing canon §11.2 row directly). Smallest possible fix; aligns docs and code in one PR.
- **Implementation path:** Build the durable retry scheduler. **YES ADR** for: persisted attempt accounting schema, backoff policy, integration with existing `nebula-resilience`. Suggested title: *"Engine-level node retry scheduler with persisted attempt accounting."*

**Architectural-fit verdict (skill-checked):**
- Removal path: 🟢, decision gate all green, no new abstraction.
- Implementation path: 🔴, new L2 contract on per-attempt durability; ADR required.

**Recommendation:** **Removal first** as a fast canon-honoring PR (E1); implementation path is a separate roadmap item that can move §11.2's row from `planned` to `implemented` later. Removing the variant unblocks Group F (#298 currently surfaces a `retryable` error that has no scheduler — once `Retry` is honest, the rate-limit error path becomes equally honest).

**Acceptance (E1 removal):** `ActionResult::Retry` is `pub(crate)` or behind `unstable-retry-scheduler` feature; `nebula-action` docs no longer describe engine-level retry as a current capability; canon §11.2 status table updated if needed.

---

### Group F — Silent error swallow (rate limiter)

**Issues:** [#298](https://github.com/vanyastaff/nebula/issues/298)

**Root-cause hypothesis:** Originally a §12.4 violation (logged-and-discarded rate limit error). **Current `main` already partially fixes it** — `engine.rs:1775-1795` now fails the node with `ActionError::retryable_with_hint(RateLimited)`. Remaining gap: the surfaced `retryable` error has no scheduler (depends on Group E). After Group E lands, this error is honestly terminal-with-classification.

**Canon impact:**
- §12.4 (errors and contracts) — original violation; mitigated.
- §11.2 (retry honesty) — surfaced error currently leans on a scheduler that does not exist. Tied to Group E.

**ADR needed:** **NO.**

**Smallest correct fix shape:** Verify the issue body's described path (`engine.rs:~1542` log-and-fall-through) is no longer present anywhere; close the issue with a commit-ref comment if the only remaining concern is the absent retry scheduler (which Group E owns); otherwise file a tiny follow-up PR adjusting the error classification.

**Sequencing:** F1 lands **after** Group E (so the resolution is coherent — limiter error is no longer pretending an unimplemented retry mechanism exists).

**Acceptance:** No code path returns `Ok` after `limiter.acquire().await.is_err()`. Log message matches actual behavior.

---

## 3. Recommended PR sequencing across groups

Dependencies (→ means "blocks"):

```
A1 (ControlConsumer skeleton + ADR)
  → A2 (start enqueue/dispatch)
  → A3 (cancel dispatch)
  → A4 (knife integration test)
       → C2 (lease lifecycle — needs cancel signal to test conflict honoring)
       → C3 (CAS reconcile — needs cancel signal)

C1 (#341 invariant guard) — independent, lands first as test scaffolding

D1 (#321 setup-failure checkpoint) — independent
D2 (#297 ordering) — independent of Group A
  → B (resume correctness inherits persist-then-announce)

D3 (#308 stateful contract ADR + impl) — independent

B1 (resume schema ADR + migration)
  → B2 (workflow input persist)
  → B3 (ActionResult variant persist)
  → B4 (resume uses real evaluate_edge)

E1 (remove/gate ActionResult::Retry) — independent
  → F1 (close #298 with commit-ref or tiny follow-up)
```

**Suggested calendar order (independent of resourcing):**

1. **C1** — invariant guard (one-line + test). Lands fast; provides scaffolding.
2. **D1** — setup-failure checkpoint symmetry. Fast.
3. **A1 → A4** — control plane wiring. **Highest priority** — without it the §13 knife is stub-grade and Groups B/C/D cannot be integration-tested.
4. **D2** — checkpoint ordering correction. Required before B.
5. **C2 → C3** — execution authority enforcement (now testable thanks to A).
6. **B1 → B4** — resume correctness (now safe to extend persistence thanks to D2).
7. **E1** — remove false capability. Independent; can land any time after this document is signed off.
8. **D3** — stateful contract. Largest design surface; lands when capacity is available.
9. **F1** — close #298.

---

## 4. ADR table

| Group | ADR needed? | Suggested title |
|---|---|---|
| A | **YES** | `execution_control_queue` consumer wiring and start-side enqueue contract |
| B | **YES** | Resume correctness: persisted edge-activation + workflow input + ActionResult variant |
| C | NO (#325, #333); maybe small note for #341 | — |
| D | NO (#297, #321); **YES** for #308 | Stateful handler state durability contract |
| E | NO if removal; **YES** if implementation | Engine-level node retry scheduler with persisted attempt accounting (only if implementing) |
| F | NO | — |

Three concurrent ADRs (A, B, D-stateful) is the maximum — they touch distinct seams and can be drafted in parallel.

---

## 5. Out of scope for this planning chip

- No code changes. No ADR drafts. No issue closes (including the #298 partial-mitigation note above — that requires a verification commit-ref, deferred to F1).
- No estimation of engineering effort per group. Tech-lead is the owner of effort calls.
- No reassignment of issue labels or milestones in GitHub.
- Group D's #297 verification (which `run_frontier` branch matches the issue body) is a **first task of D2**, not this chip.

---

## 6. Hand-off

Tech-lead review requested. Specific decisions to sign off:

1. **Cluster grouping accepted?** Six groups as above, or split / merge differently?
2. **Sequencing accepted?** Specifically: A before B/C, D2 before B, E independent.
3. **ADR scope accepted?** Three required ADRs (A, B, D-stateful); E-implementation deferred to whenever §11.2 row is moved to `implemented`.
4. **Recommendation on Group E:** removal first (E1), implementation later — confirm or override.
5. **Group F resolution:** close #298 with commit-ref after E1 lands? Or keep open as scheduler-debt tracker?

Sign-off captured as a comment on this file. Implementation chips are spun up per-group only after sign-off.

---

## 7. Tech-lead sign-off (2026-04-18)

**Verification pass before sign-off:** confirmed the smoking gun (`grep -rn "ControlQueueRepo\|ControlCommand::" crates/engine/src/` returns zero hits; API `handlers/execution.rs:338` is producer-side only), confirmed #298 partial mitigation (`engine.rs:1775-1795` fails the node on limiter error), confirmed `StatefulCheckpointSink` trait exists at `crates/runtime/src/runtime.rs:74` but is called only in tests, confirmed lease methods exist in `crates/storage/src/repos/execution.rs`. Canon §11.2 explicitly names `ActionResult::Retry` as the false-capability example ("hide or delete until end-to-end") — this is load-bearing for decision 4 below.

### Q1. Six-group clustering — **YES**

The groups cut cleanly along root causes, not symptoms. Group A correctly treats the three API issues as one architectural gap (the missing consumer half) rather than three coincidental bugs — that framing is the single most important decision in this plan and it is right. Group B's four-issues-one-schema framing passes the next-month test: fixing #311 alone would force a second migration when #324/#336/#299 land. Group C's three authority faces share enough machinery (CAS, leases, frontier checks) that splitting would duplicate tests.

### Q2. PR sequencing — **MODIFY**

Agree with A-before-B/C, D2-before-B, E-independent. **One correction: C1 (#341 invariant guard) should land first and does not block on A.** The plan already says this in §3 item 1 but the §6 question framing implies A precedes all of C. Keep the §3 ordering; disregard any implication that C1 waits on A. C1 is a one-line guard plus a test and it provides scaffolding every other group benefits from.

Also: **A2 and A3 should not be a single PR** even though they share a consumer. A2 exercises the start dispatch path end-to-end; A3 exercises cancel. Combining them obscures which dispatch direction broke when a regression hits. Keep them separate as the plan proposes.

### Q3. Three ADRs — **YES**

A, B, and D-stateful are correctly identified. Nothing missing. Specifically: #341 does **not** need an ADR (one-line guard with typed error), #325+#333 do not (implementing an existing canon section), #297+#321 do not (bug fixes with clear correct shape), F does not. Three concurrent ADRs is the ceiling — authors should coordinate so B's schema ADR does not presuppose a consumer-wiring choice A's ADR hasn't landed yet.

### Q4. Group E removal-first — **CONFIRM**

Canon §11.2 names this exact variant; removal is the shortest path to canon honesty. Implementation is a roadmap item, not a reaction to a P1. E1 should hide the variant behind `unstable-retry-scheduler` feature rather than `pub(crate)` — preserves the surface for the future implementation PR and signals intent to downstream crates. Update canon §11.2 status table wording in the same PR.

### Q5. Group F via commit-ref close — **CONFIRM**

After E1 lands the remaining #298 concern evaporates (the `retryable_with_hint` error is no longer leaning on a phantom scheduler). Close with a commit-ref comment linking E1 and the current `engine.rs:1775-1795` path. No follow-up PR needed unless reviewer spots residual log/error-classification mismatch.

### Cross-cutting concerns

- **Hidden coupling between A4 and the `simple_server.rs` example.** §12.2 requires the demo either use the real consumer or be marked `// DEMO ONLY`. The plan mentions this in A4 but buries the choice — force the decision in A1's ADR, not A4's PR body.
- **B3 is the risky PR in the whole cluster.** Persisting full `ActionResult<Value>` means any future variant must be forward-compatible or gated by a schema version. B1's ADR must call this out explicitly; otherwise a later `ActionResult` variant addition silently breaks resume.
- **Decision-gate Q4 (cross-cutting → integration leak) is worth re-checking for A.** The proposed `ControlConsumer` lives in `engine` but dispatches to `WorkflowEngine::execute_workflow`. Confirm in the ADR that no type from `api` or `storage` leaks into its public surface — the dispatch handle should be an `engine`-owned trait.
- **Memory cross-check:** prior feedback on direct state mutation (`ns.state = X` bypassing version bumps) is adjacent to Group C's authority work. C2/C3 reviewers should re-scan for `let _ = transition_node(...)` and direct `ns.state =` writes as a bycatch of the lease/CAS work.

**Overall: signed off. Proceed to spin up implementation chips starting with C1 and D1 in parallel, then A1.**
