# Decisions

## D001: Type-bound validation as primary API

Status: Adopt

Context:
- Rust platform needs compile-time safety and low runtime ambiguity.

Decision:
- keep `Validate<T>` + trait bounds as primary contract.

Alternatives considered:
- dynamic schema-only validation as primary model.

Trade-offs:
- stronger safety, but more generic type complexity.

Consequences:
- robust refactoring and fewer runtime type errors.

Migration impact:
- none.

Validation plan:
- compile-time tests + integration tests across consuming crates.

## D002: Combinator-first composition

Status: Adopt

Context:
- workflows require reusable rule pipelines.

Decision:
- compose via `and/or/not/when/unless/optional/field/json_field/cached`.

Alternatives considered:
- custom monolithic validators per use-case.

Trade-offs:
- excellent reuse, but bigger types and more compile-time cost.

Consequences:
- expressive and maintainable validation graphs.

Migration impact:
- none.

Validation plan:
- combinator law tests + behavior regression tests.

## D003: Structured error model with nested context

Status: Adopt

Context:
- downstream crates need machine-readable and human-readable diagnostics.

Decision:
- preserve `ValidationError` rich schema (code/message/field/params/nested/help/severity).

Alternatives considered:
- plain string errors.

Trade-offs:
- richer output with modest allocation overhead.

Consequences:
- easier API/UI mapping and debugging.

Migration impact:
- must keep code semantics stable.

Validation plan:
- serialization and compatibility tests for error payloads.

## D004: Error code registry governance

Status: Adopt (baseline)

Context:
- code stability rules require a single machine-readable source of truth.

Decision:
- adopt baseline canonical code catalog + compatibility fixtures now.
- publish canonical registry artifact:
  - `crates/validator/tests/fixtures/compat/error_registry_v1.json`
- enforce additive-only minor evolution for code registry.

Alternatives considered:
- immediate hard enforcement.

Trade-offs:
- faster current iteration vs delayed strictness.

Consequences:
- reduces accidental code drift in minor releases.
- governance overhead increases with fixture maintenance.

Migration impact:
- future minor-to-major planning required.

Validation plan:
- enforce through contract tests in `tests/contract/compatibility_fixtures_test.rs`.
- enforce registry integrity and documentation references in:
  - `tests/contract/governance_policy_test.rs`.
- require migration mapping in `MIGRATION.md` for behavior-significant changes.

## D006: Minor release compatibility policy

Status: Adopt

Context:
- downstream crates depend on validator behavior and diagnostics as a contract.

Decision:
- minor releases are additive only for validators/combinators/helpers.
- behavior-significant semantic changes require major release + migration map.

Alternatives considered:
- allowing silent semantic adjustments in minor releases.

Trade-offs:
- slower feature rollout, higher integration safety.

Consequences:
- release process requires explicit compatibility checks.

Migration impact:
- none for additive changes; required for major changes.

Validation plan:
- governance checks in `tests/contract/governance_policy_test.rs`.

## D005: FieldPath typed model

Status: Defer

Context:
- plain string field paths can drift in formatting.

Decision:
- evaluate typed `FieldPath` model after current docs/contract stabilization.

Alternatives considered:
- keep raw strings forever.

Trade-offs:
- typed path improves safety but may require API changes.

Consequences:
- maintain current compatibility now; prepare for eventual major upgrade.

Migration impact:
- likely breaking if introduced broadly.

Validation plan:
- prototype in proposal + adapter compatibility layer.

## D007: Shared category baseline with config crate

Status: Adopt

Context:
- config lifecycle now depends on validator outcome categories for compatibility checks.

Decision:
- keep shared category names stable and publish constants in validator core.
- changes to category semantics require major version + migration mapping.

Alternatives considered:
- leave categories implicit in docs only.

Trade-offs:
- stronger compatibility guarantees with added governance overhead.

Consequences:
- cross-crate fixtures can enforce deterministic mapping behavior.

Migration impact:
- category drift requires explicit old->new mapping.

Validation plan:
- `crates/config/tests/contract/validator_category_compatibility_test.rs`.

## D008: RFC6901 pointer canonicalization in ValidationError

Status: Adopt

Context:
- consuming crates (`config`, `parameter`, `api`) need one stable path format for machine handling.

Decision:
- canonicalize field paths to RFC6901 JSON Pointer in `ValidationError`.
- `with_field(..)` normalizes dot/bracket notation to pointer.
- expose pointer-native APIs: `with_pointer(..)` and `field_pointer()`.

Alternatives considered:
- preserve mixed path formats and normalize only in adapters.

Trade-offs:
- small behavioral break in `field` representation in exchange for deterministic cross-crate contract.

Consequences:
- adapters can consume one canonical format without per-crate path heuristics.

Migration impact:
- consumers reading `field` directly must accept pointer format.

Validation plan:
- unit tests in validator error module + config/parameter/api integration checks.

## D009: Unified validator contract at API boundary

Status: Adopt

Context:
- `nebula-api` had a parallel local validation trait/string errors, diverging from validator core.

Decision:
- use `nebula_validator::foundation::Validate<T>` in API extractor path.
- convert `ValidationError`/`ValidationErrors` into structured RFC9457 problem details extensions.

Alternatives considered:
- keep local API trait and adapt only at handler boundaries.

Trade-offs:
- breaking change for API internals, with improved DX and one contract across crates.

Consequences:
- consistent code/message/pointer propagation from validator to HTTP responses.

Migration impact:
- API users implementing custom local `Validate` must move to validator trait.

Validation plan:
- API unit tests for conversion and nested error mapping.

## D010: Panic-free regex handling in derive macros

Status: Adopt

Context:
- invalid regex in derive attributes previously caused runtime panic paths.

Decision:
- generated code now emits structured validation errors (`invalid_regex_pattern`) instead of panicking.

Alternatives considered:
- keep panic behavior.

Trade-offs:
- failures become recoverable and diagnosable; slight extra generated code.

Consequences:
- safer macro DX and predictable failure handling.

Migration impact:
- callers that relied on panic behavior must handle regular validation errors.

Validation plan:
- macro crate tests + cross-crate regression runs.

## D011: Proof token system (`Validated<T>`)

Status: Adopt

Context:
- validated data could silently flow through the system without any compile-time
  guarantee that it had passed validation, creating trust boundary bugs.

Decision:
- introduce `Validated<T>` proof token: a zero-cost newtype wrapper that can only
  be constructed through validated code paths (`Validate::validate_into`,
  `Validated::new`). Read access via `Deref`/`AsRef`/`Borrow`; ownership recovery
  via `into_inner()`. `Serialize` is derived; `Deserialize` is intentionally omitted
  to force re-validation on deserialization boundaries.

Alternatives considered:
- phantom-marker trait bounds (too invasive across call sites).
- convention-only "validated" naming (no compile-time enforcement).

Trade-offs:
- adds a wrapper type to call sites that need the guarantee; zero runtime cost.

Consequences:
- trust boundaries become visible in the type system.
- downstream crates can require `Validated<T>` in function signatures.

Migration impact:
- none; purely additive. Existing code using `validate()` directly is unaffected.

Validation plan:
- unit tests in `proof.rs` covering construction, deref, map, serialize, and combined validators.

## D012: Crate-level operational error type (`ValidatorError`)

Status: Adopt

Context:
- the crate had no dedicated operational error type; `ValidationError` served double
  duty for both validation failures and configuration errors.

Decision:
- introduce `ValidatorError` (via `thiserror`, `#[non_exhaustive]`) with variants:
  `InvalidConfig` (configuration errors) and `ValidationFailed` (wraps `ValidationError`).
- define `ValidatorResult<T> = Result<T, ValidatorError>` alias.

Alternatives considered:
- continue using `ValidationError` for everything.

Trade-offs:
- cleaner separation of concerns; one more error type to learn.

Consequences:
- `validate_into` and `Validated::new` return `ValidatorResult` instead of
  `Result<_, ValidationError>`, giving callers richer error context.

Migration impact:
- none; purely additive. Existing `validate()` still returns `Result<(), ValidationError>`.

Validation plan:
- compile-time checks + integration with proof token tests.

## D013: Combinator type deduplication

Status: Adopt

Context:
- `And`, `Or`, `Not`, `When` were defined in both `foundation/traits.rs` (inline)
  and `combinators/*.rs` (dedicated modules), creating two distinct types with the
  same name. `ValidateExt::and()` returned `foundation::And` while the free function
  `and()` returned `combinators::And`.

Decision:
- remove duplicate definitions from `foundation/traits.rs`.
- `ValidateExt` methods now import and return types from `combinators/`.
- canonical definitions live exclusively in `combinators/and.rs`, `or.rs`, `not.rs`, `when.rs`.

Alternatives considered:
- keep both and add `From` conversions (unnecessary complexity).

Trade-offs:
- minor internal refactor; no public API change since both were re-exported under the same name.

Consequences:
- single source of truth for each combinator type.
- simpler module dependency graph.

Migration impact:
- none for external consumers; types resolve to the same names.

Validation plan:
- all 293+ existing tests pass without modification.
