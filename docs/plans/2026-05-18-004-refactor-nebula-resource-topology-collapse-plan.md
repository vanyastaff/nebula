---
title: "refactor: nebula-resource topology collapse + security barrier + latent-bug closure"
type: refactor
status: active
date: 2026-05-18
origin: docs/brainstorms/2026-05-18-002-nebula-resource-topology-collapse-requirements.md
---

<!-- // budget-justified: ce-plan Deep implementation plan — one coherent phased artifact by design, not decomposable. ADR-0083 blob cap is a Rust per-function complexity proxy (clippy.toml too-many-lines, set to allow workspace-wide) and carries no meaning for a markdown plan doc. -->

# refactor: nebula-resource topology collapse + security barrier + latent-bug closure

## Summary

Collapse the 5-topology taxonomy to `Pool` + `Resident` + one parameterized `Bounded` behind the existing enum seam (native AFIT, no dyn/async-trait), folding ~35 hand-copied Manager methods into one generic acquire/register path. Ride the same seam to: replace the `DefaultHasher`→u64 cross-tenant barrier with a structural identity, fix the Pool revoke→recycle TOCTOU, stop swallowing release-hook errors, delete dead surface (identity-agnostic acquire tier, recovery group/watchdog), and fold in the per-acquire allocation wins. Executed expand-contract, no shim, no ADR; behavior preserved except the explicitly-listed latent-bug fixes.

---

## Problem Frame

`nebula-resource` is structurally sound in domain logic but its 5-topology model fans into a 3118-line God-object `Manager` (~35 near-duplicate methods, 5 byte-identical `run_*_acquire`, 3-name passthrough, ~17 register shorthands). A deep multi-agent audit additionally found a **critical cross-tenant isolation defect** (the sole barrier is a 64-bit fixed-seed SipHash equality — collision silently merges tenants, bypassing the fail-closed `Ambiguous` deny), a **high-severity Pool revoke TOCTOU** (drained-then-recycled instance serves a revoked credential), **silently swallowed release-hook errors**, and dead infrastructure. Full pain narrative and the 6 original defects: see origin (`docs/brainstorms/2026-05-18-002-nebula-resource-topology-collapse-requirements.md`).

---

## Requirements

**Carried from origin (topology collapse — see origin: R1–R14)**
- R1. Keep `Pooled` + `Resident` as distinct topologies/runtimes.
- R2. Fold `Service`/`Transport`/`Exclusive` into one parameterized `Bounded` runtime; preserve Cloned/Tracked token modes, keepalive, reset-on-release permit ordering.
- R3. One object-safe topology seam replacing the scattered 5-arm match.
- R4. One unified `Bounded` author trait replacing the 3 folded traits; `Pooled`/`Resident` author traits unchanged.
- R5. One generic `run_acquire` replacing the 5 `run_*_acquire`; dead `_ => expected X topology` arms gone.
- R6. Passthrough indirection reduced to one layer.
- R7. `RegistrationSpec`-style param aggregate; no `#[allow(clippy::too_many_arguments)]` in Manager.
- R8. All acquire through the erased seam; typed per-topology fan-out removed.
- R9. One canonical two-phase-revoke invariant doc; other sites reference it.
- R10. Registry single-pinned lookup returns a type with no `Ambiguous` variant.
- R11. Externally observable semantics unchanged except the explicitly-listed latent-bug fixes (R15–R17).
- R12. Expand-contract; per-touched-crate green at each commit, whole-workspace green at push; no shim.
- R13. All folded-trait impls + tests/examples migrated in-tree.
- R14. Rationale recorded in this plan + canonical crate module doc; no ADR.

**Added from the deep audit (ride the same seam)**
- R15. Replace the `slot_identity` `DefaultHasher`→u64 cross-tenant barrier with a collision-free **structural identity: an ordered `(SlotKey, CredentialKey)` set with derived `Eq`/`Hash`** — NOT a digest (a digest reintroduces a collidable space and, if it preserves the `u64` wire type, leaves the weak primitive callable). Subsumes R10. Security-critical. **Cross-crate consequence (not internal-only):** `slot_identity: u64` is published API (`pub use dedup::slot_identity`, `RegisterOptions.slot_identity`, `acquire_erased(.., u64)`, `has_registered_for_scope`) and the engine independently recomputes/persists it (`engine::record_resource_slot_identity`, `BindEntry.slot_identity`, `ResourceFanoutIndex::bind`, rotation-fan-out reverse-index key, engine test literals). The structural-type change therefore breaks ~6 engine production sites + the fan-out index and is part of the U11 migration + U12 atomic contract, NOT an internal `dedup.rs` change.
- R16. Fix the Pool revoke→recycle TOCTOU: a credential revoked via `drain_and_revoke` must not be reachable through a post-drain instance. **Documented-downgrade forbidden** (F1 cross-tenant defect, not a perf nicety). **Mechanism (specified, evidence-backed — HikariCP #1836 is the same TOCTOU class; the ecosystem-proven fix is post-acquisition epoch re-check, not evict-harder):** **every** path that returns an instance to the idle queue MUST consult a per-row revoke epoch and `destroy` (not recycle/admit) when the epoch advanced past the entry's checkout/creation, BEFORE `on_credential_revoke` is dispatched. The enumerated set is **all five** `pool.rs` push-to-idle sites — `release_entry` recycle, the in-flight `create`/`CreateGuard` that completes after the drain (pulled in from deferred F2; #1836 proves it the *same* defect, not separable), `warmup_sequential`, `warmup_staggered` (a staggered warmup still running, or a reload-driven re-warm, can deposit a post-revoke instance into idle with no epoch check today), and the **`run_maintenance` re-deposit** (`should_evict` consults only `fingerprint`/`max_lifetime`/`idle_timeout`, then `keep.push_back`s every non-evicted entry — a concurrent maintenance cycle running after `drain_and_revoke`'s idle walk re-admits a non-stale, non-timed-out pre-revoke instance with no epoch check today; the revoke epoch is explicitly distinct from `fingerprint`, so the existing `should_evict` arms do not cover it). Enumerating only `release_entry`+`CreateGuard` would leave warmup *and maintenance* as believed-fixed-but-open cross-tenant paths — explicitly in scope. The `ReleaseQueue`-quiesce variant is acceptable only as a complement, not the sole mechanism (HikariCP #1836 shows evict-only misses the in-flight-create race). **Scope note:** this prevents a revoked-credential instance being recycled/created-into-idle and handed onward (cross-tenant reuse); it does NOT retroactively kill an already-authenticated in-flight session — that is impossible and a weaker/different goal (HikariCP/RDS-IAM consensus), so it is explicitly out of R16.
- R17. Release-hook errors (`reset` / `close_session` / `release_token` / `destroy`) are observed (tracing + metric), not `let _ =`-swallowed; a failed `reset` must not silently hand the lock to the next caller as success.
- R18. Delete dead surface: identity-agnostic `acquire_*`/`try_acquire_*` tier, `recovery/{group,watchdog}`, `Manager::recovery_groups`; demote/uniffy `Cell` vs `SlotCell`.
- R19. **Narrowed (origin said "No performance work; the collapse is shape-only" — this supersedes it only for the inseparable item, with rationale):** the single generic `run_acquire` *structurally* removes the 5× per-topology duplication cost and the redundant double registry walk — that is a consequence of the collapse, not added perf work, and stays in scope. The *separable* micro-allocation folds (config re-clone per acquire/retry, `resilience.clone()`, `OnceLock` erased no-op accessors, broadcast-gated-on-subscribers) are **moved to Deferred-to-Follow-Up** — they do not ride the collapse and re-introducing them here is the exact scope creep the origin boundary forbids. The `InFlightCounter` ordering invariant is untouched regardless.
- R20. **REFUTED by the tokio contract (external research, settled — not a bug).** `RecoveryWaiter::wait` creates the `Notified` future *before* the state load, and every gate notifier uses `notify_waiters()` (gate.rs:119/132/142/161/343/355), never `notify_one()`. tokio's documented contract: `notified()` captures the `notify_waiters` call-count at creation; a `notify_waiters()` firing between creation and `.await` is delivered on first poll **without** `enable()` or a prior poll (`enable()` only affects `notify_one()` registration). So no wakeup is lost — the existing ordering is correct. This differs from `shutdown.rs:65` (which needs `enable()` because it re-checks an external `AtomicU64` the `Notify` does not gate, and/or uses `notify_one`). R20 is **doc-only**: add a comment recording why the ordering is correct and why it differs from `shutdown.rs`. **No `enable()` change** — adding it would be an unjustified modification of correct concurrency code.

**Origin acceptance examples:** AE1 (Cloned→owned), AE2 (Tracked→release hook), AE3 (Exclusive next-acquire waits for reset), AE4 (acquire_erased↔typed parity), AE5 (per-commit green — refined: per-touched-crate at commit, workspace at push), AE6 (pinned-lookup no-Ambiguous, identity-agnostic stays fail-closed).

---

## Scope Boundaries

- No behavior redesign of taint/drain/epoch semantics beyond R15–R17, R20.
- `Pooled`/`Resident` runtime internals not refactored, EXCEPT the R16 fix: it introduces a per-row revoke epoch consulted on **every** instance-return-to-idle path in `pool.rs` (`release_entry` recycle, in-flight `create`/`CreateGuard`, `warmup_sequential`, `warmup_staggered`, `run_maintenance`/`should_evict` re-deposit). This new revoke-epoch counter is the one sanctioned epoch addition under "no behavior redesign beyond R15–R17"; the blast radius is `pool.rs` (PoolEntry/build_guarded_handle/release_entry/warmup/CreateGuard/run_maintenance), not "revoke/recycle only".
- No ADR; no cross-layer relocation of types in this plan.
- `slot.rs` ordering model unchanged — RCU explicitly rejected (see Key Decisions / Open Questions).

**Scope-trace note (origin vs this plan).** The origin requirements doc scopes the topology collapse + barrier-fix + Gate-eval. R18 (dead-code deletion), U7 (`GuardInner` Option), U8 (recovery group/watchdog delete + R20), U9 (`Cell`/`SlotCell`) and the *structural* part of R19 are **not in the origin doc** — they are in scope under the user's explicit verbal directive to "raise the code level several-fold and untangle the web" given after the origin was written. This note is the audit trail so a reviewer comparing origin↔plan does not read these as silent origin-divergence. R15/R16/R17/R20 are audit-surfaced fixes that ride the collapse seam.

### Deferred to Follow-Up Work

Tracked, NOT silently inherited. **Each MUST be filed as a tracked issue (per the project tracker) in U13, with the severity below — the module-doc ledger links the issues, it is not the sole record** (prior nebula-resource removal was silently left half-done; a doc-only ledger decays to silent inheritance once the plan is deleted):

- **Latent bugs out of the collapse seam** (separate PRs, severity + defer-rationale):
  - `reload_config` on Service/Transport/Exclusive returns `PendingDrain` but never drains/rebuilds the live runtime — **MED-HIGH** (silent config-not-applied; comparable to R17). Deferred *only* because the redesign of the reload path is a separate concern; U4 rewrites the topology match arm so U1 MUST pin the current per-topology reload-outcome contract first (see U1) to keep this a true no-op-preservation, not a silent change.
  - pool `CreateGuard` **cancel-drop** path drops runtime sync without async `destroy()` (server-side resource leak on a *cancelled* acquire) — **MED** (resource leak only, not isolation; out of seam). NOTE: the *other* `CreateGuard` aspect — an in-flight create that **completes after a revoke** admitting a revoked-credential instance — is the SAME isolation defect as R16 (HikariCP #1836) and is **pulled into R16's active scope**, not deferred. Only the cancelled-acquire leak remains here.
  - resident recreate take()+destroy-under-lock vs dispatch no-op window — **MED** (lost-revoke window; resident internals, out of seam).
  - `graceful_shutdown` Phase 4 detached workers can outlive `release_queue_timeout` — **LOW** (bounded, eventually drains; shutdown.rs not opened here).
  - `RecoveryTicket` Drop counts a panicked probe as an attempt — **LOW** (defensible default, untested; recovery internals).
  - *No deferred item is ≥ the severity of the lowest in-scope fix except `reload_config`, which is pinned-not-fixed by explicit choice with a U1 net so the deferral is auditable as a risk decision, not a file-adjacency artifact.*
- **Separable R19 perf micro-folds** (origin "no perf work"): config re-clone hoist, `resilience.clone()` drop, `OnceLock` erased no-op accessors, broadcast-gated-on-subscribers. The single-`run_acquire` structural dedup + single registry resolution stay in scope (inseparable from the collapse).
- **Cross-crate dedup/placement**: `ErrorKind` ≈ `nebula_error::ErrorCategory` + hardcoded backoff vs `nebula_resilience::BackoffConfig`; relocate live `RecoveryGate` + `ReleaseQueue` to `nebula-resilience`; `events.rs` raw `broadcast` → `nebula_eventbus::EventBus`; `CreateGuard`/`SessionGuard` → one `DefuseGuard<T>`; `register_from_value` `{{ }}` expression coupling / two-options-module navigation.
- `ResourceMetadata`/`BaseMetadata` composition is correct reuse — do not touch.

---

## Context & Research

### Relevant Code and Patterns
- `crates/resource/src/manager/mod.rs` (3118) — God-object; `run_*_acquire` ~1993–2618; `InFlightCounter` 3069–3118; `acquire_erased` 1794; `register*` 371–860, `register_from_value` 1016; `lookup_any_for_slot_identity` 1881 (impossible-`Ambiguous` arm 1892–1902).
- `crates/resource/src/dedup.rs:49` `slot_identity` (DefaultHasher→u64); `registry.rs` `DedupKey` keying, `find_at_exact_scope:626`, `get_for:540`.
- `crates/resource/src/runtime/{service,transport,exclusive}.rs` + `topology/{service,transport,exclusive}.rs` — fold targets; `runtime/mod.rs:28` `TopologyRuntime<R>`; `runtime/managed.rs:197` `dispatch_slot_hook`.
- `crates/resource/src/manager/acquire_dispatch.rs` — already monomorphise-then-box (proves dyn-compat is a non-issue; keep enum+AFIT).
- Production callers (untouched acquire path): engine `resource_accessor.rs:103,119` (`acquire_erased`), `:291` (`register_with_identity`), `registrar.rs:362` (`register_from_value`), `resource_fanout.rs` (`refresh_slot_for`/`taint_slot_for`/`drain_and_revoke`/`acquire_resident_for`). No engine/api/example uses Service/Transport/Exclusive typed APIs (10 folded-trait impls all in-crate tests).

### Institutional Learnings
- `docs/solutions/` absent; durable record = canonical crate module doc (R9/R14) + git + phase carry-forward (coordinator transcripts lost routinely).
- Expand-contract is mandatory under per-commit green; storage spec-16 redesign precedent (ADR-0072, ~26 commits) surfaced 3 latent bugs — budget for that, it is the value.
- Windows worktree: `cargo fmt --all`/`task dev:check` fail os error 206 — verify per-crate; lefthook pre-push ≠ CI Documentation job, run `RUSTDOCFLAGS=-D warnings cargo doc -p <crate> --no-deps` per touched crate.
- ADR-0083 intent-gate: expand-contract dual-surface (duplicate-symbol) + new files trip net-LoC/new-file/blob budgets mid-migration → `// budget-justified:` per over-budget commit; clippy `too_many_lines` is inert workspace-wide so `run_acquire` length is not clippy-gated.
- Type-enforce over discipline; no shims; a prior nebula-resource removal was silently left half-done — completion honesty is the highest-salience risk here.

### Multi-agent audit (consolidated)
- **Kieran/Rust**: enum+AFIT sound; dyn/async-trait would regress (banned `Box<dyn Future>` on hot path). Real constraint: `Resident` needs `Lease: Clone` super-bound ⇒ cannot fold into `Bounded`; `Bounded` bound = Service∪Transport∪Exclusive (= current Exclusive bound). `Bounded` cap/token-mode must be **type-enforced** (sealed + `Cap` typestate assoc), not a runtime `==` (today's `TOKEN_MODE ==` lets a Tracked service silently no-op release). RCU for `slot.rs` **rejected** (speculative `bump_generation` retry → epoch gaps; current `Relaxed`+gen-in-SlotEntry already torn-read-free).
- **Correctness**: F1 Pool revoke TOCTOU (HIGH, fix). F4 `RecoveryWaiter` missing `enable()` (MED, fix). F2/F3/F7/F6/F8 → Deferred (out of seam, tracked).
- **Performance**: config `load_full`, `resilience.clone()`, erased ctx 2-Arc rebuild, broadcast-on-0-subscribers, double registry walk — all on the rewritten lines, fold in. `InFlightCounter` AcqRel ordering = TOCTOU invariant, preserve verbatim (any ordering tuning = separate reviewed commit).
- **Testing**: impl-coupled tests (topology_tag==X, explicit `TopologyRuntime` ctors, 7-arg register, 6-arg registry test sig, dx narrative, hand-mirrored `credential_slot_epoch`) break for wrong reason → port behavior-first. Must-add scenarios enumerated per unit. Strong nets to preserve verbatim: `resident_rotation_race`, `credential_slot_epoch_fold`, `manager_refresh_slot`, `manager_acquire_for`, `dedup_slot_identity`, `classify_error`.
- **Maintainability**: identity-agnostic `acquire_*`/`try_*` + `recovery/{group,watchdog}` + `Manager::recovery_groups` = dead; `Cell` ⊂ `SlotCell`; ~17 register shorthands = write-side hydra.

---

## Key Technical Decisions

- **Seam = enum-dispatch, native AFIT/RPITIT, no dyn/async-trait.** `TopologyRuntime<R>` is already an enum; the erased boundary already monomorphises-then-boxes. dyn would force a banned hot-path allocation. (origin Outstanding Q-b resolved.)
- **`Resident` stays a separate variant/trait** — its `Lease: Clone` super-bound and create-vs-rotate epoch reconcile are incompatible with the `Bounded` fold. Hard constraint.
- **`Bounded` cap + release-shape is type-enforced** (sealed trait + `Cap` typestate marker as an associated type: `Unbounded`/`Capped<N>`/`Exclusive`) so "Tracked without release", "Exclusive without reset ordering" are compile errors, not runtime `==` branches. (origin Q-a resolved, type-enforce house rule.) The sealed-`Cap`-typestate is an **established Rust idiom** (sealed-typestate / statum / Comprehensive-Rust typestate pattern), not a speculative one-consumer abstraction — this rebuts the adversarial residual; the pattern + the house "make invalid states unrepresentable" rule jointly justify it over a runtime-discriminated enum.
- **Cross-tenant identity is the structural set, NOT a digest.** `DedupKey`'s slot component is the ordered resolved `(SlotKey, CredentialKey)` set with derived `Eq`/`Hash` — the digest option is rejected at plan time (it keeps a collidable space and, if `u64`-typed, leaves the weak primitive callable). This makes collision impossible by construction (exact string equality), eliminating the class rather than shrinking it. **Ecosystem evidence (peer-project, supports the structural-key choice):** an incomplete connection-pool key is a known CVE-class cross-tenant-leak mistake — Knex #6220 (RLS leak between tenants via pool key), .NET `SqlConnectionPoolKey` (full credential set in the key by design), AWS SaaS tenant-isolation guidance. The structural set is the industry-correct pool-key shape; a lossy hash as the tenant boundary is the documented anti-pattern. Subsumes R10's 2-variant pinned-lookup (no `Ambiguous` on the pinned path; identity-agnostic keeps fail-closed `Ambiguous`; `get_any` fail-closed unchanged — refuted-not-a-leak per PR #688). The old `pub fn slot_identity` is demoted to `pub(crate)` at U2 (deleted at U12). **Correction (security review):** demotion only blocks *new cross-crate calls* — it does NOT close the U2→U12 window, because the engine already stores/reuses computed `slot_identity: u64` values (`BindEntry.slot_identity`, the fan-out reverse-index) until U11/U12 migrate those types. The weak-identity exposure during U2→U12 is **accepted on the latent rationale** (frontier, no production credential→slot resolver — ADR-0067 §M12.4), NOT "closed by demotion"; the Phase 0.5 gate minimizes the window by landing U2 ahead of the fold. **Cross-crate wire-type (resolve at U2, blocks U11):** the replacement is a concrete structural type (e.g. `Arc<[(SlotKey, CredentialKey)]>` canonical-sorted) with derived `Eq`/`Hash`; the engine (`registrar.rs` recompute, `ResourceFanoutIndex` key, `record_resource_slot_identity`) MUST construct/derive it byte-identically to the resource-side key — U11 migrates `BindEntry.slot_identity`/the reverse-index from `u64` to this type, verified by `cargo check -p nebula-engine` at U12, not an `rg`.
- **Security live-severity & landing.** R15 (cross-tenant bleed) and R16 (revoke→recycle) are **latent, not live**, today: `nebula-resource` is `frontier` with no production credential→slot resolver (`register_and_bind` has zero callers, ADR-0067 §M12.4), so the resolved-binding path that feeds `slot_identity` has no production caller. This justifies seam-coupled remediation over a standalone hotfix. **Fan-out reverse-index is also latent (S2 — traced, not assumed):** the engine `ResourceFanoutIndex` reverse-index keyed on `slot_identity` (`registrar.rs` recompute → `ResourceFanoutIndex::bind`, `resource_fanout.rs` routing) *is* live integration code, but it is **populated only through `register_and_bind`** — the same zero-production-caller seam — so in production the reverse-index is empty and a `DefaultHasher` collision has no row pair to misroute between during the U2→U12 window. The fan-out *dispatch* path is exercised only by in-crate tests with literal `slot_identity` values, not by a production credential→slot resolver. The latent-not-live justification therefore covers the fan-out routing, not just registry-row identity; this is asserted by the zero-caller fact, not assumed. **Recommendation (surfaced, not auto-decided):** still carve U1+U2 (+ the U5 R16 `pool.rs` gap) so they are independently revertable from the topology/dead-code phases — a CRITICAL-class fix entangled in a 13-unit squash has no clean partial-revert if a later unit regresses. See Open Questions.
- **Drain primitive: keep the hand-rolled dual `DrainTracker`/`InFlightCounter` — both `nebula_resilience::Gate` AND `tokio_util::task::TaskTracker` rejected.** (Round-2 review reverted an over-reaching round-1.5 library-research lean to TaskTracker — recorded so it is not re-raised.) `Gate`: no per-resource isolation. `TaskTracker`: **structurally incompatible**, not merely an ordering question — (a) `TaskTracker::wait()` completes only when the tracker is **closed AND empty** and `close()` is *terminal*, but nebula's per-resource drain is **repeated and non-terminal** (`revoke_slot` drains the same counter every revoke event; the resource keeps serving acquires afterward — taint stops the old credential's leases, not the resource), so a per-`ManagedResource` `TaskTracker` that must survive N revoke cycles can never use `wait()`; (b) one `TaskTrackerToken` decrements ONE tracker, but `InFlightCounter` pre-increments **two** (manager-wide `graceful_shutdown` + per-resource revoke) and the guard's `Drop` decrements both — a single token cannot serve the dual-tracker architecture without holding two tokens and reshaping `release_to_guard`; (c) `tokio_util = "0.7.18"` is declared with **no features**; `TaskTracker` is `#[cfg(feature = "rt")]` while the in-use `CancellationToken` is in the ungated `sync` module — "CancellationToken in use ⇒ TaskTracker available" was a non-sequitur. The hand-rolled dual-tracker is reusable-by-construction (bare-counter wait, no terminal close), proven, and `AcqRel`-correct for the taint→increment→recheck TOCTOU. Keep it verbatim; no swap, no proof gate. **Pool crates** remain rejected as a `PoolRuntime` base (sound finding): deadpool/bb8/mobc lack credential-epoch fencing (deadpool#402 "dynamic/short-lived credentials" is an *open upstream issue* = nebula's exact R15/R16 need) — hand-rolled Pool credential-epoch is justified, not reinvention.
- **`InFlightCounter` AcqRel ordering preserved verbatim**; the suggested Relaxed-increment tuning is explicitly out of this plan (separate reviewed commit with a re-stated memory-model proof).
- **RCU rejected for `slot.rs` (citation-backed)** — arc-swap docs confirm `rcu`'s closure is `FnMut`, retried (called multiple times) under contention; a side-effecting `bump_generation` in it produces epoch gaps that break the resident equality reconcile. Recorded considered-and-rejected so it is not re-raised; instead document the single-writer-per-slot assumption + add a concurrency test (Open Questions). arc-swap also confirms plain `store`/`swap` gives no inter-writer ordering — so the only correctness question is whether same-slot store is ever multi-writer, which is a codebase (engine-fan-out) fact, not an arc-swap one.
- **Expand-contract, no ADR, no shim.** Add-new → migrate in-tree → delete-old last as one atomic contract commit when `rg` proves the old surface self-referential.

---

## High-Level Technical Design

> *Illustrates the intended approach; directional guidance for review, not implementation specification. The implementing agent treats it as context, not code to reproduce.*

```
TopologyRuntime<R>           Author traits                Identity
  Pool(PoolRuntime)            Pooled (unchanged)           DedupKey {
  Resident(ResidentRuntime)    Resident (unchanged)           resource_key, scope,
  Bounded(BoundedRuntime)  ←   Bounded<Cap: CapMarker>        slot: Ordered<(SlotKey,CredentialKey)>  // was u64 hash
                                 Cap ∈ {Unbounded,            }
                                        Capped<N>, Exclusive}

acquire (one generic pipeline, was 5):
  resolve row (single registry walk, returns Arc<Managed> + matched_scope)
   → InFlightCounter::new (AcqRel, BEFORE post-taint recheck)   [PRESERVE verbatim]
   → reject_if_tainted_post_count
   → admit_through_gate
   → cfg = managed.config()  [hoisted ONCE, &cfg into closure — not per retry]
   → execute_with_resilience(&managed.resilience)               [no .clone()]
   → topology.acquire(cx)    [enum dispatch; Bounded folds Service/Transport/Exclusive]
   → settle gate / record (broadcast only if receiver_count>0)
   → with_drain_tracker

revoke (R16 fix): drain MUST account for in-flight whose recycle-to-idle
  is still queued on the ReleaseQueue — close the gap so a revoked
  credential cannot re-enter idle after the hook walked it.
```

---

## Implementation Units

### U1. Characterization + gap-coverage test baseline

**Goal:** Lock current behavior and add the missing nets BEFORE structural change, so the fold is diffed against green and the latent bugs are demonstrated.

**Requirements:** R11, R15, R16, R17, AE1–AE6 (R20 is doc-only, owned by U8 — not a U1 requirement; U1 only adds the correctness-pin test)

**Dependencies:** None

**Files:**
- Modify: `crates/resource/tests/dedup_slot_identity.rs`, `crates/resource/src/slot.rs` (`#[cfg(test)]`), `crates/resource/tests/manager_refresh_slot.rs`
- Create: `crates/resource/tests/bounded_fold_behavior.rs`, `crates/resource/tests/revoke_recycle_toctou.rs`, `crates/resource/tests/release_hook_errors.rs`

**Approach:** Add behavior nets that currently FAIL where a bug exists (RED to prove F1/R16, swallowed-release/R17), PASS where preserving. **Independent output-equivalence oracle:** because U11 re-authors the impl-coupled assertions onto `Bounded`, the preserve baseline must NOT move with the API — capture observable folded-topology OUTCOMES (ordered event log: acquire→handle-kind, drop→release-hook-fired?, second-acquire-unblock-after-reset ordering, slot-epoch sequence) as serialized golden fixtures here; U11 replays the same scenarios and asserts byte-equality against these goldens; U13 sign-off gates on an empty golden diff, not "suites green". Upgrade ~40 `sleep(50ms)` settle points in `basic_integration.rs` to polled/`Notify` waits.

**Execution note:** Characterization-first — these tests + goldens gate every later unit.

**Patterns to follow:** barrier/`Notify` style in `resident_rotation_race.rs`, poll helper in `transport_integration.rs:220`.

**Test scenarios:**
- Edge: two registrations with different resolved bindings forced to the same `slot_identity` u64 → assert no cross-tenant runtime is served (RED today → proves R15) + the `h==SLOT_IDENTITY_UNBOUND⇒1` nudge branch.
- Integration (R16, three variants — net defines "fixed"): (a) Pool acquire→drop (release queued)→`drain_and_revoke` before the ReleaseQueue worker runs → revoked credential never re-served via idle recycle; (b) **in-flight `create` started before revoke, completing after `drain_and_revoke`** (HikariCP #1836 race) → the post-drain-created instance is `destroy`ed via the revoke-epoch re-check, never admitted to idle; (c) **`run_maintenance` cycle after `drain_and_revoke`'s idle walk over a non-stale, non-timed-out pre-revoke idle entry** → `should_evict` destroys it, not `keep.push_back` (proves the fifth-path gap A1). All three RED today → prove R16.
- Error path: `reset`/`close_session`/`release_token` returns `Err` → assert observed (not silently success); `reset` Err must not hand lock to next caller as success (RED → proves R17).
- **R20 (pre-resolved — refuted by tokio contract, no falsification needed):** add a *correctness-documenting* test asserting `notify_waiters()` fired after `notified()` creation but before `.await` IS delivered (no `enable()`), pinning the existing `RecoveryWaiter::wait` ordering as correct. NOT a RED-then-fix; no `enable()` change.
- Edge (gates U9): no-op `take()` on an already-empty/never-bound resident slot, then a refresh dispatch → assert the runtime is NOT re-treated as stale (pins current `built_epoch`-only-advances-on-successful-reconcile semantics, distinct from `SlotCell.generation()` which bumps on no-op take). U9 may only fold `built_epoch` into `SlotCell.generation` if this net stays green after the fold.
- Edge (gates the reload deferral): assert current `reload_config` outcome per topology (former-Service ⇒ `PendingDrain{old_generation}`, Pool/Resident/others ⇒ `SwappedImmediately`/`NoChange`) BEFORE U4 rewrites that match arm, so the deferred reload no-op is preserved verbatim, not silently changed by the fold.
- Concurrency: N concurrent `SlotCell::store`/`take` → `load_versioned` never torn; observe generation behavior (informs the single-writer-per-slot Open Question, not a fix).
- Happy: AE1 Cloned→owned, AE2 Tracked→release fires, AE3 next-acquire waits for reset (on current separate runtimes — captured as goldens for U11 replay).

**Verification:** New RED tests fail for the documented reason; R20 falsification result recorded; goldens captured; preserve-nets green; sleep-based points converted.

---

### U2. Structural cross-tenant identity + registry pinned-lookup type

**Goal:** Replace the `DefaultHasher`→u64 barrier with a collision-free structural identity; make the pinned lookup return a no-`Ambiguous` type.

**Requirements:** R10, R15, AE6

**Dependencies:** U1

**Files:**
- Modify: `crates/resource/src/dedup.rs`, `crates/resource/src/registry.rs`, `crates/resource/src/manager/mod.rs` (`lookup_any_for_slot_identity`, the impossible-`Ambiguous` arm)
- Test: `crates/resource/tests/dedup_slot_identity.rs`, `crates/resource/src/registry.rs` `#[cfg(test)]`

**Approach:** `DedupKey` slot component = ordered `(SlotKey, CredentialKey)` structure with derived `Eq`/`Hash` (digest option rejected — see R15/Key Decisions; do NOT reintroduce "or digest" here). Pinned `get_for`/`get_typed_for`/`get_typed_for_acquire`/`get_typed_at_acquire_scope` return a 2-variant outcome (`Found`/`NotFound`); identity-agnostic `get`/`get_typed`/`get_acquire_for` keep 3-variant (real `Ambiguous` fail-closed). Delete the fabricated-error arm.

**Patterns to follow:** existing `DedupKey` derive; `LookupOutcome` shape.

**Test scenarios:**
- Edge: structurally distinct bindings never equal regardless of any hash; forced-collision scenario from U1 now PASSES (no bleed).
- Happy: pinned lookup resolves exactly one row; unknown pin → `NotFound` (never alias).
- Error path: identity-agnostic multi-tenant still `Ambiguous`/`Conflict` (AE6, fail-closed preserved).

**Verification:** U1 collision RED test now green; `lookup_any_for_slot_identity` has no `Ambiguous` branch (type-unrepresentable).

---

### U3. Unified `Bounded` author trait + `BoundedRuntime` (folds Service/Transport/Exclusive, fixes R17)

**Goal:** One type-enforced `Bounded` trait + runtime preserving all three folded capabilities, with release errors observed.

**Requirements:** R2, R4, R17, AE1, AE2, AE3

**Dependencies:** U1

**Files:**
- Create: `crates/resource/src/topology/bounded.rs`, `crates/resource/src/runtime/bounded.rs`
- Modify: `crates/resource/src/topology/mod.rs`, `crates/resource/src/runtime/mod.rs`, `crates/resource/src/lib.rs`
- Test: `crates/resource/tests/bounded_fold_behavior.rs`, `crates/resource/tests/release_hook_errors.rs`

**Approach:** Sealed trait + `Cap` typestate associated marker (`Unbounded`/`Capped<N>`/`Exclusive`) wiring the matching release shape so invalid combos are compile errors. `BoundedRuntime` bound = Service∪Transport∪Exclusive (= current `ExclusiveRuntime` bound). One consolidated release path that observes hook errors (tracing `warn` + release-error metric), and on `reset` `Err` does NOT signal success to the next caller. **Precise failed-reset semantics (S4 — pinned, not left ambiguous):** on `reset`/`close_session` `Err` the permit IS still returned (withholding it would deadlock the semaphore — the permit is stored in the handle outside the callback for exactly this reason) BUT the instance is `destroy`ed, never recycled or handed onward; "does NOT signal success" means the next acquirer gets a freshly built instance, not the failed-reset one. This closes the isolation hole (a half-reset instance is never reused) without trading it for a deadlock. Keep permit-held-until-reset ordering (#384).

**Technical design:** *(directional)* `trait Bounded: Resource { type Cap: CapMarker; fn acquire_one(..) -> impl Future<..>; fn release_one(..) -> impl Future<..>; fn keepalive(..) {default}; }` — `CapMarker` impls supply the semaphore arity + whether `release_one` is mandatory.

**Patterns to follow:** existing `topology/exclusive.rs` bound set; `guard.rs` permit-outside-catch_unwind ordering.

**Test scenarios:**
- Happy: AE1 Cloned→owned no callback; AE2 Tracked→`release_one` fires; AE3 next acquire blocks until reset completes + permit returned.
- Edge: `Capped<N>` semaphore bounds concurrency to N; keepalive fires with non-`None` interval (never exercised pre-plan).
- Error path: `release_one`/`reset`/`close` `Err` → logged + metric, not swallowed; failed `reset` → instance `destroy`ed (next acquirer gets a fresh build, never the half-reset one) AND the permit is still returned (no deadlock) — assert both, not just "not success".
- Compile-fail: Tracked-without-release / Exclusive-without-reset rejected by the type system (trybuild).

**Verification:** `bounded_fold_behavior.rs` + `release_hook_errors.rs` (U1) green; trybuild compile-fail cases assert type-enforcement.

---

### U4. `TopologyRuntime` 5→3 + dispatch + tag + macro lockstep

**Goal:** Collapse the enum to `Pool`/`Resident`/`Bounded`, 3-arm `dispatch_slot_hook`, reconciled `TopologyTag`, derive-macro/probe string set.

**Requirements:** R1, R3

**Dependencies:** U3

**Files:**
- Modify: `crates/resource/src/runtime/mod.rs`, `crates/resource/src/runtime/managed.rs`, `crates/resource/src/topology_tag.rs`, `crates/resource/macros/src/resource_attrs.rs`, `crates/resource/macros/src/resource.rs`
- Test: `crates/resource/tests/probes/derive_invalid_topology.rs`, `derive_missing_topology.rs`, `tests/derive_resource_compile_fail.rs`, `credential_slot_epoch_fold.rs`

**Approach:** 3-variant enum + `tag()`; `dispatch_slot_hook` folds the 3 single-runtime arms into the `Bounded` arm (keep Resident reconcile + Pool idle fan-out). Update accepted `topology=` strings + `TopologyTag` lockstep in the same commit (whole-workspace-green); decide whether `service/transport/exclusive` remain catalog aliases (cosmetic — macro emits only the informational const).

**Patterns to follow:** `runtime/managed.rs:197` dispatch; `resource_attrs.rs` topology parse.

**Test scenarios:**
- Happy: each variant routes to its runtime; `tag()` total.
- Edge: derive with new/aliased `topology=` accepted; removed string rejected (trybuild — warm cache, per `reference_trybuild_agent_timeout`).
- Integration: `dispatch_slot_hook` Resident reconcile + Pool idle fan-out unchanged.

**Verification:** macro probes + `credential_slot_epoch_fold.rs` green; no `TopologyRuntime::{Service,Transport,Exclusive}` remains. **Reload-deferral preservation gate (SG4 — explicit ownership, was implicit U1→U4):** the U1 "gates the reload deferral" net (per-topology `reload_config` outcome: former-Service ⇒ `PendingDrain{old_generation}`, Pool/Resident/others ⇒ `SwappedImmediately`/`NoChange`) MUST still be green after U4 rewrites the topology match arm — U4 is the unit that owns proving the deferred `reload_config` no-op was preserved verbatim, not silently changed by the fold. A red here means the deferral became a silent behavior change (R11 violation), not a passing collapse.

---

### U5. One generic `run_acquire` (collapse + inseparable structural dedup only)

**Goal:** Replace 5 `run_*_acquire` with one generic pipeline; take ONLY the structural win inseparable from the collapse (single registry resolution). Keep the hand-rolled dual drain verbatim. R16 is split out to U14; the separable R19 micro-folds are Deferred (origin "no perf work").

**Requirements:** R5, R8, R11

**Dependencies:** U4

**Files:**
- Modify: `crates/resource/src/manager/mod.rs` (run_*_acquire, acquire_erased, record_acquire_result; InFlightCounter site **preserved verbatim**), `crates/resource/src/manager/acquire_dispatch.rs`, `crates/resource/src/manager/execute.rs`
- Test: `crates/resource/tests/acquire_erased_dispatch.rs`, `manager_acquire_for.rs`

**Approach:** One `run_acquire` over `TopologyRuntime::acquire(cx)`; dead `_ => expected X` arms gone. The single inseparable structural perf consequence: thread the resolved `Arc<dyn AnyManagedResource>` out of the first registry walk so the typed lookup is a downcast (no second DashMap walk) — this is a consequence of one pipeline, not added perf work. **Explicitly NOT in U5 (Deferred per R19, origin no-perf-work boundary):** config re-clone hoist, `resilience.clone()` drop, `OnceLock` erased no-op accessors, broadcast-gated-on-subscribers. **Preserve `InFlightCounter::new` strictly before `reject_if_tainted_post_count` with `AcqRel` verbatim** — no primitive swap (TaskTracker/Gate rejected, see Key Decisions); the hand-rolled dual `DrainTracker`/`InFlightCounter` is kept exactly.

**Execution note:** Behavior-preserving (no R16 here — that is U14); re-state the TOCTOU/ordering invariant in the commit message for the InFlightCounter-adjacent restructure.

**Patterns to follow:** existing `run_pooled_acquire` skeleton; `execute_with_resilience` `&Option` signature.

**Test scenarios:**
- Integration: AE4 acquire_erased↔typed parity on every collapsed variant (same instance/create-count/scope).
- Concurrency: revoke-vs-acquire post-taint recheck still rejects the late acquire (preserved verbatim); `manager_acquire_for` nets green.
- Verification net: no `run_*_acquire` clones remain; golden-replay (U1) empty-diff for the acquire pipeline.

**Verification:** `manager_acquire_for`/`acquire_erased_dispatch` behavior-equal; no `run_*_acquire` clones; InFlightCounter ordering unchanged (diff shows no atomic-ordering edit).

---

### U14. R16 revoke-epoch fence on every pool return-to-idle path

**Goal:** Close the Pool revoke→recycle/create/warmup TOCTOU: a credential revoked via `drain_and_revoke` is unreachable through ANY post-drain idle instance. Split from U5 so the security fix is an independently-reviewable, independently-revertable commit (see Phased Delivery Phase 0.5).

**Requirements:** R16

**Dependencies:** U1 (revoke-epoch + `pool.rs` sites) + U4 always; U5 only under Phase 0.5b option (ii) (fold into single-pipeline). Under option (i) U14 lands against pre-collapse `run_*_acquire` and is re-pointed in U5 — see Phased Delivery Phase 0.5b.

**Files:**
- Modify: `crates/resource/src/runtime/pool.rs` (per-entry checkout/creation epoch snapshot on `PoolEntry`; epoch re-check at `release_entry` recycle, `build_guarded_handle` release closure, `warmup_sequential`, `warmup_staggered`, `run_maintenance`/`should_evict` re-deposit, and the `CreateGuard` admit path — destroy-not-admit when the row epoch advanced), `crates/resource/src/manager/mod.rs` (`drain_and_revoke` bumps the per-row revoke epoch synchronously, before the hook, alongside the existing taint)
- Test: `crates/resource/tests/revoke_recycle_toctou.rs`

**Approach:** **Plan-time decision point to resolve at impl (named, not designed here):** where the per-row revoke epoch *counter* lives (an `AtomicU64` on `PoolRuntime`/the row, distinct from the existing config `fingerprint`) and how it composes with the existing `tainted`/`fingerprint` stale checks. **Pinned at plan level (correctness constraint, not deferred):** the comparison epoch MUST be snapshotted onto the `PoolEntry` at creation/checkout time. The existing `fingerprint` precedent loads `current_fp` at *release* time (`build_guarded_handle`), which is structurally insufficient for the revoke epoch — a release-time load reads the post-revoke epoch on both pre- and post-revoke entries and cannot distinguish them, so a checkout-time snapshot on the entry is mandatory and is the field missing from today's `PoolEntry`. Mechanism (HikariCP #1836 prior art): every push-to-idle site (all five — incl. `run_maintenance`/`should_evict`) consults the row epoch and `destroy`s (not recycles/admits) when the row epoch advanced past the entry's snapshotted checkout/creation epoch, BEFORE `on_credential_revoke` dispatches. Downgrade forbidden (F1 cross-tenant defect). `ReleaseQueue`-quiesce is an optional complement, not the sole mechanism (evict-only misses the in-flight-create race per #1836). Scope note: prevents a revoked-credential instance being admitted to idle or handed onward (cross-tenant reuse); does NOT retroactively kill a session a single caller is already mid-use of (impossible; out of scope) — the in-scope line is admission/hand-onward, not handshake completion.

**Execution note:** Independently revertable from the topology fold (per the adopted security-landing decision).

**Test scenarios:**
- Integration (R16, four variants — defines "fixed"): (a) acquire→drop (release queued)→`drain_and_revoke` before the worker runs → never re-served via recycle; (b) in-flight `create` completing after the drain → destroyed via epoch re-check, never admitted; (c) `warmup_sequential`/`warmup_staggered` running concurrently with/after `drain_and_revoke` → warmed instances epoch-checked, post-revoke ones destroyed not admitted; (d) a `run_maintenance` cycle running after `drain_and_revoke`'s idle walk over a **non-stale, non-timed-out** pre-revoke idle entry → `should_evict` destroys (not `keep.push_back`s) it via the revoke-epoch arm.
- Concurrency: epoch bump in `drain_and_revoke` is synchronous-before-hook (same discipline as the existing taint).

**Verification:** all five `pool.rs` push-to-idle paths consult the epoch; U1 `revoke_recycle_toctou` four-variant RED net green; no push-to-idle site (incl. `run_maintenance` re-deposit) admits a stale-epoch instance.

---

### U6. `RegistrationSpec` param struct + register chain/shorthand collapse

**Goal:** One param aggregate; collapse the 3-deep register chain + ~17 shorthands; remove the 4 `#[allow(too_many_arguments)]`.

**Requirements:** R6, R7

**Dependencies:** U4

**Files:**
- Modify: `crates/resource/src/manager/mod.rs` (register*, register_from_value, validate_config_value), `crates/resource/src/manager/options.rs`
- Test: `crates/resource/tests/register_from_value.rs`, `register_from_value_rejects_secret.rs`, `dx_audit.rs`, `dx_evaluation.rs`

**Approach:** `RegistrationSpec<R>` plain struct (public fields, `Option` for resilience/recovery_gate, no builder) consumed by one `register` + `register_from_value`; delete the 10 `register_<topo>[_with]` shorthands and the 3-deep delegation (keep one internal row builder). `register_from_value` keeps its phase order, just threads the spec.

**Patterns to follow:** existing `RegisterOptions`; `validate_config_value` already-factored core.

**Test scenarios:**
- Happy: register via `RegistrationSpec` round-trips; `register_from_value` resolves a template AND the rendered config is observable (upgrade from contains-only).
- Error path: closed-set secret rejection preserved; unknown slot binding rejected.
- Edge: no `#[allow(clippy::too_many_arguments)]` remains in `manager/`.

**Verification:** `register_from_value*` green with rendered-config assertion; zero register shorthands; clippy clean without the allows.

---

### U7. `ResourceGuard::inner: Option<GuardInner>` (kills detach sentinel + Deref panic)

**Goal:** Type-enforce post-detach unreachability; remove `mem::replace` dummy + `unreachable!()` + `panic!` in `Deref`.

**Requirements:** R11 (structural-soundness, no behavior change)

**Dependencies:** U3 (merged release path touches `GuardInner`)

**Files:** Modify `crates/resource/src/guard.rs`; Test: `crates/resource/src/guard.rs` `#[cfg(test)]`

**Approach:** `inner: Option<GuardInner<R>>`; `detach` = `self.inner.take()` (consumes self); `Drop` sees `None` naturally; `Deref` post-detach becomes a borrow error (unrepresentable), not a runtime panic. Add a one-line unwind-safety rationale on the `catch_unwind` (lease moved into callback, `self` retains no alias).

**Test scenarios:**
- Happy: owned/guarded/shared deref + drop unchanged; detach returns lease, skips callback.
- Edge: detach on a lease type with observable `Drop` does not double-invoke/leak.
- Error path: panicking release callback still returns the permit (currently untested — add).

**Verification:** no `panic!`/`unreachable!` in `guard.rs` non-test; guard tests green incl. new panic-callback/permit test.

---

### U8. Delete dead recovery infra + fix `RecoveryWaiter` lost-wakeup

**Goal:** Remove `recovery/{group,watchdog}` + `Manager::recovery_groups` (zero consumers); document `RecoveryWaiter::wait` as correct (R20 refuted via tokio contract — doc-only, no code change).

**Requirements:** R18, R20

**Dependencies:** None (independent; sequence anytime after U1)

**Files:**
- Delete: `crates/resource/src/recovery/group.rs`, `crates/resource/src/recovery/watchdog.rs`
- Modify: `crates/resource/src/recovery/mod.rs`, `crates/resource/src/recovery/gate.rs` (`RecoveryWaiter::wait`), `crates/resource/src/manager/mod.rs` (field+accessor), `crates/resource/src/lib.rs` (re-exports)
- Test: `crates/resource/src/recovery/gate.rs` `#[cfg(test)]`

**Approach:** Delete the dead modules + the `recovery_groups` field/accessor + trimmed re-exports (live `RecoveryGate` stays). `RecoveryWaiter::wait` is **correct as-is** (tokio contract, settled): `notified()` is created before the state load and the notifier uses `notify_waiters()`, so a notify between creation and `.await` is captured-by-count and delivered without `enable()`. **No code change** — add only a doc comment stating the precise reason it is correct: every gate notifier uses `notify_waiters()` (not `notify_one()`), and `notified()` captures the `notify_waiters` count at creation — so a notify between creation and `.await` is delivered without `enable()`. (The `shutdown.rs:65` `enable()` is needed there because that path uses the `notify_one`/permit semantics where pre-poll registration matters — the distinguishing axis is notify_waiters-vs-notify_one, NOT "external AtomicU64", which was imprecise.)

**Test scenarios:**
- Correctness-pin (not RED): `notify_waiters()` fired after `notified()` creation but before `.await` is delivered with no `enable()` — pins the existing ordering as correct so a future refactor cannot silently regress it.
- Happy: `RecoveryGate` admit/settle path unchanged.

**Verification:** dead types absent from `lib.rs`; `RecoveryWaiter` unchanged + documented + correctness-pin test green; gate behavior preserved.

---

### U9. `Cell`/`SlotCell` de-duplication

**Goal:** Remove the two-ArcSwap-cell duplication; demote `pub use cell::Cell`.

**Requirements:** R18

**Dependencies:** U1 (resident epoch tests gate this)

**Files:** Modify `crates/resource/src/cell.rs`, `crates/resource/src/runtime/resident.rs`, `crates/resource/src/lib.rs`

**Approach:** Default = demote `pub use cell::Cell` to `pub(crate)` only (zero external consumers; the unambiguous correctness cleanup — removes a misleading public API). **Do NOT fold `built_epoch` into `SlotCell.generation` by default:** they are semantically distinct counters — `built_epoch` advances only on a *successful* stale reconcile (resident.rs), `SlotCell.generation()` bumps on *every* transition including no-op `take()`; folding them would make a correctly-bound runtime spuriously test stale and force redundant revoke-hook re-delivery (a credential-isolation behavior change R11 forbids). The fold is permitted ONLY if the U1 no-op-`take()` divergence net (gates U9) proves the two counters observationally equivalent after the change; absent that proof, keep `Cell` private and leave `built_epoch` as the external counter, documenting the intentional divergence.

**Test scenarios:**
- Integration: `resident_rotation_race.rs` + `credential_slot_epoch_fold.rs` green verbatim (the gate for the unify).
- Happy: resident acquire/clone/reload unchanged.

**Verification:** `Cell` no longer crate-public; resident reconcile nets green.

---

### U10. Canonical two-phase-revoke invariant doc

**Goal:** One authoritative module doc; the other ~8 prose copies become references.

**Requirements:** R9, R14

**Dependencies:** U5 (the 5 `run_*` comment copies die with the bodies there)

**Files:** Modify `crates/resource/src/manager/mod.rs` (module doc), `crates/resource/src/runtime/managed.rs`, `crates/resource/src/guard.rs`, `crates/resource/src/manager/shutdown.rs`

**Approach:** Author the canonical "two-phase revoke / sync-taint / lazy-future-timeout / TOCTOU / per-resource-drain" rationale once at the `manager` module level; replace the 8 restatements with one-line `see` references. Also record here the topology-collapse rationale + the RCU-rejected decision (R14, no ADR).

**Test scenarios:** Test expectation: none — doc-only. `RUSTDOCFLAGS=-D warnings cargo doc -p nebula-resource --no-deps` must pass (intra-doc links).

**Verification:** rustdoc gate green; one canonical block, references elsewhere.

---

### U11. Migrate consumers (expand-contract phase 2)

**Goal:** Move every in-tree caller to the new API while old surface still compiles.

**Requirements:** R12, R13

**Dependencies:** U2–U10, U14

**Files:** Modify the ~10 in-crate folded-trait impls (`tests/basic_integration.rs`, `tests/transport_integration.rs`, `runtime/service.rs` doubles), `examples/examples/m6_*`, `tests/dx_audit.rs`, `tests/dx_evaluation.rs`, `tests/acquire_erased_dispatch.rs`, `tests/register_from_value.rs`, `crates/resource/src/registry.rs` test sig. **Engine PRODUCTION slot-identity consumers (the R15 structural-type change breaks these, not just tests):** `crates/engine/src/resource/registrar.rs` (recomputes `slot_identity` verbatim into `ResourceRegistrationOutcome`), `crates/engine/src/engine.rs` (`record_resource_slot_identity`), `crates/engine/src/credential/rotation/resource_fanout.rs` (`BindEntry.slot_identity`, `ResourceFanoutIndex::bind`, `b.slot_identity != slot_identity` match), `crates/engine/src/resource_accessor.rs` (`HashMap<ResourceKey, u64>` built from `slot_identity`), plus engine test literals (`0xDEAD_BEEF` etc.). Engine tests: `crates/engine/tests/**`.

**Approach:** Port impl-coupled tests behavior-first onto `Bounded` (keep semaphore/reset/session assertions, swap `topology_tag==Service/Transport/Exclusive`); migrate register call sites to `RegistrationSpec`; re-point registry test to the new pinned-lookup type; **re-verify the hand-mirrored `credential_slot_epoch` in `resident_rotation_race.rs`/`credential_slot_epoch_fold.rs` against post-collapse derive output** (or make the fixture `#[derive(Resource)]`-able). Stage resource crate + moved consumers together at each per-touched-crate-green commit.

**Test scenarios:** Integration: every migrated suite green on the new API; the preserve-nets unchanged in behavior; hand-mirrored epoch matches real derive.

**Verification:** whole workspace builds with BOTH old+new present; no consumer references deleted-surface yet.

---

### U12. Contract: delete old surface (expand-contract phase 3, atomic)

**Goal:** Remove the old taxonomy in one atomic commit once `rg` proves it self-referential.

**Requirements:** R5, R6, R8, R12, R18

**Dependencies:** U11

**Files:** Delete `crates/resource/src/topology/{service,transport,exclusive}.rs`, `crates/resource/src/runtime/{service,transport,exclusive}.rs`; Modify `crates/resource/src/manager/mod.rs` (delete 5 `run_*_acquire`, identity-agnostic `acquire_*`/`try_acquire_*`, 3-name passthrough, `acquire_dispatch.rs` per-topology fns), `crates/resource/src/dedup.rs` (remove `slot_identity` u64 path if fully replaced).

**Approach:** Gate = `cargo check -p` for **every** consumer crate (resource, engine, examples, api) — NOT an `rg` text grep alone: `rg` matches surface syntax and misses monomorphised / trait-dispatched / macro-expanded uses of a deleted symbol, so an `rg`-green atomic delete can still break the workspace build, discovered only at push with no incremental fallback. `rg` is a fast pre-filter; `cargo check -p <consumer>` is the authoritative gate before the atomic contract commit. No `#[deprecated]` window.

**Test scenarios:** Test expectation: none new — the full suite (incl. all U1/U11 nets) must stay green post-deletion. AE5: per-touched-crate green at this commit, workspace green at push.

**Verification:** `rg` finds no live reference to any deleted symbol; full nextest + per-crate clippy/fmt/doctest/rustdoc green; engine/api/examples compile.

---

### U13. Final verification + deferred-bug ledger

**Goal:** Prove the whole-workspace contract and record the explicitly-deferred latent bugs.

**Requirements:** R11, R12, R14, Success Criteria

**Dependencies:** U12

**Files:** Modify `crates/resource/README.md` (drop dead-API examples), the canonical module doc (U10) — append the deferred-bug ledger; `docs/brainstorms/...requirements.md` unchanged.

**Approach:** Per-crate `cargo fmt -p`/`clippy -p --all-targets -D warnings`/`nextest run -p`/`test -p --doc`/`RUSTDOCFLAGS=-D warnings cargo doc -p` for `nebula-resource` + every touched consumer crate (do NOT report `task dev:check` from this Windows worktree — os error 206). Record the Deferred-to-Follow-Up latent bugs (F2/F3/F7/F6/F8 + cross-crate items) in the canonical doc so they are tracked, not silently inherited.

**Test scenarios:** Test expectation: none — verification only. All AE1–AE6 + U1 added nets green; zero `run_*_acquire`/dead-tier/`recovery_groups`.

**Verification:** whole workspace green at push; deferred ledger present; README teaches only the surviving API. **Quantitative Success-Criteria gate (was unenforced — SG2):** record actual post-collapse numbers and assert against the Success Criteria, not "materially reduced": (a) `manager/mod.rs` line count — target ≤ ~1000 (origin goal ~800; a result > ~1500 is a FAIL, not a pass, and must be explained or the collapse is incomplete); (b) public `register_*`/`acquire_*` method count on `Manager` — assert a small constant (≤ ~8), down from ~35; (c) `rg` count of `run_*_acquire` = 0, identity-agnostic `acquire_*`/`try_acquire_*` tier = 0, `recovery_groups` = 0. The exact numbers go in the U13 phase report + the canonical module-doc ledger so the reduction is auditable, not asserted by adjective.

---

## Phased Delivery

- **Phase 0 — characterization:** U1.
- **Phase 0.5 — security fix, separable & independently revertable (GATE, not advisory).** Split into two sub-phases because the two security fixes have *different* separability profiles — conflating them is the trap M1 flagged:
  - **Phase 0.5a — R15 cross-tenant barrier (cleanly independent):** U1 + U2. U2's only dependency is U1 (it does not touch the topology fold), so U1+U2 land as their own commit/PR, green and **truly independently revertable**, before any Phase 1 topology unit (U3+) begins. This is the clean separable security commit.
  - **Phase 0.5b — R16 revoke-epoch fence:** U14. **U14 is NOT "before U3+":** U14 → U5 → U4 → U3 transitively, so a U14 that rides the single-pipeline context drags U3/U4/U5 with it (it cannot precede the topology units it depends on — the prior "before any Phase 1 topology unit (U3+)" claim was incoherent for R16 and is corrected here). Two landing options, **implementer's call before `ce-work`, recorded as the one Phase 0.5 sequencing latitude**: **(i)** land U14 against the **pre-collapse `run_*_acquire`** bodies (depends only on U1/U4 for the `drain_and_revoke` epoch + `pool.rs` sites, NOT on U5), keeping R16 a small independently-revertable commit, then re-point its call sites in U5 (the re-point MUST be covered by the U1 four-variant `revoke_recycle_toctou` net so a missed `pool.rs` site is caught); or **(ii)** fold U14 into the U5 single-pipeline commit, accepting that R16 is then not independently revertable from the topology collapse. Option (i) preserves the independent-revertability goal for R16 and is the recommended default; (ii) is the lower-effort path if the collapse is not expected to be reverted. (Adopted security-landing option (a) overall — separable security ahead of the fold; user may override to fully-bundled (b) before `ce-work`.)
- **Phase 1 — expand (add-new, old still present):** U3, U4, U5, U6, U7, U8, U9, U10 (dependency-ordered; U8 independent). U5 here always; U14 lands here (option ii) or in Phase 0.5b against pre-collapse bodies (option i).
- **Phase 2 — migrate:** U11.
- **Phase 3 — contract (atomic delete-old-last):** U12.
- **Phase 4 — verify:** U13.

Commit at per-touched-crate-green boundaries (lefthook pre-commit is per-staged-crate; workspace proof at pre-push/CI). Carry-forward each phase boundary in the commit/plan (coordinator transcripts are lost routinely). `// budget-justified:` on any commit the ADR-0083 intent-gate flags for the expand-contract dual-surface window.

---

## System-Wide Impact

- **Interaction graph:** acquire/register/revoke/rotation seam; engine `resource_accessor`/`registrar`/`resource_fanout` (acquire path untouched — erased; register path via `RegistrationSpec`).
- **Error propagation:** release-hook errors gain a tracing+metric seam (R17); `Ambiguous` stays caller-`Conflict` on the identity-agnostic path only.
- **State lifecycle risks:** R16 revoke-recycle; `InFlightCounter` ordering (preserve verbatim); resident create-vs-rotate epoch (preserve); Bounded permit-held-until-reset (preserve).
- **API surface parity:** typed `acquire_*`/`register_*` shorthands removed (no production caller); `acquire_erased`/`register_from_value`/`register_with_identity` (→spec)/fan-out preserved in shape. **`slot_identity: u64` is a cross-crate published type, NOT internal:** the R15 structural-set change alters `RegisterOptions.slot_identity`, `acquire_erased`/`has_registered_for_scope` signatures, and the engine's recompute/persist sites + rotation-fan-out reverse-index key type — all migrate in U11 and land in the U12 atomic contract (U12's gate is `cargo check -p` every consumer crate, not an `rg` text grep, since monomorphised/trait-dispatched uses do not match a symbol-name grep).
- **Unchanged invariants:** scope-precedence lookup, taint-before-drain, slot epoch change-token equality, `get_any` fail-closed (refuted-not-a-leak per PR #688 — do not "fix").

---

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Behavior regression in the fold (R11) | U1 characterization-first; preserve-nets verbatim; per-phase green |
| R15 identity change weakens isolation | Structural `Eq` (no hash), AE6 + forced-collision net, fail-closed agnostic path preserved |
| R16 revoke-recycle fix changes drain timing | Fix-not-downgrade pinned in U14 (all five `pool.rs` return-to-idle sites, incl. `run_maintenance`); U1 four-variant RED net; not silent |
| `InFlightCounter` ordering perturbed by the rewrite | Preserve verbatim; ordering tuning explicitly out-of-plan; TOCTOU proof re-stated in commit |
| Deferred latent bugs silently inherited | Explicit ledger (U13) + Scope Boundaries; not claimed fixed |
| ADR-0083 intent-gate blocks expand-contract commits | `// budget-justified:` per over-budget dual-surface commit; net trends negative |
| Windows worktree false-green | Per-crate verification only; never report `dev:check`; rustdoc per touched crate |
| RCU re-raised for slot.rs | Recorded considered-and-rejected with rationale (Open Questions) |

---

## Open Questions

### Resolved During Planning
- Seam mechanism: enum-dispatch + native AFIT (dyn/async-trait rejected — would regress, banned hot-path alloc).
- `Bounded` shape: sealed trait + `Cap` typestate marker (type-enforced, not runtime `==`).
- `Resident` cannot fold (Lease:Clone super-bound) — stays separate.
- R15 identity: structural ordered `(SlotKey,CredentialKey)` **set** with derived `Eq`/`Hash` — digest option **rejected**; old `slot_identity` demoted `pub(crate)` at U2.
- R16: **fixed, not downgraded** — mechanism specified in U14 (per-row revoke epoch snapshotted at checkout, re-checked at all five `pool.rs` return-to-idle sites incl. `run_maintenance`, before the hook); "fixed" defined by the U1 four-variant RED net.
- Drain primitive: **keep hand-rolled dual `DrainTracker`/`InFlightCounter`**. `Gate` rejected (no per-instance isolation); `TaskTracker` rejected (round-2: `wait()` needs terminal `close()` vs nebula's repeated non-terminal drain; one token ≠ dual manager+per-resource trackers; `rt` feature not enabled). No swap, no proof gate. Pool crates rejected as `PoolRuntime` base (deadpool#402 credential-epoch is an open upstream gap — hand-rolled justified).
- U9: default **demote-only**; `built_epoch`↔`SlotCell.generation` fold permitted only if the U1 no-op-`take()` divergence net proves equivalence.
- R20: **refuted via external research** (tokio `Notify` contract — `notified()` captures the `notify_waiters` count at creation, delivered without `enable()`; gate uses `notify_waiters()` not `notify_one()`). `RecoveryWaiter::wait` is correct as-is → doc-only, no `enable()`.
- `slot.rs` RCU: **rejected, now citation-backed** (arc-swap docs: `rcu`'s closure is `FnMut` and retried/called-multiple-times under contention → a side-effecting `bump_generation` inside it yields epoch gaps, breaking the resident equality reconcile; current `Relaxed`+gen-in-`SlotEntry` is already torn-read-free).

### Resolved This Session (was a surfaced fork; user chose auto-resolve)
- **Security-fix landing strategy — RESOLVED (option a adopted; user-confirmed "auto-resolve, best judgment").** R15/R16 are *latent* today (frontier, no production credential→slot resolver — ADR-0067 §M12.4), so seam-coupled remediation is economically defensible — but a CRITICAL-class fix entangled in a 13-unit squash with a U12 atomic delete has no clean partial-revert. **Decision:** the security units (U1 + U2 + U14) land as a **separable, independently-revertable security commit/PR ahead of the topology fold** — enforced by the Phase 0.5 gate in Phased Delivery (not advisory). The user may still override to the bundled option (b) by saying so before `ce-work`; absent that, (a) is the plan of record.

### Deferred to Implementation
- [Affects U1][Needs codebase research — NOT external; narrowed] Is concurrent same-slot `SlotCell::store` reachable? Closed by research: (a) arc-swap plain `store`/`swap` has no inter-writer ordering (confirmed) — so non-monotonicity is real *only* under concurrent same-slot store; (b) the engine rotation fan-out (`resource_fanout.rs`) dispatches per-resource-**concurrently** for ONE rotation event via `join_all` — distinct resources = distinct `SlotCell`s, **no same-slot contention at the fan-out layer**. Residual (the only remaining unknown): are two rotation *events* for the **same credential** serialized upstream (rotation driver/ledger)? If yes → single-writer-per-slot holds, document the invariant (expected). If no → real non-monotonicity. Guarded by the U1 concurrency test; resolve the upstream-serialization trace at U1, not here (different crate's driver internals).
- [Affects U4][Technical] Keep `service/transport/exclusive` as accepted `topology=` catalog aliases or collapse to one string (cosmetic — macro emits only the informational const).

---

## Sources & References

- **Origin document:** [docs/brainstorms/2026-05-18-002-nebula-resource-topology-collapse-requirements.md](docs/brainstorms/2026-05-18-002-nebula-resource-topology-collapse-requirements.md)
- Related code: `crates/resource/src/{manager/mod.rs,dedup.rs,registry.rs,guard.rs,slot.rs,runtime/*,topology/*}`, `crates/engine/src/{resource_accessor.rs,resource/registrar.rs,credential/rotation/resource_fanout.rs}`
- Precedent: nebula-storage spec-16 redesign (ADR-0072) — expand-contract under per-commit green, surfaced 3 latent bugs
- Related issues: #684 (slot_identity barrier — closed by rationalization, structurally addressed here via R15)
