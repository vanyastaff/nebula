# Phase 1 — DX authoring pain report (nebula-action)

**Date:** 2026-04-24
**Agent:** dx-tester (newcomer persona — Rust 3+ yr, zero nebula-action knowledge)
**Inputs read before authoring:**
- `crates/action/README.md`
- `crates/action/src/lib.rs` module-level doc comment + re-export block
- `docs/superpowers/specs/2026-04-06-action-v2-design.md`
- Phase 0 critical findings C1-C4, S4 (to calibrate what to probe)
**Inputs consulted when stuck:**
- `crates/action/src/stateless.rs` (trait definition — for HasSchema discovery)
- `crates/action/src/stateful.rs` (ActionResult variant names)
- `crates/action/src/context.rs` (credential helper signatures)
- `crates/action/src/resource.rs` (ResourceAction associated types)
- `crates/action/macros/src/action_attrs.rs` (to understand silent string-drop)
- `crates/credential/src/lib.rs` (find SecretToken / AuthScheme)
- `crates/core/src/dependencies.rs` (CredentialLike / ResourceLike trait shape)

**Scratch files (all build-green unless noted):**
- `.scratch/a1_stateless.rs`
- `.scratch/a1b_parameters.rs` (C2 repro — intentionally broken)
- `.scratch/a2_stateful.rs`
- `.scratch/a3_resource_cred.rs`
- `.scratch/a3b_optional_cred.rs` (spec amend #3 repro — intentionally broken)

---

## 0. Methodology

**Isolation.** Worktree at `C:\Users\vanya\RustroverProjects\nebula\.claude\worktrees\agent-ab9576e3`. Separate `.scratch/` subdirectory with its own `[workspace]` Cargo.toml — detached from the main nebula workspace so builds don't pollute the parent and clippy settings don't leak across.

**Starting knowledge.** Zero. README + v2 spec + `lib.rs` doc. No Nebula internal docs, no AGENT_PROTOCOL, no prior memory of action internals used to shortcut API discovery.

**Time budget.** 90-minute equivalent. Actual elapsed clock time ~55 min given the repeated compile waits (first full build of the dep graph took ~120 s; subsequent incremental builds <2 s). A human newcomer on cold build caches would experience the 2-minute first-compile wall as significant friction by itself.

**Measurement unit for "docs/source lookup count".** One lookup = one navigation to a file outside `README.md` + `lib.rs` + v2 spec. Rereading README/lib.rs/spec sections doesn't count. Reading inside the same source file to find the next thing does count as one (a single hop = one lookup; multiple lookups in one file = one lookup). This is the "how far away is the answer from the docs the README directs me at" metric.

---

## 1. Action type 1: StatelessAction (HTTP GET)

**Goal.** Struct with url field; output {status, body}. Use `#[derive(Action)]`.

**Time to first successful compile.** ~12 min.

**LOC (final, green).** 37 total; 29 business-logic-independent boilerplate (import list, derive attrs, trait impl skeleton, stub main). **Net business logic: ~8 LOC.**

**Docs/source lookup count.** 4.
1. `stateless.rs` — discover `fn execute(&self, input, ctx: &(impl ActionContext + ?Sized))` signature (spec example shows `ctx: &ActionContext` which is a trait, not a concrete type)
2. `has_schema.rs` — discover what `HasSchema` is (v2 spec doesn't mention it; README mentions `ValidSchema parameters` but not the `Input: HasSchema` bound)
3. `lib.rs` re-exports — figure out that `Action` refers to the trait (not the derive) because both are re-exported at root under the same name — `pub use action::Action;` AND `pub use nebula_action_macros::Action;` (🟠 collision shadow)
4. `action_attrs.rs` — to understand the `::semver::Version` emission once the initial `cannot find semver` error surfaced

### Pain points — Action 1

**🔴 BLOCKING-at-first-compile #1 — `#[derive(Action)]` emits `::semver::Version::new(...)` but `semver` is not a re-export of `nebula-action`.**
First compile error: `error[E0433]: cannot find 'semver' in the crate root`. Newcomer has no idea why: the only thing written was `#[derive(Action)]`. README gives the derive as the happy path, zero mention that the consuming crate must add `semver = "1"` as a direct dep. Compare to `serde_derive` / `thiserror` — both re-export the crate root symbols they depend on via `$crate` paths, so the caller never needs a transitive dep.
*Evidence:* `action_attrs.rs:141-142` emits `::semver::Version::new(#major, #minor, #patch)` — the `::` absolute path bypasses action's own re-export and demands the user crate have `semver` in Cargo.toml.
*Severity:* 🔴 **blocking** for the strict first-minute newcomer; 🟠 major once you know (fix is 10 s).

**🔴 BLOCKING-at-first-compile #2 — `Input: HasSchema` bound is mandatory but undocumented in the README / v2 spec.**
First trait-bound error: `the trait bound 'HttpGetInput: nebula_schema::has_schema::HasSchema' is not satisfied`. The v2 spec example writes `type Input = Self;` without explaining that `Self` must derive some schema trait. The README (§Public API) names `ValidSchema parameters` but doesn't say "and `Input` must impl `HasSchema`." The only way forward — discovered in `has_schema.rs` lines 82-84 — is either (a) use `serde_json::Value` / a primitive, (b) derive `#[derive(Schema)]` from `nebula-schema-macros`, or (c) hand-write `impl HasSchema`. The README does not re-export `Schema` (the parameter-derive) at all from action; you have to discover nebula-schema independently.
*Severity:* 🔴 — the spec's "struct IS the input" pattern won't compile without a bound that's missing from the canonical example.

**🟠 MAJOR — `Action` (trait) and `Action` (derive macro) share the same name at the crate root.**
`lib.rs:93` re-exports `pub use action::Action;` (trait) and `lib.rs:111` re-exports `pub use nebula_action_macros::Action;` (derive). Both named `Action`. This works because traits and derives live in different namespaces, but `use nebula_action::Action;` is ambiguous to the reader: is it the trait or the derive? Newcomer has no way to know which to import without reading the lib.rs source (re-exports collapse to one line).
*Severity:* 🟠 — compiles fine, but hurts discoverability; every other nebula crate I've seen separates these (`Action` trait + `ActionDerive`, or derives behind a `macros` submodule).

**🟠 MAJOR — v2 spec `ctx.input_data()` does not exist.**
Spec §2 If/Switch example line 111: `let input_data = ctx.input_data();`. Grep across action crate for `input_data` — zero matches. The spec canonical example for the If branch trait is non-compiling against today's code.
*Severity:* 🟠 for Stateless-with-branch use case.

**🟡 MINOR — `ctx: &(impl ActionContext + ?Sized)` signature is verbose.**
The trait-object-aware bound is the right call but it is 25 chars of boilerplate every time vs. `ctx: &ActionContext` in the v2 spec example. Not blocking — just jarring as the very first thing a newcomer types after `#[derive(Action)]`.

---

## 2. Action type 2: StatefulAction (PaginatedAction — GitHub issues)

**Goal.** Paginate a list; cursor-based. Declare `State` type explicitly.

**Time to first successful compile.** ~8 min (because Action 1 knowledge transferred).

**LOC (final, green).** 58 total; 40 boilerplate (imports + struct defs + attr). **Net business logic: ~18 LOC.**

**Docs/source lookup count.** 3.
1. `stateful.rs:38-78` — trait shape (`type State: Serialize + DeserializeOwned + Clone + Send + Sync`; `init_state`; mutable `&mut state` in `execute`). The trait doc is actually quite good.
2. `result.rs` — discover `ActionResult::continue_with(output, progress)` / `ActionResult::break_completed(output)`. **The v2 spec §2 uses `ActionResult::r#continue(...)` / `ActionResult::break_completed(...)` — one matches code, the other does not.** I wrote `r#continue` first, got "method not found", had to grep `result.rs`.
3. `stateful.rs:80+` — skip `PaginatedAction` DX trait (spec instruction asked for StatefulAction, and PaginatedAction requires an extra `impl_paginated_action!` macro call which is not in the re-exports of the prelude — you have to know the name).

### Pain points — Action 2

**🟠 MAJOR — v2 spec name `ActionResult::r#continue` does not exist in code. Actual name is `ActionResult::continue_with`.**
Quote from spec (line 172): `ActionResult::r#continue(response.data, Some(state.pages_fetched as f64 / self.max_pages as f64))`. Actual API in `result.rs:542`: `pub fn continue_with(output: T, progress: Option<f64>) -> Self`. This is drift S4 as Phase 0 predicted — the spec pair doesn't match any single code constructor.
*Severity:* 🟠 — you immediately hit this writing the spec example verbatim.

**🟠 MAJOR — `PaginatedAction` DX trait is re-exported but its activation macro is not.**
`lib.rs:131-132` re-exports `PaginatedAction`, `PaginationState`, etc. To actually activate a `PaginatedAction` as a `StatefulAction`, you need `nebula_action::impl_paginated_action!(ListRepos)` — the macro is not in the `lib.rs` re-export block nor in `prelude.rs`. If you read README + lib.rs you do not learn this. You read the doctest in `stateful.rs:120` to find it.
*Severity:* 🟠 — a DX specialization whose activation contract is hidden from the public docs makes the specialization feel undocumented.

**🟡 MINOR — `type Input` is duplicated between `Action` struct (container fields via derive) and `StatefulAction::Input` (the dispatch input).**
The v2 spec's resolution — "struct IS the input" (`type Input = Self`) — is one path, but not forced by the trait. Authors end up writing `type Input = Value;` and passing the struct fields plus the unrelated input Value around, which is the "two inputs" antipattern. Unclear from the docs which is the preferred idiom.

---

## 3. Action type 3: ResourceAction + Credential (stub Postgres pool)

**Goal.** Acquire a Postgres pool configured from a credential; later execute a query; clean up. Exercises action + credential + resource crates.

**Time to first successful compile.** ~32 min — **by far the hardest**. Three independent blockers had to be understood and bypassed (see below), each requiring source-reading into a new crate.

**LOC (final, green).** 64 total; 46 boilerplate (imports + stub PgPool + struct + attr + trait impl skeleton). **Net business logic: ~6 LOC** (and I never actually exercised the credential, only resolved it — because the v2 spec's `ctx.credential::<S>(key)` signature does not exist).

**Docs/source lookup count.** 8.
1. `resource.rs` — discover trait has `type Resource; configure; cleanup` (README had this)
2. `context.rs:563-689` — discover what `CredentialContextExt` actually offers. Three methods: `credential_by_id`, `credential_typed<S>(id)`, `credential<S>()` — **none of them is the v2-spec-mandated `credential::<S>(key)` pair**.
3. `secrets/guard.rs` — discover `CredentialGuard<S: Zeroize>` bound when `ctx.credential::<SecretToken>().await?` exploded with "trait `zeroize::DefaultIsZeroes` is not implemented for `SecretToken`"
4. `credential/src/lib.rs` — realise `AuthScheme` is imported at crate root **both as a trait AND as a derive macro** (line 145: derive, line 158: trait). Glob-import conflict. Have to use explicit paths.
5. `credential/src/scheme/secret_token.rs` — find the canonical "bearer secret" type — the v2 spec calls it `BearerSecret` but code calls it `SecretToken`.
6. `action/macros/src/action_attrs.rs:56-62` — understand why `#[action(credential = "SecretToken")]` silently declared ZERO credentials (`get_type_skip_string`: string literal → return None).
7. `core/src/dependencies.rs:144` — see `CredentialLike::KEY_STR: &'static str` — then confirm zero implementations in the workspace.
8. `credential/macros/src/lib.rs` — confirm `#[derive(Credential)]` does NOT emit `CredentialLike`, so no clean path to the type-form `#[action(credential = MyCredential)]` either.

### Pain points — Action 3

**🔴 BLOCKING #1 — `ctx.credential::<S>(key)` as documented in v2 spec §3 does not exist.**
v2 spec §3 promise (line 211):
```rust
pub fn credential<S: AuthScheme>(&self, key: &str) -> Result<S, ActionError>;
pub fn credential_opt<S: AuthScheme>(&self, key: &str) -> Result<Option<S>, ActionError>;
```
Actual `CredentialContextExt` (`context.rs:573-686`) offers three unrelated methods:
- `credential_by_id(id) -> Result<CredentialSnapshot, _>` — untyped, no projection.
- `credential_typed<S: AuthScheme>(id) -> Result<S, _>` — typed + keyed, **but not named `credential`**.
- `credential<S: AuthScheme + Zeroize>() -> Result<CredentialGuard<S>, _>` — **takes no key**; derives the key from `std::any::type_name::<S>().rsplit("::").next().to_lowercase()`. Adds extra `Zeroize` bound that built-in schemes don't satisfy.
- `credential_opt` — does not exist. No optional variant of any kind.
The "keyed + typed" spec signature is not a single method call against today's code. Every newcomer following the spec verbatim writes `ctx.credential::<BearerSecret>("bearer_secret")` — none of the three methods has that exact shape (wrong name, wrong arg count, wrong bound).
*Severity:* 🔴 **blocking for the core credential use case.**

**🔴 BLOCKING #2 — `ctx.credential::<SecretToken>()` (type-only form) fails with a cryptic bound error.**
Attempted: `let guard = ctx.credential::<SecretToken>().await?;`. Compile error walls 64 lines, leading line: `the trait bound 'nebula_credential::SecretToken: zeroize::DefaultIsZeroes' is not satisfied`. Novice has no idea why a credential method needs a `zeroize` bound — and the built-in scheme types (the one a newcomer grabs from `nebula_credential::*`) do NOT satisfy it. (Confirmed in `credential/scheme/secret_token.rs` — no `#[derive(Zeroize)]`, no `impl Zeroize` in file.) So the type-only variant is structurally unusable with built-in schemes.
*Severity:* 🔴 — documents an API that can't be called with the code's own shipped types.

**🔴 BLOCKING #3 — Derive-macro string-form attribute silently drops `credential = "name"` declarations with zero diagnostic.**
Wrote `#[action(credential = "SecretToken")]`. Compile succeeded. Runtime check (`PgResource::dependencies().credentials().len()`) returned **0**. The string-form attribute parses cleanly via `get_type_skip_string` (`sdk/macros-support/src/attrs.rs:166-175`), deliberately returns `None`, and silently emits `Dependencies::new()` without the credential. **No warning, no error, no doc.** The v2 spec example line `#[action(credential = "bearer_secret")]` (line 62 and line 128) is silently non-functional.
*Severity:* 🔴 — correct-looking code produces silently-broken dependencies. This is a security-relevant footgun: an action that looks like it declares "I need this credential" passes registration with zero declared deps, and at dispatch time the lookup fails only when the runtime tries to actually use it.

**🔴 BLOCKING #4 — Derive-macro type-form attribute (`credential = SecretToken`) requires `CredentialLike` trait with ZERO implementors workspace-wide.**
Switched to `#[action(credential = SecretToken)]`. Compile error: `the trait 'CredentialLike' is not implemented for 'SecretToken'`. Grep for `impl CredentialLike` in the entire workspace: **zero matches** (just the trait definition in `nebula-core` and the emission site in `action/macros/src/action_attrs.rs:164-167`). `#[derive(Credential)]` on a user's wrapper does not emit `CredentialLike`. Confirmed by grep against `crates/credential/macros/src/`.
Conclusion: **neither path (string form nor type form) of `#[action(credential = …)]` works with any type shipped by the credential crate.** You have to hand-write `impl CredentialLike for MyNewtype { const KEY_STR: &'static str = "…" }` yourself — and the macro gives you no hint.
*Severity:* 🔴 — the advertised credential declaration attribute has zero working consumers.

**🔴 BLOCKING #5 — v2 spec amendment §3 `credential(optional) = "key"` fails to parse.**
Wrote `#[action(credential(optional) = "signing_key")]`. Compile error: `error: expected ,`. The spec-documented optional-credential syntax is not accepted by the attribute parser. See `.scratch/a3b_optional_cred.rs`.
*Severity:* 🔴 — optional credentials, as specified, cannot be declared.

**🟠 MAJOR — `AuthScheme` is imported at `nebula_credential::*` twice (trait + derive).**
```rust
pub use nebula_credential_macros::{AuthScheme, Credential};  // line 145 (derive)
pub use scheme::{AuthPattern, AuthScheme, ...};              // line 157-160 (trait)
```
Glob-importing the credential crate gives you one of the two (shadowed). Explicit imports work (Rust namespaces are separate for derives + traits) but the collision is confusing — you'd want to call them `AuthScheme` (trait) and `AuthSchemeDerive` or `AuthSchemeMacro`.
*Severity:* 🟠 — compiles if you know; opaque if you don't.

**🟠 MAJOR — README § Public API line 63 says `CredentialContextExt — credential resolution from context.` — but does not indicate which method to use among the three, and none matches the v2 spec.**
Newcomer who reads the README cannot predict which method to call. Doc comment on `credential<S>()` (`context.rs:633`) says "Retrieve a typed credential by `AuthScheme` type. Returns a zeroizing `CredentialGuard<S>`." No mention that this method uses type-name lowercase as the lookup key; no mention that `Zeroize` is required beyond the trait bound itself.
*Severity:* 🟠 — naming collision + no decision guidance.

**🟡 MINOR — `BearerSecret` (v2 spec name) = `SecretToken` (code name).**
The spec freely uses `BearerSecret` across §2 and §7 examples. Code calls the same thing `SecretToken`. Minor rename-blocker for anyone following the spec verbatim.

---

## 4. Cross-cutting pain patterns (hit across ≥2 action types)

### CC1 — `#[derive(Action)]` emits `::semver::Version` path unconditionally.
Every action type hits this on first compile. Not nebula-crate-owned — user's Cargo.toml must have `semver` directly. 🔴 first-time, 🟢 after you've been stung once.

### CC2 — `Input: HasSchema` bound is inherited by Stateless + Stateful + Paginated traits; undocumented in README.
All three traits (`StatelessAction::Input`, `StatefulAction::Input`, `PaginatedAction::Input`) carry `nebula_schema::HasSchema`. None of the README examples write out what to derive to satisfy it. Newcomer either uses `serde_json::Value` (forever untyped) or reads into nebula-schema to find `#[derive(Schema)]`. 🟠.

### CC3 — v2 spec signatures do not match code across two methods (`ctx.credential`, `ActionResult::continue`).
Both are one-off naming drift. 🟠.

### CC4 — The 10 re-exported action trait surfaces in `lib.rs:13-20` force authors to spend 5+ minutes figuring out which one to implement.
README says "4 core traits" but lib.rs exports 10 trait surfaces (including DX specializations). Newcomer has to eyeball the list and guess which to pick (StatefulAction? PaginatedAction? BatchAction?). For Action 2 I initially planned to use `PaginatedAction`; switched to `StatefulAction` when I noticed PaginatedAction required the `impl_paginated_action!` macro step that's not re-exported. Net: I read three separate trait modules before committing. 🟠.

### CC5 — Every action author needs the `Debug` derive to print metadata.
Not supplied by `#[derive(Action)]`. Minor ergonomic nag. 🟡.

---

## 5. Macro expansion friction

**Ran `cargo expand --bin a1_stateless` successfully.** Expansion is clean, not pathological — small, understandable, one module per trait impl. The expansion quality itself is OK. The *diagnostics* when the attribute parser or the emitted code mismatches are the problem:

1. **C2 confirmed (`parameters = Type`)** — wrote `#[action(..., parameters = HttpParams)]` (`.scratch/a1b_parameters.rs`). Error points at the derive site:
   ```
   error[E0599]: no method named `with_parameters` found for struct `ActionMetadata`
     --> a1b_parameters.rs:15:17
      |
   15 | #[derive(Debug, Action)]
      |                 ^^^^^^ method not found in `ActionMetadata`
      |
      = note: this error originates in the derive macro `Action`
   ```
   Error is truthful but unlocalized — it points at the derive, not at the attribute line that caused the emission. Newcomer has no way to know which attribute triggered the emission. The derive's error message says `method not found`; the real bug is "this attribute is documented but was never connected to a real ActionMetadata builder method." That's an emitter-contract mismatch that should be a macro-time `compile_error!`, not a downstream type-check failure.

2. **Silent drop on string-form credential** — no diagnostic at all. Author writes `#[action(credential = "name")]`, which is the canonical spec example, and it silently compiles to `Dependencies::new()`. No warning, no compile_error! on ambiguity. This is the worst category: code that looks correct and is wrong.

3. **`credential(optional) = "key"` (spec-documented syntax) fails at attribute tokenizer level** with `expected ,`. Would benefit from a macro-level "did you mean `optional_credential = \"key\"`?" diagnostic, if that's the intended form — but right now the syntax is simply undefined.

**No trybuild/macrotest harness exists** (Phase 0 finding T1), so the emitter cannot regression-test these diagnostic-quality issues. Any new diagnostic added has no golden-file guard against regression.

---

## 6. Top-N authoring friction findings, severity-ranked

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 1 | 🔴 BLOCKING | `#[action(credential = "string")]` silently declares ZERO credentials — the canonical v2-spec example is non-functional and produces no diagnostic. | `action_attrs.rs:58` → `get_type_skip_string` at `sdk/macros-support/src/attrs.rs:166-175`; runtime probe in `.scratch/a3_resource_cred.rs` prints `declared credentials: 0` |
| 2 | 🔴 BLOCKING | `#[action(credential = Type)]` type-form fails because `CredentialLike` has **zero implementors** across the entire workspace. Neither path to declarative credential deps works. | grep `impl CredentialLike` returns 0 hits in `crates/`; `credential/macros/src/lib.rs` does not emit it |
| 3 | 🔴 BLOCKING | v2-spec-mandated `ctx.credential::<S>(key)` + `ctx.credential_opt::<S>(key)` signatures do not exist on `CredentialContextExt`. Code offers 3 unrelated methods with different names, args, and bounds. | `context.rs:573-686` vs. spec §3 line 211-214 |
| 4 | 🔴 BLOCKING | `#[action(credential(optional) = "key")]` fails to parse (`expected ,`). The v2 spec §3 optional-credential syntax is undefined. | `.scratch/a3b_optional_cred.rs` first compile |
| 5 | 🔴 BLOCKING | `#[derive(Action)]` emits `::semver::Version::new(...)` but action does not re-export semver. Every user crate must add `semver` to its own Cargo.toml. | `action_attrs.rs:141`; first compile of any `#[derive(Action)]` caller fails with `E0433: cannot find 'semver'` |
| 6 | 🔴 BLOCKING | `#[derive(Action)]` `parameters = Type` attribute is documented + parsed but emits a method call against `ActionMetadata::with_parameters` which does not exist. Phase 0 finding C2 is reproducible. | `.scratch/a1b_parameters.rs` → `E0599: no method named 'with_parameters'`; `action_attrs.rs:129-134` vs. `metadata::ActionMetadata` API |
| 7 | 🔴 BLOCKING | `Input: HasSchema` bound on `StatelessAction`/`StatefulAction`/`PaginatedAction` is not documented in README or v2 spec. Canonical spec example `type Input = Self;` does not compile for a user struct without extra `#[derive(Schema)]` or hand-rolled `impl HasSchema`. | `stateless.rs:78` trait bound; spec §2 line 80 example |
| 8 | 🟠 MAJOR | `ctx.credential::<S>()` (no-key variant) requires `S: Zeroize`, which the built-in `SecretToken` / `SecretString`-based schemes do not satisfy. The method is unusable against the crate's own shipped types. | `guard.rs:35` bound + `scheme/secret_token.rs` (no Zeroize impl); compile error walls |
| 9 | 🟠 MAJOR | `CredentialContextExt::credential<S>()` uses `type_name::<S>().rsplit("::").next().unwrap_or(…).to_lowercase()` as the lookup key (Phase 0 C3). Two credential types with the same short name silently collide at runtime. | `context.rs:643-645` |
| 10 | 🟠 MAJOR | `ActionResult::r#continue(...)` (v2 spec name) does not exist. Actual name is `continue_with(output, progress)`. Following spec verbatim produces "method not found". | `result.rs:542` vs. spec §2 line 172 |
| 11 | 🟠 MAJOR | `lib.rs` re-exports 10 trait surfaces (4 core + 6 DX); README says "4 core". Newcomer spends ≥5 min picking the right trait. DX specializations require non-re-exported macros (`impl_paginated_action!`) to activate. | `lib.rs:93-154`; `stateful.rs:120` doctest |
| 12 | 🟠 MAJOR | `nebula_credential::AuthScheme` is exported twice from the crate root — as a derive (`nebula_credential_macros::AuthScheme`, line 145) and as a trait (`scheme::AuthScheme`, line 158). Glob import collapses to one; explicit import works but is opaque. | `credential/src/lib.rs:145` vs `:157-160` |
| 13 | 🟠 MAJOR | `nebula_action::Action` is exported both as the trait (`lib.rs:93`) and as the derive (`lib.rs:111`) under the same name. Both compile fine; newcomer cannot see from a bare `use nebula_action::Action;` which namespace they imported. | `action/src/lib.rs:93, 111` |
| 14 | 🟠 MAJOR | v2 spec §2 example `ctx.input_data()` method does not exist on any context trait. Grep for `input_data` in action crate: 0 matches. Stateless-with-Branch example is non-compiling. | spec §2 line 111; grep |
| 15 | 🟠 MAJOR | Built-in scheme type name `SecretToken` ≠ v2 spec name `BearerSecret`. Following the spec literally fails to import. | `scheme/secret_token.rs:22` vs. spec §2 line 85 |
| 16 | 🟡 MINOR | `ctx: &(impl ActionContext + ?Sized)` signature is verbose but necessary; v2 spec elides the `+ ?Sized` and `(impl …)` wrapping. | `stateless.rs:101` |
| 17 | 🟡 MINOR | `ResourceAction` trait documents `type Resource` but the underlying handler (`ResourceHandler`) erases it to `Box<dyn Any + Send + Sync>`. Composing with `ctx.resource::<R>(key)` from v2 spec §4 (which does not exist today) would close this — right now you get typed configure but untyped access from downstream nodes. | `resource.rs:38, 60`; spec §4 |
| 18 | 🟡 MINOR | `impl_paginated_action!` activation macro is not in `lib.rs` re-exports nor `prelude.rs`. Only discoverable through the `PaginatedAction` trait doctest. | `stateful.rs:119-120` |
| 19 | 🟡 MINOR | No `#[derive(Action)]` auto-impl of `Debug`; every author writes `#[derive(Debug, Action)]` just to print metadata in tests. | any scratch example |

---

## Summary measurements

| Metric | Action 1 | Action 2 | Action 3 | Target |
|---|---|---|---|---|
| Time to first green compile | ~12 min | ~8 min | ~32 min | <5 min |
| LOC boilerplate | 29 | 40 | 46 | — |
| LOC business logic | ~8 | ~18 | ~6 | — |
| Docs/source lookup count | 4 | 3 | 8 | ideally 0-1 |
| First-compile errors seen | 2 | 0 | 3 (sequential) | 0-1 |
| Spec examples that fail verbatim | 1 (Input) | 1 (`r#continue`) | 4 (`credential`, `credential_opt`, `BearerSecret`, `credential(optional)`) | 0 |

**Verdict:** 👎 for `nebula-action` authoring DX as of 2026-04-24, dominated by the credential-integration surface (Action 3 alone accounts for 8 of the 12 🔴 / 🟠 findings). The core stateless + stateful traits are individually workable once you adapt to the undocumented `HasSchema` bound and the `semver` re-export gap, but the declarative credential attribute system is structurally unusable and the v2 spec examples touching credentials do not compile.

---

## Files referenced (absolute paths)

- Scratch workspace: `C:\Users\vanya\RustroverProjects\nebula\.claude\worktrees\agent-ab9576e3\.scratch\`
- `a1_stateless.rs`, `a1b_parameters.rs` (C2 repro), `a2_stateful.rs`, `a3_resource_cred.rs`, `a3b_optional_cred.rs` (optional-credential repro), `Cargo.toml`

*End of Phase 1 dx-tester authoring report. Orchestrator consolidates with security-lead / rust-senior / tech-lead into `02-pain-enumeration.md`.*
