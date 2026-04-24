# Phase 1 — nebula-action Idiomatic Rust Review (02c)

**Date:** 2026-04-24
**Author:** rust-senior (sub-agent, findings only; no fixes proposed)
**Target:** Rust 1.95.0 (pinned in `rust-toolchain.toml:17`), edition `2024` (workspace). MSRV = 1.95.
**Inputs:** `crates/action/src/**` (21 files), `crates/action/macros/src/**` (3 files), `cargo expand` output of `tests/derive_action.rs` (re-run this invocation; text quoted below), Phase 0 S6 (HRTB verbosity) and T1 (no macro harness). No overlap with Phase 0 01a surface inventory.
**Severity legend:** 🔴 WRONG (actively miswritten idiom) / 🟠 DATED (pre-1.95 idiom that can modernize) / 🟡 PREFERENCE (stylistic or tradeoff-context).
**Guide references:** `docs/guidelines/research/cluster-02-nomicon-unsafe-rfcs.md:491-499` (RPITIT + `trait_variant::make`); `docs/guidelines/research/cluster-04-perf-effective-macros.md` (macro emission quality).

---

## 1. Trait shape — RPITIT vs HRTB vs `async-trait`-style

### What the crate actually uses (concurrent idioms in one crate)

| Trait | Form | Location |
|---|---|---|
| `StatelessAction::execute` | RPITIT: `-> impl Future<Output = …> + Send` | `stateless.rs:98-102` |
| `StatefulAction::execute` | RPITIT | `stateful.rs:72-77` |
| `TriggerAction::start`/`stop` | RPITIT | `trigger.rs:63-72` |
| `ResourceAction::configure`/`cleanup` | `async fn` sugar (desugars to RPITIT) | `resource.rs:248-260` impls |
| `ControlAction::evaluate` | RPITIT, with docs showing both sugar and explicit form | `control.rs:426-430` |
| `PaginatedAction::fetch_page` | RPITIT | `stateful.rs:149-154` |
| `BatchAction::process_item` | RPITIT | `stateful.rs:293-297` |
| `PollAction::poll` / `validate` / `initial_cursor` | RPITIT with default bodies | `poll.rs:822-867` |
| `StatelessHandler::execute` (dyn) | explicit HRTB: `for<'life0, 'life1, 'a> … Pin<Box<dyn Future<…> + Send + 'a>>` | `stateless.rs:313-322` |
| `StatefulHandler::execute` (dyn) | explicit HRTB (4 lifetimes) | `stateful.rs:461-472` |
| `TriggerHandler::start`/`stop`/`handle_event` (dyn) | explicit HRTB | `trigger.rs:328-335`, `346-353`, `373-381` |
| `ResourceHandler::configure`/`cleanup` (dyn) | explicit HRTB | `resource.rs:83-91`, `98-106` |
| `CredentialContextExt::credential_by_id`/`credential_typed`/`credential` | explicit return-as-`Pin<Box<…>>`, not trait methods | `context.rs:575-685` |

Two-layer pattern: typed trait = RPITIT (ergonomic); dyn-safe companion = HRTB-boxed (for `Arc<dyn *Handler>` storage in `ActionHandler` enum). This is a **deliberate, correct split** for a handler registry.

### Findings

**🟠 DATED — `*Handler` HRTB boilerplate is the pre-1.85 shape.** The explicit `for<'life0, 'life1, 'a>` + `Pin<Box<dyn Future<Output = …> + Send + 'a>>` + `where Self: 'a, 'life0: 'a, 'life1: 'a` pattern visible at `stateless.rs:313-322`, `stateful.rs:461-472`, `trigger.rs:328-335 / 346-353 / 373-381`, `resource.rs:83-91 / 98-106` (plus mirrored on every adapter `impl … for *Adapter`) is the **historical async-trait-by-hand shape**. It dates to the era when an `async fn` in a trait couldn't be made dyn-safe.

As of Rust 1.75 `async fn` in trait is stable (with each impl's opaque type being per-impl, therefore **not** dyn-compatible). As of 1.85 return-type-notation (`Trait<method(..): Send>`) is stable, and `#[trait_variant::make(Trait: Send)]` auto-generates a `Send`-bounded sibling trait. But none of those tools make `dyn Trait` with `async fn` dyn-safe today — so the `Pin<Box<dyn Future<…>>>` return shape is genuinely still needed for the registry use case.

What *is* dated, however: the explicit lifetime-naming (`'life0`, `'life1`) and the `where 'life0: 'a, 'life1: 'a` dance. On 1.95 this entire shape can be written as:

```rust
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait StatelessHandler: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
    fn execute<'a>(
        &'a self,
        input: Value,
        ctx: &'a dyn ActionContext,
    ) -> BoxFuture<'a, Result<ActionResult<Value>, ActionError>>;
}
```

with a single `'a` covariant over both `self` and `ctx`. Rustc's elision rules accept this since 1.51 (RFC 2115). The `'life0`/`'life1` split is what `async-trait` emits for macro-hygiene reasons — but this crate **isn't using `async-trait`** (grep: zero occurrences), so inheriting its naming convention buys nothing. This is verbosity without semantic benefit, repeated ~14 times across the handler family.

**🟡 PREFERENCE — `ResourceHandler` shows the cleaner alias approach already.** At `resource.rs:58-64` the crate defines:
```rust
pub type ResourceConfigureFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, ActionError>> + Send + 'a>>;
pub type ResourceCleanupFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + 'a>>;
```
These aliases turn the method signatures into readable shapes (`fn configure(…) -> ResourceConfigureFuture<'a>`). The other four handler traits **don't** use this pattern — they inline the `Pin<Box<dyn Future<…>>>` at each use site. Inconsistent discipline within one crate.

**🟢 RIGHT — RPITIT on typed trait family.** `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`, `ControlAction`, `PaginatedAction`, `BatchAction`, `PollAction` all use `impl Future<Output = …> + Send` — the canonical 1.95 idiom. Send-bound is explicit on every method, which matches the style guide recommendation (cluster-02 lines 494-497: RPITIT requires explicit `Send` or lose executor portability). No `#[async_trait]` anywhere in the crate — correct modern choice.

**🟡 PREFERENCE — `control.rs:403-421` documents both sugar and explicit RPITIT forms.** Rare positive: the trait's own docs teach the reader what `async fn evaluate` desugars to. Other traits should adopt this pattern.

**🟡 PREFERENCE — associated-type bound elision on `ActionContext`.** Every trait method takes `ctx: &(impl ActionContext + ?Sized)`. The `+ ?Sized` is defensive — `ActionContext` is a marker-composed umbrella (`context.rs:80-89`, see §3). Blanket impl at `context.rs:91-101` makes any `T: ResourceAccessor + CredentialAccessor + Logger + …` a valid `ActionContext`. The `+ ?Sized` bound is right for `&dyn ActionContext` call sites but redundant when callers pass a monomorphized ctx. Keep it — cost is zero, benefit is dyn-dispatch support.

### Send-bound discipline

- 🟢 All typed trait RPITIT methods say `+ Send` explicitly. No silent single-threaded trait.
- 🟢 `*Handler` trait objects carry `Send + Sync` as supertrait bound (`stateless.rs:304`, `stateful.rs:404`, `trigger.rs:276`, `resource.rs:74`) — required for `Arc<dyn *Handler>` storage.
- 🟢 Returned `Pin<Box<dyn Future + Send + 'a>>` carries `Send`.
- 🟡 `TriggerHandler::handle_event` default body (`trigger.rs:382-389`) returns `Box::pin(async { Err(…) })`. The underscore pattern `let _ = (event, ctx);` silences unused-warning without telling the compiler it can drop args early — but since the future is immediately ready, not load-bearing.

### Unnecessary `dyn`

- `ActionHandler` enum (`handler.rs:41-50`) stores `Arc<dyn *Handler>` per variant. Given there are exactly 4 variants and each variant represents one dispatch family, this is correct dyn usage (registry + polymorphic dispatch at runtime).
- `CredentialContextExt::credential_by_id` etc (`context.rs:575-685`) return `Pin<Box<dyn Future<…> + Send + '_>>` even though these are trait methods on a default-body trait that could use RPITIT. Reason: the trait is designed to work on `&dyn HasCredentials` (see Phase 0 01a §5). Correct dyn usage — `dyn HasCredentials` is not RPITIT-compatible.
- `Arc<dyn Fn() -> BoxFuture>` appears nowhere — good, no callback-dispatcher noise.

### Summary for §1

The trait shape split (RPITIT for typed author-facing, HRTB-boxed for dyn registry) is **correct**. The execution of the HRTB side is **verbose by ~30-40% of line count** due to `'life0`/`'life1` naming copied from `async-trait` without using `async-trait`. This is a 🟠 DATED finding: idiomatic, functional, but mechanically replaceable with a single-lifetime shape that reads better.

---

## 2. Macro expansion quality (`cargo expand` evidence attached)

### Ground-truth expansion

Ran `cargo expand --test derive_action -p nebula-action` (cache-warm this session). For the input:

```rust
#[derive(Action)]
#[action(key = "test.no_cred", name = "No Cred", description = "no credentials")]
struct NoCredAction;
```

the macro emits:

```rust
impl ::nebula_core::DeclaresDependencies for NoCredAction {
    fn dependencies() -> ::nebula_core::Dependencies {
        ::nebula_core::Dependencies::new()
    }
}
impl ::nebula_action::Action for NoCredAction {
    fn metadata(&self) -> &::nebula_action::metadata::ActionMetadata {
        use ::std::sync::OnceLock;
        static METADATA: OnceLock<::nebula_action::metadata::ActionMetadata> = OnceLock::new();
        METADATA.get_or_init(|| {
            ::nebula_action::metadata::ActionMetadata::new(
                ::nebula_core::ActionKey::new("test.no_cred")
                    .expect("invalid action key in #[action] attribute"),
                "No Cred",
                "no credentials",
            )
            .with_version_full(::semver::Version::new(1u64, 0u64, 0u64))
        })
    }
}
```

### Observations

**🟢 RIGHT — no dead-branch allocation.** Each `#[action(...)]` flag produces code only when present: zero credentials → `Dependencies::new()` with no `.credential(...)` chain. No `Vec::new()` where an empty chain would do. Verified in `action_attrs.rs:121-146 / 147-196` — both emission paths early-short when the attr collection is empty.

**🟢 RIGHT — `OnceLock` is the idiomatic "initialize-once, read-many" choice for a `fn metadata(&self) -> &ActionMetadata` contract.** `get_or_init` is allocation-free after first call; the stored `ActionMetadata` lives in BSS for the process lifetime. `lazy_static!` would be a pre-1.70 anti-pattern; `once_cell::sync::Lazy` would be slightly heavier (additional `Lazy<T>` newtype) with no benefit. Per-invocation cost after warm-up: one acquire-load. 🟢 right choice for 1.95.

**🔴 WRONG — broken `.with_parameters(...)` emission.** `action_attrs.rs:129-134`:
```rust
let params_expr = match &self.parameters {
    Some(ty) => quote! { .with_parameters(<#ty>::parameters()) },
    None => quote! {},
};
```
`ActionMetadata` has no `with_parameters` method — the only schema-input method is `.with_schema(schema: ValidSchema)` (per Phase 0 01a §4 verification). Any user writing `#[action(parameters = MyCfg)]` produces a **cryptic compile error pointing at user-code span**. Masked because no workspace caller exercises it. Same finding as Phase 0 C2; listed here because it's the single clearest 🔴 idiomatic error in macro emission quality.

**🟠 DATED — `.expect("invalid action key in #[action] attribute")` inside `get_or_init` means the panic site is lazy.** `ActionKey::new("test.no_cred")` runs on first `metadata()` call, not at `#[derive]` expansion time or at program start. For a compile-time-known string this is a **missed opportunity to validate at macro expansion**: `proc-macro2::Literal` + `const fn ActionKey::new_const(...)` (if added to `nebula-core`) would let the macro emit `ActionKey::new_const!("test.no_cred")` and fail at compile time. Current shape defers invalid-key discovery until *runtime first call*, which in a trigger crate means first event dispatch. This is a 🟠: not wrong, but ships a class of bugs from compile to runtime. Fix belongs in `nebula-core` (expose `const fn` / macro constructor); macro consumes it. Flag for orchestrator (design-time decision, not rust-senior scope).

**🟡 PREFERENCE — `1u64, 0u64, 0u64` suffix noise.** `semver::Version::new` takes `(u64, u64, u64)`. `ActionAttrs::metadata_init_expr` emits `#major`, `#minor`, `#patch` where the fields are `u64`. `quote!` inserts them as typed literals; the `u64` suffix is redundant but cosmetic. Rustfmt leaves these alone. No action.

**🟡 PREFERENCE — `::std::sync::OnceLock` vs `::core::sync::OnceLock`.** Std since 1.70. `core::sync::OnceLock` wasn't added until 1.76 (yes, later — `std::sync::OnceLock` predates it). On 1.95 both exist; macro uses `std::` which is fine for std-assumed crate (which `nebula-action` is — it depends on `tokio`). No-op.

**🟡 PREFERENCE — hidden allocation in `dependencies()`.** The macro emits:
```rust
.credential(::nebula_core::CredentialRequirement::new(
    <#ty as ::nebula_core::CredentialLike>::KEY_STR,
    ::std::any::TypeId::of::<#ty>(),
    ::std::any::type_name::<#ty>(),
))
```
per credential. `CredentialRequirement::new` likely does one `String::from` for the `&'static str` KEY_STR (need to confirm in `nebula-core::dependencies`). If so, every call to `dependencies()` allocates N strings. Since `DeclaresDependencies::dependencies()` is called once per registration this is a one-time cost — 🟡. If called per dispatch it would be 🔴. Recommend orchestrator verify frequency against `nebula-core`.

**🟢 RIGHT — no `Box<dyn Trait>` where a generic would suffice in macro emission.** The macro emits concrete types only. No `Box<dyn Any>`, no `Arc<dyn Fn…>`. Clean generation.

**🟢 RIGHT — no unnecessary `Arc::clone` in emission.** No `.clone()` in the emitted code except what the user types explicitly.

**🟠 DATED — the macro is 359 LOC doing work that could be ~150 LOC.** Breakdown of `action_attrs.rs` (243 LOC):
- 33 LOC: `ActionAttrs` struct with 11 fields
- 64 LOC: `parse(…)` with duplicate-cred detection
- 16 LOC: `all_credentials`/`all_resources` helpers
- 26 LOC: `metadata_init_expr`
- 50 LOC: `dependencies_impl_expr`
- 39 LOC: `parse_version` (semver normalization "X.Y" → "X.Y.0")
- 15 LOC: doc comments

Of these: `all_credentials`/`all_resources` exist only because `credential: Option<Type>` and `credentials: Vec<Type>` are split into two fields (to preserve attribute-syntax flexibility). A single `Vec<Type>` with "accept both spellings" parsing would fold the helpers out. `parse_version` carries a `owned_buf` + three `.push_str(".0")` loop iterations to handle `"1.0"` vs `"1.0.0"` — `semver` 1.0.22+ has a `VersionReq::parse` path but for a fixed `Version` the normalization is unavoidable unless you require full triples.

Not a defect. Flag: **the macro carries its weight for what it does**, but what it does is narrower than typical proc-macros (no field walking, no type-level rewriting). Most of the LOC budget is parse-side error reporting and duplicate-detection. If Phase 3 keeps `#[derive(Action)]` as-is, there's no idiomatic reason to shrink it. If Phase 3 moves to a full attribute macro (`#[action]` doing field-type rewriting per CP6 Tech Spec §2.7), expect 3–5× this LOC.

### Summary for §2

Emission quality is **good** modulo one 🔴 (`with_parameters` dead branch) and one 🟠 (late `ActionKey` validation). The 359 LOC budget is carrying its weight for the current scope. Per T1 in Phase 0, the **critical defect is not macro quality, it's macro test coverage** — no `trybuild` / `macrotest` harness means regressions in generated code go undetected. That is a tooling gap, not an idiom gap.

---

## 3. Associated type design ergonomics

### What the trait family declares

| Trait | Assoc types | Bounds |
|---|---|---|
| `StatelessAction` | `Input: HasSchema + Send + Sync`, `Output: Send + Sync` | minimal |
| `StatefulAction` | `Input: HasSchema + Send + Sync`, `Output: Send + Sync`, `State: Serialize + DeserializeOwned + Clone + Send + Sync` | heavy on State |
| `TriggerAction` | *(none — only methods)* | — |
| `ResourceAction` | `Resource: Send + Sync + 'static` | minimal |
| `ControlAction` | *(none)* | — |
| `PaginatedAction` | `Input: HasSchema + Send + Sync`, `Output: Send + Sync`, `Cursor: Serialize + DeserializeOwned + Clone + Send + Sync` | heavy on Cursor |
| `BatchAction` | `Input: HasSchema + Send + Sync`, `Item: Serialize + DeserializeOwned + Clone + Send + Sync`, `Output: Serialize + DeserializeOwned + Clone + Send + Sync` | heaviest |
| `PollAction` | `Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync`, `Event: Serialize + Send + Sync` | Cursor must have `Default` |

### Findings

**🟢 RIGHT — `State: Serialize + DeserializeOwned + Clone + Send + Sync` on `StatefulAction`.** Clone for pre-execute snapshot (rollback), ser/de for engine checkpointing, Send+Sync for async crossing. Every bound carries weight per the adapter's state-flush invariant documented at `stateful.rs:427-474`. Tight-but-correct.

**🟢 RIGHT — `HasSchema` on Input.** `fn schema() -> ValidSchema` default body derives input schema automatically (`stateless.rs:84-89`). `()` and `serde_json::Value` have blanket `HasSchema` impls (per `stateless.rs:76-78` doc). This is a forward-ergonomic choice that replaces per-author boilerplate.

**🟠 DATED — the `Output: Send + Sync` on `StatelessAction` is **under-constrained** when considered against the adapter.** At `stateless.rs:349-354`:
```rust
impl<A> StatelessHandler for StatelessActionAdapter<A>
where
    A: StatelessAction + Send + Sync + 'static,
    A::Input: serde::de::DeserializeOwned + Send + Sync,
    A::Output: serde::Serialize + Send + Sync,
```
The adapter requires `Serialize` on Output, but the trait itself doesn't. Authors implementing `StatelessAction` directly (without the adapter) can pick an `Output` type that `impl StatelessAction` accepts but `StatelessActionAdapter` rejects — they'll see a **confusing "X does not implement StatelessHandler" error at registration** instead of a crisp bound-on-assoc-type error at trait impl time.

Two idioms for this in 1.95:
1. Lift `Serialize + DeserializeOwned` to the trait's assoc type (tight coupling, matches `StatefulAction::State`).
2. Move the bound to a supertrait-like marker (e.g. `type Input: HasSchema + DeserializeOwned`).

Current shape is a **leaky adapter invariant**. Same pattern applies to `StatefulAction::Input / Output` (`stateful.rs:43-45`) vs adapter `stateful.rs:505-507`, and `TriggerEvent` family downcasts. This is 🟠 because it doesn't actively break anything but it shifts error surface from trait-impl site to registration site. v2 spec §2 implies "type Input = Self" as default, which would fold this into one decision — the design has not landed in code.

**🟡 PREFERENCE — `PollAction::Cursor: Default`.** The only associated-type bound in the crate that requires `Default`. Reason documented at `poll.rs:836-842`: `initial_cursor(&self, _ctx) -> impl Future<Output = Result<Self::Cursor>>` has a default body `async { Ok(Self::Cursor::default()) }`. Reasonable. Forces cursor types to be `Default`-able — `String`, numeric, and the custom `DeduplicatingCursor` already are. 🟢 right call.

**🟡 PREFERENCE — no GAT usage anywhere.** `ActionContext` is `&(impl ActionContext + ?Sized)` everywhere, not `&Self::Ctx<'_>`. For the current use case (context is runtime-supplied, not author-declared) this is right. A GAT-ified `type Context<'a>` would let authors constrain the context shape per-action but isn't worth the complexity budget unless Phase 3 lands ContextExt / credential-type ties. Flag for architect — GAT usage might be load-bearing if Pattern 2 CP6 dispatch lands.

**🟡 PREFERENCE — `TriggerAction` has zero assoc types but 4 trigger-family *children* do (`WebhookAction`, `PollAction`, `TriggerEvent::Payload`).** The base `TriggerAction` is a **method-only lifecycle trait** — everything typed is pushed to specialization traits. Given the transport-agnostic envelope pattern in `trigger.rs:75-122` (type-erased payload via `Box<dyn Any>` with `TypeId` capture), this is deliberate and correct — but note that `TriggerEvent::downcast<T>` (`trigger.rs:182-202`) returns `Err(self)` on mismatch, a signature that's unusual (typically `Option<T>` or a classic `Result<T, MyError>`). Giving up the payload on failure matches the documentation's "engine routing bug, unreachable" framing but means callers must `match e.downcast::<X>() { Err(e) => e.payload_type_name() }` to diagnose — 🟡, defensible.

### Summary for §3

Assoc-type design is **mostly right**. One systemic 🟠: adapter bounds on `Input: DeserializeOwned, Output: Serialize` are not lifted to the trait itself, so error UX degrades at adapter instantiation. Folding these into trait bounds (or adopting the v2-spec `type Input = Self` default) would consolidate the surface. Cursor / State / Event bounds are correctly tight.

---

## 4. Cancellation safety discipline

### Cancel-safety patterns in the crate

| Location | Pattern | Verdict |
|---|---|---|
| `poll.rs:1336-1418` `PollTriggerAdapter::start` main loop | `tokio::select!` between `cancellation().cancelled()` and `sleep(interval)` | 🟢 textbook cancel-safe |
| `poll.rs:1341-1343` pre-poll cancel check | `if ctx.cancellation().is_cancelled() { return Ok(()) }` before expensive poll | 🟢 defensive, correct |
| `poll.rs:1354-1365` per-poll timeout | `tokio::time::timeout(config.poll_timeout, self.action.poll(...))` | 🟢 correct — `timeout` is cancel-safe; drop resumes inner future |
| `webhook.rs:1266-1298` `handle_event` race | `tokio::select! { biased; () = cancellation().cancelled() => …; result = handle_request(...) => … }` | 🟢 correct; `biased;` ensures cancellation checked first |
| `webhook.rs:1111-1119` `RwLock` guard scope | explicit `{ let mut guard = self.state.write(); … }` block, guard drops before any `.await` | 🟢 prevents non-Send guard across `.await` |
| `webhook.rs:1239-1241` state clone under read guard | `let state = if let Some(s) = self.state.read().as_ref().cloned() { … }` — guard scope is the RHS of `if let`, drops before `.await` | 🟢 correct; doc comment `webhook.rs:1238-1240` explicitly calls this out |
| `webhook.rs:1147-1174` stop() in-flight wait | `loop { if counter==0 { break } let notified = idle_notify.notified(); pin!; enable(); recheck; await }` | 🟢 textbook `tokio::sync::Notify` wait pattern (enable-before-check prevents missed-wake) |
| `poll.rs:900-906` `StartedGuard(&'a AtomicBool)` RAII | Drop clears `started` flag via `Ordering::Release` | 🟢 correct — "defused RAII pattern, NOT mem::forget" per trigger.rs:1032-1034 — matches the `scopeguard::defer` idiom |
| `poll.rs:1059-1071` `InFlightGuard` RAII | `fetch_sub(1, AcqRel) == 1 → notify_waiters()` | 🟢 atomic + notify is correct release-side paired with `load(Acquire)` on the waiter |

### Findings

**🟢 RIGHT — `parking_lot::Mutex/RwLock` guards never held across `.await`.** Exhaustive grep (`RwLock|Mutex|MutexGuard` in `crates/action/src/`) turned up 20 matches; every `.write()` or `.read()` guard is bounded by a block or an `if let` RHS that drops before any `.await`. Explicit developer awareness visible in docstrings (`webhook.rs:1105-1110` lampshades the risk). Non-trivial — the standard footgun is holding `parking_lot` guards across `.await` and getting Send-bound violations at runtime.

**🟢 RIGHT — use of `parking_lot::Mutex` for sub-microsecond sync paths + `tokio::sync::Notify` for wait paths.** `poll.rs:877 WarnThrottle::last_logged`, `webhook.rs:132 response_tx`, `webhook.rs:1017 state` use parking_lot because locks are held for nanoseconds. `webhook.rs:1023 idle_notify: Arc<Notify>` handles the cross-await coordination. Correct tool split.

**🟢 RIGHT — `biased;` in select! where ordering matters.** `webhook.rs:1267` uses `biased;` so cancellation is always polled first. Without `biased;` the macro uses a randomized polling order — fine for fairness, wrong for "cancellation must pre-empt". Deliberate and correct.

**🟢 RIGHT — `CancellationToken` usage is textbook.** `tokio_util::sync::CancellationToken` (declared in `context.rs:28`) is the canonical 1.95 cancellation primitive. `ctx.cancellation().cancelled()` returns `WaitForCancellationFuture` which is cancel-safe by construction. No hand-rolled cancellation futures. `context.rs` threads the token into `ActionRuntimeContext` / `TriggerRuntimeContext` via `BaseContext` — one cancellation point for the whole action graph.

**🟡 PREFERENCE — `PollTriggerAdapter::stop()` fires cancellation but doesn't await.** `poll.rs:1446-1457`:
```rust
fn stop<'life0, 'life1, 'a>(&'life0 self, ctx: &'life1 dyn TriggerContext) -> ... {
    ctx.cancellation().cancel();
    Box::pin(async { Ok(()) })
}
```
Documented at `poll.rs:1430-1446`: "This does not wait for the loop to finish." Restart-after-stop races with the `StartedGuard` drop; caller must hold a `JoinHandle` from spawning `start()` and await it. The fact that **start() itself is doc'd as either "setup-and-return" (shape 1) or "run-until-cancelled" (shape 2)** at `trigger.rs:280-302` makes this a **hidden fire-and-forget problem** at the callsite level — the trait itself doesn't return a join handle, so callers can't enforce the pattern structurally. Phase 0 01a already flags this as "F-level work: runtime-owned task handle will hide this footgun". 🟡 documented, not idiom-defective.

**🟡 PREFERENCE — `std::sync::atomic::AtomicBool` used directly for `started` sentinel.** `poll.rs:1053` + `compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)` at `1312`. Textbook pattern, zero criticism. Better than `Mutex<bool>` here (uncontended atomics are faster) and better than `OnceLock<bool>` (OnceLock isn't resettable for stop/restart).

### Summary for §4

Cancel safety is **the strongest discipline in the crate**. All the classic footguns (guards across await, missed Notify wakes, fire-and-forget spawn without JoinHandle storage) are either handled or explicitly documented. The poll + webhook adapters are reference-quality for how to write cancel-safe Tokio code in 1.95. One outstanding design-time issue (`stop()` doesn't return JoinHandle) is architectural, not idiomatic.

---

## 5. SlotBinding / Pattern 2 composition shape (conditional on Phase 2 Option A)

**Scope:** Only relevant IF Phase 2 chooses Option A (action adopts CP6 credential Tech Spec vocabulary). Else skip this section.

### The proposed Tech Spec §7.1 shape

```rust
pub struct SlotBinding {
    pub slot: &'static str,
    pub resolve_fn: for<'ctx> fn(
        &'ctx CredentialContext<'ctx>,
        &'ctx CredentialId,
    ) -> BoxFuture<'ctx, Result<RefreshOutcome, RefreshError>>,
    // ... metadata fields
}
```

### Findings (idiomatic review of the **proposed** shape, not current code)

**🟢 RIGHT — HRTB fn-pointer (`for<'ctx> fn(…) -> BoxFuture<'ctx, …>`) is idiomatic for phantom-type erasure.** The HRTB universally quantifies over the context lifetime, so the fn-pointer can be stored in a static slot table (no lifetime in the outer struct). Alternatives (`Arc<dyn Fn…>`, `Box<dyn Fn…>`, generic `F: for<'ctx> Fn(…) -> …`) are all **more expensive or less erasing**:
- `Arc<dyn Fn>` adds one pointer indirection + one atomic refcount bump per clone. Over a handshake that happens once per dispatch, ~2-5 ns overhead. Negligible at the individual call; load-bearing in a 10k-slot-per-sec hot path.
- `Box<dyn Fn>` loses `Clone`, which SlotBinding may need.
- Generic `F` forces monomorphization per credential type, blowing up the registry code size and precluding storage in a type-erased `&'static [SlotBinding]` array.

The `for<'ctx> fn(...)` shape is the smallest thing that works for the stated invariants. If Phase 2 chooses Option A, **resist the temptation to "simplify" to `Arc<dyn Fn>`** — it loses the zero-cost property.

**🟡 PREFERENCE — `BoxFuture<'ctx, Result<RefreshOutcome, RefreshError>>` is the unavoidable erasure point.** Without returning a `dyn Future`, the fn pointer's return type would vary per callee and break the HRTB signature. Fine.

**🟠 DATED — resist any urge to write this with `async fn` pointers.** As of 1.95 there is **no** `async fn` pointer syntax. You cannot write `for<'ctx> async fn(&'ctx CredContext) -> Result<…>`. Any proposal to "modernize" by using `async fn` in this slot is wrong — the macro emitter must keep the HRTB + `BoxFuture` shape.

**🟡 PREFERENCE — consider a newtype over the fn-pointer.** Instead of:
```rust
pub resolve_fn: for<'ctx> fn(&'ctx Ctx, &'ctx Id) -> BoxFuture<'ctx, ...>,
```
define:
```rust
pub struct ResolveFn(pub for<'ctx> fn(&'ctx Ctx, &'ctx Id) -> BoxFuture<'ctx, ...>);
```
This lets downstream `impl Debug for ResolveFn` print the fn-pointer address (for diagnostics) without polluting `SlotBinding`'s derive(Debug). Minor DX.

### Summary for §5

The HRTB fn-pointer shape is **load-bearing and correct** for the phantom-erasure invariants described in CP6 Tech Spec §7.1. No idiomatic simplification exists. If Phase 2 lands Option A, emit the HRTB verbatim and don't second-guess it. If Phase 2 does not choose Option A, this section is moot.

---

## 6. Dyn-safety modernization potential

### What Rust 1.95 makes possible that 1.75 did not

- 1.75: `async fn` in trait stabilized (not dyn-safe).
- 1.80+: `-> impl Trait` in trait methods (RPITIT) stabilized.
- 1.85: Return-type-notation `Trait<method(..): Send>` — lets generic bounds name the future's Send-ness.
- 1.95 (current pin): all of the above stable; `#[trait_variant::make(Trait: Send)]` is idiomatic tooling (external crate, adds zero runtime cost).

### Findings

**🟠 DATED — the explicit HRTB lifetime boilerplate on `*Handler` traits can be tightened idiomatically.** Current (`stateless.rs:313-322`):
```rust
fn execute<'life0, 'life1, 'a>(
    &'life0 self,
    input: Value,
    ctx: &'life1 dyn ActionContext,
) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send + 'a>>
where
    Self: 'a,
    'life0: 'a,
    'life1: 'a;
```

Idiomatic 1.95 replacement using alias + single lifetime:
```rust
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait StatelessHandler: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
    fn execute<'a>(
        &'a self,
        input: Value,
        ctx: &'a dyn ActionContext,
    ) -> BoxFut<'a, Result<ActionResult<Value>, ActionError>>;
}
```

The single `'a` is covariant over both `&self` and `&dyn ActionContext` (rustc elision: the outer `'a` is the shorter of the two input lifetimes). This compiles on 1.95, passes dyn-safety checks (method has a single lifetime, associated with `self`), and cuts 8 lines per handler trait.

**Why this matters beyond line count:** the `'life0`/`'life1`-style boilerplate is actively misleading — readers unfamiliar with `async-trait` will try to figure out why the explicit naming was chosen. The answer is "historical: it's what `async-trait` emits." Removing it **reduces the knowledge prerequisite** for new contributors. T1's missing macro harness means nobody is enforcing this discipline either way.

**🟠 DATED — `#[trait_variant::make(Handler: Send)]` is not used, but could replace the RPITIT/HRTB split entirely.** The canonical 1.95 story for "I want `async fn` in trait + dyn-safety + Send" is:

```rust
#[trait_variant::make(StatelessHandler: Send)]
pub trait LocalStatelessHandler {
    async fn execute(&self, input: Value, ctx: &dyn ActionContext)
        -> Result<ActionResult<Value>, ActionError>;
}
```

This generates *two* traits: `LocalStatelessHandler` (executor-local, future need not be Send) and `StatelessHandler` (Send-bounded, dyn-compatible via RPITIT). Callers use `StatelessHandler` for cross-thread storage. The crate does not currently import `trait_variant` (grep confirmed).

**Tradeoff:**
- `trait_variant` is a single external crate, well-maintained (rust-lang stewardship, 2M+ downloads).
- Net LOC reduction: ~40-60 lines across the handler family, plus all the adapter `impl ... for *Adapter` methods.
- New contributor burden: one macro to learn.
- Breaks existing public `*Handler` trait surface (callers writing `impl StatelessHandler` by hand would need to adopt the `async fn` shape).

**Verdict:** 🟠 DATED but not 🔴 WRONG — the existing shape works on 1.95, doesn't `cargo check --workspace` warn, and is internally consistent. `trait_variant` would be a Phase 3 redesign decision, not a Phase 1 idiom fix. **Flag for architect** — this is a legitimate `trait_variant::make` candidate and the LOC payoff is real.

**🟢 RIGHT — no dyn-incompatible trait surfaces.** None of the 4 `*Handler` traits use GATs or `Self: Sized` in method signatures. Dyn-safety checked manually in `handler.rs::tests` (4 `fn *_is_dyn_compatible` tests, `handler.rs:273-302`) — a nice defensive pattern.

### Summary for §6

HRTB lifetime verbosity is a real 🟠 and can be tightened without touching semver by switching to single-lifetime + type alias. `trait_variant` adoption is a Phase 3 redesign decision with clear LOC payoff — don't do it as a Phase 2 "mechanical cleanup". Current shape is idiomatic-for-1.75; can be idiomatic-for-1.95 with meaningful LOC reduction.

---

## 7. Error taxonomy coherence

### The `ActionError` variant matrix (error.rs:152-266)

| Variant | `is_retryable()` | `is_fatal()` | `Classify::code()` | User hint field |
|---|---|---|---|---|
| `Retryable { error, code, backoff_hint, partial_output }` | ✅ | ❌ | `ACTION:RETRYABLE` | `code: Option<RetryHintCode>` |
| `Fatal { error, code, details }` | ❌ | ✅ | `ACTION:FATAL` | `code: Option<RetryHintCode>` |
| `Validation { field: &'static str, reason: ValidationReason, detail: Option<String> }` | ❌ | ✅ | `ACTION:VALIDATION` | — |
| `SandboxViolation { capability, action_id }` | ❌ | ✅ | `ACTION:SANDBOX_VIOLATION` | — |
| `Cancelled` | ❌ | ❌ | `ACTION:CANCELLED` | — |
| `DataLimitExceeded { limit_bytes, actual_bytes }` | ❌ | ✅ | `ACTION:DATA_LIMIT` | — |
| `CredentialRefreshFailed { action_key, source }` | ✅ | ❌ | `ACTION:CREDENTIAL_REFRESH_FAILED` | — |

### Findings

**🟢 RIGHT — two-axis classification preserved cleanly.** The typed `RetryHintCode` is an **action-author hint** about *how* to retry (`RateLimited`, `AuthExpired`, `UpstreamTimeout`, …). The `Classify::code()` + `Classify::category()` axis is a **framework-level classifier** for observability (`ACTION:VALIDATION` vs `Internal`/`External`/`Validation`/`Authorization`/`Cancelled`/`Exhausted` categories). The rename from `ErrorCode` → `RetryHintCode` documented at `error.rs:13-17` shows deliberate discrimination between the two concepts. **Strong idiomatic discipline** — most crates collapse these and end up with an error enum that serves neither observability nor retry policy well.

**🟢 RIGHT — `#[from]` / `#[source]` on `CredentialRefreshFailed.source`** (`error.rs:263-264`). The `#[source]` attribute ensures `std::error::Error::source()` walks the chain — diagnostic correctness. `#[from]` isn't used because the wrapping must be explicit (the source is an `Arc<dyn Error>`, not a typed `CredentialAccessError`). Deliberate, correct.

**🟢 RIGHT — `Arc<dyn std::error::Error + Send + Sync>` for wrapped sources.** Enables `Clone` on `ActionError` (which the engine needs for retry state replay). Alternative: `Box<dyn Error>` — would force `ActionError` to be non-`Clone`. Arc-wrapping the source is the correct idiom for Clone-required error types.

**🟢 RIGHT — `ActionError::validation(field, reason, detail)` sanitizes attacker-supplied input.** `sanitize_detail()` at `error.rs:100-116` escapes control characters to `\uXXXX` and caps length. Log-injection protection. Tested at `error.rs:898-913`. `field: &'static str` by design — compile-time constant only. **Reference-quality** input sanitization for an error type.

**🟢 RIGHT — `From<CredentialAccessError>` and `From<CoreError>` preserve error classification.** `error.rs:305-318` and `320-333`: map `AccessDenied` to the typed `SandboxViolation` variant; everything else funnels through `fatal_from`. This preserves the sandbox-violation classification across crate boundaries — a direct `Fatal` wrap would lose the information.

**🟠 DATED — `DisplayError` wrapper at `error.rs:121-145`.** This is a private struct that wraps any `Display + Debug + Send + Sync` into a `dyn Error`. Its `Display` impl just writes `self.message` which was built from `format!("{source}")`. Since Rust 1.81, `std::error::Error` has been `impl<T: Display> From<T> for Box<dyn Error>` for Send+Sync Display types (well, `std::error::Error::from` isn't quite that, but the pattern exists). The `DisplayError` struct exists because `ActionError::retryable/fatal` want to accept `Display` types (not just typed `Error`), and wrapping via a named newtype keeps the `Arc<dyn Error>` field uniform. 🟡 PREFERENCE — the wrapper exists for a reason (uniform Arc-wrapped error field); could be replaced by `anyhow::Error` but nebula-action is a library crate (user memory confirms libs use `thiserror`, not `anyhow`). Current shape is correct; the "DATED" tag is weak.

**🟡 PREFERENCE — `ActionErrorExt<T>` for `Result<T, E>`.** `error.rs:586-631`. Provides `.retryable()? / .fatal()? / .retryable_with_hint(h)? / .fatal_with_hint(h)?` suffix methods. Ergonomic; the DX payoff is real for action bodies that otherwise would chain `.map_err(|e| ActionError::retryable_from(e))?`. The bound is `E: std::error::Error + Send + Sync + 'static` — correct. ✅ Justified — answers the question "is `ActionErrorExt` worth its weight": **yes, the alternative is repetitive map_err chains at every `?` site**.

**🟢 RIGHT — `is_retryable()` + `is_fatal()` + `Cancelled` is neither.** `error.rs:696-700` tests that `Cancelled` returns false for both. This is subtly correct: a cancelled action shouldn't be retried (the user cancelled it) but also isn't a "failure" in the business sense (it just stopped). Most error enums get this wrong and classify `Cancelled` as a failure.

**🟡 PREFERENCE — `RetryHintCode` is 8 variants, all `#[non_exhaustive]`.** Stable enough to be part of the wire format (serialized as string discriminant: `"RateLimited"` not `0`), extensible via `#[non_exhaustive]`. Good. ✅

**🟡 PREFERENCE — `ValidationReason` is 6 variants, `#[non_exhaustive]`, `as_str()` returns stable lowercase ids.** `error.rs:73-86`. Matches the "observability dashboard bucketing" use case from the docs. ✅

### Summary for §7

Error taxonomy is **the cleanest part of the crate idiomatically**. Two-axis hint vs classify split is disciplined; `Arc<dyn Error>` for Clone is correct; input sanitization is reference-quality; `ActionErrorExt` is DX-justified. The `DisplayError` wrapper is a minor curiosity, not a defect. No 🔴 findings in error design.

---

## 8. Top-N idiomatic findings, severity-ranked

Ranked by idiom-currency impact (not business severity; that's Phase 1 security / dx / tech-lead ground).

| # | Severity | Location | Finding | Idiom currency |
|---|---|---|---|---|
| 1 | 🔴 WRONG | `action_attrs.rs:129-134` | Macro emits `.with_parameters(...)` against non-existent method; user attribute `parameters = Ty` produces cryptic compile error on user-code span. | Same bug Phase 0 C2; listed here as the single clearest idiomatic defect. |
| 2 | 🟠 DATED | `stateless.rs:313-322`, `stateful.rs:461-472`, `trigger.rs:328-381`, `resource.rs:83-106` | `*Handler` trait methods use explicit `for<'life0, 'life1, 'a>` HRTB lifetime naming inherited from `async-trait` emission convention. Single `'a` lifetime + type alias (`BoxFut<'a, T>`) would tighten ~30-40% of boilerplate. | Pre-1.85 idiom; 1.95 can use simpler shape without losing dyn-safety. |
| 3 | 🟠 DATED | 5 adapter domains | HRTB boilerplate is repeated on every `impl *Handler for *Adapter`. `ResourceHandler` uses `ResourceConfigureFuture<'a>` / `ResourceCleanupFuture<'a>` aliases (`resource.rs:58-64`) — correct pattern not propagated to sibling handlers. | Inconsistent application of the cleaner pattern the crate already knows. |
| 4 | 🟠 DATED | Whole handler family | `#[trait_variant::make(Handler: Send)]` is the canonical 1.95 story for "async fn in trait + Send + dyn-safety". Crate does not use it. Would collapse the RPITIT trait + HRTB-boxed trait into a single source, generating both. | Phase 3 redesign candidate; clear LOC payoff; don't do mechanically. |
| 5 | 🟠 DATED | `stateless.rs:349-354`, `stateful.rs:502-508` | Adapter instantiation bounds (`A::Input: DeserializeOwned`, `A::Output: Serialize`) are **not** lifted to the typed trait's assoc-type bounds — errors surface at adapter-registration site instead of impl site. | Error-UX shift; fixable by tightening trait assoc bounds or adopting v2-spec's `type Input = Self` default. |
| 6 | 🟠 DATED | Macro emission site, `action_attrs.rs:137-144` | `ActionKey::new(#key).expect(...)` fires at first `metadata()` call, not at compile time. Compile-time-known string should validate in the proc-macro or via `const fn` in `nebula-core`. | Orchestrator flag: ships class of bugs from compile to runtime. |
| 7 | 🟡 PREFERENCE | `error.rs:121-145` | `DisplayError` private newtype wraps `Display + Debug` into `dyn Error` for uniform `Arc<dyn Error>` field. Idiomatic-enough; could arguably use `anyhow::Error` but lib policy says `thiserror`. | Defensible; the tag is weak. |
| 8 | 🟡 PREFERENCE | `context.rs:635-669` `credential<S>()` | Type-name-lowercase heuristic as credential key (Phase 0 C3). Idiomatically offensive even if the contract is "deprecated-on-arrival" — it's a footgun in a type-safe API. | Cross-reference Phase 0; rust-senior classifies as a 🔴 from idiom POV but 🔴 already covered in Phase 0. |
| 9 | 🟡 PREFERENCE | `trigger.rs:182-202` `TriggerEvent::downcast<T>` | Returns `Err(self)` on mismatch rather than `Option<T>` or `Result<T, DowncastError>`. Unusual signature justified by "engine routing bug" framing; caller diagnostic requires `match err.payload_type_name()`. | Defensible design; stylistic tradeoff only. |
| 10 | 🟡 PREFERENCE | `poll.rs:1446-1457` `stop()` | Fires cancellation but doesn't await join handle; caller must store their own. Trait shape prevents structural enforcement. | Architectural, not idiomatic — flag for architect if Phase 3 revisits `TriggerHandler` shape. |

### What is **explicitly NOT** a finding (style-guide-pass)

- RPITIT on typed trait family — correct 1.95 idiom.
- `CancellationToken` usage — textbook.
- `parking_lot::Mutex` + `tokio::sync::Notify` split — right tool for each path.
- `#[non_exhaustive]` on every public enum (Error variants, Result variants, TerminationReason, RetryHintCode, ValidationReason, EmitFailurePolicy, TriggerEventOutcome) — disciplined futureproofing.
- `OnceLock` for `fn metadata(&self) -> &ActionMetadata` — idiomatic, zero-cost after warm-up.
- `Arc<dyn Error>` inside `ActionError` for Clone — correct tradeoff.
- Guards inside `{ … }` blocks or `if let` RHS to drop before `.await` — explicitly and correctly disciplined.
- `forbid(unsafe_code)` — verified clean across 21 source files.

---

*End of Phase 1 idiomatic review. No fixes proposed. Findings forwarded to orchestrator for Phase 2 consensus.*
