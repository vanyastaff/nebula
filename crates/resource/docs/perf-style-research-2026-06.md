<!-- intent-gate marker (rendered-invisible); this file is a generated research deliverable, not source:
# budget-justified: generated deep-research report — table-heavy prose artifact with citation tables, not source code
-->

# nebula-resource — Prioritized Engineering Report (Rust perf + style, 1.96 upgrade)

> **Verification update (2026-06-03, post-implementation).** A follow-up
> adversarial verification pass (4 web-grounded lenses) corrected two P1 items:
> - **`#[inline]` (perf finding #1 / action #3) — REFUTED, not applied.** The
>   target accessors are methods on *generic* impls (`impl<S> SlotCell<S>`,
>   `impl<T> Cell<T>`); their MIR is already serialized + monomorphized in the
>   consumer crate, so they are inline-eligible cross-crate **with or without**
>   `#[inline]`. The "forces cross-crate availability" rationale is the
>   *non-generic* rule (rustc 1.75 / PR #116505) misapplied. Adding the attribute
>   is diff-noise with no inlining win — **deliberately not done.** (Cite:
>   rustc-dev-guide monomorphization CGU table; matklad "generic functions are
>   implicitly inlinable".)
> - **Ordering (action #2) — RESOLVED by downgrade, not a comment.** Verified
>   sound; the `generation()` None-branch `Acquire` load was *redundant* (it pairs
>   with no `Release` — the writer bumps `Relaxed` under the lock — and guards no
>   payload). Changed `Acquire`→`Relaxed` + added a terse coherence comment in
>   `src/slot.rs`. A reader that observes the cleared slot already synchronized
>   with the `take`'s `inner.swap(None)` (arc-swap acquire/release), sequenced
>   after the generation bump. **Do not add a `Release`.** Clippy clean,
>   336/336 crate tests pass.
> - **`#[must_use]` (action #4) — already present** on `ResourceGuard`
>   (`guard.rs:83`); drop path already emits `ResourceEvent::Released`. No change.


> Deep-research output (2026-06-03). 5 web-research angles, adversarially
> verified, grounded against the live crate source. Every non-obvious claim
> keeps its inline citation; the "Unverified / killed claims" section lists
> what the verify pass could **not** confirm — do not act on those as fact.

## TL;DR

- **The crate's hard-design choices are sound — do not "modernize" them.** SlotCell (ArcSwapOption readers + `Mutex<()>` writer-serialization + `AtomicU64` generation), the sealed `Cap` typestate, the `ReleaseQueue` background-cleanup worker, and the poison-tolerant `Mutex` are each verified as the correct, idiomatic 2026 choice. The verify pass refuted every "rewrite it" alternative (seqlock, `rcu`, `parking_lot`, `AtomicCell`, AsyncDrop, nonpoison::Mutex).
- **1.96 buys this crate almost nothing for perf** — zero new atomics/sync/async/codegen. Real reasons to bump: two cargo CVE fixes and `assert_matches!` for tests. Every perf lever below already works on the pinned 1.95.
- **Highest-leverage perf change: `#[inline]` on ~5 lock-free accessors** (`SlotCell::{load,load_versioned,generation,is_some}`, `Cell::load`). BUT urgency is **downgraded** — the repo's `[profile.release]` already sets `lto = "thin"` (`Cargo.toml:356`), so cross-crate inlining is *eligible* already; the marginal win is small and must be benchmarked, not assumed.
- **One concurrency item needs eyes-on-source, not a rewrite:** confirm the writer-side `next_generation` mutation is `Release` (or `AcqRel`) so the no-entry-fallback `Acquire` load has something to synchronize-with. Likely already belt-and-suspenders via the ArcSwap store; verify in `src/slot.rs`.
- **The god-file split is sanctioned but is local policy, not an upstream rule.** Split `manager/mod.rs` (2991 LoC, ~80 methods, one `impl`) into concern-named sibling `impl Manager` blocks (`acquire.rs`/`shutdown.rs`/`registry_ops.rs`/`recovery.rs`). Authority = RFC-735 multiple-impl idiom + the repo's own `.doctorrc.yml`, NOT a published LoC limit.
- **Net-new from peer pools (deadpool/bb8/sqlx):** add FIFO admission fairness (semaphore-permit-in-guard) for Bounded, idle-eviction + max-lifetime reaper for Pooled, and jitter+bounded-replenishment around RecoveryGate. These are real capability gaps, not idiom fixes — gate on whether Bounded acquires actually contend.
- **Add `#[must_use]` + an explicit `async release(self) -> Result` on `ResourceGuard`** (Drop stays the best-effort fallback). The *panicking* `DropBomb` is forbidden in lib code by the no-panic rule; use it only in tests.
- **Clippy: adopt `#![warn(clippy::pedantic)]` (warn, not deny).** Do NOT rely on `redundant_clone` to triage the 148 clones — it is **nursery** (allow-by-default, known false positives). The reliable clone-triage is the warn-by-default **perf** lints (`unnecessary_to_owned`, `large_enum_variant`).

---

## Rust 1.96 upgrade — what actually changes for this crate

**Stable in 1.96 (vs 1.95), and what it means here:**

| Item | Relevance to nebula-resource |
|------|------------------------------|
| `assert_matches!` / `debug_assert_matches!` | **Real test win.** Replace `assert!(matches!(err, ResourceError::CapExceeded{..}))` → `assert_matches!(...)` (prints the actual Debug value on failure). NOT in prelude — `use std::assert_matches::assert_matches;`. Tests are panic-exempt. [https://blog.rust-lang.org/2026/05/28/Rust-1.96.0/] |
| `From<T> for LazyCell<T,F>` / `LazyLock<T,F>` | Minor ergonomic win only if the crate uses lazy statics. |
| `From<T> for AssertUnwindSafe<T>` | Not relevant. |
| `core::range` Copy range types + iterators | Not relevant. |
| `ManuallyDrop` constants as patterns | Drop-adjacent; relevant only if the spawn-from-drop fallback ever matches on a `ManuallyDrop` constant (unlikely). [https://releases.rs/docs/1.96.0/] |
| **CVE-2026-5223** (Medium, symlink tarball) + **CVE-2026-5222** (Low, normalized-URL auth) | **The actual reason to bump** — both cargo, both fixed in 1.96. [https://blog.rust-lang.org/2026/05/28/Rust-1.96.0/] |

**Upgrade gates — run BEFORE changing `rust-toolchain.toml`:**

1. **RPITIT-overly-private is now a HARD error** (PR #152543). Run `cargo +1.96 check -p nebula-resource`. If any trait method returning `-> impl Trait` resolves to a less-public-than-the-trait concrete type (plausible across the ~36 `dyn`/trait surfaces in `registry.rs`), 1.96 fails where 1.95 allowed it. Confirmed as a hard error; whether *this crate* trips it is UNVERIFIED — must compile-check. [https://github.com/rust-lang/rust/pull/152543]
2. **New default-group clippy lints under `-D warnings`.** 1.96 added 3 (all complexity): `manual_noop_waker`, `manual_option_zip`, `manual_pop_if`. 1.95 added `manual_checked_ops`, `manual_take` (complexity), `disallowed_fields` (style, fires only if configured). Only `manual_noop_waker` is plausibly relevant (if the ReleaseQueue hand-rolls a no-op `Waker`). `unnecessary_trailing_comma` + `duration_suboptimal_units` are **pedantic** → won't fire under default `-D warnings`. Run `task clippy` under 1.96 first. [https://raw.githubusercontent.com/rust-lang/rust-clippy/refs/heads/master/CHANGELOG.md]

**Still NIGHTLY in 1.96 — do NOT recommend these as "upgrades":**

- **AsyncDrop** (`#![feature(async_drop)]`, tracking #126482) — validates the `ReleaseQueue` design; no stabilization PR. [https://github.com/rust-lang/rust/issues/126482]
- **`nonpoison::Mutex`** (`sync_nonpoison` #134645) — the poison-tolerant `Mutex<()>` + `PoisonError::into_inner` is the std-blessed stable pattern; keep it. [https://doc.rust-lang.org/nightly/std/sync/nonpoison/index.html]
- **AFIT/RPITIT dynamic dispatch** — still static-only on stable; the 22 `Box::pin` `ErasedAcquireFn` erasures are correct-by-necessity (they live behind `dyn`).

**Refuted upgrade fear:** the `Pin<Foo>` unsize-coercion restriction (PR #149218) does **NOT** affect the 22 `Box::pin` sites — `Box<T>: Deref`, so all erased futures are explicitly unaffected. Not an upgrade blocker. [https://github.com/rust-lang/rust/pull/149218]

**Net:** 1.95→1.96 is **low-risk, low-reward for runtime perf**. Bump for the CVEs + test ergonomics; the perf leverage is the existing thin-LTO + targeted `#[inline]`, not the toolchain.

---

## Performance findings

Ordered by leverage.

| Finding | Crate target | Recommendation | Confidence | Effort | Risk |
|---------|--------------|----------------|------------|--------|------|
| Lock-free reads not cross-crate-inlinable by attribute | `src/slot.rs`: `SlotCell::{load, load_versioned, generation, is_some}`; `Cell::load` (Resident) | Add plain `#[inline]` (NOT `inline(always)`) to these small non-generic accessors. Their MIR contains an `ArcSwap::load` call → NOT auto-inlined since 1.75. `#[inline]` forces availability in every referencing CGU (`codegen-units=16`), which thin-LTO summary-import may skip for cold callers. **Benchmark the engine acquire loop after** — perf-book says it can slow code down. | Med (mechanism High; magnitude Med — thin-LTO already covers most) | S | Low |
| Cache-hit acquire path may allocate/clone | `Manager::run_acquire` cache-hit branch; the 22 `Box::pin` `ErasedAcquireFn` sites | Reserve the boxed future strictly for the slow create/recycle path; ensure the already-resident branch returns the guard with no boxed future and no owned-`String`/`Vec`/`serde_json` clone. Arc clones (`drain_tracker`, `event_bus`) are cheap refcount bumps — fine. Deadpool's thesis: contended/allocating work behind the slow path. | Med (need to read the hit branch) | M | Med |
| Rare branches not marked cold | `Manager::run_acquire` (gate-admission fail, drain-reject, cap-exceeded); `SlotCell` no-entry fallback | Wrap rare branches in `core::hint::cold_path()` (const fn, stable since 1.95) to keep the common path straight-line. **Benchmark-gate** — std warns wrong cold-marking regresses the hot path. | High (availability); Med (benefit) | S | Med (must bench) |
| Hand-written lock-free CAS retry loops | `RecoveryGate` single-probe CAS; `InFlightCounter` (`AtomicUsize`) — IF they have real retry loops | Replace genuine `compare_exchange_weak` retry loops with `AtomicU64::update`/`try_update` (stable 1.95). Note: still takes **two** orderings (`set_order`, `fetch_order`) — collapses the loop, not the ordering surface. **Does NOT fit `next_generation`** (bumped Relaxed *under* the write_lock, not a lock-free loop). | High | S | Low |

---

## Code-style / structure findings

| Finding | Crate target | Recommendation | Confidence | Effort | Risk |
|---------|--------------|----------------|------------|--------|------|
| God-file: 2991 LoC, ~80 methods on one `impl Manager` | `src/manager/mod.rs` | Keep `mod.rs` thin (`struct` + field docs + `mod` decls); move method clusters into concern-named sibling files, each its own `impl Manager` block: `acquire.rs`, `shutdown.rs`, `registry_ops.rs`, `recovery.rs`. Group by concern, not one-method-per-file. Privacy invariants preserved (same crate). **Authority = RFC-735 idiom + repo `.doctorrc.yml`, NOT an upstream LoC limit.** | High (sanctioned); Med (necessary) | M | Low |
| Other large files | `runtime/pool.rs` (2104), `registry.rs` (1661), `guard.rs` (887), `runtime/resident.rs` (881), `recovery/gate.rs` (813) | Apply the same impl-split *only* where a single `impl` dominates; otherwise large-but-cohesive — no upstream source condemns size alone. | Med | M | Low |
| Verbose per-method doc rationale | `src/slot.rs` (multi-paragraph on nearly every method) | Concentrate counter-intuitive concurrency rationale (Acquire/Relaxed choice, poison-tolerance, monotone-generation, torn-read-freedom) at module level (`//!`) and/or on `struct SlotCell`. Reduce each `///` to an RFC-505 summary line. Move pure impl notes from `///` to `//`. **Relocate, don't delete** — the ordering argument is exactly the kind of decision that SHOULD be documented. | High (summary-line form); Med (over-documented verdict rests on a blog, not canon) | M | Low |
| `Cell<T>` is public doc-noise (two near-duplicate cells) | `src/slot.rs` `Cell<T>` | If `Cell<T>` is `pub` only for cross-module use (not user-facing), apply `#[doc(hidden)]` per C-HIDDEN — removes the "two cells" doc-noise without touching the intentional design duplication. (Note: in this crate `cell::Cell` is already `pub(crate)`, not re-exported — verify whether any doc-noise remains.) | High | S | Low |
| Clippy lint set | crate root `lib.rs` / `crates/resource/Cargo.toml [lints]` | `#![warn(clippy::pedantic)]` (**warn, NOT deny** — `-D warnings` makes deny self-defeating on 16k LoC; expect generous `#[allow]`). Rely on default **perf** group for clone triage. Cherry-pick `clippy::clone_on_ref_ptr` (restriction) only if the team accepts `Arc::clone(&x)` rewrite churn. Do NOT bulk-enable `nursery`. | High (group assignments read from clippy source) | S add / M burn-down | Low |
| `#[non_exhaustive]` on public errors | any `pub enum …Error` exposed toward engine/api | Confirm it carries `#[non_exhaustive]` OR is funneled through `nebula-error`'s canonical opaque type (which already gives add-variants-without-breaking). | High | S | Low |

**Do NOT:** annotate the ~80 `Manager` methods or any large fn with `#[inline]` (per-CGU duplication waste); use `#[inline(always)]` anywhere (std-dev-guide "just about never"; clippy `inline_always` flags it); treat "add `lto`" as an action (already `lto = "thin"` at `Cargo.toml:356`).

---

## Concurrency / SlotCell review

**Verdict: the SlotCell design is sound and is the idiomatic 2026 choice. Do not redesign it.** It is verified strictly superior to every alternative a reviewer might propose. Concretely:

- **ArcSwapOption for lock-free readers** — textbook read-biased shape (reads on every acquire, writes only on rotation/revoke). The maintainer's own bias: *"read performance is much more important than write performance."* [https://vorner.github.io/2019/04/06/tricks-in-arc-swap.html]
- **One immutable `Arc<SlotEntry>` publish = torn-read-free AND the only design compatible with `#![forbid(unsafe_code)]`.** A **seqlock would be wrong**: it needs `unsafe`, raw-pointer volatile, and `T: Copy` (SlotEntry is neither). [https://pitdicker.github.io/Writing-a-seqlock-in-Rust/]
- **The generation counter is the useful residue of the seqlock idea** — the even/odd retry dance is correctly omitted because the atomic Arc swap already provides the consistency. [https://mara.nl/atomics/inspiration.html]
- **Relaxed generation bump *under the write_lock* is correct, not lucky** — the Mutex carries the happens-before (unlock→lock edge); the new value reaches readers via the ArcSwap pointer swap (the Acquire/Release edge). The `AtomicU64` needn't carry ordering on the writer side. [https://mara.nl/atomics/memory-ordering.html]
- **`Mutex<()>` serializing rare writers is right — NOT `rcu`, NOT a CAS loop.** arc-swap `rcu` *"can call the closure multiple times to retry"* — actively dangerous for the side-effecting (event-emitting) rotation/revoke writers, which need exactly-once. [https://docs.rs/arc-swap/latest/arc_swap/struct.ArcSwapAny.html]
- **`std::sync::Mutex` is the correct 2026 choice over `parking_lot`** — since Rust 1.62 std Mutex is futex-based, ~5 bytes, const-constructible, no `Box`. parking_lot's only remaining edge is fairness *under contention*, and this lock is rarely-held + near-zero-contention by design. [https://blog.rust-lang.org/2022/06/30/Rust-1.62.0/]
- **Poison-tolerant `unwrap_or_else(PoisonError::into_inner)` is std-blessed** and satisfies the no-`unwrap`/`panic` rule (the lock guards only writer *ordering*, no torn data). Implementer nuance: recover the **`MutexGuard`** from `PoisonError<MutexGuard<()>>::into_inner()`, not a `PoisonError<()>`. [https://doc.rust-lang.org/std/sync/struct.PoisonError.html]
- **`crossbeam::AtomicCell<SlotEntry>` would silently fall back to a global lock** (non-Copy, not pointer-sized) — strictly worse. Pre-empt this suggestion. [https://docs.rs/crossbeam/latest/crossbeam/atomic/struct.AtomicCell.html]

**The two concrete refinements (both need eyes-on-source, neither is a rewrite):**

1. **Verify the no-entry-fallback ordering pairing.** The fallback reads `next_generation` with `Acquire` *without holding the write_lock*. For that to be meaningful, the writer's `next_generation` mutation must be `Release` (or `AcqRel`). If the fallback observes the generation *through* the swapped Arc, the ArcSwap store already provides the edge and the standalone Acquire is harmless belt-and-suspenders. If it reads the *bare* atomic when the Arc is `None`, confirm the writer uses `Release`. Add a one-line `// ordering:` comment pairing them. Effort S, risk Low (worst case: stricter than necessary, never a regression). [https://mara.nl/atomics/memory-ordering.html] *(Source note: `bump_generation` currently uses `Ordering::Relaxed` `fetch_add` and the fallback uses `Ordering::Acquire` `load` — this is the exact pairing to document/verify.)*
2. **Don't let an ArcSwap `Guard` cross `.await`.** arc-swap has only ~8 fast borrow slots/thread and *"these are not intended to be... used across async yield points."* `SlotCell::load` already returns an owned `Option<Arc<S>>` via `load_full()`, so it is safe — confirm no internal path swaps to a borrowing `load()` whose `Guard` is held across `.await`. Effort S, risk Low. [https://docs.rs/arc-swap/latest/arc_swap/docs/limitations/index.html]

**Do not invent problems here.** The two items above are *verifications*, not known defects.

---

## Ideas to do better (beyond fixes)

Net-new capability ideas from peer pools and 2026 idioms. Each is a real gap (verify contention/need first), not an idiom fix.

- **FIFO admission fairness for Bounded/Capped<N>.** Back Bounded capacity with a `tokio::sync::Semaphore` (permit == slot, held inside `ResourceGuard` for the lease) instead of a bare `InFlightCounter` atomic. A counter gives admission but **no ordering** — under contention, wakeups are arbitrary and a task can starve. sqlx is FCFS, deadpool gates with a semaphore, tokio Semaphore is FIFO-fair. `Exclusive` maps to `acquire_many(N)` (inherits tokio's documented head-of-line caveat). *Confidence: High that it's the idiom; Med that the crate needs it — in-process resources may rarely queue.* [https://docs.rs/sqlx/latest/sqlx/struct.Pool.html] [https://docs.rs/crate/deadpool/latest/source/README.md] [https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html]
- **Idle eviction + max-lifetime retirement for Pooled.** Add `idle_timeout` + `max_lifetime` knobs and a reaper that spawns **only if at least one TTL is configured** (bb8's exact guard — zero overhead when unused). Retirement-by-age bounds latent state growth (fd/handle/cache accumulation) even for in-process resources, same as sqlx retires DB sessions. bb8 defaults: 30 min lifetime / 10 min idle. *High confidence — table stakes for a reusable-resource pool.* [https://docs.rs/sqlx/latest/sqlx/pool/struct.PoolOptions.html] [https://docs.rs/bb8/latest/bb8/struct.Builder.html]
- **FIFO-vs-LIFO as an explicit Pooled policy knob.** Surface `QueueStrategy { Fifo, Lifo }` the way bb8 does — **always `pop_front`, choose the push side** (`push_back`=FIFO even-wear/keep-warm; `push_front`=LIFO hot-set reuse, lets the tail age out so `idle_timeout` shrinks the pool). A real elastic-shrink-vs-keep-warm decision, not cosmetic. [https://docs.rs/bb8/latest/bb8/enum.QueueStrategy.html]
- **Jitter + bounded replenishment around RecoveryGate.** RecoveryGate already does singleflight well. Add (1) jitter to retry/replenish cadence so simultaneous expiries don't re-line-up, and (2) a deadline-bounded, rate-capped replenisher (bb8's "approval token" + sqlx's deadlined best-effort min_connections) so a cold-start fleet doesn't stampede the backend. *Med that the crate lacks it — verify the warmup/replenish cadence specifically.* [https://raw.githubusercontent.com/djc/bb8/main/bb8/src/inner.rs]
- **Explicit `async release(self) -> Result` on `ResourceGuard`** (all three Owned/Guarded/Shared modes), with Drop as the queue-handoff fallback. Textbook C-DTOR-FAIL: callers who care observe teardown errors/ordering; Drop only fires for the "user forgot" case. **Verify it doesn't already exist.** [https://rust-lang.github.io/api-guidelines/dependability.html]
- **`#[must_use]` on `ResourceGuard` + the Cap topology guards** (std's own convention via `MutexGuard`/`RwLockWriteGuard` — there is no `C-MUST-USE` API-guideline code). Pair with a `tracing` event on the implicit-drop path. The literal **panicking `DropBomb` is forbidden in lib code** by the no-panic rule — use it only in tests (panic-exempt) to assert "every test path explicitly released." [https://doc.rust-lang.org/std/sync/struct.MutexGuard.html]
- **Graceful-drain "wake all waiters immediately" on shutdown.** Verify `Manager::shutdown` does sqlx's three-step `close()`: (1) flip closed flag + **immediately wake every queued waiter** (the step bespoke drain logic most often misses), (2) await leases draining, (3) complete the future. If admission moves to a semaphore (above), `Semaphore::close()` gives step (1) free. [https://docs.rs/sqlx/latest/sqlx/struct.Pool.html]
- **Consider tokio's first-party shutdown primitives for the ReleaseQueue drain** — `CancellationToken` (broadcast stop) + `TaskTracker` (`tracker.wait().await` to drain in-flight releases). Models exactly the bounded/non-blocking/cancellable shape the crate wants, avoiding `block_in_place`-at-shutdown hangs. Advisory, depends on current ReleaseQueue internals. [https://tokio.rs/tokio/topics/shutdown]

**What nebula already does BETTER than the peer pools (keep these):** compile-time release correctness via the sealed `Cap` typestate (no peer pool does this — all enforce capacity at runtime); the generation/epoch-versioned lock-free slot for *rotatable* credentials (DB pools key on liveness, not a versioned identity that changes mid-lease); `#![forbid(unsafe_code)]` + no-unwrap across the whole lifecycle (bb8/sqlx/tokio use unsafe + unwrap freely internally); singleflight RecoveryGate as a first-class primitive.

---

## Prioritized action list

1. **[P1]** `rust-toolchain.toml` — `cargo +1.96 check -p nebula-resource` + `task clippy` under 1.96 **before** bumping the pin — payoff: catches RPITIT-private hard error (#152543) and ~5 new default-group clippy lints up front.
2. **[P1]** `src/slot.rs` `SlotCell::store`/`take` + `generation` — confirm writer-side `next_generation` is `Release`/`AcqRel` and document the pairing with the fallback `Acquire` — payoff: proves the lock-free ordering sound by source, not by argument.
3. **[P1]** `src/slot.rs` — `#[inline]` on `SlotCell::{load,load_versioned,generation,is_some}` + `Cell::load`, then microbench the engine acquire loop — payoff: forces cross-crate inlining of hot reads beyond what thin-LTO guarantees (small, benchmark-gated).
4. **[P1]** `src/guard.rs` — add `#[must_use]` to `ResourceGuard` + Cap guards (+ tracing on implicit drop) — payoff: nudges callers to explicit release without violating the no-panic rule.
5. **[P2]** `src/slot.rs` — verify `SlotCell::load` returns owned `Arc` (it does, via `load_full()`) / audit internal paths for borrowing `load()` across `.await` — payoff: prevents arc-swap `Guard` 8-slot exhaustion.
6. **[P2]** `src/guard.rs` — add `async release(self) -> Result` (if absent); keep Drop as fallback — payoff: callers can observe teardown errors/ordering (C-DTOR-FAIL).
7. **[P2]** `src/manager/mod.rs` — split the one ~80-method `impl Manager` into concern-named sibling `impl` blocks (`acquire`/`shutdown`/`registry_ops`/`recovery`) — payoff: reviewability; satisfies `.doctorrc.yml` god-file gate.
8. **[P2]** `Manager::run_acquire` — audit the cache-hit branch for owned-data clones / boxed-future construction; keep it allocation-free — payoff: fast path stays a refcount bump, not an alloc.
9. **[P2]** crate root — `#![warn(clippy::pedantic)]` (warn, not deny); rely on perf-group lints for clone triage; do NOT enable `redundant_clone` (nursery, false positives) — payoff: backs the `missing_docs` discipline + guards inline policy.
10. **[P2]** `runtime/pool.rs` — add `idle_timeout` + `max_lifetime` + TTL-gated reaper for Pooled — payoff: bounds latent resource-state growth (table-stakes).
11. **[P3]** `Manager::run_acquire` rare branches — `core::hint::cold_path()`, benchmark-gated — payoff: straight-line common acquire path.
12. **[P3]** `InFlightCounter`/Bounded — back capacity with `tokio::sync::Semaphore` (permit-in-guard) — payoff: FIFO admission fairness, no starvation under contention (gate on whether Bounded actually contends).
13. **[P3]** `src/slot.rs` — relocate verbose per-method rationale to module-level `//!`; reduce each `///` to an RFC-505 summary line — payoff: cuts doc-noise, keeps rationale.
14. **[P3]** `recovery/gate.rs` + warmup — add jitter + bounded replenishment around RecoveryGate — payoff: completes the thundering-herd story (verify cadence first).
15. **[P3]** `RecoveryGate`/`InFlightCounter` — collapse any genuine lock-free CAS retry loop into `AtomicU64::update`/`try_update` — payoff: clearer retry code (does NOT fit `next_generation`).
16. **[P3]** tests — adopt `assert_matches!` for `ResourceError`/Cap/topology assertions (post-1.96) — payoff: better failure diagnostics.

---

## Sources

**Rust 1.96 / language / std:**
- https://blog.rust-lang.org/2026/05/28/Rust-1.96.0/
- https://blog.rust-lang.org/2026/04/16/Rust-1.95.0/
- https://releases.rs/docs/1.96.0/
- https://releases.rs/docs/1.95.0/
- https://raw.githubusercontent.com/rust-lang/rust/master/RELEASES.md
- https://github.com/rust-lang/rust/pull/152543 (RPITIT-private hard error)
- https://github.com/rust-lang/rust/pull/149218 (Pin<Foo> coercion — refuted as risk)
- https://github.com/rust-lang/rust/pull/116505 (1.75 auto cross-crate inlining)
- https://github.com/rust-lang/rust/issues/135894 (atomic `update`/`try_update`)
- https://github.com/rust-lang/rust/pull/148590 (`fetch_update` deprecation timeline)
- https://github.com/rust-lang/rust/issues/126482 (AsyncDrop tracking)
- https://blog.rust-lang.org/2022/06/30/Rust-1.62.0/ (futex std Mutex)
- https://doc.rust-lang.org/std/hint/fn.cold_path.html
- https://doc.rust-lang.org/std/future/trait.AsyncDrop.html
- https://doc.rust-lang.org/nightly/std/sync/nonpoison/index.html

**Lock-free / atomics / arc-swap:**
- https://mara.nl/atomics/memory-ordering.html
- https://mara.nl/atomics/inspiration.html
- https://docs.rs/arc-swap/latest/arc_swap/docs/performance/index.html
- https://docs.rs/arc-swap/latest/arc_swap/docs/limitations/index.html
- https://docs.rs/arc-swap/latest/arc_swap/struct.ArcSwapAny.html
- https://vorner.github.io/2019/04/06/tricks-in-arc-swap.html
- https://pitdicker.github.io/Writing-a-seqlock-in-Rust/
- https://docs.rs/crossbeam/latest/crossbeam/atomic/struct.AtomicCell.html
- https://doc.rust-lang.org/std/sync/struct.Mutex.html
- https://doc.rust-lang.org/std/sync/struct.PoisonError.html

**Connection-pool design (deadpool/bb8/sqlx/r2d2/tokio):**
- https://docs.rs/crate/deadpool/latest/source/README.md
- https://docs.rs/deadpool/latest/deadpool/managed/trait.Manager.html
- https://docs.rs/sqlx/latest/sqlx/struct.Pool.html
- https://docs.rs/sqlx/latest/sqlx/pool/struct.PoolOptions.html
- https://docs.rs/bb8/latest/bb8/struct.Builder.html
- https://docs.rs/bb8/latest/bb8/enum.QueueStrategy.html
- https://raw.githubusercontent.com/djc/bb8/main/bb8/src/inner.rs
- https://docs.rs/r2d2/latest/r2d2/trait.ManageConnection.html
- https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html
- https://docs.rs/tokio/latest/tokio/task/fn.block_in_place.html
- https://tokio.rs/tokio/topics/shutdown

**RAII / typestate / async cleanup / API guidelines:**
- https://without.boats/blog/asynchronous-clean-up/
- https://rust-lang.github.io/api-guidelines/dependability.html (C-DTOR-FAIL / C-DTOR-BLOCK)
- https://rust-lang.github.io/api-guidelines/future-proofing.html (C-SEALED)
- https://rust-lang.github.io/api-guidelines/checklist.html
- https://rust-lang.github.io/api-guidelines/documentation.html (C-HIDDEN / C-FAILURE)
- https://cliffle.com/blog/rust-typestate/
- https://docs.rs/drop_bomb/latest/drop_bomb/
- https://doc.rust-lang.org/std/sync/struct.MutexGuard.html
- https://doc.rust-lang.org/reference/attributes/diagnostics.html

**Inline / code-style / clippy / errors:**
- https://matklad.github.io/2021/07/09/inline-in-rust.html
- https://nnethercote.github.io/perf-book/inlining.html
- https://std-dev-guide.rust-lang.org/policy/inline.html
- https://matklad.github.io/2021/08/22/large-rust-workspaces.html
- https://rust-lang.github.io/rfcs/0505-api-comment-conventions.html
- https://raw.githubusercontent.com/rust-lang/rust-clippy/refs/heads/master/CHANGELOG.md
- https://doc.rust-lang.org/stable/clippy/lints.html
- https://github.com/rust-lang/rust-clippy/pull/10873 (redundant_clone → nursery)
- https://github.com/dtolnay/thiserror
- https://nrc.github.io/error-docs/error-design/error-type-design.html

---

## Unverified / killed claims

Do **not** act on these as if confirmed:

- **[MUST COMPILE-CHECK] Whether nebula-resource actually trips RPITIT-private (#152543).** The changelog line is confirmed; the crate-specific trigger depends on real trait signatures. Run `cargo +1.96 check -p nebula-resource` before acting.
- **[CODE-READ GAP] Which ordering sub-case the no-entry generation fallback is in.** The *rule* is confirmed; the *pairing* needs eyes on `src/slot.rs` (current code: `bump_generation` = `Relaxed fetch_add` under lock; fallback = `Acquire load`).
- **[CODE-READ GAP] Whether `Manager::run_acquire`'s cache-hit branch actually allocates/clones owned data**, whether `ResourceGuard` already has an explicit `release()`, and whether the sealed `Cap` `private::Sealed` supertrait + ZST-marker shape is wired and rustdoc-documented. All are "verify, then act."
- **[REFUTED — do not treat as urgent] Adding `lto = "thin"`.** Already set at `Cargo.toml:356`. The `#[inline]` campaign is therefore a small benchmark-gated optimization, not a clear pessimization fix.
- **[REFUTED] `redundant_clone` is a perf lint.** It is **nursery** (PR #10873, false-positive-driven, allow-by-default).
- **[REFUTED] The `Pin<Foo>` coercion change (#149218) blocks the bump.** `Box<T>: Deref` → all 22 `Box::pin` sites unaffected.
- **[REFUTED — mechanism] "bb8 does LIFO by back-popping."** bb8 always `pop_front`; the strategy is the **push** side.
- **[CORRECTED — atomic `update`]** Takes **two** orderings — collapses the retry loop, does NOT simplify the ordering surface. Does NOT fit `next_generation`.
- **[CORRECTED attribution] Doc-density rules** ("rarely discuss WHY unless counter-intuitive") are from the Tangram Vision blog (opinion), NOT RFC-505/api-guidelines. The "over-documented" verdict is well-reasoned opinion, not a rule violation.
- **[CORRECTED attribution] The opaque `#[error(transparent)]` pattern** is from the dtolnay thiserror README, not nrc's error-docs.
- **[PARTIAL] "deadpool core is ~100 lines."** Confirmed only as deadpool's README self-description. The architectural claim (semaphore admission + one return-mutex + recycle-on-get) IS independently confirmed.
- **[PARTIAL] "AFIT dynamic dispatch still unsupported in 2026"** rests on absence-of-stabilization + still-open #91611. Directionally certain.
- **[UNVERIFIED] `ArcSwapOption::load` is itself `#[inline]` upstream.** Not checked. If it is (likely), the cost of omitting `#[inline]` on the SlotCell wrappers is higher (lost fusion).
- **[SECONDARY only] Thundering-herd singleflight/jitter sources** are engineering blogs, not specs. Supporting commentary only.
- **[INTERNAL JUDGMENT, no external authority] The 148 `.clone()` / 12 `.to_owned()` / 21 `.to_string()` counts**, per-acquire `Arc::clone` (cheap), `SlotCell`/`Cell` duplication (intentional), `SlotIdentity::Structural` collision-free key. None promotable to a finding without a profiler.
