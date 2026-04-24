# Phase 0 — nebula-action Code Audit (01a)

**Date:** 2026-04-24
**Author:** rust-senior (sub-agent, audit-only; no fixes proposed)
**Scope:** `crates/action/src/**` (21 files, ~10 k lines), `crates/action/macros/src/**` (3 files), `crates/action/README.md`, reconciled against `docs/superpowers/specs/2026-04-06-action-v2-design.md` and `docs/superpowers/specs/2026-04-24-credential-tech-spec.md` §§2.7 / 3.4 / 7.1 / 15.7.
**Severity legend:** 🔴 CRITICAL · 🟠 MAJOR · 🟡 MINOR.

---

## 1. Module structure

Module cascade under `crates/action/src/` (code paths verified against `lib.rs` lines 37-89):

| File | LOC | Role | Pub surface kind |
|---|---|---|---|
| `action.rs` | 27 | Base `Action` supertrait (`metadata()` only) | Trait |
| `metadata.rs` | 620 | `ActionMetadata`, `ActionCategory`, `IsolationLevel`, `MetadataCompatibilityError`, `for_*<A>` constructors | Data + builders |
| `error.rs` | 1016 | `ActionError`, `RetryHintCode`, `ValidationReason`, `ActionErrorExt` trait, `CredentialRefreshFailed` variant | Enum + trait |
| `result.rs` | 1680 | `ActionResult`, `TerminationReason`, `TerminationCode`, `BreakReason`, `WaitCondition`, `map_output` / `try_map_output` / `into_primary_output` | Enum + helpers |
| `output.rs` | 1437 | `ActionOutput`, `BinaryData`, `DeferredOutput`, `StreamOutput`, `DataReference`, `Producer`, etc. | Enum family |
| `handler.rs` | 386 | `ActionHandler` enum (4 variants: Stateless/Stateful/Trigger/Resource wrapping `Arc<dyn *Handler>`) | Dispatcher enum |
| `stateless.rs` | 597 | `StatelessAction` trait + `StatelessHandler` dyn-safe + `StatelessActionAdapter` + `FnStatelessAction` / `FnStatelessCtxAction` | Trait + handler + 2 adapters |
| `stateful.rs` | 906 | `StatefulAction` + `StatefulHandler` + adapter, plus DX: `PaginatedAction`, `BatchAction` with `impl_paginated_action!`/`impl_batch_action!` macros | Trait + 2 DX macros + handler + adapter |
| `trigger.rs` | 648 | `TriggerAction` + `TriggerHandler` dyn + `TriggerEvent` (type-erased payload via `Box<dyn Any>`) + `TriggerEventOutcome` + `TriggerActionAdapter` | Trait + handler + envelope + adapter |
| `resource.rs` | 317 | `ResourceAction` (single `type Resource` — split removed) + `ResourceHandler` + `ResourceActionAdapter` (boxed `Any` downcast) | Trait + handler + adapter |
| `control.rs` | 945 | `ControlAction` DX trait + `ControlInput` / `ControlOutcome` + `ControlActionAdapter` (erases to `StatelessHandler`, stamps `ActionCategory::Control`/`Terminal`) | Trait + adapter over StatelessHandler |
| `webhook.rs` | 1852 | `WebhookAction` DX + `WebhookTriggerAdapter` + `WebhookRequest` + `WebhookConfig` / `SignaturePolicy` / `RequiredPolicy` / `SignatureScheme` + HMAC SHA-256 primitives (`verify_*`, `hmac_sha256_compute`) | Trait + adapter over TriggerHandler + HTTP types |
| `poll.rs` | 1466 | `PollAction` DX + `PollConfig` + `PollCursor`, `DeduplicatingCursor` + `PollTriggerAdapter` (cancel-safe loop) + `POLL_INTERVAL_FLOOR` (100 ms) | Trait + adapter over TriggerHandler |
| `context.rs` | 683 | Umbrella traits `ActionContext`/`TriggerContext` (marker-style blanket over `HasResources + HasCredentials + HasLogger + HasMetrics + HasEventBus + HasNodeIdentity` for action; adds `HasTriggerScheduling + HasWebhookEndpoint` for trigger); concrete `ActionRuntimeContext` / `TriggerRuntimeContext`; `CredentialContextExt` blanket helpers | Trait hierarchy + runtime ctx structs + ext trait |
| `capability.rs` | 331 | `TriggerScheduler`, `ExecutionEmitter`, `TriggerHealth` (atomics snapshot), `Noop*` fallbacks, `default_*()` accessors | dyn-safe capability traits + fail-closed defaults |
| `port.rs` | 511 | `InputPort`/`OutputPort`/`SupportPort`/`DynamicPort`/`ConnectionFilter`/`FlowKind`/`PortKey` | Enum + data structs |
| `macros.rs` | 204 | Internal crate (`#[macro_export]`) assertion macros: `assert_success!`, `assert_branch!`, `assert_continue!`, `assert_break!`, `assert_skip!`, `assert_wait!`, `assert_retry!` (feature-gated), `assert_retryable!`, `assert_fatal!`, `assert_validation_error!`, `assert_cancelled!` | 11 assertion macros |
| `testing.rs` | 548 | `TestContextBuilder`, `TestActionContext`, `TestTriggerContext`, `StatefulTestHarness`, `TriggerTestHarness`, `SpyEmitter`/`SpyLogger`/`SpyScheduler` | Test harness types |
| `validation.rs` | 198 | `ActionPackageValidationError` + `validate_action_package` (metadata + port-structure checks) | Thin validator |
| `prelude.rs` | 54 | Curated re-export bundle for action authors | Re-export list |
| `lib.rs` | 153 | Module declarations + crate-root `pub use` | Module root |

Macro crate `crates/action/macros/`:

| File | LOC | Role |
|---|---|---|
| `lib.rs` | 53 | `#[proc_macro_derive(Action, attributes(action, nebula))]` entry point |
| `action.rs` | 63 | `derive()` + `expand()` + `validate_struct()`; emits `impl Action` + `impl DeclaresDependencies` + `OnceLock<ActionMetadata>` |
| `action_attrs.rs` | 243 | `ActionAttrs` struct + `parse()` (required: `key`, `name`; optional: `description`, `version`, `credential`/`credentials`, `resource`/`resources`, `parameters`) + `metadata_init_expr()` + `dependencies_impl_expr()` + `parse_version()` |

---

## 2. Public API surface

### Trait family — as actually exposed via `crate::*` re-exports (lib.rs:93–153)

**Base:** `Action: DeclaresDependencies + Send + Sync + 'static` — single method `fn metadata(&self) -> &ActionMetadata`. Marked `#[diagnostic::on_unimplemented]` with hint "derive it: #[derive(Action)]".

**5 primary execution traits:**
- `StatelessAction: Action` — assoc `Input: HasSchema`, `Output`; methods `schema()` (default: `<Input>::schema()`), `execute(input, ctx) -> impl Future<Output = Result<ActionResult<Output>, ActionError>> + Send`. RPITIT form.
- `StatefulAction: Action` — adds `type State: Serialize + DeserializeOwned + Clone + Send + Sync`; methods `init_state()`, `migrate_state(_old)` (defaults `None`), `execute(input, &mut state, ctx)`.
- `TriggerAction: Action` — `start(ctx) -> Result<(), ActionError>` + `stop(ctx) -> Result<(), ActionError>`. (**No `on_event` method** — see §3 drift.)
- `ResourceAction: Action` — single assoc `type Resource: Send + Sync + 'static`; `configure(ctx) -> Self::Resource` + `cleanup(resource, ctx)`. (Split `Config`/`Instance` **removed**, documented inline at resource.rs:28-31.)
- `ControlAction: Action` — `evaluate(input: ControlInput, ctx) -> ControlOutcome`. **DX over StatelessAction** (adapter desugars to `StatelessHandler`).

**4 DX specialization traits (over primary):**
- `PaginatedAction: Action` (over `StatefulAction`) — `fetch_page(input, cursor, ctx) -> PageResult<Output, Cursor>`; activation via `impl_paginated_action!(Ty)` macro (exported as `#[macro_export]` from stateful.rs:170).
- `BatchAction: Action` (over `StatefulAction`) — `extract_items` / `process_item` / `merge_results`; activation via `impl_batch_action!(Ty)`.
- `WebhookAction` (over `TriggerAction`) — lifecycle + HMAC signature policy. **Not a `: Action` supertrait** — it is wrapped by `WebhookTriggerAdapter` which itself implements `TriggerHandler`.
- `PollAction` (over `TriggerAction`) — poll-loop declarative config; `PollTriggerAdapter` wraps.

### Handler enum (lib.rs:103 → handler.rs:41)
```rust
pub enum ActionHandler {
    Stateless(Arc<dyn StatelessHandler>),
    Stateful(Arc<dyn StatefulHandler>),
    Trigger(Arc<dyn TriggerHandler>),
    Resource(Arc<dyn ResourceHandler>),
}
```
`#[non_exhaustive]`, `Clone`, `Debug`, `metadata()`, `is_*()` predicates. **4 variants — not 5** (no `Control` variant; ControlAction erases to `Stateless`).

### Adapter pattern (consistent across domains)
Every typed DX trait has a corresponding `*Adapter<A>` struct in the same domain file:
- `StatelessActionAdapter<A: StatelessAction>` — deserializes JSON → `A::Input`, calls `.execute()`, serializes `Output` via `ActionResult::try_map_output`.
- `StatefulActionAdapter<A: StatefulAction>` — same shape; bridges JSON state → typed state; has state-migration hook.
- `TriggerActionAdapter<A: TriggerAction>` — pure delegation (no I/O transform; `start`/`stop` pass-through).
- `ResourceActionAdapter<A: ResourceAction>` — boxes `A::Resource` to `Box<dyn Any + Send + Sync>`; downcast on cleanup (invariant check; fatal on mismatch).
- `ControlActionAdapter<A: ControlAction>` — wraps to `StatelessHandler`; stamps `ActionCategory::Control` or `Terminal` based on output-port count; caches metadata in `Arc<ActionMetadata>`.
- `WebhookTriggerAdapter<A: WebhookAction>` — wraps to `TriggerHandler`; holds `RwLock<Option<Arc<State>>>`; handles double-start rejection; downcasts `TriggerEvent` payload to `WebhookRequest`.
- `PollTriggerAdapter<A: PollAction>` — wraps to `TriggerHandler`; runs cancel-safe loop inside `start()`; uses `POLL_INTERVAL_FLOOR = 100 ms`.

**Dyn-safety:** all four `*Handler` traits (Stateless/Stateful/Trigger/Resource) use **explicit HRTB lifetime syntax** (`for<'life0, 'life1, 'a>` ... `Pin<Box<dyn Future<...> + Send + 'a>>`) rather than `async fn` in trait. This is the pre-RPITIT-dyn dispatch pattern — used because `dyn *Handler` is stored as `Arc<dyn *Handler>` in the registry. 🟡 MINOR (§3 below for full analysis).

### `#[derive(Action)]` macro surface
- Attribute: `#[action(...)]` container attribute only (no field-level `#[action]`).
- Supported keys: `key` (req), `name` (req), `description` (optional; falls back to doc-comment via `utils::doc_string`, else `name`), `version` (optional; defaults `"1.0"`; accepts `X.Y` or full semver via `parse_version`), `parameters = Type`, `credential = Type` / `credentials = [T1, T2]`, `resource = Type` / `resources = [R1, R2]`.
- String-variant `credential = "key"` silently ignored (typed only); documented as such in `#[derive(Action)]` docs.
- Generates: `impl DeclaresDependencies { fn dependencies() }` building `Dependencies::new().credential(...).resource(...)` via `CredentialLike::KEY_STR` / `ResourceLike::KEY_STR` per type; and `impl Action { fn metadata() }` returning a `OnceLock<ActionMetadata>`.

### Crate-wide attributes
- `#![forbid(unsafe_code)]` (lib.rs:34) — verified clean, 0 unsafe blocks in `src/`.
- `#![warn(missing_docs)]` (lib.rs:35).

---

## 3. Trait hierarchy — actual vs declared vs canon §3.5

### Canon §3.5 invariant (verbatim, canon.md:82)
> "Action — what a step does. Dispatch via action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`). Adding a trait requires canon revision (§0.2)."

### lib.rs docstring declares **10** trait surfaces (lines 13-20)
1. `Action` (base)
2. `StatelessAction`
3. `StatefulAction`
4. `TriggerAction`
5. `ResourceAction`
6. `PaginatedAction` ("DX over `StatefulAction`")
7. `BatchAction` ("DX over `StatefulAction`")
8. `WebhookAction` ("DX over `TriggerAction`")
9. `PollAction` ("DX over `TriggerAction`")
10. `ControlAction` ("flow-control nodes")

### Canon §3.5 names **4** primary traits. Actual code matches **4** primary + `ControlAction` as a fifth DX specialization. Plus 4 DX-over-primary traits.

🟠 **MAJOR — canon §3.5 / §0.2 drift risk.** `ControlAction` is documented (control.rs:393-431) as "**public and non-sealed** — community plugin crates may implement it directly". Community-facing dispatch-time public trait outside the canon-enumerated 4 is a literal invariant violation per the §3.5 text "adding a trait requires canon revision". The `ActionHandler` enum (handler.rs:41) has 4 variants and ControlAction adapts to `Stateless` — so from engine-dispatch POV the invariant is preserved (no 5th `ActionHandler` variant), but the public trait surface exposed at the `nebula_action::ControlAction` re-export IS a fifth dispatch-time public trait. No canon revision lives anywhere in `docs/` for it.

🟠 **MAJOR — lib.rs line 11 wording collision.** "Canon §3.5 (trait family; adding a trait requires canon revision)" is the header of a module whose DX-family public API (10 traits) already exceeds canon's 4. The invariant as written is already violated on day 1 unless "trait" means "something that shows up as a variant in `ActionHandler`" — which canon does not clarify.

🟡 MINOR — `TriggerAction` has no `on_event` method (trigger.rs:61-73 shows only `start`/`stop`). The v2 design spec example at line 192 shows `async fn on_event(&self, event: Value, ctx)`. Actual event handling goes via `TriggerHandler::handle_event` (trigger.rs:373-389) with a `default impl` returning Fatal, and `accepts_events()` sentinel at line 359. The typed `TriggerAction` trait never sees events — only adapters do. This is a deliberate **decoupling** post-v2-spec, not a code bug, but docs should admit it.

🟡 MINOR — `AgentAction` from v2 spec §1 Core Type Hierarchy is **absent from code** (grep returns only a doc mention in `metadata.rs:32` listing `AgentAction` alongside other traits in a comment). `ActionCategory::Agent` enum variant exists (metadata.rs:61) but no trait. This is cleaner than v2 spec — not a drift, a documented simplification.

### Dyn-compatibility discipline
Every execution trait has a dyn-safe parallel:
| Typed trait | Dyn-safe handler trait | Adapter |
|---|---|---|
| `StatelessAction` | `StatelessHandler` | `StatelessActionAdapter` |
| `StatefulAction` | `StatefulHandler` | `StatefulActionAdapter` |
| `TriggerAction` | `TriggerHandler` | `TriggerActionAdapter` |
| `ResourceAction` | `ResourceHandler` | `ResourceActionAdapter` |
| `ControlAction` | (erases to `StatelessHandler`) | `ControlActionAdapter` |
| `PaginatedAction` | (activates via macro → `StatefulAction`) | (none direct) |
| `BatchAction` | (activates via macro → `StatefulAction`) | (none direct) |
| `WebhookAction` | (erases to `TriggerHandler`) | `WebhookTriggerAdapter` |
| `PollAction` | (erases to `TriggerHandler`) | `PollTriggerAdapter` |

🟡 MINOR — `*Handler` traits use explicit HRTB lifetime boilerplate (`for<'life0, 'life1, 'a> ... Pin<Box<...>>`) rather than `async fn`. This is the historically-correct way to get dyn-safe async-trait methods under Rust 1.95, but it's verbose. Consider whether `trait_variant::make` or return-type-notation could tighten this post-Phase 1.

---

## 4. `#[action]` macro emission contract

### What the macro actually emits (verified in macros/src/action.rs:39-50 + action_attrs.rs:121-197)

For:
```rust
#[derive(Action)]
#[action(key = "slack.send", name = "Send Slack", description = "...", version = "2.1", credential = SlackOAuthCredential)]
pub struct SlackSendAction;
```

the macro emits:
```rust
impl ::nebula_core::DeclaresDependencies for SlackSendAction {
    fn dependencies() -> ::nebula_core::Dependencies {
        ::nebula_core::Dependencies::new()
            .credential(::nebula_core::CredentialRequirement::new(
                <SlackOAuthCredential as ::nebula_core::CredentialLike>::KEY_STR,
                ::std::any::TypeId::of::<SlackOAuthCredential>(),
                ::std::any::type_name::<SlackOAuthCredential>(),
            ))
    }
}

impl ::nebula_action::Action for SlackSendAction {
    fn metadata(&self) -> &::nebula_action::metadata::ActionMetadata {
        static METADATA: ::std::sync::OnceLock<...> = OnceLock::new();
        METADATA.get_or_init(|| {
            ::nebula_action::metadata::ActionMetadata::new(
                ::nebula_core::ActionKey::new("slack.send").expect("..."),
                "Send Slack", "...",
            )
            .with_version_full(::semver::Version::new(2, 1, 0))
        })
    }
}
```

### Observations

🔴 **CRITICAL — broken `parameters = Type` attribute.** `action_attrs.rs:129-134` emits `.with_parameters(<#ty>::parameters())` but `ActionMetadata` has **no `with_parameters` method** (searchable: `grep with_parameters crates/action` only returns the macro site itself). The metadata builder surface is `with_schema(schema: ValidSchema)` (metadata.rs:292). Any user passing `parameters = HttpConfig` will hit a cryptic "method not found" error on the generated code, pointing at user-code span not macro span. The `parameters` attribute is parsed (action_attrs.rs:56) but not usable. No workspace caller exercises it (grep for `#[action(...parameters` returns only macro internals + 1 unrelated `engine.rs:6502` line).

🔴 **CRITICAL — NO silent phantom rewrite.** Credential Tech Spec §2.7 declares: "`#[action]` macro **rewrites silently** `CredentialRef<dyn BitbucketBearer>` → `CredentialRef<dyn BitbucketBearerPhantom>` in generated code". Actual `#[derive(Action)]` macro does **zero** field-type rewriting — `derive()` only walks container attributes. `CredentialRef<C>` is not imported anywhere in `nebula-action` or `nebula-credential` (grep confirmed: 0 files). The §2.7 shape does not exist in the current crate.

🔴 **CRITICAL — NO slot-binding emission.** Credential Tech Spec §3.4 step 2 (line 848-865) expects `#[action]` macro to emit `impl ActionSlots for Foo { fn credential_slots() -> &[SlotBinding] { ... resolve_fn: resolve_bearer_slot, ... } }`. Actual macro emits `DeclaresDependencies` which lists `CredentialRequirement { key, type_id, type_name }` — a flat string-keyed list with no `resolve_fn` pointer, no slot metadata, no HRTB function pointer. Tech Spec §7.1 HRTB `for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx CredentialId) -> BoxFuture<'ctx, Result<RefreshOutcome, RefreshError>>` has no code analogue in the action crate at all.

🟠 **MAJOR — string-variant attribute silently ignored.** Per `action_attrs.rs:58,61` (`get_type_skip_string`), `credential = "bearer_secret"` as a string literal is silently accepted and **ignored**. This is the v2-design-spec's Example 1 syntax (design.md:61 `#[action(credential = "bearer_secret")]`). So any code following the v2 design example will compile but produce an action with zero credential dependencies declared. No warning, no error.

🟠 **MAJOR — no `optional` credential support.** v2 design spec §3 (line 219-222) mandates `#[action(credential(optional) = "signing_key")]`. Macro's `ActionAttrs::parse` has no `optional` subkey handling. The `CredentialRequirement::optional()` builder exists on `nebula-core` (dependencies.rs:137) but the macro never emits `.optional()`.

🟠 **MAJOR — no `#[action(...)]` in attribute-macro position.** The spec is ambiguous: credential Tech Spec §2.7 says `#[action]` (attribute macro); action v2 spec §2 says `#[derive(Action)]` (derive). The CODE implements ONLY the derive. An attribute macro `#[action]` (not `#[derive(Action)]`) would be necessary for type-rewriting field types — which a derive cannot do. If Tech Spec §2.7 intended an attribute macro, it does not yet exist.

🟡 MINOR — `#[nebula]` attribute registered alongside `#[action]` (lib.rs:50 `attributes(action, nebula)`) but no code branch handles it. Presumably forward reservation; flag for cleanup.

🟡 MINOR — `version` defaults to `"1.0"` which becomes `Version::new(1, 0, 0)`. Matches `BaseMetadata` default. OK.

🟡 MINOR — `metadata_init_expr` uses `OnceLock<ActionMetadata>` (action.rs:46) which makes `metadata()` allocation-free after first call. Good. But the `OnceLock` is per-impl (keyed by struct type); across process re-init (plugin reload) it does not reset. Fine in practice.

🟡 MINOR — ports are never emitted by the derive macro. Authors who want custom ports must construct `ActionMetadata` manually. v2 design spec promises "Metadata generated from `#[action(...)]` attrs" (design.md:372) — the actual macro emits *base metadata only* (key/name/description/version/deps) and never touches ports. Undocumented gap.

---

## 5. Credential integration vs credential Tech Spec §§2.7 / 3.4 / 7.1 / 15.7

Every dimension of current `nebula-action` credential integration diverges from the CP6-frozen Tech Spec. Summary matrix:

| Tech Spec artifact | §§ | Expected shape | Actual shape in action crate | Status |
|---|---|---|---|---|
| `CredentialRef<C>` typed handle | §2.9 | Phantom-parameterized, sized or dyn | **Absent.** Credentials are `CredentialSnapshot`, resolved by string key. | 🔴 missing |
| Phantom-rewrite via `#[action]` macro | §2.7 | Silent `dyn Bearer` → `dyn BearerPhantom` | **Absent.** Derive macro does no field-type rewriting. | 🔴 missing |
| `AnyCredential` object-safe supertrait | §2.8 | Engine holds `Box<dyn AnyCredential>` with `type_id_marker()` | **Absent** from action side. `CredentialAccessor::resolve_any -> Box<dyn Any + Send + Sync>` is what action code consumes. | 🔴 missing |
| `SlotBinding` + `resolve_fn` HRTB fn pointer | §3.4 | Macro emits `ActionSlots` impl with `for<'ctx> fn(...) -> BoxFuture<...>` | **Absent.** Macro emits flat `Dependencies::new().credential(CredentialRequirement { key, type_id, type_name })` — no resolve function pointer at all. | 🔴 missing |
| `resolve_as_bearer<C>` where-clause resolution | §3.4 step 3 | Engine-owned capability-specific helper with `where C: Credential<Scheme = BearerScheme>` | **Absent.** No capability-specific resolvers anywhere in action. Credential Tech Spec assumes this lives in engine. | not in scope (engine side) |
| Action body receives `&Scheme` not `&dyn Phantom` | §3.4 step 4 | Engine reflects; action body sees `&BearerScheme` | Action body goes through `ctx.credential_typed::<S>(id)` (context.rs:597-625) returning `S: AuthScheme + 'a` via `CredentialSnapshot::into_project::<S>()` | partial — no capability checking at compile time |
| `RefreshDispatcher::refresh_fn: for<'ctx> fn(...) -> BoxFuture<...>` | §7.1 | Per-credential-type refresh fn-pointer registered at registration | **Absent** from action crate. | not in scope (engine) |
| `SchemeGuard<'a, C>` — `!Clone`, `ZeroizeOnDrop`, `Deref` | §15.7 | Owned, lifetime-bound, passed to `Resource::on_credential_refresh` | **Absent** from action crate. Actions still see `CredentialGuard<S>` (from `nebula-credential`), not `SchemeGuard`. | 🔴 missing (but partial substitute exists) |
| `SchemeFactory<C>` companion | §15.7 | `Arc<dyn Fn() -> BoxFuture<'static, ...>>` for re-acquisition in long-lived resources | **Absent.** | 🔴 missing |

### What actually exists in action code for credentials

`CredentialContextExt` (context.rs:567-683) — blanket over any `HasCredentials`:
1. `credential_by_id(id: &str) -> Future<CredentialSnapshot>` — untyped
2. `credential_typed<S: AuthScheme>(id: &str) -> Future<S>` — typed projection by string key
3. `credential<S: AuthScheme + Zeroize>() -> Future<CredentialGuard<S>>` — **type-name-lowercase-as-key heuristic** (context.rs:637-643)
4. `has_credential_id(id: &str) -> Future<bool>`

🔴 **CRITICAL — type-name-lowercase-as-key fallback (context.rs:637-643).**
```rust
let type_name = std::any::type_name::<S>();
let short_name = type_name.rsplit("::").next().unwrap_or(type_name);
let key_str = short_name.to_lowercase();
let key = CredentialKey::new(&key_str).map_err(...)?;
```
This is exactly the anti-pattern Tech Spec §2.7 is built to remove. Naming a credential `MySlackCred` at the user site silently resolves to key `"myslackcred"`. Collision is trivial. Debug-compiled type names can include generic bits, module paths, etc. — the `rsplit("::")` heuristic hides surprises.

### Credential-related `ActionError` integration

- `ActionError::CredentialRefreshFailed { action_key, source }` (error.rs:254-265) — `retryable` default, `Classify::code() = "ACTION:CREDENTIAL_REFRESH_FAILED"`, Clone-via-Arc source. ✅ Solid.
- `From<CredentialAccessError>` (error.rs:305-318) — maps `AccessDenied` to `SandboxViolation`, else `fatal_from`. ✅ Solid.
- `From<CoreError>` (error.rs:320-333) — mirrors above. ✅.

### Summary: the action crate is **~3 API revisions behind** the CP6-frozen credential Tech Spec

- Current action code thinks in "credentials are keyed snapshots, project to `AuthScheme`".
- Tech Spec thinks in "capability-phantom-typed `CredentialRef<C>` at declaration, HRTB-fn-pointer-dispatched slot bindings at resolution, `SchemeGuard` RAII at handoff".
- No bridging layer exists. Any implementation of Tech Spec §§2.7/3.4/7.1/15.7 will require trait-level changes to `Action`, macro-level changes (likely a new `#[action]` attribute macro or phantom rewriting in `#[derive]`), and a new runtime path in `ActionContext`.

🔴 **CRITICAL blocker for cascade decision** — user policy states "Credential Tech Spec frozen at CP5 — action cascade cannot require credential spec revision". Every `SchemeGuard`/`SchemeFactory`/`CredentialRef`/`AnyCredential`/phantom-rewrite mechanism assumes shapes that do not exist in `nebula-action` today. The gap is **structurally load-bearing**: the action redesign either:
  1. Adopts new trait shapes (phantom-CredentialRef, slot bindings, HRTB resolve) — large blast radius, spans macro crate + handler family + context extension trait.
  2. Deprecates the `CredentialContextExt::credential<S>()` type-name heuristic (🔴 path today) without adopting the full Tech Spec vocabulary — leaves action stuck between idioms.
  3. Negotiates with credential tech-lead on whether Tech Spec §§2.7/3.4/15.7 can be partially deferred for action scaffolding.

Phase 1 / Phase 3 decision question — tag for orchestrator consensus.

---

## 6. Resource integration vs `2026-04-06-resource-v2-design.md`

Cross-check against resource v2 design is **partially in scope** (resource design is older, not frozen). Key observations about current `ResourceAction` vs usage patterns:

- `ResourceAction` has a single `type Resource` (resource.rs:32). Documented cleanup comment (resource.rs:28-31) says: earlier `Config`/`Instance` split was removed because the adapter boxed `Config` and downcast to `Instance`, losing safety. Zero impls ever used distinct types. 🟢 sensible consolidation — matches resource v2 direction of "one typed resource per ResourceAction".
- `ResourceHandler` (resource.rs:69-102) uses `Box<dyn Any + Send + Sync>` on configure/cleanup. Downcast invariant enforced with fatal error on mismatch (resource.rs:186-196). 🟢 OK.
- `nebula-action` depends on `nebula-resource` (Cargo.toml:29) but the only use in `src/` is... none that I can find via grep. Let me double-check: the `ResourceAccessor` trait comes from `nebula-core` (lib.rs:108 `pub use nebula_core::accessor::ResourceAccessor`). So the `nebula-resource` crate dep is unused at source level. 🟡 MINOR — potentially dead dep, flag for dx-tester / devops.

🟡 MINOR — `Resource::on_credential_refresh` hook from credential Tech Spec §15.7 / resource v2 design does NOT exist in `nebula-action::ResourceAction`. The action-side `ResourceAction` has no refresh hook at all (just `configure`/`cleanup`). If the cascade needs this, it's new trait API.

---

## 7. Canon invariant drift

### §3.5 — trait family
See §3 above. 🟠 `ControlAction` is an implicit fifth public trait by canon's strict reading.

### §11.2 — engine-level retry
- `ActionResult::Retry { after, reason }` gated behind `unstable-retry-scheduler` feature (result.rs:188-196). Default-off verified in Cargo.toml:15.
- `is_retry()` predicate is always-available and always returns false when feature disabled (result.rs:628-638). Clever. ✅ 🟢 canon compliance.
- Docs marker on variant (result.rs:169-187) is explicit about "Unstable. Reserved for a future engine retry scheduler."
- `ActionError` does not expose any retry-variant-related surface outside the feature flag. ✅.
- `assert_retry!` macro also gated (macros.rs:134-144). ✅.

🟢 **§11.2 compliance looks clean end-to-end** — probably the crate's strongest canon discipline.

### §11.3 — idempotency
- Not enforced at trait level. `ActionError::Retryable` and `CredentialRefreshFailed` are the only hooks. Per README line 84, "For non-idempotent or risky side effects ... action handlers must guard execution with the engine idempotency key path before calling the remote system. See `crates/execution/src/idempotency.rs`." — this is a cross-crate contract, not enforceable inside nebula-action.
- 🟡 MINOR — no test utility in `testing.rs` for idempotency-key guarding. Add if cascade plans to elevate §11.3 enforcement.

### §12.6 — isolation honesty
- `IsolationLevel::None | CapabilityGated | Isolated` (metadata.rs:13-24). Docs explicitly call out WASM non-goal (metadata.rs:21-23, lib.rs:9).
- Execution drivers (`ProcessSandbox`, OS hardening) are in `nebula-sandbox`, not here. ✅ separation.

### §4.5 — operational honesty
- `ActionResult::Retry` is gated. ✅ canon.
- **🟠 MAJOR** — `ActionResult::Terminate` (result.rs:220-223) docs (lines 206-218) admit: "Full scheduler integration ... is tracked as Phase 3 of the ControlAction plan and is **not yet wired**. Do not rely on `Terminate` in v1 to cancel sibling branches; it only gates the local subgraph downstream of the terminating node." This is a partially-implemented capability behind a public variant — it is precisely the "false capability" pattern §4.5 forbids. The variant ships without a feature flag. Whether to gate it (mirror `Retry`) is a cascade decision.
- `TerminationCode` (result.rs:277) is documented to swap backing to `ErrorCode` in "Phase 10 of the action-v2 roadmap" — a roadmap phase that does not exist in any live plan document as of this audit's repo snapshot. 🟠 MAJOR — drift anchor.

### §3.5-adjacent: canon's own §3.5 update process (§0.2 canon revision requirement)
No `docs/adr/` entry exists for `ControlAction`. 🟠 MAJOR — either canon should be revised to include 5 primary trait surfaces, or `ControlAction` should be more clearly demoted to "not-a-trait-family-member" (it's currently positioned as a first-class DX trait).

---

## 8. Doc vs code diff — LOAD-BEARING

Diff between `docs/superpowers/specs/2026-04-06-action-v2-design.md` (2.5 weeks old) and current code at `crates/action/src/`. Tracked at spec-section granularity.

### v2 spec §1 — 5 core traits
- Spec says: `StatelessAction, StatefulAction, TriggerAction, ResourceAction, AgentAction`.
- Code has: `StatelessAction, StatefulAction, TriggerAction, ResourceAction` + 5 DX traits (`ControlAction`, `PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`).
- **AgentAction is absent** from code. `ActionCategory::Agent` enum variant exists (metadata.rs:61). No trait, no handler, no adapter. 🟠 partial removal — docs at `lib.rs:13-20` list 10 traits (including ControlAction), not 5.
- 🟠 **The "5 traits, no extras" principle from v2 spec §Philosophy is not reflected in the code.** Code ships 10 surfaces. ControlAction alone is 50+ % of the drift.

### v2 spec §2 — Derive macro boilerplate-only
- Spec says: `#[derive(Action, Parameters, Deserialize)]` where `Parameters` is a companion derive and struct IS the input.
- Code has: `#[derive(Action)]` from `nebula-action-macros`. No `Parameters` derive in action crate (would live in nebula-schema — not checked here).
- `type Input = Self` is permitted but not enforced. Action authors can still split input out. 🟡 MINOR — partial DX drift.
- **Spec Example (design.md:79-89)** calls `ctx.credential::<BearerSecret>("bearer_secret")` — actual API is `ctx.credential_typed::<BearerSecret>("bearer_secret")` (context.rs:597). The v2-spec shape `credential<S>(key)` (design.md:211) maps to `credential_typed<S>(id)` in code, but another `credential<S>()` (no key; type-name-as-key heuristic) exists at context.rs:629. 🟠 **Two competing credential-access methods.** `credential_opt<S>(key)` from spec (design.md:214) has **no code analogue**.

### v2 spec §3 — Credential access always keyed
- Spec: `ctx.credential::<S>(key)` + `ctx.credential_opt::<S>(key)`.
- Code: `credential_typed::<S>(id)` (exists) + `credential_by_id(id)` (untyped) + `credential::<S>()` (type-name heuristic, no key). 🟠 DRIFT — 3 method variants, none of them the spec's 2-method pair.
- Spec: `#[action(credential(optional) = "signing_key")]`. Code: optional annotation not supported in macro parser. 🟠 DRIFT.

### v2 spec §4 — Resource access typed
- Spec: `ctx.resource::<R>(key) -> Lease` + `resource_opt<R>(key)`.
- Code: `ActionRuntimeContext::resource(key: &str) -> Box<dyn Any + Send + Sync>` (context.rs:228-235). 🟠 DRIFT — **untyped**, just a `Box<dyn Any>` downcast site.

### v2 spec §5 — Handler layer (InternalHandler)
- Spec: single `InternalHandler` trait with JSON in / `ActionResult<Value>` out.
- Code: **5 handler traits** (`StatelessHandler`, `StatefulHandler`, `TriggerHandler`, `ResourceHandler`, plus ControlAction erases to `StatelessHandler`). ActionHandler enum dispatches by variant (handler.rs:41). 🟡 divergence but **sensibly richer** — code is more mature.

### v2 spec §6 — ActionRegistry version-aware
- Spec: `VersionedActionKey`, `get(key, version)`, `get_latest(key)`.
- Code: 🔴 **registry is not in the action crate.** Docs point at `nebula_runtime::ActionRegistry` (handler.rs:5-7). Out of scope for this audit but flag: can't evaluate against v2 spec §6 without reading runtime.

### v2 spec §7 — Test fixtures
- Spec: `TestContextBuilder`, `SpyLogger`, `assert_success!`/`assert_branch!`/`assert_continue!` macros.
- Code: all present (testing.rs + macros.rs). ✅ **strong v2 alignment.**

### v2 spec §8 — Port system + DataTag
- Spec: "Flow / Support / Provide / Dynamic — unchanged." DataTag: 58+ hierarchical tags.
- Code: `port.rs` has `Flow` / `Support` / `Dynamic` (no `Provide`). 🟠 DRIFT — `Provide` port kind not implemented. "DataTag registry: 58+ hierarchical tags" — the only tag-related code in action is `ConnectionFilter::allowed_tags: Vec<String>` (port.rs:~). No hierarchical registry. 🟠 DRIFT — DataTag is totally absent.

### v2 spec §10 — What changes vs current
- Spec table promised: "Metadata generated from `#[action(...)]` attrs". Actually generated: key/name/description/version only (not ports, not parameters — macro's `with_parameters` branch is broken). 🟠 DRIFT.
- "No extra traits (removed Execute, SimpleAction, TransformAction)": ✅ these are indeed absent. But **5 new DX traits were added** (ControlAction, PaginatedAction, BatchAction, WebhookAction, PollAction). Net trait count increased, not decreased. 🟠 DRIFT.

### v2 spec §11 — Not in scope
- Spec: InteractiveAction, TransactionalAction, StreamingAction, ProcessAction, QueueAction, CachePolicy, Task<T>.
- Code confirms these are absent. `TransactionalAction` was removed on 2026-04-10 (M1) with rationale in stateful.rs:377-391. ✅ **confirms spec-vs-code alignment.**

### v2 spec post-conference amendments
- B1 (durable IdempotencyManager): cross-crate, not evaluable here.
- B2 (compensating txns = author responsibility): ✅ stateful.rs:385-391 echoes this.
- B3 (serde_json recursion limit at adapter boundary): 🔴 **NOT IMPLEMENTED** — grep for `set_max_nesting` / `recursion_limit` / `recursion` in `crates/action/src`: zero matches. `StatelessActionAdapter::execute` (stateless.rs:356-383) calls `serde_json::from_value` with no depth cap. Attack surface real.
- B4 (streaming `BlobStorage::write`): output.rs types exist but streaming impl is cross-crate; not evaluable here.
- B6 (`action_version` pinning on `NodeDefinition`): cross-crate.
- B8 (StatefulAction cancel safety docs): stateful.rs trait doc does mention cancellation (stateful.rs:33 — "Cancellation is enforced by the runtime (same as StatelessAction)"), but does NOT include the mandated "perform state mutations atomically — update local copy, then assign to `*state` at the end, not field-by-field" language from B8. 🟠 DRIFT.
- B9 (manual registration example without proc macros): no such example in `nebula-action` docs (README or lib.rs). 🟡 MINOR.
- B7 (CostMetrics on ActionResult — v1.1): not present, not expected in v1. ✅.
- B5 (derive macro semver contract): not documented anywhere. 🟡 MINOR.

### README vs code diff
- README `Public API` section (lines 22-78) is **consistent** with the code (after verification). README makes the `CheckpointPolicy: planned` status explicit at line 87. Actionable.
- README line 109: "DX specializations (`PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`) are implemented and tested" — matches code. ControlAction conspicuously **omitted** from this list despite being re-exported. 🟡 MINOR — README under-inventories.
- README line 116: `(siblings) nebula-resource (ResourceAction, ResourceAccessor)`. `ResourceAction` lives in nebula-action (`resource.rs`) — not in nebula-resource. 🟠 MAJOR doc drift.
- README missing altogether: `ControlInput`/`ControlOutcome`/`ControlActionAdapter`. 🟡 MINOR.

---

## 9. v1/v2 vestige flags

**Historical markers still live in code:**

1. 🟡 `result.rs:169-187` — `ActionResult::Retry` variant gated behind `unstable-retry-scheduler`. Intentional gate; not a vestige. Correctly documented.
2. 🟡 `result.rs:207-219` — `ActionResult::Terminate` docs admit "Phase 3 of the ControlAction plan is **not yet wired**." Partial-implementation vestige; unclear whether Phase 3 lives on any current plan.
3. 🟡 `result.rs:248 + 260` — Phase 10 roadmap for `TerminationCode` swap to structured `ErrorCode`. Forward-reference to work not in any live plan.
4. 🟠 `stateful.rs:377-391` — `TransactionalAction` + `TransactionPhase` + `TransactionState` + `impl_transactional_action!` **removed on 2026-04-10 (M1)**. Clean removal — comment explains why (3-phase saga state machine was unreachable past first phase). Not a vestige; good archaeology note.
5. 🟠 `output.rs:1213` — `version: Some("v1".into())` — literal string `"v1"` inside code. Investigate context (did not pull surrounding lines but worth flagging).
6. 🟠 `poll.rs:10` — "Cross-restart persistence requires runtime storage integration (post-v1)." Known gap.
7. 🟠 `webhook.rs:585` — "responsibility (post-v1)." Context not read; flag.
8. 🟢 `context.rs:6-11` — "Spec 23/27 make [`ActionContext`] and [`TriggerContext`] **umbrella marker traits**." References "Spec 28" relocation (context.rs:139-141). These internal spec numbers are not spec documents in `docs/superpowers/specs/` — they appear to be in-crate convention. Audit-note: spec numbering ambiguous.
9. 🟢 `context.rs:136-141` — "Lives in `nebula-action` as the canonical runtime context. Spec 28 schedules a physical relocation to `nebula-engine::context`..." Planned move; track during cascade.

**No v1-vs-v2 dual trait duplicates exist** — no old `SimpleAction`, `Execute`, `TransformAction` residuals (confirmed absent). The drift pattern is not "v1 vs v2 coexistence" but "spec promises vs code reality" — see §8.

---

## 10. TODO/FIXME/placeholder/unimplemented indicators

- 🟡 `stateful.rs:117` — `todo!()` inside a **doc-comment rustdoc example** (`/// impl PaginatedAction for ListRepos { ... fetch_page(...) { todo!() }`). Not production code; documentation artifact. Low priority.
- **No production `todo!()` / `unimplemented!()`** anywhere in `crates/action/src/` (verified via grep).
- **No `FIXME` comments** (verified via grep).
- **No `placeholder` comments** (verified via grep).

### §11.2 "planned" markers (canon-honest)

- `Cargo.toml:15-20` — `unstable-retry-scheduler` feature flag docs explicitly call out: "Currently a planned capability without persisted attempt accounting (canon §11.2). The engine does not honor the variant end-to-end."
- `result.rs:169-196` — `Retry` variant doc echoes this: "The engine does **not** honor this variant end-to-end today ... there is no persisted attempt accounting, no CAS-protected counter bump, and no consumer wired through `ExecutionRepo`."
- **The feature gate is honored correctly end-to-end** — `Retry` variant hidden, `is_retry()` is unification-safe, `assert_retry!` macro also gated, `compile_fail` doctest verifies variant is unnameable in default builds (result.rs:46-52).
- 🟢 This is the **healthiest** discipline in the whole crate. Model for other partial capabilities.

### Other partial-implementation surfaces

- `Terminate` variant (result.rs:220-223) — not gated. Ships with "do not rely on this to cancel sibling branches" warning. Inconsistent discipline vs `Retry`.
- `PollAction` cross-restart cursor persistence (poll.rs:10) — "post-v1." Not gated.
- `WebhookAction` (webhook.rs:585 area) — "post-v1." Not gated.

---

## 11. Ground truth summary — executive top 10

> **Read this if you read nothing else.** Top 10 findings ranked by cascade-blocking severity.

1. 🔴 **Credential Tech Spec §§2.7 / 3.4 / 7.1 / 15.7 shapes are entirely absent from `nebula-action`.** `CredentialRef<C>`, phantom rewriting, slot bindings, HRTB `resolve_fn` pointers, `SchemeGuard<'a, C>`, `SchemeFactory<C>` — none exist. Action credential integration is "string-keyed `CredentialSnapshot` → project to `AuthScheme`", a paradigm the Tech Spec supersedes. Since the user policy says "Tech Spec frozen at CP5 — action cascade cannot require credential spec revision", Phase 3 must either (a) adopt the entire Tech Spec vocabulary on the action side (large blast radius) or (b) negotiate scope split with tech-lead. **Cascade-blocking decision.**

2. 🔴 **`#[derive(Action)]` macro has a broken `parameters = Type` path.** Emits `.with_parameters(<Type>::parameters())` but `ActionMetadata` only has `.with_schema(schema)` — no `with_parameters` method anywhere. Any user trying the documented `parameters = HttpConfig` attribute gets a cryptic compile error. Zero workspace callers exercise it today — masked but poisonous.

3. 🔴 **`CredentialContextExt::credential<S>()` uses a type-name-lowercase heuristic as the credential key** (context.rs:637-643). `type_name::<S>().rsplit("::").next().to_lowercase()` as the lookup key is a footgun. Collisions trivial. Violates "always keyed" principle from v2 spec §3.

4. 🔴 **No serde_json recursion limit at `StatelessActionAdapter` deserialization boundary** (v2 spec B3 amendment mandates default 128). `StatelessActionAdapter::execute` calls `serde_json::from_value` with no depth cap. Attack surface real — stack overflow via deeply-nested attacker input.

5. 🟠 **Canon §3.5 / §0.2 invariant drift via `ControlAction`.** `ControlAction` is a public, non-sealed, DX-mapped-to-Stateless 5th trait surface exposed at the crate root (`pub use crate::control::ControlAction`). Canon enumerates 4 traits and requires "canon revision" to add one. No ADR exists. Lib.rs docstring line 11 self-contradicts: "trait family; adding a trait requires canon revision" while re-exporting 10 trait surfaces.

6. 🟠 **v2 design spec §3 (`credential_opt`) + §4 (typed `resource<R>(key) -> Lease`) — zero code correspondence.** Only untyped `resource(key) -> Box<dyn Any>` exists. v2 spec promised typed, keyed resource access; code has not tracked. Either cascade needs spec update or impl update.

7. 🟠 **Five DX specialization traits (`ControlAction`, `PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`) vs v2 design spec's "5 traits, no extras" principle.** The design principle has been violated. The DX layer is genuinely useful (and well-tested) but it needs canon sign-off. Flag for architect redesign.

8. 🟠 **`ActionResult::Terminate` is a partially-implemented public variant without feature gating** (result.rs:207-219). Docs admit it only gates local subgraph; Phase 3 scheduler integration "not yet wired". By canon §4.5 ("public surface exists iff engine honors it end-to-end"), this is a false capability candidate. Compare vs the correctly-gated `Retry` variant.

9. 🟠 **Macro emits `DeclaresDependencies::dependencies()` with flat `CredentialRequirement { key: CredentialLike::KEY_STR, type_id, type_name }` — NOT the `ActionSlots` / `SlotBinding` / `resolve_fn` shape** the credential Tech Spec §3.4 step 2 expects. No HRTB function pointer, no per-slot resolver, no compile-time phantom-bound check. Any credential-side Phase 3 work that depends on slot metadata will find it absent.

10. 🟠 **README drift.** README line 116 claims `ResourceAction` lives in `nebula-resource`; it lives in `nebula-action::resource`. Plus 5 smaller doc-vs-code gaps (see §8). Under-inventories `ControlAction`. Drift is fixable in one PR but signals coordination gaps between crate README and implementation.

### Signal strength for cascade

- **Strong signal of canon discipline:** `unstable-retry-scheduler` feature gate handling for `ActionResult::Retry` is disciplined end-to-end.
- **Strong signal of rough edges:** Credential integration layer and derive macro have quiet-but-compounding drift from the Tech Spec and v2 design; neither is catastrophic in isolation, but together they signal a crate that evolved incrementally past its own spec docs.
- **Structural ambiguity:** The "5 traits, no extras" philosophy in v2 spec vs the actual 10-trait DX-rich API is a **design-intent gap** that Phase 1 (pain enumeration) and Phase 3 (proposal) must resolve before macro / credential integration redesign.

---

*End of Phase 0 audit. No fixes proposed. Orchestrator consolidates with Devops workspace audit for Phase 1 pain enumeration.*
