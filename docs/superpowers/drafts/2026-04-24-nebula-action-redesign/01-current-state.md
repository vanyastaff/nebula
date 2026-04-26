# Phase 0 — Current state consolidated

**Date:** 2026-04-24
**Orchestrator:** claude/upbeat-mendel-b30f89
**Inputs:** [`01a-code-audit.md`](./01a-code-audit.md) (rust-senior), [`01b-workspace-audit.md`](./01b-workspace-audit.md) (devops)
**Reconciliation status:** Convergent. No contradictions between code-side and workspace-side findings. Devops + rust-senior observations compose additively; the macro-test-harness gap (devops) mechanically explains rust-senior's unprotected attribute-rejection code paths.

---

## 0. Gate decision

**Gate status:** ✅ PROCEED to Phase 1.

Phase 0 reconciliation succeeded — both audits produce complementary ground truth. Phase 1 briefs will incorporate the 🔴 CRITICAL findings as load-bearing context so that pain enumeration is driven by evidence, not speculation.

**Escalation flag raised but NOT hard-stop:** Rust-senior identified a **structural spec-reality gap** between credential Tech Spec §§2.7/3.4/7.1/15.7 and current `nebula-action` implementation. This is not yet escalation rule 10 territory (action doesn't break credential — credential spec specifies shapes action lacks). Phase 2 scope narrowing is where the decision lives. Orchestrator will flag prominently but not pre-empt.

**Credential Tech Spec freeze level correction:** Prompt declared "CP5-frozen"; recent commits (`65443cdb`, `33eb3f01`, `883ccfbf`) indicate **CP6**. Rust-senior audit referenced CP6 content; no §-number migration implied but tracking the freeze level correctly matters for cross-crate escalation wording.

---

## 1. Executive summary

`nebula-action` is a mature but drift-accumulating crate. The base handler family and adapter pattern are internally consistent and well-tested (🟢 strong canon compliance on `ActionResult::Retry` feature gating). The crate's **integration surface** — macro emission contract, credential resolution path, canon §3.5 trait enumeration, doc vs code alignment — is where the drift compounds.

Three categorical drift patterns emerge:

1. **Spec-reality gap (🔴)**: The 2026-04-24 credential Tech Spec frozen at CP6 describes an API paradigm (`#[action]` attribute macro + `CredentialRef<C>` phantom + `SlotBinding` HRTB `resolve_fn` + `SchemeGuard<'a, C>`) that is **structurally absent** from `nebula-action`. Current action code is "string-keyed `CredentialSnapshot` → project to `AuthScheme`" — an older idiom superseded by the Tech Spec but not migrated.

2. **Canon §3.5/§0.2 drift (🟠)**: Canon enumerates 4 action traits and requires explicit canon revision to add another. `nebula-action` ships 10 trait surfaces including `ControlAction` (public, non-sealed, fifth dispatch-time trait) with no ADR. Library-level docstring on `lib.rs:11` self-contradicts by re-stating the "canon revision required" rule while re-exporting 10 traits.

3. **Tooling discipline gap (🟠)**: 359-line proc-macro with 9+ attribute rejection rules has zero `trybuild`/`macrotest` coverage. 7,500+ LOC of webhook + result + port code has zero benchmark coverage. `unstable-retry-scheduler` is a feature declared in two manifests but semantically dead (empty feature, no cfg-deps, turned on every CI run via `--all-features`). Lefthook pre-push does not mirror `doctests`/`msrv`/`doc` jobs despite user-memory-stated policy.

Migration blast radius is understood: **7 direct reverse-deps** (engine, api, sandbox, sdk, plugin, cli + macros sibling) across **69 source files** consuming **63 public items** of action's `lib.rs`, plus **~40 cascaded through `nebula-sdk::prelude`** (the officially-sanctioned user contract surface).

---

## 2. Critical findings (🔴) — inform Phase 1 pain enumeration

### Finding C1 — Credential Tech Spec CP6 vocabulary absent from action crate

**Source:** 01a §5, §11 finding 1.

**Claim:** `CredentialRef<C>` typed handle, phantom rewriting via `#[action]`, `AnyCredential` object-safe supertrait, `SlotBinding` with HRTB `resolve_fn` pointer, `resolve_as_bearer<C>` where-clause resolution, action-body-sees-`&Scheme`-not-`&dyn-Phantom` dispatch, `RefreshDispatcher::refresh_fn` HRTB, `SchemeGuard<'a, C>` RAII (`!Clone`, `ZeroizeOnDrop`, `Deref`), `SchemeFactory<C>` re-acquisition arc — **none exist in `nebula-action`**.

**What exists instead:** `CredentialContextExt` blanket with three methods (`credential_by_id(id) -> CredentialSnapshot`, `credential_typed<S>(id) -> S`, `credential<S>() -> CredentialGuard<S>` — last one uses `type_name::<S>().to_lowercase()` as the lookup key).

**Cascade implication:** Phase 2 must choose between:
- **Option A** — action adopts the CP6 Tech Spec vocabulary in full. Blast radius: new `#[action]` attribute macro (distinct from `#[derive(Action)]`, required for field-type rewriting — derives structurally cannot do this), new `CredentialRef<C>` wrapper type, new `SlotBinding` contract, HRTB fn-pointer emission, context extension replacement. Touches macro crate + handler family + context extension trait + adapter layer.
- **Option B** — action deprecates the type-name-as-key heuristic but does not adopt the full CP6 vocabulary. Action remains in an intermediate idiom; future engine work must bridge.
- **Option C** — escalate to tech-lead: can CP6 §§2.7/3.4/7.1/15.7 be partially deferred for action scaffolding? This is **escalation rule 10 territory** because it amends a frozen Tech Spec.

**Decision owner:** Phase 2 (architect + tech-lead + security-lead co-decision). Orchestrator flags and does not pre-empt.

### Finding C2 — `#[derive(Action)]` has a broken `parameters = Type` attribute path

**Source:** 01a §4 finding on `action_attrs.rs:129-134`.

**Claim:** Macro emits `.with_parameters(<Type>::parameters())`, but `ActionMetadata` only has `.with_schema(schema: ValidSchema)`. No `with_parameters` method exists anywhere. The attribute is documented (spec side + macro parse side at `action_attrs.rs:56`) but unusable.

**Masked because:** Zero workspace callers exercise `parameters = Type` today (grep confirms).

**Cascade implication:** This is a contained bug that any redesign touches by contract — either the attribute is removed or the emission is fixed to align with actual `ActionMetadata` surface. Affects macro emission contract decision in Phase 2 scope.

### Finding C3 — `CredentialContextExt::credential<S>()` type-name-lowercase-as-key heuristic

**Source:** 01a §5 finding on `context.rs:637-643`.

**Claim:**
```rust
let type_name = std::any::type_name::<S>();
let short_name = type_name.rsplit("::").next().unwrap_or(type_name);
let key_str = short_name.to_lowercase();
```

**Risk:** Collision-trivial. `MySlackCred` resolves to `"myslackcred"`; any two credential types with the same short name collide silently. `rsplit("::")` hides generic parameters / associated types. Violates v2 spec §3 "always keyed" mandate explicitly.

**Cascade implication:** A path-today 🔴 must be resolved regardless of which credential migration path (Option A/B/C from C1) gets chosen. If the redesign adopts `CredentialRef<C>` (Option A), this method disappears. If it defers (Option B), the method must at minimum be deprecated with `#[deprecated]` + require explicit key.

### Finding C4 — No `serde_json` recursion limit at `StatelessActionAdapter` deserialization boundary

**Source:** 01a §8 v2 spec amendment B3 audit, confirmed via grep.

**Claim:** v2 design spec post-conference amendment B3 mandates 128-depth recursion limit at adapter JSON boundaries. `StatelessActionAdapter::execute` (`stateless.rs:356-383`) calls `serde_json::from_value` with **no depth cap**. Zero matches for `set_max_nesting` / `recursion_limit` / `recursion` in `crates/action/src/`.

**Risk:** Stack-overflow via deeply-nested attacker-controlled input. Action crates receive JSON at the trust boundary with plugin-supplied action definitions.

**Cascade implication:** Security-lead (Phase 1) will weight this. Fix is small in isolation but must be part of any adapter redesign touch.

---

## 3. Major structural findings (🟠)

### S1 — Canon §3.5 / §0.2 drift via `ControlAction`

Canon (`PRODUCT_CANON.md §3.5`): "Action — what a step does. Dispatch via action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`). Adding a trait requires canon revision (§0.2)."

Code (`lib.rs:13-20`): declares 10 trait surfaces. `ControlAction` (`control.rs:393-431`) is documented "public and non-sealed — community plugin crates may implement it directly". No ADR exists for ControlAction.

The engine-dispatch invariant **is preserved** (ActionHandler enum has 4 variants; ControlAction erases to Stateless). But the user-facing public trait surface exposes a 5th dispatch-time trait. This is a literal canon-revision event by §0.2's wording.

### S2 — v2 design spec's "5 traits, no extras" principle violated

The 2026-04-06 action v2 design spec is prescriptive about keeping the trait family minimal (§Philosophy + §1). Actual code ships 5 DX specialization traits (`ControlAction`, `PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`) on top of the 4 primary + 1 base. The DX layer is well-tested and genuinely reduces author boilerplate, but it has landed without design-intent reconciliation.

### S3 — `ActionResult::Terminate` partially-implemented public variant not gated

`result.rs:207-219`: "Full scheduler integration is tracked as Phase 3 of the ControlAction plan and is **not yet wired**. Do not rely on `Terminate` in v1 to cancel sibling branches."

This violates canon §4.5 ("public surface exists iff engine honors it end-to-end"). By contrast `ActionResult::Retry` is correctly feature-gated. Asymmetric discipline.

### S4 — v2 spec §3 / §4 credential + resource access API drift

- Spec `ctx.credential::<S>(key)` + `ctx.credential_opt::<S>(key)` pair → code has 3 variants, none of which is the spec pair.
- Spec `ctx.resource::<R>(key) -> Lease` → code has `resource(key: &str) -> Box<dyn Any>` (untyped).
- `#[action(credential(optional) = "key")]` → macro has no `optional` handling.

### S5 — v2 spec §8 Port system drift

- `Provide` port kind absent (spec names `Flow/Support/Provide/Dynamic`; code has `Flow/Support/Dynamic`).
- DataTag hierarchical registry (spec: 58+ tags) totally absent — only `ConnectionFilter::allowed_tags: Vec<String>` in code.

### S6 — Handler dyn-safety uses verbose HRTB lifetime boilerplate

Every `*Handler` trait uses `for<'life0, 'life1, 'a> ... Pin<Box<dyn Future<...> + Send + 'a>>` rather than `async fn in trait`. Historically correct for Rust 1.95 dyn-safety but verbose. `trait_variant::make` or return-type-notation might tighten this.

---

## 4. Major coverage & tooling findings (🟠)

### T1 — No macro test harness

`crates/action/macros/Cargo.toml` has **no `[dev-dependencies]` section**. 359-LOC `#[derive(Action)]` with 9+ attribute rejection rules (string-cred, non-unit struct, missing attrs, etc.) has zero `trybuild`/`macrotest` coverage. Only happy-path runtime-derive test exists. Combined with C2 (broken `parameters = Type` path), this explains why a bug that would have been caught by expansion tests made it to mainline.

### T2 — `unstable-retry-scheduler` dead empty feature flag

`crates/action/Cargo.toml:20`: `unstable-retry-scheduler = []` — no deps, no cfg-gates. `crates/engine/Cargo.toml:21` forwards as convenience alias. `ci.yml:109` `cargo check --workspace --all-features --all-targets` unconditionally turns it on. `ActionResult::Retry` un-hides (via `#[cfg(feature = "unstable-retry-scheduler")]`) but has no runtime wiring. Canon §11.2 drift documented in two manifests but is a documentary-only gate.

### T3 — Dead `nebula-runtime` reference in CI matrix

`test-matrix.yml:66` includes `"nebula-runtime"` in FULL list; `crates/runtime/` does not exist (workspace has 36 members, none named runtime). Also in `.github/CODEOWNERS:52`. Matrix has `fail-fast: false`, so other shards continue, but `Tests` aggregator requires all shards success — **this should be red on push-to-main**. Out of action's scope; adjacent and worth filing separately.

### T4 — `zeroize` pinned inline instead of workspace

`crates/action/Cargo.toml:36` pins `zeroize = { version = "1.8.2" }` inline; workspace (`Cargo.toml:116`) already declares `zeroize = { version = "1.8.2", features = ["std"] }`. Inline drops the `std` feature and de-unifies. Crypto stack is shared with credential + api + storage; de-unified pin is a drift risk.

### T5 — Lefthook pre-push does not mirror doctests / msrv / doc

`lefthook.yml:45`: "Doctests/docs/MSRV remain CI-owned checks" — intentional but contradicts user feedback memory (`feedback_lefthook_mirrors_ci.md` — "lefthook pre-push MUST mirror every CI required job"). Action has 20+ doctests; none gated locally.

### T6 — No action benchmarks anywhere

`ci.yml:219` bench job runs only `nebula-log`. CodSpeed (`codspeed.yml:88-102`) skips action. 1,852 LOC webhook.rs (HMAC hot path) + 1,680 LOC result.rs (dispatcher) + ~500 LOC port.rs (routing) have no perf regression guard.

### T7 — SDK prelude is public contract surface

`crates/sdk/src/prelude.rs:15-33` re-exports ~40 action types. Any rename/relocation in action cascades to the officially-sanctioned user-facing API. Redesign must treat this as a fixed contract.

### T8 — Engine tight-coupled

27+ import sites across `engine.rs`, `runtime.rs`, `registry.rs`, `error.rs`, `stream_backpressure.rs`. Touches `ActionHandler`, `ActionResult`, `ActionMetadata`, `ActionError`, `PortKey`, `TerminationCode`, `BreakReason`, `Overflow`, `ResourceHandler`, `StatefulHandler`, `TriggerHandler`.

### T9 — No layer-enforcement deny rule for `nebula-action`

`deny.toml` has positive layer bans for engine/storage/sandbox/sdk but not for action. Today "implicit correct" because no lower-layer crate happens to reach action — redesign could silently drift without guardrail.

---

## 5. Minor findings (🟡) — tracked but not cascade-shaping

- Inline `syn`/`quote`/`proc-macro2` pins in action-macros (workspace lacks declaration)
- `nebula-core` + `hex` listed in both `[dependencies]` and `[dev-dependencies]` with no feature delta
- `nebula-resource` direct dep but no source-level use in action's `src/` (verify by deeper grep — flag for Phase 1 devops)
- `#[nebula]` attribute registered alongside `#[action]` but no code branch — forward reservation
- `Resource::on_credential_refresh` hook absent on `ResourceAction` (cross-cut with C1 if Option A)
- README `ResourceAction lives in nebula-resource` claim is false (it lives in nebula-action) — docs drift
- `output.rs:1213` — literal `"v1"` string; context unread
- Test-matrix uses `@stable` not `@1.95` (low-risk drift)
- Action lacks `no-default-features` CI gate (no-op today since `default = []`)
- `cargo-semver-checks` advisory-only during alpha (redesign breaks won't block CI)

---

## 6. v1/v2 vestiges catalogue

No true v1/v2 dual-trait coexistence exists in code. Drift pattern is "spec promise vs code reality", not "legacy left behind". Specific markers:

- `ActionResult::Retry` — correctly feature-gated, **not a vestige** (🟢 model)
- `ActionResult::Terminate` — partial implementation, not gated (S3)
- `TerminationCode` Phase-10 swap forward-reference — roadmap phase absent from live plans
- `TransactionalAction` removed cleanly on 2026-04-10 (stateful.rs:377-391 has archaeology comment)
- `output.rs:1213` literal `"v1"` string — flag for context review

---

## 7. Load-bearing decisions for Phase 2

Orchestrator tracks these as explicit Phase 2 scope options (do not resolve now):

| Question | Options | Decision owner |
|---|---|---|
| Credential integration shape | A: adopt CP6 vocabulary / B: deprecate-only / C: escalate to tech-lead for spec deferral | co-decision (architect + tech-lead + security-lead) |
| ControlAction canon status | Revise canon §3.5 to enumerate 5 / retrofit ControlAction as non-trait-family helper / accept §3.5 as loose guideline | tech-lead priority call + architect |
| DX trait layer status | Keep all 5 DX (Paginated/Batch/Webhook/Poll/Control) with canon blessing / collapse some / demote all to helpers | architect + dx-tester |
| `#[action]` macro vs `#[derive(Action)]` | Introduce attribute macro for field rewriting (if Option A) / keep derive only / dual surface | architect + rust-senior |
| `ActionResult::Terminate` gating | Feature-gate like Retry / remove / wire end-to-end in cascade | tech-lead priority call |
| Macro test harness | Add trybuild / add macrotest / add both | devops (post-Phase 3 implementation) |
| Lefthook parity | Fix divergence per user policy / ratify divergence as intentional | devops priority call |
| `zeroize` workspace pin | Migrate to workspace=true / ratify inline pin | devops |

---

## 8. Migration blast radius consolidated

- **7 direct reverse-deps**: engine, api, sandbox, sdk, plugin, cli + action-macros sibling
- **3 indirect doc-only refs**: workflow, storage, execution (rustdoc comments, no compile edge)
- **69 source files** import `nebula_action::*` symbols across the workspace
- **63 public items** re-exported from `crates/action/src/lib.rs:91-153`
- **~40+ items cascaded** through `nebula-sdk::prelude`
- **~55 files** in 6 crates + 1 app touched by a rename-only refactor of public-facing type names
- **Heaviest weight**: `nebula-engine` (dispatcher + registry + error paths, 27+ import sites) and `nebula-sandbox` (dyn-handler ABI across in-process and out-of-process runners, 7 files)

Semver-checks are advisory-only during alpha (`semver-checks.yml:27`) — breaking changes do not CI-block. Per user feedback memory `feedback_hard_breaking_changes.md`, hard breaks are acceptable for spec-correct outcomes.

---

## 9. Pointers

- Full code audit: [`01a-code-audit.md`](./01a-code-audit.md) (~450 lines, 11 sections)
- Full workspace audit: [`01b-workspace-audit.md`](./01b-workspace-audit.md) (~380 lines, 11 sections)
- Cascade log: [`CASCADE_LOG.md`](./CASCADE_LOG.md)

---

## 10. Phase 1 dispatch readiness

Phase 1 (pain enumeration) dispatches 4 agents in parallel per prompt:

| Agent | Slice | Load-bearing input |
|---|---|---|
| dx-tester | 3 action types authoring (Stateless / Stateful / ResourceAction+Credential) — newcomer DX measurement | C1 + C3 credential API discord; S2 trait surface count |
| security-lead | Threat model: credential-in-body leak / cancellation zeroize / output sanitization / webhook signature boundary | C1 + C3 + C4 (JSON depth cap); T2 retry feature flag |
| rust-senior | Idiomatic Rust: trait shape, async Send bounds, macro expansion quality, associated-type design, cancellation discipline | S1 canon drift; S6 HRTB verbosity; T1 macro harness |
| tech-lead | Architectural coherence: trait hierarchy weight, macro surface scope, Action×Credential×Resource tangling | C1 (Option A/B/C); S1; S2; T7 SDK prelude contract |

Phase 1 consolidated output: `02-pain-enumeration.md`. Gate: if total 🔴 = 0 AND 🟠 < 3, escalate "redesign not justified". Phase 0 already surfaces 4 🔴 + 9 🟠 — gate will pass.

---

*End of Phase 0 consolidation. Orchestrator proceeds to Phase 1 dispatch.*
