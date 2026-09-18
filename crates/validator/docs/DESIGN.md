# nebula-validator — design

| Field | Value |
|-------|-------|
| **Status** | `frontier` (ADR-0052/0080) — the programmatic API is stable; the `Rule` wire format changed recently |
| **Layer** | Core — the rules engine that `nebula-schema` delegates rule execution to |
| **Redesign role** | **Not directly affected** by the credential/resource redesign (no dependencies on those crates, none in reverse). An indirect participant: the only emitter of `required` errors (ADR-0052 P2) on the credential write path (P4) through `nebula-schema` |
| **Related** | ADR-0052, ADR-0080, PRODUCT_CANON §3.5 / §4.5, `origin/refactor/error-unify-validation` (unmerged error-unification branch) |

---

## 1. Purpose and boundaries

`nebula-validator` is the Core-layer shared rules engine with two surfaces:

1. **Programmatic validators** — the `Validate<T>` trait (`src/foundation/traits.rs`) plus
   `.and()` / `.or()` / `.not()` composition through `ValidateExt<T>`. Integration authors
   compose checks directly in Rust code.
2. **Declarative `Rule`** — a JSON-serializable typed sum-of-sums (`src/rule/mod.rs`) that
   schema fields carry. The engine executes it at lint / activation / runtime.

**Owns:** the validation traits (`Validate`, `ValidateExt`, `Validatable`), type erasure
(`AnyValidator<T>`), the structured `ValidationError` (≤80 bytes, `Cow`-based, RFC 6901
paths), the declarative `Rule` and its executor (`engine.rs`), the `Validated<T>` proof token
(canon §4.5), the visibility/required policy engine (`policy/`), the catalog of built-in
validators (length/pattern/content/range/size/boolean/nullable plus network/temporal behind
features), and the `#[derive(Validator)]` macro.

**Explicitly does not do** (see the README non-goals): it is **not** a schema system —
`Field`/`Schema` and the `ValidValues → ResolvedValues` pipeline live in `nebula-schema`; not
an expression evaluator (`nebula-expression`); not a resilience pipeline
(`nebula-resilience`); not an API error formatter — the RFC 9457 `problem+json` mapping lives
in `nebula-api`. There is no KDF or hashing here either (that is
`nebula-credential`/`nebula-crypto`).

## 2. Public surface

| Item | Location |
|------|----------|
| `Validate<T>` — core validator trait | `src/foundation/traits.rs` |
| `ValidateExt<T>` — `.and()` / `.or()` / `.not()` / `.when()` | `src/foundation/traits.rs` |
| `Validatable` | `src/foundation/traits.rs` |
| `AsValidatable` — fallible input conversion | `src/foundation/validatable.rs` |
| `ValidationError` — structured error (≤80 bytes, `Cow`, RFC 6901) | `src/foundation/error/validation_error.rs` |
| `ValidationErrors` — multi-error aggregate | `src/foundation/error/validation_errors.rs` |
| `FieldPath` — validated RFC 6901 pointer | `src/foundation/field_path.rs` |
| `AnyValidator<T>` — type-erased validator | `src/foundation/any.rs` |
| `Rule` — bounded arena, manual `Serialize`/`Deserialize` | `src/rule/mod.rs` |
| `RuleRef` / `RuleView` / `RuleChildren` / `RuleKind` | `src/rule/mod.rs` |
| `ValueRule` / `Predicate` / `DeferredRule` / `PredicateContext` | `src/rule/{value,predicate,deferred,context}.rs` |
| `ExecutionMode` / `EvaluationOutcome` / `DeferredReason` / `DiagnosticDisclosure` | `src/engine.rs` |
| `validate_rules` / `validate_rules_with_ctx` | `src/engine.rs` |
| `Validated<T>` — proof token (`Serialize` present, `Deserialize` deliberately absent) | `src/proof.rs` |
| `Presence` / `Requiredness` / `VisibilityPolicy` / `RequiredPolicy` | `src/policy/mod.rs` |
| `resolve_field_policies` — the single entry point for `nebula-schema::validate` | `src/policy/mod.rs` |
| `FieldDirective` / `FieldPolicyDecl` / `FieldPlan` / `FieldPolicyResolution` | `src/policy/mod.rs` |
| `ValidatorError` — operational error (`#[derive(nebula_error::Classify)]`) | `src/error.rs` |
| `#[derive(Validator)]` proc-macro (feature `derive`, subcrate `macros/`) | re-export in `src/lib.rs` |
| Built-in factories/types: length/pattern/content/range/size/boolean/nullable (+network/temporal) | `src/validators/mod.rs` |
| `__private::regex` re-export for derive-generated code | `src/lib.rs` |

## 3. Dependencies and dependents

- **Deps:** `nebula-error` (features `["derive"]`, used **only** for `Classify` in
  `src/error.rs`), `nebula-validator-macros` (path `macros`, optional behind the `derive`
  feature); external — `thiserror`, `smallvec`, `regex`, `serde`, `serde_json`, `num-cmp`,
  `tracing`.
- **Depended on by:** `nebula-schema`, `nebula-sdk`, `nebula-api` (all path dependencies).
- **Features:** `default = derive + network + temporal`.

## 4. Internal architecture

- `foundation/` — the `Validate`/`ValidateExt`/`Validatable` traits, `AnyValidator`,
  `AsValidatable` conversions, `ValidationError` (+ codes/severity/mode/pointer split into
  `error/`), `FieldPath`. No nested prelude: the crate-level `prelude` is the single import
  surface.
- `combinators/` — `And`/`Or`/`Not`/`When`/`Unless`/`Each`/`Field`/`MultiField`/`JsonField`/
  `Lazy`/`WithMessage`/`NestedValidate`/`OptionalNested`/`CollectionNested`/`Optional`/
  `AllOf`/`AnyOf`.
- `validators/` — built-ins by category: length, pattern, content, range, size, boolean,
  nullable, network (cfg), temporal (cfg).
- `rule/` — the `Rule` arena plus `value`/`predicate`/`logic`/`deferred`/`context`/`pattern`,
  bounded manual deserialization, constructors, and helpers.
- `engine.rs` — `validate_rules` / `validate_rules_with_ctx` plus `ExecutionMode` and
  `EvaluationOutcome`.
- `policy/` — the `When(Rule)` engine for visibility/required conditions; typed verdicts
  instead of a bare `bool`.
- `proof.rs` — the `Validated<T>` proof token (canon §4.5).
- `error.rs` — `ValidatorError` (operational), kept distinct from the inbound
  `ValidationError`.
- `macros.rs` — the `validator!` macro. The module is private, but the macro itself is
  `#[macro_export]`ed, so downstream crates see it at the crate root. Composition is the
  `.and()` / `.or()` methods from `ValidateExt`.
- `macros/` — the `nebula-validator-macros` subcrate: `parse/` → `model.rs` → `emit/` for
  `#[derive(Validator)]`.

**Data flow (declarative path):** a schema carries `Rule` values →
`validate_rules(_with_ctx)` selects which categories to execute from `ExecutionMode` →
`Rule` dispatches through `RuleView` onto the matching inner surface → the result is
`Result<EvaluationOutcome, ValidationErrors>`. The programmatic path:
`Validate<T>::validate` (plus combinators) → optionally `Validated<T>`.

## 5. Invariants and contracts

- **[L1-§4.5] proof token by construction.** `Validated<T>` cannot be obtained without
  calling `validate`; `Deserialize` is **deliberately not implemented** for it — deserialized
  data must be re-validated.
- **Rule cross-kind safety.** Each inner kind (`ValueRule`, `Predicate`, `DeferredRule`) is
  reachable only through the `RuleView` variant that makes sense for it; calling a value-only
  operation on a predicate-carrying `Rule` is a compile error. This replaces the old flat
  enum's silent-pass ergonomics by construction. Seam: `src/rule/mod.rs`.
- **[L1-§3.5] schema delegation.** `nebula-schema` executes field rules through this crate;
  `resolve_field_policies` (`src/policy/mod.rs`) is the **single** entry point for
  `nebula-schema::validate` (visibility/required), and its verdicts are typed.
- **ADR-0052 P2 — the only `required` emitter.** Required errors are emitted only by the
  validator (through the policy engine), never independently by each layer.
- **The wire format is frozen.** `Rule` serializes as externally-tagged tuple-compact;
  changing the encoding breaks stored rules. Error codes are frozen by the fixture
  (`tests/fixtures/compat/error_registry_v1.json`) and guarded by adversarial contract tests.
- **Deserialization is bounded.** `Rule` enforces depth, node, operand, JSON-node, JSON-depth,
  and text budgets before an instance can exist.

## 6. Known tensions / debt

1. **Duplicate `ValidationError`.** This crate defines its own `ValidationError`
   (`src/foundation/error/validation_error.rs`), duplicating the canonical
   `nebula-error::ValidationError`. Unification **is done** on
   `origin/refactor/error-unify-validation` but **is not merged** — `main` still defines the
   local type, and `nebula-error` is pulled in only for `Classify`. The branch is stale
   (hundreds of commits behind `main`), so merging it is a project, not a fast-forward.
2. **Three-prelude sprawl — resolved.** `foundation::prelude` and `combinators::prelude`
   were removed; `crate::prelude` is the single import surface.
3. **Wire format and codes are a compatibility obligation.** Externally-tagged tuple-compact
   plus the frozen `error_registry_v1.json` mean any serialization or code change is breaking
   for stored rules.
4. **Crate-wide `#![allow(clippy::result_large_err)]`** — deliberate, because the 80-byte
   error travels by value on every validation call.
5. **Error-tree traversals are recursive — bounded at construction.** `kind`,
   `total_error_count`, `flatten`, `to_json_value`, `Display`, and the derived
   `Clone`/`PartialEq`/`Debug`/`Drop` impls all recurse once per nesting level, so the tree is
   capped at `MAX_ERROR_TREE_DEPTH` (64) when `with_nested*` builds it. Diagnostics below the
   ceiling are dropped and the count is recorded as a `nested_errors_omitted` param on the
   node where the cut happened. Contract coverage:
   `tests/contract/error_tree_bounds_test.rs`.

## 7. Role in the post-0092 credential/resource model

The crate is **not an artifact** of the ADR-0092 consolidation: it has no dependencies on
`nebula-credential`/`nebula-crypto`/`nebula-resource`, and those crates do not depend on it
directly. Its participation in the new model is **indirect, through `nebula-schema`**, and
unchanged:

- **Credential write path (ADR-0052 P4).** The unified `nebula-credential` validates `data`
  **before** persisting it. The call travels `nebula-credential` → `nebula-schema::validate`
  → `nebula-validator`. The validator therefore remains the **only `required` emitter** (P2)
  for credential data too — a seam the redesign does not move.
- **Values-only persistence + `HasSchema`.** In the post-0092 model, slots (`slot_bindings`)
  and parameters are separated, and values are stored without their schema; the schema is
  reconstructed from registered types through `HasSchema → nebula-metadata → API catalog`.
  The validator executes the rules of that reconstructed schema — the **execution side of the
  same seam** — without knowledge of credential/resource topology (`CredentialSelector`,
  leases, rotation fan-out in `nebula-resource`, the `RefreshTransport` seam all live outside
  the validator).
- **What stays.** `Rule` cross-kind safety, the `Validated<T>` proof token, the
  visibility/required policy engine, and the `Rule` wire format are all stable and survive the
  redesign unchanged. `nebula-resource`'s `SlotCell`/`Manager`/topology do not intersect with
  the validator.
- **What changes.** Only crate topology: once everything collapses behind the sole public
  `nebula-sdk`, the validator becomes a **private implementation detail** (`sdk` already
  depends on it). There is then no external semver obligation on its API, which relieves some
  pressure on items 1–5 in §6 (error unification, deprecated-macro removal, prelude
  collapse can be internal breaking changes).

## 8. Forward design / open questions

- **Merge the error unification.** Bring `origin/refactor/error-unify-validation` into `main`:
  drop the local `ValidationError` in favour of `nebula-error::ValidationError`. That removes
  the only reason to pull in `nebula-error` (currently only for `Classify`) and closes tension
  #1. Risk: it touches RFC 6901 paths and the contract fixtures — `error_registry_v1.json`
  must be reconciled, and the branch must be rebased across hundreds of commits.
- **Decide the wire format's fate before durable growth.** Externally-tagged tuple-compact
  plus frozen codes is an obligation to stored rules; plan any `Rule` evolution as a versioned
  migration (analogous to the versioned envelope in `nebula-crypto`), never a silent encoding
  change.
- **Open question.** Should the validator know about `PredicateContext` extensions for
  credential data (for example, cross-field rules over a schema reconstructed from
  `HasSchema`), or does that stay entirely `nebula-schema`'s responsibility? Resolve this
  before extending the predicate surface.
