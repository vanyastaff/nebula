---
name: Tech Spec CP1 — Rust-Senior Review
date: 2026-04-24
reviewer: rust-senior (subagent dispatch)
scope: §0-§3 of `docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md`
spike_baseline: docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/spike/
verdict: RATIFY_WITH_EDITS
---

# Tech Spec CP1 — Rust-Senior Review

## Verdict — RATIFY_WITH_EDITS

CP1 §2-§3 is sound. The shape is the spike's shape, faithfully elaborated. The trait
contract compiles cleanly (mentally) against the live `nebula_credential::Credential`
trait at [`crates/credential/src/contract/credential.rs:100-127`](../../../../crates/credential/src/contract/credential.rs);
the dispatcher trampoline is the spike's trampoline with production naming and the
per-resource timeout override threaded through. Three architect-flagged decisions
(Q2 TypeId, Q3 Box::pin + 32, Q4 hybrid timeout) are defensible and I ratify all
three. The required edits below are bounded, mostly cross-reference and one
correctness nit on the dispatcher lifetime discipline; they do NOT trigger an iterate.

I am NOT signing off the §3.5 aggregate-vs-per-resource event question — that's a
security/observability call that needs explicit security-lead reaffirmation against
their B-2 amendment. CP1 records this open in §3 final paragraph; that's the right
disposition. Flagging here that my ratification does not extend to that
disposition's design correctness.

## §2.1 Resource trait full signature

🟢 The trait signature in §2.1 (Tech Spec lines 113-298) compiles against the live
`Credential` trait shape. Specifically verified:

- `type Credential: Credential` bound resolves against the live `Credential` trait
  ([`crates/credential/src/contract/credential.rs:100`](../../../../crates/credential/src/contract/credential.rs)
  — `pub trait Credential: Send + Sync + 'static`). The `Send + Sync + 'static`
  super-bounds on `Credential` cascade naturally to `Self::Credential`, satisfying
  the `Send` requirements on the rotation hook futures.

- `<Self::Credential as Credential>::Scheme` projection works because `Credential::Scheme: AuthScheme`
  is declared at [`credential.rs:111`](../../../../crates/credential/src/contract/credential.rs). The
  spec's `create(&self, ..., scheme: &<Self::Credential as Credential>::Scheme, ...)`
  signature matches the spike `resource.rs:73-77` and Strategy §4.1 verbatim.

- `impl Future<Output = ...> + Send` with the `async { Ok(()) }` default body is
  the same pattern the live `Credential` trait uses for `test`/`refresh`/`revoke`
  defaults (see [`credential.rs:228-249`](../../../../crates/credential/src/contract/credential.rs)).
  Mixing `impl Future + Send { async { ... } }` declaration with `async fn` impl-side
  is well-formed RPITIT — clippy 1.95's `manual_async_fn` is the only friction, and
  §2.1.1 (Tech Spec lines 300-325) documents the impl-side `async fn` shorthand
  correctly. The clippy lint applies to `fn ... -> impl Future ... { async { ... } }`
  on impls, not on trait declarations — the spec's framing is accurate.

🟢 Default body for `on_credential_refresh` / `on_credential_revoke` (lines 222-258)
is RPITIT-compatible. `let _ = new_scheme; async { Ok(()) }` is the pattern the live
`Credential::test` default uses; the `let _ = …` outside the `async` block is the
correct way to suppress unused-arg warnings without forcing the parameter to be
captured into the future. Spike `resource.rs:90-110` does exactly this and it
compiled clean. **No `Pin<Box<dyn Future>>` needed** — RPITIT default in trait +
override mismatch is not an issue; both the trait default and the impl-side
override return `impl Future + Send`, the override just uses `async fn` shorthand
that desugars to the same.

🟡 Edit suggested: §2.1 line 188 says `+ Send` on `create`. This is correct, but the
trait already has `Resource: Send + Sync + 'static` super-bound (line 155), and
`Self::Credential: Credential` carries `Send + Sync + 'static`. The `+ Send` on the
return future is therefore not redundant (RPITIT does NOT auto-add `Send` —
`trait_variant::make` would, but that's not in use). Keep `+ Send`. Just calling
out that this is load-bearing per architect's explicit Q on Send-bound mismatches:
the spec is correct; the discipline is right; if a future migration ever adopts
`trait_variant::make(Send)`, those `+ Send`s become removable.

🟢 `metadata()` default (lines 287-296) calls `ResourceMetadata::for_resource::<Self>(...)`
which I verified exists at [`crates/resource/src/resource.rs:118-132`](../../../../crates/resource/src/resource.rs)
— the live `ResourceMetadata::for_resource<R>` constructor takes `(key, name, description)`
and pulls schema from `R::Config`. The spec's three-arg call site matches.

## §2.2 NoCredential placement

🟢 `nebula-credential` is the right home. Three reasons reinforced beyond §2.5 Q1:

1. **Trait + impl together is the orthodox layering.** `NoPendingState` already
   lives in `nebula-credential`; symmetric placement of `NoCredential` doesn't
   introduce a new pattern.
2. **No "couples resource to credential more than necessary"** concern — `nebula-resource`
   already takes a hard dep on `nebula-credential` (the new `type Credential: Credential`
   bound forces it). Putting `NoCredential` in `nebula-credential` is downstream of an
   already-existing dependency edge.
3. **Re-export at `nebula_resource::NoCredential` for ergonomics** (Tech Spec line
   416-418) handles the consumer DX — `use nebula_resource::NoCredential;` works at
   call sites.

🟡 One observation, not an edit: the spec's `NoCredential::resolve` (Tech Spec
lines 401-410) returns `ResolveResult::Complete(NoScheme)`. This is structurally
honest but **operationally unreachable** — Manager skips dispatch for
`NoCredential`-typed resources per §3.1 (Tech Spec lines 625-655). The
unreachability is a property of the system, not the trait. A future reader might
look at `NoCredential::resolve` and wonder when it's called. Suggest a short
`# Operational note` in the docstring explicitly stating "this method is never
called by `nebula-resource::Manager` — `register_inner` short-circuits for
`type Credential = NoCredential`." Useful for the next reader; not blocking.

## §2.3 Default body invariants

🟢 The post-invocation invariant ("no further authenticated traffic on revoked
credential") is correctly framed as a **contractual obligation on overriding
implementations**, not a runtime-enforceable property (Tech Spec line 246-252).
Manager genuinely cannot verify this — the impl is what owns the connection /
pool / token lifecycle, and the trait has no mechanism to introspect "did you
emit traffic?".

🟢 Borrow invariant on `&Scheme` (Tech Spec line 430) is the right call. Each
clone is a zeroize obligation per `PRODUCT_CANON.md §12.5`; the dispatcher
holding `&Scheme` for the duration matches Strategy §4.3 hot-path constraint 7.
This composes correctly with the `&'a (dyn Any + Send + Sync)` lifetime in §3.2 —
all per-resource futures share the same `&scheme` reference for the lifetime of
the outer `on_credential_refreshed` call.

🟡 Edit: §2.3 mentions "Idempotency expectation" (lines 432) but defers
**Manager-side retry policy** to CP2 §6. That's fine, but the trait docstring on
`on_credential_refresh` (Tech Spec line 200-228) does not mention idempotency.
Consider adding a one-line `# Idempotency` doc section on the method itself —
"Manager MAY retry; impls SHOULD treat repeated invocations as no-op" — so impl
authors writing against the trait don't have to discover this from the Tech Spec.
The doc can elide retry mechanics until CP2.

## §2.4 Sub-trait composition

🟢 All five topology sub-traits compose cleanly. Verified against live shapes:

- **Pooled** ([`crates/resource/src/topology/pooled.rs`](../../../../crates/resource/src/topology/pooled.rs)):
  `Pooled: Resource` — the parent trait reshape propagates `type Credential` to
  every `Pooled` impl without touching `recycle` / `prepare` / `is_broken`. No
  collision.
- **Resident** ([`topology/resident.rs:21-38`](../../../../crates/resource/src/topology/resident.rs)):
  `Resident: Resource where Self::Lease: Clone` — `where` clause is orthogonal
  to `type Credential`. The spec's §2.4 listing (Tech Spec lines 474-477) keeps
  `Self::Lease: Clone` correctly.
- **Service** ([`topology/service.rs:30-55`](../../../../crates/resource/src/topology/service.rs)):
  `Service: Resource` with `const TOKEN_MODE: TokenMode`. Compose-clean.
- **Transport** ([`topology/transport.rs:18-50`](../../../../crates/resource/src/topology/transport.rs)):
  `Transport: Resource`. `open_session` returns `Self::Lease` — that propagates.
- **Exclusive** ([`topology/exclusive.rs`](../../../../crates/resource/src/topology/exclusive.rs)):
  `Exclusive: Resource` with `reset(&runtime)`. No friction.

🟢 Spike validated cross-topology dispatch (Tech Spec lines 521-522 referencing
[`spike/.../resource-shape-test/src/lib.rs`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)
test `parallel_dispatch_crosses_topology_variants`) — `MockPostgresPool: Pooled`
and `MockKafkaTransport: Transport` bound to the same credential both received
the rotation hook through the same `Vec<Arc<dyn ResourceDispatcher>>`. Type
erasure works across topology variants because the dispatcher trampoline only
needs `R: Resource`, not the topology sub-trait.

🟢 `TopologyRuntime<R>` enum shrink (7 → 5 variants, ADR-0037) does not affect
the trait shape — that's a runtime data carrier inside `ManagedResource`, not
part of the public trait contract. CP1 correctly defers the engine-side landing
to CP3 §13 (Tech Spec line 519).

## §3.2 Dispatcher signature

🟢 `dyn ResourceDispatcher: Send + Sync + 'static` is correct and matches the
spike (`spike/.../manager.rs:89`). `Send` is required because `join_all` runs
on a multi-threaded runtime and the futures must cross thread boundaries between
polls. `Sync` is required because the dispatchers are stored in `DashMap<…, Vec<Arc<dyn …>>>`
which is shared across tasks. `'static` is required because the `Arc` is held
across `.await` points in `join_all`.

🟢 `&(dyn Any + Send + Sync)` as the scheme parameter (Tech Spec line 705-706)
is the spike's hard-won iteration finding (NOTES.md lines 56-62). `&dyn Any`
alone fails the `Send` bound on the dispatched future; `+ Send + Sync` is
load-bearing. This is correctly carried forward.

🟡 **Required edit** — §3.2 (Tech Spec lines 700-712): the trampoline trait method
`dispatch_refresh<'a>(&'a self, scheme: &'a (dyn Any + Send + Sync)) ->
Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>` is correct, but
the calling code in `Manager::on_credential_refreshed` (Tech Spec lines 792-805)
does:

```rust
let futures = dispatchers.iter().map(|d| {
    let d = Arc::clone(d);
    ...
    async move {
        ... d.dispatch_refresh(scheme) ...
    }
});
```

The `async move` captures `d: Arc<dyn ResourceDispatcher>` by value, but
`d.dispatch_refresh(scheme)` borrows `d` for `'a`. The `'a` is bounded by both
`&'a self` (i.e. the moved `d`) AND `&'a scheme`. The outer fn holds `scheme:
&'b (dyn Any + Send + Sync)` for some `'b: 'a`. The borrow on `d` is rooted in
the `async move` block (`d` is moved into the future, then re-borrowed for each
poll). This works only because the inner `Pin<Box<dyn Future + 'a>>` is awaited
to completion before the `async move` block returns — which it is, via `.await`.

This is correct as written, but the lifetime gymnastics are non-obvious. **Add a
SAFETY-style comment** on the closure body explaining that the `Arc::clone(d)`
move + the `'a` reborrow on `dispatch_refresh` is the pattern that makes
`scheme` outlive each per-resource future. Spike `manager.rs:269-278` did this
implicitly; production code with this much erasure deserves the explicit prose
for the next reader. Suggest:

```rust
// Each future captures its own `Arc<dyn ResourceDispatcher>` (the
// `Arc::clone(d)` move below) and reborrows it for the dispatch.
// `scheme` is the same `&(dyn Any + Send + Sync)` shared across all
// per-resource futures — it outlives `join_all` because the outer fn
// holds it for the entire scope. NO clone of Scheme — Strategy §4.3
// hot-path invariant.
```

🟡 Edit: §3.2 (Tech Spec line 743) writes `Error::scheme_type_mismatch::<R>()`. This
constructor does not exist in the live error type — verified via grep. The error
constructor needs to be added to `crates/resource/src/error.rs` as part of the
implementation. CP1's job is to specify the contract, not to land the impl, but
the spec should call out "NEW error variant required: `Error::scheme_type_mismatch`"
explicitly so the implementer doesn't miss it. Same for `Error::missing_credential_id`
(Tech Spec line 641). Suggest adding a short subsection §3.6.x or a footnote
listing the NEW error constructors required.

🟢 Aggregation type — `Vec<(ResourceKey, RefreshOutcome)>` (Tech Spec line 775,
925). Order matches register insertion order is the right semantics; matches
spike `RotationOutcome::per_resource` (`spike/.../manager.rs:64-77`).
`RefreshOutcome::TimedOut { budget: Duration }` carrying the budget value is
nice — operators reading the event/log can immediately see whether the per-resource
override fired or the manager default. Better than spike's tag-only `TimedOut`.

🟡 Minor: §3.2 line 803 uses `tracing::info_span!(...)` then `span.in_scope(|| join_all(futures)).await`.
This is correct — `in_scope` enters the span synchronously around the `join_all`,
and the await happens inside the span. But for an async span discipline, `Instrument`
+ `.instrument(span)` is the more idiomatic pattern (each future gets the span
attached for cross-poll continuity). With `in_scope(...).await`, the span is
detached on the first .await suspend then re-attached on resume — works for the
outer `join_all` but per-resource child futures won't carry the parent span context
unless they do their own `instrument`. CP1 doesn't need to commit; CP2 §7
(observability) is the natural place. Just flagging the choice.

## §3.5 Event semantics

🟡 §3.5 (Tech Spec lines 933-937) commits to **aggregate** event shape only
(`ResourceEvent::CredentialRefreshed { credential_id, resources_affected, outcome }`)
with per-resource detail in tracing spans, NOT events. Then says
`HealthChanged { healthy: false }` per security amendment B-2 fires for every
resource where `RevokeOutcome != Ok`.

This is a genuine tension I want to flag explicitly to security-lead:

- Security amendment B-2 explicitly wanted per-resource `HealthChanged { healthy: false }`
  (Strategy §4.2 line 252). The spec honors this for revocation-failure cases
  but defers per-resource `CredentialRevoked` event to CP2.
- The shape "aggregate `CredentialRevoked` event + per-resource `HealthChanged`
  events" is structurally inconsistent: revocation produces N+1 events
  (1 aggregate + N per-resource health), refresh produces 1 event + N tracing
  spans only. This asymmetry is hard to reason about for downstream subscribers.

I do NOT think CP1 should iterate on this — it's a observability-design call
that depends on broadcast cardinality budget (the `events.rs` channel is
`broadcast::Sender` with capacity 256, [`manager.rs:275`](../../../../crates/resource/src/manager.rs)).
A loud rotation event (1000 resources × refresh) could exhaust that channel.

🔴 **Required edit** — §3.5 must explicitly delegate this asymmetry to security-lead
in CP2 §7, NOT just "CP2 §7 finalizes broadcast cardinality" (which is what line
935 currently says). Specifically:

> Open item flagged for CP2 §7 + security-lead: refresh emits 1 aggregate event
> + N tracing spans; revocation emits 1 aggregate event + N `HealthChanged`
> events when failed. The asymmetry needs explicit security-lead ratification
> against B-2 — either (a) refresh also emits per-resource events for
> consistency, or (b) revocation drops to spans-only for failed cases (rejected
> by B-2), or (c) the asymmetry is documented as load-bearing.

This is the only 🔴 in my review. The shape itself is OK; the **disposition** of
the open question is what needs sharpening.

## Architect-flagged Q2/Q3/Q4 ratifications

### Q2 TypeId vs sealed-trait

**RATIFY — `TypeId` is the right call.**

Specific reasons that strengthen §2.5 Q1 rationale:

- **Monomorphization is not a concern.** `TypeId::of::<R::Credential>()` is
  evaluated at the call site of `register_inner::<R>`, which is monomorphized
  per `R`. The `TypeId` value for `NoCredential` is computed at compile time
  per the `'static` requirement on the `Credential` trait
  ([`credential.rs:100`](../../../../crates/credential/src/contract/credential.rs)
  — `pub trait Credential: Send + Sync + 'static`). There is no edge case where
  a `NoCredential` sneaks past the check.
- **Hot-reload is not a concern in this codebase.** Nebula does not ship a
  hot-reload story (verified — no `dlopen`-style plugin loading at the resource
  layer; plugins are statically linked per workspace). Even if it did, `TypeId`
  works correctly across `dlopen` boundaries when the `dyn` library shares the
  ABI; the more relevant failure mode is divergent `nebula-credential` versions,
  which would surface at link time long before `TypeId` mismatched.
- **Sealed-trait alternative buys nothing measurable.** A sealed
  `IsCredentialBearing` (default `true`, override to `false` for `NoCredential`)
  would let the compiler optimize the branch via const-eval. But the branch
  fires once per `register` call. At 5 in-tree consumers × ~3 resources each ×
  one-shot startup, that's 15 `TypeId` comparisons in a process lifetime. The
  optimization budget is below measurement noise.
- **Sealed-trait pulls more glue.** As §2.5 Q2 notes, the marker trait would
  have to live in `nebula-credential` (because `NoCredential` lives there).
  That's another trait import, another doc page, another newcomer concept to
  load. `TypeId` is host-crate-agnostic and a one-line check.

Compile-time guarantees from sealed-trait are also overstated: the actual
compile-time guarantee that matters (`type Credential` must impl `Credential`)
is enforced by the trait bound at impl time and is captured by spike's
compile-fail probe `_credential_bound_enforced_must_fail`
([`spike/.../compile_fail.rs:117-150`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)).
Sealed-trait does not strengthen that.

### Q3 32-concurrency default

**RATIFY — 32 is right, with one edit on observability framing.**

The math (5 consumers × 3 resources = 15; doubled = 32) is conservative but
reasonable for an MVP cap. Three points reinforcing:

- **Production load patterns at this stage are unknown.** Nebula has zero
  external adopters per Strategy §2.5; the in-tree consumer pattern (at most
  ~3 resources sharing one credential, e.g., a Postgres pool + one Redis +
  one Kafka all sharing a vault-issued token) suggests N=15 is a realistic
  steady-state; doubling to 32 leaves margin for one or two new consumer
  resources without re-tuning.
- **Soft cap, not hard cap.** The spec correctly notes (§3.4 line 893) that
  N ≤ 32 runs unbounded `join_all`; the FuturesUnordered fan-out at N > 32
  is deferred. So 32 is "concurrency target" not "hard ceiling that crashes
  at 33". Operators will see the histogram p99 latency creep before they see
  a soft outage.
- **The histogram `nebula_resource.credential_rotation_dispatch_latency_seconds`
  is sufficient observability** for the Box::pin overhead. At N=32, that's 32
  Box allocations per rotation event. Box allocation latency is sub-microsecond
  on a healthy allocator; even at N=1024 it's negligible against actual hook
  execution (network round trip, lock acquire). I confirm no separate
  Box::pin metric is needed.

🟡 Edit: §3.4 line 897 says "operators monitoring p99 latency at high N will
see the box-allocation overhead." This understates how subdominant the box
overhead actually is. Suggest rephrasing:

> Box::pin allocation per dispatch is sub-microsecond on a typical allocator;
> at N=32 the aggregate cost is ~tens of microseconds, statistically dominated
> by hook execution (DB connection rebuild, pool swap, etc.). The histogram
> captures total dispatch latency including the box; operators tuning for
> rotation SLOs read aggregate p99, not per-allocation cost.

This is observability framing, not a contract change.

### Q4 timeout config hybrid

**RATIFY — `ManagerConfig::credential_rotation_timeout` (default 30s) +
`RegisterOptions::credential_rotation_timeout: Option<Duration>` (per-resource
override) is the right shape.**

- **Matches existing `RegisterOptions` discipline.** Live `RegisterOptions`
  ([`crates/resource/src/manager.rs:220-227`](../../../../crates/resource/src/manager.rs))
  already carries `scope`, `resilience`, `recovery_gate` — all per-resource
  configuration. Adding `credential_rotation_timeout` matches that pattern
  without bloating the struct. `RegisterOptions` derives `Clone`, so the new
  `Option<Duration>` field composes for free.
- **Default 30s is right for blue-green pool builds.** The blue-green swap
  pattern from credential Tech Spec §3.6 lines 961-993 builds a fresh pool
  before the swap. A fresh Postgres pool with `pool_size=20` and 100ms per
  connection takes ~2s steady state, but cold-start over a TLS handshake
  with a remote vault token resolution can stretch to 10-15s in pathological
  cases. 30s leaves headroom; 5s would force operators to override on every
  registration (pattern erosion).
- **No options-bloat tension.** `Option<Duration>` with `None` = inherit
  default is the orthodox Rust idiom for "I want override OR fall through";
  the live `resilience: Option<AcquireResilience>` and
  `recovery_gate: Option<Arc<RecoveryGate>>` are exactly this shape.
  Adding one more `Option<…>` field does not cross the bloat threshold.

🟡 Edit suggestion (Q4 framing): §3.3 (Tech Spec lines 822-885) declares the
fields but does not show a builder method on `RegisterOptions` for setting
the override. The current pattern is direct field assignment. CP2 §5 might
add a builder method (`with_credential_rotation_timeout(Duration)`); I don't
want to commit to that here, but flagging that this is a CP2 DX item to revisit.

## Required edits (bounded)

In priority order:

1. **🔴 §3.5 disposition of event-shape asymmetry.** Update the open item (Tech
   Spec line 935 / 970) to explicitly delegate the refresh-vs-revoke event
   cardinality asymmetry to security-lead in CP2 §7 with the three options
   (refresh-also-per-resource, revoke-spans-only, asymmetry-is-load-bearing).
   Currently reads as "CP2 §7 finalizes broadcast cardinality" without naming
   the specific tension.

2. **🟡 §3.2 dispatcher lifetime documentation.** Add a SAFETY-style comment
   on the `Manager::on_credential_refreshed` closure body (around Tech Spec
   line 794-806) explaining the `Arc::clone(d)` move + `'a` reborrow + shared
   `&scheme` lifetime pattern. The code is correct but non-obvious; production
   reviewers will hit the same lifetime puzzle the spike NOTES.md flagged.

3. **🟡 §3.2 list NEW error constructors.** Add a footnote or short subsection
   listing `Error::missing_credential_id(ResourceKey)` (line 641) and
   `Error::scheme_type_mismatch::<R>()` (line 743) as NEW error variants the
   implementation must add to `crates/resource/src/error.rs`. The spec assumes
   them silently; implementer must not miss.

Plus three minor 🟡 suggestions inline above (Q3 latency framing wording,
§2.3 idempotency docstring, §2.2 NoCredential::resolve operational-note).

## What I am not signing off

- **§3.5 aggregate-vs-per-resource event correctness** (covered above).
- **CP2 deliverables** (§4 lifecycle, §5 storage, §6 security, §7 operational,
  §8 testing). My ratification covers §0-§3 only.
- **Whether the engine-side `EventSource → TriggerAction` adapter signature is
  right** — that's CP3 §13 deliverable per §2.4 / ADR-0037; out of CP1 scope.
- **Whether `FuturesUnordered` cap at N>32 is the right deferred design** —
  I ratify *deferring* it (§3.4 line 893); I do not ratify its *eventual* shape.

---

**Verdict reaffirmed: RATIFY_WITH_EDITS.** Three required edits (one 🔴, two
🟡); none triggers iterate. CP1 ships with these edits applied; tech-lead
ratification flips ADR-0036 + ADR-0037 to `accepted` per §0.1.
