# ADR-0052 P3 — HasSchema convergence (Action/Credential ISP fold) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Converge `Action` and `Credential` onto the clean `Resource` HasSchema pattern — delete the redundant `Action::input_schema`/`output_schema` and `Credential::properties_schema` trait methods, add a free `nebula_schema::schema_of::<T: HasSchema>()` helper, migrate every in-tree consumer to the associated-type / `schema_of` form.

**Architecture:** The `type Input/Output/Properties: HasSchema` associated-type bound is already the single source of truth; the schema methods are pure ISP redundancy (verified: every body is `OnceLock` wrapping `<T as HasSchema>::schema()`, zero custom overrides, zero production callers of the `Action` methods). Deletion is behaviorally lossless. `HasSchema` stays in `nebula-schema` (Core); zero new crates; zero `deny.toml` change. `!`-breaking trait-surface change (crates are `frontier`/`partial`, pre-1.0 — canon-legal, same justification as P1/P2). L2 trait-surface change ⇒ ADR-0052 P3 amendment + seam test in the same PR (canon §0.1/§17), plus a truthful forward-pointer in ADR-0043 §4 (which defined these methods).

**Tech Stack:** Rust 1.95 (edition 2024), `cargo nextest`, `trybuild` compile-fail, `task`/lefthook gate (full-workspace `clippy -D warnings` per commit), `cargo deny`.

---

## Authority & verified facts (read before any step — do NOT trust the design spec's stale line numbers)

Source spec: `docs/superpowers/specs/2026-05-15-nebula-schema-finalization-design.md` §"HasSchema convergence" + §"Phasing" P3. Spec file:line refs are PRE-#671/PRE-P2 — every signature below was re-verified against `main`@001e9022.

- **`HasSchema`** — `crates/schema/src/has_schema.rs:20`: `pub trait HasSchema { fn schema() -> ValidSchema; }` (owned, Arc-backed). Re-exported at `crates/schema/src/lib.rs:219`: `pub use has_schema::{HasSchema, HasSelectOptions};`. **No `schema_of` exists** (P2 did not add it).
- **ADR-0061** (`docs/adr/0061-nebula-schema-core-ratification.md`) ratifies exactly this shape: `fn schema() -> ValidSchema // owned (not &'static)`, "Caller may cache via `OnceLock` if performance matters", and **defers** any object-safe `HasSchemaObject` companion (YAGNI). `schema_of` is a free fn, NOT a trait method — do not add an object-safe variant.
- **`#[derive(Schema)]`** (`crates/schema/macros/src/derive_schema.rs:70-107`) caches internally behind `static __CACHE: OnceLock<ValidSchema>` and returns `.clone()` (cheap Arc clone) — so `<T as HasSchema>::schema()` is already O(1) for derived types. Deleting the redundant `OnceLock` in `input_schema()`/`properties_schema()` introduces NO per-call rebuild.
- **`Action`** — `crates/action/src/action.rs:65`: `pub trait Action: Sized + Send + Sync + 'static` (NOT object-safe; `dyn Action` does not compile — engine dispatch uses `ErasedAction`/`ActionFactory` per ADR-0043 §7). `type Input: HasSchema + DeserializeOwned + Send + Sync` (67), `type Output: HasSchema + Serialize + Send + Sync` (70). **Delete** `fn input_schema() -> &'static ValidSchema;` (76) and `fn output_schema() -> &'static ValidSchema;` (79) — both **required** methods.
- **Zero production callers of `Action::input_schema()`/`output_schema()`.** Verified: the only `[:.]input_schema()`/`[:.]output_schema()` invocations in `crates/**/src` are none; the real schema-consumer path is `ActionMetadata::for_stateless::<A>()` / `for_stateful` / `for_trigger` / `for_resource` (`crates/action/src/metadata.rs:195,210,225,240`) which **already** calls `<A::Input as nebula_schema::HasSchema>::schema()` directly. Action is already converged at the consumer; P3 = pure dead-surface deletion.
- **`Credential`** — `crates/credential/src/contract/credential.rs:119`: `pub trait Credential: Send + Sync + 'static`; `type Properties: nebula_schema::HasSchema + Send + Sync + 'static` (133). **Delete** the provided method `fn properties_schema() -> ValidSchema where Self: Sized { <Self::Properties as nebula_schema::HasSchema>::schema() }` (158-163). All `Credential` methods are `where Self: Sized` (outside any vtable) — removing one cannot affect object safety.
- **`Credential::properties_schema()` real callers (3 production):**
  - `crates/credential/src/metadata.rs:71` — `BaseMetadata::new(key, name, description, C::properties_schema())` inside `CredentialMetadata::for_credential<C: Credential>`.
  - `crates/credential/src/no_credential.rs:76` — `.schema(Self::properties_schema())` (here `Self::Properties = ()`).
  - `crates/credential/macros/src/credential.rs:477` — `.schema(Self::properties_schema())` in the `#[derive(Credential)]` `has_extras` builder branch.
- **`#[derive(Action)]` macro** (`crates/action/macros/src/action.rs:83-97`) emits `fn input_schema`/`fn output_schema` `OnceLock` boilerplate — delete those two blocks; keep `type Input`/`type Output`.
- **`nebula-sdk` `simple_action!` macro** (`crates/sdk/src/lib.rs:261-275`) emits the same two blocks — delete them.
- **Hand-written `impl Action` blocks** carrying the two redundant methods (all `S.get_or_init(<X as HasSchema>::schema)` — no custom logic): production `crates/action/src/{resource.rs:213, stateful.rs:647 & 776, trigger/mod.rs:613, webhook/providers/stripe.rs:76, webhook/providers/slack.rs:78, webhook/providers/generic.rs:120}`; `#[cfg(test)]` in-crate fixtures `crates/action/src/{control.rs:772 & 821, stateless.rs:219}`; `#[cfg(test)]` engine fixtures `crates/engine/src/engine.rs` (×11), `crates/engine/src/runtime/runtime.rs` (×7), `crates/engine/src/runtime/registry.rs` (×1); plus ~150 occurrences across `crates/{action,api,engine,credential}/tests/**`.
- **ADR-0043 §4** (`docs/adr/0043-dependency-declaration-dx.md`, `status: accepted`) literally defines the trait with `fn input_schema() -> &'static ValidSchema; // = Self::Input::schema()` — P3 modifies this; the ADR-0052 P3 amendment must forward-reference it and ADR-0043 §4 gets a truthful pointer so canon does not lie.
- **deny.toml / layering:** `HasSchema` already in `nebula-schema` (Core), already imported by `nebula-action`/`nebula-credential`/`nebula-resource` (Business). Adding a free fn to an existing module changes no layer edge. **No `deny.toml` change.**
- **NOT in scope (P4, separate phase):** API write-path validation V2, catalog `json_schema()` V3, public OpenAPI DTO `x-nebula-root-rules` strip, ADR-0047 amendment. Proof-token pipeline (INTEGRATION_MODEL §29/§33) untouched — P3 is trait-surface only.

### Open verification items the implementer MUST confirm against source before the step that needs them (name confirmations, not design gaps)

1. **MATURITY.md location.** Determine whether the repo tracks its own `docs/MATURITY.md` or only the external L1 `C:\Users\vanya\RustroverProjects\docs\MATURITY.md`. Run `git -C <worktree> ls-files docs/MATURITY.md`. Update whichever copy is git-tracked in THIS repo (Task 4 Step). If only external/untracked, record that in the PR body instead of editing an untracked file.
2. **Credential lib re-export surface.** `rg -n "pub use" crates/credential/src/lib.rs` — confirm how/whether `nebula-schema` symbols are re-exported, to place `pub use nebula_schema::schema_of;` consistently (Task 1 Step 6).
3. **Credential macro non-extras path.** Read `crates/credential/macros/src/credential.rs:485-515` — confirm the non-`has_extras` branch delegates to `CredentialMetadata::for_credential::<Self>()` (so fixing `for_credential` internally covers it and only the `has_extras` branch at :477 needs an inline edit).
4. **trybuild infra.** Confirm the compile-fail harness pattern used by P2 (`crates/schema/tests/compile_fail/*.rs` + `*.stderr`, driven by a `#[test]` using `trybuild::TestCases`). Mirror it for the action/credential probes. Find the action-crate test entrypoint: `rg -n "trybuild|TestCases|compile_fail" crates/action/tests crates/credential/tests`.
5. **`derive_action.rs` schema test.** `crates/action/tests/derive_action.rs:40` `fn input_schema_matches_input_type` CALLS `NoCredAction::input_schema()` — this test must be rewritten (Task 3) to assert via `nebula_schema::schema_of::<<NoCredAction as Action>::Input>()`, not deleted (it guards the convergence — keep its behavioral intent).

---

## File structure / change map

| Area | Files | Responsibility |
|---|---|---|
| Helper | `crates/schema/src/has_schema.rs`, `crates/schema/src/lib.rs` | Add `schema_of` free fn + crate-root re-export (canonical `nebula_schema::schema_of`) |
| Macro hygiene | `crates/credential/src/lib.rs` | Re-export `pub use nebula_schema::schema_of;` so `#[derive(Credential)]` emits a path resolvable without forcing plugin authors onto a direct `nebula-schema` dep |
| Credential fold | `crates/credential/src/contract/credential.rs` (delete method + doc), `crates/credential/src/metadata.rs:71`, `crates/credential/src/no_credential.rs:76`, `crates/credential/macros/src/credential.rs:477` (+ doc 20,44), `crates/credential/src/lib.rs` (doc 21,26), `crates/credential/README.md` (48,155,178) | Remove `properties_schema`; route all callers through `schema_of::<…::Properties>()` |
| Action fold | `crates/action/src/action.rs` (delete 76,79 + doc/doctest), `crates/action/macros/src/action.rs` (delete 83-97 + doc 4), `crates/sdk/src/lib.rs` (delete 261-275), `crates/action/README.md` (34,35), hand-written `impl Action` in `crates/action/src/{control,resource,stateless,stateful,trigger/mod,webhook/providers/*}.rs`, engine `#[cfg(test)]` fixtures in `crates/engine/src/{engine.rs,runtime/runtime.rs,runtime/registry.rs}` | Remove the two redundant methods everywhere they are defined |
| Test migration | `crates/{action,api,engine,credential}/tests/**` (~48 files) | Remove the two method blocks from each `impl Action`; replace `*::properties_schema()` calls with `schema_of::<…>()`; rewrite `derive_action.rs` schema test |
| Seam tests | `crates/action/tests/seam_hasschema_convergence.rs` (new, runtime), `crates/action/tests/compile_fail/action_input_schema_removed.rs` + `.stderr` (new, trybuild), `crates/credential/tests/compile_fail/credential_properties_schema_removed.rs` + `.stderr` (new, trybuild) | Lock the P3 L2 invariant: schema reachable only via assoc-type/`schema_of`; the removed methods do not resolve |
| ADR / canon | `docs/adr/0052-schema-validator-condition-seam.md` (P3 amendment), `docs/adr/0043-dependency-declaration-dx.md` (§4 forward-pointer), `docs/MATURITY.md` (if git-tracked here), `docs/superpowers/plans/2026-05-17-adr0052-p3-hasschema-convergence.md` (this plan) | Canon §0.1/§17 compliance, same PR |

**Commit boundaries (lefthook runs full-workspace `clippy -D warnings` every commit ⇒ commit only at workspace-green; coarse atomic commits, NOT per-file):**
- C0: this plan doc.
- C1: `schema_of` helper + re-exports + helper unit test (pure addition, always green).
- C2: Credential fold (delete `properties_schema`, migrate 3 prod callers + credential/engine-credential tests, credential seam + compile-fail). Workspace green (Action untouched).
- C3: Action fold (delete trait methods + both macros + all hand-written impls + ~150 test edits, action seam + compile-fail). Workspace green.
- C4: ADR-0052 P3 amendment + ADR-0043 §4 pointer + MATURITY + READMEs/rustdoc.
All commits via `bash scripts/worktree.sh commit refactor action "<summary>"` (convco-validated `refactor(action): …`); the PR title carries `!`.

---

## Task 0: Commit this plan

**Files:**
- Create: `docs/superpowers/plans/2026-05-17-adr0052-p3-hasschema-convergence.md` (this file, already written into the persistent worktree)

- [ ] **Step 1: Stage and commit the plan**

```bash
cd C:/Users/vanya/RustroverProjects/nebula/.worktrees/adr0052-p3
git add docs/superpowers/plans/2026-05-17-adr0052-p3-hasschema-convergence.md
bash scripts/worktree.sh commit docs action "ADR-0052 P3 HasSchema convergence plan"
```

Expected: convco accepts `docs(action): ADR-0052 P3 HasSchema convergence plan`; lefthook pre-commit passes (no code change → clippy/fmt clean).

---

## Task 1: `schema_of` helper + re-exports (TDD)

**Files:**
- Modify: `crates/schema/src/has_schema.rs` (add free fn + rustdoc + unit test)
- Modify: `crates/schema/src/lib.rs:219` (re-export)
- Modify: `crates/credential/src/lib.rs` (re-export — verify item 2 first)

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` in `crates/schema/src/has_schema.rs` (the `Dummy` type already exists there with a 2-field schema):

```rust
    #[test]
    fn schema_of_equals_has_schema_schema() {
        // schema_of::<T>() is exactly <T as HasSchema>::schema() — the free
        // helper so call sites need not restate the trait-qualified path.
        assert_eq!(crate::schema_of::<Dummy>(), <Dummy as HasSchema>::schema());
        assert_eq!(
            crate::schema_of::<()>(),
            <() as HasSchema>::schema(),
            "unit blanket impl routes through schema_of"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd C:/Users/vanya/RustroverProjects/nebula/.worktrees/adr0052-p3 && cargo test -p nebula-schema --lib has_schema::tests::schema_of_equals_has_schema_schema 2>&1 | tail -20`
Expected: FAIL — `cannot find function `schema_of` in crate `crate``.

- [ ] **Step 3: Add the free helper**

In `crates/schema/src/has_schema.rs`, immediately after the `HasSchema` trait definition (after line 23, before `HasSelectOptions`), add:

```rust
/// Return the canonical [`ValidSchema`] for `T` without restating the
/// trait-qualified `<T as HasSchema>::schema()` at every call site.
///
/// This is the ergonomic, free-function form of [`HasSchema::schema`] — the
/// single way `Action`/`Credential`/`Resource` consumers reach a companion
/// type's schema (the associated-type bound, e.g. `Action::Input`, is the
/// sole source of truth; there is no per-trait `*_schema()` method). The
/// returned value is `Arc`-backed and cheap to clone; for derived types it
/// is already memoized inside `#[derive(Schema)]`. Per ADR-0061 a caller may
/// still wrap this in its own `OnceLock` if a `&'static` is required.
#[must_use]
pub fn schema_of<T: HasSchema>() -> ValidSchema {
    T::schema()
}
```

- [ ] **Step 4: Re-export from the schema crate root**

In `crates/schema/src/lib.rs`, change line 219 from:

```rust
pub use has_schema::{HasSchema, HasSelectOptions};
```

to:

```rust
pub use has_schema::{HasSchema, HasSelectOptions, schema_of};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p nebula-schema --lib has_schema::tests::schema_of_equals_has_schema_schema 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Re-export from `nebula-credential` (macro hygiene)**

First confirm verification item 2: `rg -n "pub use nebula_schema|pub use ::nebula_schema" crates/credential/src/lib.rs`. Add, next to the other `nebula_schema` re-exports (or in the prelude/root re-export block), `crates/credential/src/lib.rs`:

```rust
pub use nebula_schema::schema_of;
```

Rationale: `#[derive(Credential)]`'s `has_extras` branch (credential.rs:477) must emit a path resolvable in the plugin author's crate. Plugin authors depend on `nebula-credential` (they derive `Credential`) but not necessarily directly on `nebula-schema`; `::nebula_credential::schema_of` always resolves. Do NOT add a `nebula_action::schema_of` re-export — the action macro emits no schema path after P3 (YAGNI, ADR-0061 ethos).

- [ ] **Step 7: Workspace-green check + commit**

```bash
cargo build -p nebula-schema -p nebula-credential 2>&1 | tail -5
cargo clippy --workspace --all-targets -q -- -D warnings 2>&1 | tail -5
cargo fmt -p nebula-schema -p nebula-credential
git add crates/schema/src/has_schema.rs crates/schema/src/lib.rs crates/credential/src/lib.rs
bash scripts/worktree.sh commit refactor action "add nebula_schema::schema_of helper + credential re-export (ADR-0052 P3)"
```

Expected: clippy clean workspace-wide; convco accepts `refactor(action): add nebula_schema::schema_of helper + credential re-export (ADR-0052 P3)`. **Never** `cargo fmt --all` / `task fmt` on this worktree path (Windows os error 206) — per-crate only.

---

## Task 2: Credential convergence — delete `properties_schema`, migrate, seam + compile-fail (atomic commit)

**Files:**
- Modify: `crates/credential/src/contract/credential.rs` (delete method 158-163 + doc 49-54,120-132,152-157)
- Modify: `crates/credential/src/metadata.rs:71`, `crates/credential/src/no_credential.rs:76`, `crates/credential/macros/src/credential.rs:477` (+ doc 20,44), `crates/credential/src/lib.rs:21,26`, `crates/credential/README.md:48,155,178`
- Modify: `crates/credential/tests/{properties_pipeline.rs,runtime_duplicate_key_fatal.rs,registry_capabilities_iter.rs}`, `crates/engine/tests/{credential_thundering_herd_tests.rs,credential_resolver_refresh_coalesced.rs,credential_pending_lifecycle_tests.rs,credential_executor_tests.rs}` and any other `*::properties_schema()` caller
- Create: `crates/credential/tests/compile_fail/credential_properties_schema_removed.rs` + `.stderr`
- Create/Modify: credential seam assertion (add to `crates/credential/tests/properties_pipeline.rs`)

- [ ] **Step 1: Write the trybuild compile-fail probe (the P3 seam enforcement)**

First confirm verification item 4 (trybuild harness in credential tests). Create `crates/credential/tests/compile_fail/credential_properties_schema_removed.rs`:

```rust
//! Seam (ADR-0052 P3): `Credential::properties_schema()` is removed — schema
//! is reachable only via the `Properties: HasSchema` associated-type bound /
//! `nebula_schema::schema_of`. This MUST fail to compile.
use nebula_credential::no_credential::NoCredential;

fn main() {
    let _ = NoCredential::properties_schema();
}
```

Wire it into the credential trybuild entrypoint (mirror P2's `crates/schema/tests/*` pattern found in verification item 4 — e.g. a `#[test] fn compile_fail() { trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs"); }` test file; if a credential trybuild entrypoint already exists, add the glob/case there).

- [ ] **Step 2: Run the probe to verify it currently does NOT fail (method still present ⇒ trybuild test red)**

Run: `cargo test -p nebula-credential --test <trybuild_entrypoint> 2>&1 | tail -30`
Expected: trybuild reports the case **compiled but was expected to fail** (because `properties_schema` still exists). This is the RED state.

- [ ] **Step 3: Delete the `properties_schema` provided method**

In `crates/credential/src/contract/credential.rs` delete lines 152-163 (the doc comment block `/// Returns the schema for credential setup parameters. …` through the method body and its closing brace):

```rust
    /// Returns the schema for credential setup parameters.
    ///
    /// Defaults to `<Self::Properties as HasSchema>::schema()`. Phase 5
    /// shifts schema ownership from instance metadata to the type-level
    /// properties struct; consumers should call this rather than
    /// reading a baked schema from `metadata().schema`.
    fn properties_schema() -> ValidSchema
    where
        Self: Sized,
    {
        <Self::Properties as nebula_schema::HasSchema>::schema()
    }
```

Then fix the surrounding doc that points at it:
- Lines 49-54 (`# Associated types` → `**`Properties`**` bullet) and 120-132 (the `type Properties` doc): replace any `[`properties_schema()`](Credential::properties_schema)` reference with: "read its schema via [`nebula_schema::schema_of::<Self::Properties>()`](nebula_schema::schema_of)".
- If `ValidSchema` import becomes unused after deletion, drop it from the `use nebula_schema::{FieldValues, ValidSchema};` at line 31 → `use nebula_schema::FieldValues;` (verify with `cargo build -p nebula-credential` in Step 6; clippy `-D warnings` will flag an unused import).

- [ ] **Step 4: Migrate the 3 production callers**

`crates/credential/src/metadata.rs:71` — change:

```rust
            base: BaseMetadata::new(key, name, description, C::properties_schema()),
```
to:
```rust
            base: BaseMetadata::new(key, name, description, nebula_schema::schema_of::<C::Properties>()),
```
(`C: crate::Credential` is in scope; `C::Properties: nebula_schema::HasSchema` is guaranteed by the trait bound.)

`crates/credential/src/no_credential.rs:76` — change `.schema(Self::properties_schema())` to `.schema(nebula_schema::schema_of::<Self::Properties>())` (here `Self::Properties = ()`).

`crates/credential/macros/src/credential.rs:477` — change the emitted token `.schema(Self::properties_schema())` to `.schema(::nebula_credential::schema_of::<Self::Properties>())`. Then per verification item 3 confirm the non-`has_extras` branch delegates to `::nebula_credential::CredentialMetadata::for_credential::<Self>()` (fixed internally by the metadata.rs:71 edit); if instead it also emits `Self::properties_schema()`, apply the same `::nebula_credential::schema_of::<Self::Properties>()` replacement there. Update the macro module doc lines 20 & 44 to describe `schema_of::<Self::Properties>()` instead of "the default `Credential::properties_schema()` body".

- [ ] **Step 5: Migrate credential + engine-credential test/doc callers**

Replace every remaining `properties_schema()` invocation (NOT definitions — there are none left) with the `schema_of` form. Enumerate exactly:

```bash
rg -n "properties_schema" crates --glob '!**/contract/credential.rs' | rg -v '//' | cat
```

For each `X::properties_schema()` (e.g. `ApiKeyCredential::properties_schema()`, `Self::properties_schema()`, `C::properties_schema()`) substitute `nebula_schema::schema_of::<<X as nebula_credential::Credential>::Properties>()` (in tests prefer the explicit `<X as Credential>::Properties` form; for `Self`/`C` use `Self::Properties`/`C::Properties`). Known files: `crates/credential/tests/properties_pipeline.rs` (8 — including renaming/keeping `properties_schema_matches_companion_struct`'s assertion to compare `schema_of::<<ApiKeyCredential as Credential>::Properties>()` vs `<<ApiKeyCredential as Credential>::Properties as HasSchema>::schema()`), `crates/credential/tests/runtime_duplicate_key_fatal.rs` (2), `crates/credential/tests/registry_capabilities_iter.rs` (2), `crates/engine/tests/credential_*` (the `properties_schema` hits from the count map). Update `crates/credential/src/lib.rs:21,26` and `crates/credential/README.md:48,155,178` prose to reference `schema_of` / the assoc-type bound (delete the `fn properties_schema()` code sample in README:48; the README:155 migration note becomes "the schema is read via `nebula_schema::schema_of::<Self::Properties>()`").

- [ ] **Step 6: Add the runtime convergence seam assertion**

Append to `crates/credential/tests/properties_pipeline.rs`:

```rust
#[test]
fn for_credential_metadata_schema_is_schema_of_properties() {
    // ADR-0052 P3 seam: the converged path. CredentialMetadata built via
    // for_credential exposes exactly schema_of::<C::Properties>() — no
    // separate Credential::properties_schema() method exists.
    use nebula_credential::Credential;
    let meta = ApiKeyCredential::metadata();
    assert_eq!(
        meta.base.schema,
        nebula_schema::schema_of::<<ApiKeyCredential as Credential>::Properties>(),
        "credential metadata schema must equal schema_of::<Properties>()"
    );
}
```

(Confirm `meta.base.schema` field path against `CredentialMetadata`/`BaseMetadata` — `rg -n "pub schema|base\.schema" crates/credential/src crates/metadata/src` — adjust accessor if it is a method.)

- [ ] **Step 7: Verify the compile-fail probe now fails to compile (GREEN)**

Run: `cargo test -p nebula-credential --test <trybuild_entrypoint> 2>&1 | tail -30`
Expected: trybuild PASS — the probe now fails with `no function or associated item named `properties_schema``. Refresh the `.stderr` via `TRYBUILD=overwrite cargo test -p nebula-credential --test <entrypoint>` then inspect the `.stderr` is sane (no absolute paths).

- [ ] **Step 8: Workspace-green verification**

```bash
cargo build -p nebula-credential -p nebula-engine 2>&1 | tail -5
cargo nextest run -p nebula-credential 2>&1 | tail -15
cargo nextest run -p nebula-engine -E 'test(credential)' 2>&1 | tail -15
cargo clippy --workspace --all-targets -q -- -D warnings 2>&1 | tail -5
```
Expected: builds clean, credential + engine-credential tests pass, clippy clean workspace-wide (Action methods still present and untouched ⇒ `nebula-action` and the rest still compile).

- [ ] **Step 9: Commit**

```bash
cargo fmt -p nebula-credential
git add crates/credential crates/engine/tests
bash scripts/worktree.sh commit refactor action "delete Credential::properties_schema; route via schema_of (ADR-0052 P3)"
```

---

## Task 3: Action convergence — delete trait methods + both macros + all impls + tests, seam + compile-fail (atomic commit)

**Files:**
- Modify: `crates/action/src/action.rs` (delete 76,79; doc 15; doctest 28-57)
- Modify: `crates/action/macros/src/action.rs` (delete 83-97; doc 4)
- Modify: `crates/sdk/src/lib.rs` (delete 261-275)
- Modify: production `impl Action` in `crates/action/src/{resource.rs,stateful.rs,trigger/mod.rs,webhook/providers/stripe.rs,webhook/providers/slack.rs,webhook/providers/generic.rs}`; `#[cfg(test)]` fixtures in `crates/action/src/{control.rs,stateless.rs}`, `crates/engine/src/{engine.rs,runtime/runtime.rs,runtime/registry.rs}`
- Modify: ~48 test files under `crates/{action,api,engine}/tests/**` (count map in Authority section)
- Modify: `crates/action/README.md:34,35`
- Create: `crates/action/tests/compile_fail/action_input_schema_removed.rs` + `.stderr`; `crates/action/tests/seam_hasschema_convergence.rs`

- [ ] **Step 1: Write the trybuild compile-fail probe**

Create `crates/action/tests/compile_fail/action_input_schema_removed.rs`:

```rust
//! Seam (ADR-0052 P3): `Action::input_schema()`/`output_schema()` are removed
//! — schema is reachable only via the `Input/Output: HasSchema` bound /
//! `nebula_schema::schema_of`. This MUST fail to compile.
use nebula_action::Action;

struct Probe;
impl Action for Probe {
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    fn metadata() -> &'static nebula_action::ActionMetadata { unimplemented!() }
    fn dependencies() -> &'static nebula_core::Dependencies { unimplemented!() }
}

fn main() {
    let _ = Probe::input_schema();
}
```

Wire into the action trybuild entrypoint (verification item 4 — mirror P2 pattern; if `crates/action/tests` has no trybuild entrypoint, create `crates/action/tests/compile_fail.rs` with `#[test] fn compile_fail() { trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs"); }` and add `trybuild` to `crates/action/Cargo.toml` `[dev-dependencies]` from the workspace pin already used by `nebula-schema`).

- [ ] **Step 2: Write the runtime convergence seam test**

Create `crates/action/tests/seam_hasschema_convergence.rs`:

```rust
//! ADR-0052 P3 seam: Action schema is reachable solely via the
//! `Input/Output: HasSchema` associated-type bound exposed through
//! `nebula_schema::schema_of`, and the converged consumer path
//! (`ActionMetadata::for_stateless::<A>`) agrees with it.
use nebula_action::{Action, ActionMetadata};
use nebula_schema::{HasSchema, schema_of};

#[derive(serde::Deserialize, serde::Serialize, Default)]
struct In;
impl HasSchema for In {
    fn schema() -> nebula_schema::ValidSchema { nebula_schema::ValidSchema::empty() }
}

struct A;
impl Action for A {
    type Input = In;
    type Output = serde_json::Value;
    fn metadata() -> &'static ActionMetadata {
        static M: std::sync::OnceLock<ActionMetadata> = std::sync::OnceLock::new();
        M.get_or_init(|| ActionMetadata::for_stateless::<A>(
            nebula_core::action_key!("seam.p3"), "Seam", ""))
    }
    fn dependencies() -> &'static nebula_core::Dependencies {
        static D: std::sync::OnceLock<nebula_core::Dependencies> = std::sync::OnceLock::new();
        D.get_or_init(nebula_core::Dependencies::new)
    }
}

#[test]
fn schema_of_is_the_single_source_of_truth() {
    assert_eq!(schema_of::<<A as Action>::Input>(), <In as HasSchema>::schema());
}

#[test]
fn metadata_schema_equals_schema_of_input() {
    // for_stateless::<A>() already routes through <A::Input as HasSchema>::schema()
    assert_eq!(A::metadata().base.schema, schema_of::<<A as Action>::Input>());
}
```

(Confirm `ActionMetadata::for_stateless` signature and `metadata().base.schema` accessor against `crates/action/src/metadata.rs:186-196,309-312`; adjust the `for_stateless` arg list / schema accessor to match exactly.)

- [ ] **Step 3: Run both — verify probe is RED, seam is GREEN-able**

Run: `cargo test -p nebula-action --test compile_fail 2>&1 | tail -30` → Expected: RED (probe compiles, expected-fail not met, because methods still exist).
Run: `cargo test -p nebula-action --test seam_hasschema_convergence 2>&1 | tail -20` → Expected: PASS already (converged consumer path pre-exists; this test must stay green through the deletion).

- [ ] **Step 4: Delete the trait methods + fix trait doc/doctest**

`crates/action/src/action.rs`: delete line 76 `fn input_schema() -> &'static ValidSchema;` and line 79 `fn output_schema() -> &'static ValidSchema;` (and their `///` doc lines 75,78). Update the trait-level doc: line 15 drop "validation schemas (`input_schema` / `output_schema`), and"; in the `# Example` doctest (28-57) delete the two `fn input_schema`/`fn output_schema` blocks (44-51). If `ValidSchema` becomes unused in `use nebula_schema::{HasSchema, ValidSchema};` (line 7), reduce to `use nebula_schema::HasSchema;` (clippy will confirm).

- [ ] **Step 5: Delete macro emission (both macros)**

`crates/action/macros/src/action.rs`: delete the two emitted blocks (lines 83-97, the `fn input_schema` and `fn output_schema` `quote!` arms inside `action_impl`). Update module doc line 4 to `- `impl Action for Foo` with static `metadata`, `dependencies` functions.`
`crates/sdk/src/lib.rs`: delete lines 261-275 (the `fn input_schema`/`fn output_schema` arms in `simple_action!`).

- [ ] **Step 6: Strip the two methods from every hand-written `impl Action` (deterministic mechanical transform)**

The transform is identical everywhere: inside any `impl Action for X { … }` (and macro-free fixtures), delete exactly the two method items:

```rust
        fn input_schema() -> &'static ValidSchema {
            static S: OnceLock<ValidSchema> = OnceLock::new();
            S.get_or_init(<… as HasSchema>::schema)
        }
        fn output_schema() -> &'static ValidSchema {
            static S: OnceLock<ValidSchema> = OnceLock::new();
            S.get_or_init(<… as HasSchema>::schema)
        }
```

Apply to production: `crates/action/src/resource.rs:213-220`, `crates/action/src/stateful.rs:647-654 & 776-783`, `crates/action/src/trigger/mod.rs:613-620`, `crates/action/src/webhook/providers/{stripe.rs:76-84,slack.rs:78-86,generic.rs:120-128}`. Apply to `#[cfg(test)]` in-crate fixtures: `crates/action/src/control.rs` (TestIf ~772-779, TestStop ~821-828), `crates/action/src/stateless.rs:219-226`. Apply to engine `#[cfg(test)]` fixtures: every `impl Action` in `crates/engine/src/engine.rs`, `crates/engine/src/runtime/runtime.rs`, `crates/engine/src/runtime/registry.rs` (the engine.rs/runtime.rs/registry.rs hits in the count map — all `#[cfg(test)]`; the same two-method block). After each crate, drop now-unused `ValidSchema`/`OnceLock`/`HasSchema` imports only if clippy `-D warnings` flags them (some fixtures still use `OnceLock` for `metadata`).

- [ ] **Step 7: Strip the two methods from every test-crate `impl Action`; rewrite the schema test**

Files (from the count map): `crates/action/tests/{dx_poll.rs,dx_control.rs,dx_webhook.rs,dx_paginated.rs,dx_batch.rs,execution_integration.rs,resource_roundtrip.rs,schema_validator_expression_pipeline.rs,probes/missing_trigger_source.rs}`, `crates/api/tests/{webhook_transport_integration.rs,common/mod.rs,knife.rs}`, `crates/engine/tests/{end_to_end_pipeline.rs,retry.rs,resource_integration.rs,lease_takeover.rs,integration.rs,control_dispatch.rs}`. Same two-method deletion. Then rewrite `crates/action/tests/derive_action.rs` `fn input_schema_matches_input_type` (≈ line 40) to preserve its intent without the removed method:

```rust
#[test]
fn input_schema_matches_input_type() {
    use nebula_schema::{HasSchema, schema_of};
    let schema = schema_of::<<NoCredAction as nebula_action::Action>::Input>();
    assert_eq!(schema, <<NoCredAction as nebula_action::Action>::Input as HasSchema>::schema());
}
```

Done-check (no occurrence survives outside docs/historical):
```bash
rg -n "fn input_schema|fn output_schema" crates --glob '!**/*.md' | cat   # expect: zero lines
rg -n "fn properties_schema" crates --glob '!**/*.md' | cat               # expect: zero lines
```

- [ ] **Step 8: Verify probe GREEN + seam GREEN + workspace builds**

```bash
cargo test -p nebula-action --test compile_fail 2>&1 | tail -20      # trybuild PASS (probe now fails to compile)
cargo test -p nebula-action --test seam_hasschema_convergence 2>&1 | tail -20  # PASS
cargo build --workspace --all-targets 2>&1 | tail -10                # clean
```
Refresh probe `.stderr`: `TRYBUILD=overwrite cargo test -p nebula-action --test compile_fail` then sanity-check the `.stderr` (relative paths only).

- [ ] **Step 9: Full per-crate gate + commit**

```bash
cargo nextest run -p nebula-action -p nebula-sdk -p nebula-api -p nebula-engine 2>&1 | tail -20
cargo clippy --workspace --all-targets -q -- -D warnings 2>&1 | tail -5
cargo test -p nebula-action -p nebula-sdk --doc 2>&1 | tail -10
cargo fmt -p nebula-action -p nebula-action-macros -p nebula-sdk -p nebula-engine -p nebula-api
git add crates
bash scripts/worktree.sh commit refactor action "delete Action::input_schema/output_schema; converge on HasSchema bound (ADR-0052 P3)"
```

---

## Task 4: ADR-0052 P3 amendment + ADR-0043 §4 forward-pointer + MATURITY + READMEs/rustdoc

**Files:**
- Modify: `docs/adr/0052-schema-validator-condition-seam.md` (append P3 amendment)
- Modify: `docs/adr/0043-dependency-declaration-dx.md` (§4 truthful pointer)
- Modify: `docs/MATURITY.md` (if git-tracked in this repo — verification item 1)
- Verify final state of `crates/action/README.md`, `crates/credential/README.md`, crate `lib.rs` rustdoc (already edited in Tasks 2-3 — confirm no stale `input_schema`/`properties_schema` prose remains)

- [ ] **Step 1: Append the ADR-0052 P3 amendment**

Append to `docs/adr/0052-schema-validator-condition-seam.md` (after the 2026-05-16 P2 amendment), wording in the same register as the P1/P2 amendments:

```markdown
## Amendment (2026-05-17) — P3: HasSchema convergence (Action/Credential ISP fold)

P3 of the recorded cascade converges the three business traits onto one
schema-access shape. `Action::input_schema()`/`output_schema()` (required
methods, ISP fat-interface redundancy — every in-tree body was an `OnceLock`
wrapping `<Self::Input as HasSchema>::schema()`, with zero custom overrides
and zero production callers; the real consumer `ActionMetadata::for_*::<A>`
already used the associated type) and `Credential::properties_schema()` (a
provided method whose body already was `<Self::Properties as
HasSchema>::schema()`) are **deleted, not deprecated** (no-shim discipline).
The `type Input/Output/Properties: HasSchema` associated-type bound is the
sole source of truth; `Resource` already had this clean shape and is
untouched (the convergence reference). A free
`nebula_schema::schema_of::<T: HasSchema>() -> ValidSchema` helper (ratified
shape per ADR-0061: owned, no object-safe companion) lets call sites avoid
restating the trait-qualified path; it is re-exported from
`nebula-credential` so `#[derive(Credential)]` emits a path resolvable
without forcing plugin authors onto a direct `nebula-schema` dependency.
Behaviorally lossless (the deleted bodies were pure redundancy);
signature-invisible to the `ValidSchema → ValidValues → ResolvedValues`
proof-token pipeline (INTEGRATION_MODEL §29/§33 unchanged). Breaking: the
public trait surface of `nebula-action` and `nebula-credential` loses three
methods — canon-legal because both are `frontier`/pre-1.0 (no
UPGRADE_COMPAT contract); ships `!` with the seam tests in the same PR.
This amends ADR-0043 §4 (which defined `input_schema`/`output_schema` as
`= Self::Input::schema()`); see the forward-pointer added there. Zero new
crates, zero `deny.toml` change (`HasSchema`/`schema_of` stay in
`nebula-schema` Core). Seam anchors: `crates/action/tests/seam_hasschema_convergence.rs`
(runtime: `schema_of` == assoc-type schema; `ActionMetadata::for_stateless`
== `schema_of::<Input>`), `crates/action/tests/compile_fail/action_input_schema_removed.rs`
and `crates/credential/tests/compile_fail/credential_properties_schema_removed.rs`
(the removed methods do not resolve), plus the credential
`for_credential_metadata_schema_is_schema_of_properties` regression. P4
(API write-path V2 / catalog `json_schema()` V3 / public DTO projection /
ADR-0047 amendment) is the remaining cascade phase, out of P3 scope.
```

- [ ] **Step 2: Add the truthful forward-pointer to ADR-0043 §4**

In `docs/adr/0043-dependency-declaration-dx.md`, in the §4 "Variant A trait shape" code block, replace the two comment lines:

```rust
    fn input_schema() -> &'static ValidSchema;     // = Self::Input::schema()
    fn output_schema() -> &'static ValidSchema;    // = Self::Output::schema()
```
with a note line directly under the `dependencies()` line inside that block:
```rust
    fn dependencies() -> &'static Dependencies;    // slot fields
    // NOTE: input_schema()/output_schema() were removed by ADR-0052 P3
    // (2026-05-17). The `Input/Output: HasSchema` bound is the single
    // source of truth; consumers use `nebula_schema::schema_of::<T>()`.
```
(Keep the prose elsewhere in §4 intact — only the trait code block changes, so the ADR no longer advertises deleted methods as current contract.)

- [ ] **Step 3: MATURITY.md note (verification item 1)**

Run `git ls-files docs/MATURITY.md`. If tracked in this repo: add a `Last targeted revision: 2026-05-17` block at the top of the revision log (mirror the existing entry style) describing the ADR-0052 P3 trait-surface fold for `nebula-action`/`nebula-credential`/`nebula-schema`/`nebula-sdk` (tiers unchanged — all already `frontier`/`partial`; public surface narrowed). If NOT tracked here (external L1 only): do not edit an untracked file; instead note in the PR body "MATURITY.md lives in the external L1 canon archive; tiers unchanged (frontier/partial), trait-surface narrowing recorded in ADR-0052 P3 amendment."

- [ ] **Step 4: README/rustdoc final sweep**

```bash
rg -n "input_schema|output_schema|properties_schema" crates --glob '**/*.md' --glob '**/lib.rs' --glob '**/README.md' | cat
```
Every remaining hit must be either deleted or rephrased to the `schema_of`/assoc-type wording (Tasks 2-3 already touched `crates/action/README.md:34-35`, `crates/credential/README.md:48,155,178`, `crates/credential/src/lib.rs:21,26`). Confirm `crates/action/src/action.rs` `//!`/trait doc and `crates/credential/src/contract/credential.rs` doc carry no dangling intra-doc link to the removed methods (rustdoc `-D warnings` would fail otherwise — recall the project pitfall: do not bracket unresolved intra-doc paths).

- [ ] **Step 5: Commit**

```bash
git add docs crates
bash scripts/worktree.sh commit docs action "ADR-0052 P3 amendment + ADR-0043 forward-pointer + MATURITY/READMEs"
```

---

## Task 5: Full verification gate + PR

- [ ] **Step 1: Run the full pre-PR gate (per-crate fmt; never `task fmt`/`cargo fmt --all` on this worktree)**

```bash
cd C:/Users/vanya/RustroverProjects/nebula/.worktrees/adr0052-p3
cargo nextest run --workspace 2>&1 | tail -20
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
cargo test --workspace --doc 2>&1 | tail -10
cargo deny check 2>&1 | tail -10
$env:RUSTDOCFLAGS="-D warnings"; cargo doc --workspace --no-deps 2>&1 | tail -10; Remove-Item Env:\RUSTDOCFLAGS
for ($c in @('nebula-schema','nebula-action','nebula-action-macros','nebula-credential','nebula-credential-macros','nebula-sdk','nebula-engine','nebula-api')) { cargo fmt -p $c -- --check }
```
Expected: nextest all-pass (≈ prior P2 baseline 4308±, adjust for new seam tests, 0 failed); clippy clean; doctests pass; deny ok; rustdoc clean; fmt check clean per-crate. If fmt:check flags a file, `cargo fmt -p <crate>` then re-commit (coarse). Record exact pass/fail counts — do not assert green without reading the tail.

- [ ] **Step 2: Push and open the PR**

```bash
git push -u origin refactor/action-adr0052-p3
```
Then `gh pr create --repo vanyastaff/nebula --base main` following `.github/PULL_REQUEST_TEMPLATE.md`:
- Title: `refactor(action)!: ADR-0052 P3 — HasSchema convergence (delete Action/Credential schema methods, add schema_of)`
- Body: summary; "Refs ADR-0052 (P3 amendment)"; tick the "L2 invariant changed → ADR + seam test in this PR" box; Breaking-changes section (three methods removed from `nebula-action`/`nebula-credential` public surface; `frontier`/pre-1.0 justification; migration = use `nebula_schema::schema_of::<T>()` / the `Input/Output/Properties: HasSchema` bound); Test plan (seam runtime + 2 trybuild compile-fail + credential regression + full gate counts); Docs checklist (ADR-0052 P3 amendment, ADR-0043 §4 pointer, MATURITY, READMEs); Safety (no `unwrap`/`expect`/`panic!` added; proof-token custody unchanged; no secret-path change). End body with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.

- [ ] **Step 3: Triage ALL bot reviews verify-first (P1/P2 each shipped a real bug past green CI)**

For every CodeRabbit/Copilot/Codex thread: reproduce/verify the claim against source before agreeing; implement real fixes (new commit on the branch, re-run the affected gate); rebut false positives with concrete evidence; reply + resolve every thread by `comment_id`. Never blind-merge on green CI + clean mergeability alone.

- [ ] **Step 4: Squash-merge only when CI is fully green and confirmed stage-by-stage**

After merge: `cd C:/Users/vanya/RustroverProjects/nebula && bash scripts/worktree.sh finish adr0052-p3`. Then spawn P4 the same way (self-contained, against P3's landed signatures): P4 = credential write-path validates `data` against the resolved `ValidSchema` before persist (V2); catalog endpoints populate `json_schema()` (V3); public OpenAPI DTO strips `x-nebula-root-rules`; one-paragraph ADR-0047 amendment. P4 is the final cascade phase.

---

## P4 backlog (record so it is not lost)

P4 is authored against P3's landed signatures (own plan, own PR): (a) API credential write path validates `req.data` against the resolved `ValidSchema` before persist — implements the standing `crates/api/.../credential` TODO, closes design-spec V2; (b) catalog endpoints populate `CredentialTypeInfo.schema` from `ValidSchema::json_schema()` (closes V3); (c) public OpenAPI DTO strips `x-nebula-root-rules` + per-field rule operands (design-spec hole #6); (d) one-paragraph in-place ADR-0047 amendment. P4 does NOT touch `slot_bindings` confused-deputy (design-spec Non-goal, tracked separately — credential resolution stays confused-deputy-exposed after P4; do not read "cascade complete" as "that is closed").

## Self-Review

**1. Spec coverage** (`2026-05-15-…-design.md` §"HasSchema convergence" + Phasing P3):
- Delete `Action::input_schema`/`output_schema` → Task 3 Steps 4-7. ✓
- Delete `Credential::properties_schema` → Task 2 Step 3. ✓
- Free `nebula_schema::schema_of::<T>()` → Task 1 Steps 3-4. ✓
- Update all in-tree consumers → Task 2 Steps 4-5, Task 3 Steps 5-7. ✓
- Zero new crates / zero `deny.toml` change → asserted in Authority + Architecture; verified `HasSchema` already cross-importable. ✓
- `Resource` is the reference, untouched → stated; no `crates/resource` edits in change map. ✓
- L2 trait-surface ⇒ ADR-0052 amendment + seam test same PR (canon §0.1/§17) → Task 1-3 land seam tests; Task 4 lands the ADR amendment; all one PR. ✓
- ADR-0061 ratified shape honored (owned, no object-safe companion) → Task 1 Step 3 helper is a free fn, doc cites ADR-0061. ✓
- ADR-0043 §4 truthfulness (it defined the deleted methods) → Task 4 Step 2. ✓
- P4 explicitly out of scope → Architecture + P4 backlog. ✓

**2. Placeholder scan:** No "TBD"/"add error handling"/"similar to Task N". The five "Open verification items" are explicit name/location confirmations against real source (MATURITY path, lib re-export line, macro non-extras branch, trybuild entrypoint, metadata accessor) with the exact `rg`/`git` command to resolve each — not design gaps; every type/method used (`HasSchema`, `ValidSchema`, `schema_of`, `Action`, `Credential`, `ActionMetadata::for_stateless`, `CredentialMetadata`/`BaseMetadata`, `Dependencies`, `trybuild::TestCases`) is read from verified source at 001e9022. ✓

**3. Type consistency:** `schema_of<T: HasSchema>() -> ValidSchema` identical in Task 1 (def), Task 2 (`schema_of::<C::Properties>()`), Task 3 (`schema_of::<<A as Action>::Input>()`), seam tests, ADR amendment. `Credential::Properties` / `Action::Input`/`Action::Output` assoc-type names match the trait defs. Re-export symbol `schema_of` consistent (`nebula_schema::schema_of`, `nebula_credential::schema_of`). Commit type `refactor`/`docs`, scope `action`, every commit via `scripts/worktree.sh commit`. ✓

**4. Scope/granularity:** 6 tasks, atomic-commit boundaries aligned to the lefthook full-workspace-clippy-per-commit constraint (C1 addition-only, C2 credential-only green, C3 action-only green, C4 docs). Mechanical bulk (≈150 test edits) is one deterministic transform with an exact done-check (`rg` returns zero). No P4 work present. ✓
