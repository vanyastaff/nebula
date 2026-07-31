# Nebula Engineering Roadmap - Week 32

> **Window:** 2026-08-03 through 2026-08-09  
> **Theme:** Make the durable runtime real in first-party deployments  
> **Baseline:** `origin/main` at `3a394101` (local `main` was one commit behind when researched)  
> **Status:** Execution plan, not product canon. `docs/PRODUCT_CANON.md` wins on conflict.

---

## Executive decision

Week 32 is a runtime-integrity week. The repository already has strong component contracts, but
the first-party server and worker do not yet prove one durable execution path end to end. The
new expected-RED harness makes this measurable: ten scenarios currently reach real product
defects across start deduplication, control-claim fencing, restart/resume, and cancellation.

The week's goal is therefore:

> **Turn the ten named runtime-repair failures into ordinary passing conformance tests without
> weakening, skipping, retrying, or deleting an oracle.**

Breaking internal APIs are allowed. Use that freedom to remove ambiguous ownership rather than
adding compatibility shims. Keep `nebula-sdk` compatibility decisions explicit because it is the
sole supported Rust surface.

## Research snapshot

### What is already true

- Rust **1.97.1** is the current stable point release and is already pinned in
  `rust-toolchain.toml`; the workspace declares Rust 1.97 and edition 2024. No toolchain upgrade is
  required this week. The point release fixes an LLVM miscompilation, so downgrading is not an
  acceptable simplification. Source: [Rust 1.97.1 release announcement](https://blog.rust-lang.org/2026/07/16/Rust-1.97.1/).
- `ExecutionStore::commit(TransitionBatch)` is the execution aggregate's CAS-and-fencing commit
  boundary. It remains the only legal execution-state mutation path.
- `ControlConsumer` and all five control commands exist, but no non-test first-party composition
  root installs the complete consumer/dispatch pair against the same durable backend as HTTP.
- The expected-RED profile names ten genuine failures: C0 on SQLite/PostgreSQL, C1 on
  SQLite/PostgreSQL, C7 on all three backends, and StartKey on all three backends.
- Ordered migrations and the exact plan/flavor vocabulary are present. SQL catalog adapters,
  atomic start materialization, and production exact-flavor consumption are not.
- PostgreSQL control claiming still contains the ambiguous unqualified `RETURNING id` defect in
  [issue #756](https://github.com/vanyastaff/nebula/issues/756).
- Resilience issues [#631](https://github.com/vanyastaff/nebula/issues/631) and
  [#634](https://github.com/vanyastaff/nebula/issues/634) have regression behavior in the current
  tree and appear stale. Issue [#633](https://github.com/vanyastaff/nebula/issues/633) has an
  explicit bounded API, but `Gate::close()` itself can still wait forever. Issue
  [#632](https://github.com/vanyastaff/nebula/issues/632) remains reproducible in the manual
  `Future` implementation.

### Architecture constraints

1. **One aggregate writer.** HTTP accepts intent; runtime control owns lifecycle transitions.
2. **One durable control path.** EventBus can wake a consumer but cannot carry correctness.
3. **One backend per deployment profile.** HTTP and worker must share the selected SQLite file or
   PostgreSQL database. Separate defaults cannot be called an end-to-end deployment.
4. **A claim is an attempt, not a worker name.** A stable processor identity cannot fence an ABA
   reclaim. Completion must present an unrepeatable claim generation.
5. **A start key identifies one accepted command.** Same key plus same fingerprint returns the
   original execution; same key plus a different fingerprint fails with no durable delta.
6. **Exact revisions fail closed.** Runtime dispatch never falls back to “latest” plugin,
   workflow, plan, or flavor state.
7. **Remote effects remain at-least-once.** Do not claim exactly-once behavior before the ADR-0120
   operation ledger and provider capability contract are implemented.

---

## Week outcome and scorecard

| Measure | Start | Exit target |
|---|---:|---:|
| Expected-RED cases | 10 failing | 0 remaining |
| Shared C0-C10 backend oracle | Partial | C0, C1, C7, StartKey shared across applicable backends |
| PostgreSQL control-queue bug #756 | Open/reproducible | Fixed and exercised in required live-PostgreSQL CI |
| First-party durable control composition | Test-only/manual | Installed and supervised in the app-owned profile |
| Same-processor ABA acceptance | Accepted incorrectly | Rejected by generation fencing on every backend |
| Duplicate keyed starts | Creates multiple executions | One durable execution identity |
| API-owned terminal cancellation | Present | Removed; runtime control terminalizes |
| Latest-revision fallback in repaired path | Reachable | Zero |

The week is successful only if the runtime-repair manifest, tests, and North Star state remain
truthful together. A passing test obtained by weakening evidence is a failed week.

---

## Delivery sequence

### Day 1 - Establish the green floor and repair PostgreSQL claiming

**W32.1 - Synchronize and prove the baseline**

- Fast-forward the working branch to `origin/main`; do not reimplement work already present in
  the equivalent `vanyastaff/audit-rust-workflow-runtime` tree.
- Run the North Star registry validator and expected-RED manifest validator before editing.
- Capture the exact ten expected failures with the serial, retry-free profile.
- Run the live PostgreSQL control-queue slice and preserve the #756 failure as a regression test.

**W32.2 - Fix #756 at the SQL boundary**

- Add an update-specific qualified projection (`e.id`, `e.execution_id`, and so on) rather than
  changing the shared single-table projection.
- Keep `FOR UPDATE SKIP LOCKED`, ordering, and atomic update/return semantics unchanged.
- Add a live-PostgreSQL assertion that exercises the CTE plus `UPDATE ... FROM ... RETURNING`
  shape. A compile-only SQL test is insufficient.
- Confirm the PostgreSQL job fails when the service or `DATABASE_URL` is absent; skip-clean is not
  release evidence.

**Exit gate:** all control-queue tests pass on a live PostgreSQL instance and no query-site
projection becomes less explicit.

### Day 2 - Replace processor-name fencing with claim-generation fencing

**W32.3 - Close C7 on InMemory, SQLite, and PostgreSQL**

Introduce one port-level claim capability, for example:

```rust
pub struct ControlClaim {
    pub entry: ControlQueueEntry,
    pub token: ControlClaimToken,
}
```

The token contains the row identity plus a storage-minted, monotonically increasing generation.
`claim_pending` returns claims; `mark_completed` and `mark_failed` consume or borrow the opaque
token. They no longer accept `processor: &[u8]` as authority.

Implementation rules:

- Increment generation in the same atomic statement/critical section that changes Pending to
  Processing.
- Fence acknowledgement on `(row_id, status = Processing, claim_generation)`.
- Keep processor identity only as bounded observability data.
- Reclaim clears current ownership but never decrements or reuses a generation.
- Use checked integer conversion and fail closed at overflow.
- Add migration `0042` only after reviewing the ordered-migration catalog floor/head contract;
  update both dialects and the executable catalog-boundary assertion in the same change.
- Share one behavioral suite across InMemory, file SQLite, and live PostgreSQL. Prove the same
  processor identity cannot acknowledge an earlier generation after reclaim.

**Exit gate:** all three C7 cases pass, stale acknowledgements produce zero state change, and
NS03 evidence can be generated from the shared oracle.

### Day 3 - Make start acceptance atomic and idempotent

**W32.4 - Close StartKey and the split create/enqueue gap**

Create a runtime-control command boundary for start acceptance. The owning transaction must:

1. Canonicalize the accepted request and compute a versioned fingerprint.
2. Reserve `(tenant_scope, start_key)` with that fingerprint.
3. Materialize the exact execution/workflow/bundle/plan/flavor identities.
4. Create the execution aggregate.
5. Enqueue the Start control row.
6. Commit once and return the durable execution identity.

Required outcomes:

- Absent key: create exactly one execution and Start command.
- Same key and fingerprint: return the original acceptance receipt without new rows.
- Same key and different fingerprint: return typed `StartConflict`/HTTP 409 with no mutation.
- Lost HTTP response: a retry converges on the same receipt.
- Logs and errors expose hashes and typed IDs, never workflow inputs.

The HTTP idempotency middleware may cache a response, but it is not the execution aggregate's
authority and cannot substitute for this transaction. Put the port contract below API and the
backend transaction in `nebula-storage`; keep orchestration policy out of SQL adapters.

**Exit gate:** all three StartKey cases pass and the database shows one execution plus one start
command for the accepted key.

### Day 4 - Install the durable consumer and correct cancellation ownership

**W32.5 - Close C0 first-party drive reachability**

- Build one app-owned runtime profile that selects a backend once and hands the same durable
  ports to HTTP acceptance, `ControlConsumer`, engine dispatch, timers, and reads.
- Supervise every top-level task under one cancellation tree. Return join failures to the app;
  do not detach scanner or consumer tasks.
- Start readiness only after migrations/admission, exact catalog loading, consumer startup, and
  dispatch readiness succeed.
- On restart, reclaim expired claims, load the exact pinned revisions, and resume from persisted
  state. Missing or draining revisions return typed fail-closed errors.
- Preserve SQLite as the no-Docker local path and PostgreSQL as the required multi-process path.

**W32.6 - Close C1 cancellation authority**

- API cancellation performs authorization and submits durable intent; it may expose `Cancelling`
  but must not synthesize terminal `Cancelled`.
- Append the cancel command in the same execution commit as the non-terminal cancellation state.
- `ControlConsumer` dispatches cancellation to the live engine or recovers it after restart.
- Runtime control records the terminal transition and journal evidence only after the engine has
  honored the command.
- Duplicate Cancel is idempotent; Terminate remains a distinct escalation command.

**Exit gate:** C0 and C1 pass on file SQLite and live PostgreSQL, including process restart and a
cancel issued before handler completion.

### Day 5 - Converge evidence, observability, and stale backlog

**W32.7 - Promote RED cases without deleting the evidence contract**

- Convert repaired scenarios to normal conformance coverage; update the expected-failure
  manifest atomically so an unexpected pass cannot hide.
- Retain the deterministic clocks, barriers, independent provider probes, and sanitized artifacts.
- Emit bounded outcome vocabulary for accepted, fenced, deferred, recovered, and mismatch paths.
- Update `docs/MATURITY.md`, `docs/QUALITY_GATES.md`, relevant crate READMEs, and North Star states
  only for guarantees actually demonstrated.

**W32.8 - Resolve live issue hygiene**

- Verify and close #631 and #634 with their existing regression tests and fixing commit references.
- For #633, make the primary shutdown API bounded and typed. Prefer a required deadline/budget or
  `Result<(), GateCloseTimeout>` over a silent built-in timeout. Remove or privatize the unbounded
  public path; update callers in the same breaking change.
- For #632, remove the manual poll implementation in favor of one owned async `select!` future,
  unless a stored cancellation future can be expressed without self-reference. Add a
  frequently-yielding regression benchmark and cancellation wakeup test.
- Re-triage the remaining issues using the queue below; do not mix low-value cleanup into the
  runtime repair pull requests.

**Exit gate:** `task dev:check`, live PostgreSQL conformance, rustdoc, both feature-matrix Clippy
passes for affected crates, North Star validation, and the repaired runtime suite are green.

---

## Pull request boundaries

Keep changes reviewable and preserve bisectability:

| PR | Scope | Depends on |
|---|---|---|
| A | PostgreSQL projection fix and mandatory regression (#756) | Baseline |
| B | Opaque control claim token plus three-backend C7 conformance | A |
| C | Atomic keyed start acceptance and receipt | B |
| D | App-owned durable consumer composition and C0 | B, C |
| E | Cancellation ownership and C1 | D |
| F | Evidence promotion, docs, issue closure, bounded resilience APIs | A-E |

Do not combine the exact-revision SQL catalog expansion or remote-effect ledger with PR A or B.
Those are separate aggregate contracts with different failure modes.

## Required verification

Run focused checks after each PR, then the full gate at convergence:

```bash
cargo check -p nebula-storage --features sqlite,postgres
cargo nextest run -p nebula-storage --features sqlite,postgres
cargo nextest run -p nebula-resilience
cargo nextest run -p nebula-server
cargo clippy -p nebula-storage --all-targets --all-features -- -D warnings
cargo clippy -p nebula-storage --all-targets --no-default-features -- -D warnings
cargo clippy -p nebula-resilience --all-targets --all-features -- -D warnings
cargo clippy -p nebula-resilience --all-targets --no-default-features -- -D warnings
cargo xtask north-star-gates validate
cargo xtask runtime-repair-red validate-manifest
task dev:check
```

The live PostgreSQL suite must run with the repository's required database service. A skipped
test is not a pass. Keep chaos/loom checks focused on the changed ownership protocol.

---

## Follow-on queue after Week 32

### P0 - Release integrity

1. **Exact revision catalog, Task 13B.** Implement SQLite/PostgreSQL catalog adapters, atomic
   reference mutation, `materialize_start`, terminal dereference, draining, retention, and
   three-backend conformance. This closes NS01/NS02 rather than merely storing identities.
2. **Durable operation ledger, ADR-0120.** Storage-mint `EffectSlotId` and `OperationId`, inject the
   ID into adapters, persist prepared/outcome/unknown states, and add crash/fencing evidence for
   NS17. Stable-key recovery must be capability-gated and bounded; reconciliation is read-only.
3. **Checkpoint policy enforcement.** Either honor every non-`Inherit` `CheckpointPolicy` through
   deployment backends or remove it from the public action metadata until it is real.
4. **Resource activation and bind population.** Add the production caller that populates the
   credential-slot reverse index, then prove refresh/revoke fan-out after restart and total
   EventBus loss (NS10).
5. **Credential K3.** Make the controller plus operation ledger the sole semantic writer; add
   owner-qualified poison reconciliation, durable sentinel-to-reauth commands, and transactional
   audit/outbox evidence.

### P1 - Supported product surface

6. **Finish engine-honored API phases.** Implement only endpoints backed by a real runtime command
   and aggregate owner; keep the rest honest 501 responses or remove them.
7. **SDK client and embedded personas.** Ship curated façades that cannot expose raw stores,
   mutation authority, claim tokens, or tenant proofs. Add clean external one-dependency fixtures
   for NS18.
8. **SDK release discipline.** Add packaged-manifest checks, exact internal dependency versions,
   API snapshots, semver classification, MSRV, rustdoc, and supported feature matrices (NS19/20).
9. **Activation diagnostics.** Standardize code, path, expected, actual, and remediation fields;
   prove the full shape in compiler/activation conformance (NS14).
10. **Operator evidence.** Unify durable journal and telemetry outcomes, then add the seeded
    incident drill and bounded diagnosis measurement (NS16/21).

### P2 - Capacity, DX, and research

11. Add versioned saturation/fairness measurements (NS06), fresh-project typed-action timing
    (NS12), and credential-backed resource timing (NS13).
12. Add OpenAPI-to-live-runtime server/client conformance beyond structural spec drift (NS15).
13. Enforce the governance block on hybrid persistence until an approved benchmark and reversal
    check exist (NS22). Do not begin a new backend before this evidence.

---

## Live GitHub issue disposition

| Issue | Decision |
|---:|---|
| #756 | Week 32 PR A; release-path correctness blocker |
| #634 | Verify existing hint-preservation regression and close as stale/fixed |
| #633 | Week 32 PR F; make the primary close contract bounded and typed |
| #632 | Week 32 PR F; replace per-poll cancellation-future recreation |
| #631 | Verify pipeline hint-floor regression and close as stale/fixed |
| #603 | Defer; positioning research does not unblock runtime integrity |
| #601 | Defer; language-evolution watch item, no current migration trigger |
| #600 | Defer to the NS06 performance program after correctness gates |
| #599 | Defer until the release surface and version policy are active |
| #597 | Reassess after C7 and durable drive ownership; then specify priority/DLQ/backoff from evidence |
| #596 | Fold actionable checks into review policy after the runtime repair, not as a docs-only detour |
| #594 | Defer flight-recorder evaluation until NS21 outcome vocabulary is stable |
| #593 | Small documentation candidate after runtime repairs; no architecture dependency |
| #592 | Design with real I/O caller evidence; do not expand error taxonomy speculatively |
| #589 | Low-risk cleanup only after lifecycle correctness and NS10 evidence |
| #586 | Add the async CPU-pool rule when a real handler needs Rayon or as a focused policy PR |
| #585 | Add to the next engine concurrency hardening slice; useful but not on the Week 32 critical path |
| #584 | Reproduce and prioritize with queue-pressure evidence after durable dispatch composition lands |

---

## Explicit non-goals for the week

- No WASM, process-isolated, dynamically loaded, FFI, or remote plugin execution.
- No Redis execution backend, local-filesystem backend, or hybrid persistence design.
- No `ActionResult::Retry` variant; operator-declared retry remains the implemented engine surface.
- No provider-specific Plane-B OAuth ceremony.
- No result-driven claims of exactly-once remote effects.
- No broad dependency refresh or Rust upgrade; Rust 1.97.1 is already the stable target.
- No compatibility shims for internal APIs replaced by claim tokens or runtime commands.

## End-of-week definition of done

- All repaired behavior enters through the real first-party composition root and real backend
  adapters, not a test-only alternate implementation.
- InMemory, SQLite, and PostgreSQL share semantic oracles where the capability applies.
- Every new state/error/hot path has a typed error, tracing span/event, bounded metric labels, and
  invariant test.
- No secret, workflow input, raw business payload, DSN, or authority token enters evidence or logs.
- Migrations are forward-only, ordered, locked, and admitted under the repository's catalog rules.
- Documentation states implemented, best-effort, experimental, and planned behavior accurately.
- The full pre-PR gate is green, and required PostgreSQL evidence did not skip.

