# Changelog

All notable changes to `nebula-schema` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking Changes: Data Foundation

- **Root shapes are explicit.** `RootShape` is the sole structural contract;
  scalar roots have checked type/range descriptors rather than synthetic fields
  or `Any` fallbacks. `schema_of::<()>()` and unit-struct derives now describe
  JSON `null`; `ValidSchema::empty()` continues to describe an empty object.
  Primitive schemas retain their known types and bounds. Existing serialized
  empty records are not reinterpreted as unit schemas.
- **Assignability retains uncertainty.** `explain_assignable(&OutputSchema,
  &InputSchema)` and `OutputSchema::explain_successor_of` return `Yes`, `No`, or
  `Unknown`. The binary `is_assignable_schema` and
  `is_compatible_successor_of` APIs are removed. Only `Yes` is a static proof;
  accepting `Unknown` must be an explicit consumer policy. Empty records are
  not universal consumers, and unknown producers do not prove concrete types.
- **One phase-indexed value tree.** `ValueTree<E>` replaces `FieldValue` and
  `FieldValues`, without compatibility aliases. `AuthoredValue` contains
  `Expression`, `CompiledValue` contains retained `CompiledProgram`s, and
  `ResolvedValue` uses `Infallible` to make expressions uninhabited. `Literal`
  contains checked `ScalarValue`, never a JSON object or array; mode envelopes
  are ordinary `Object` nodes with `mode` and optional `value` properties.
- **Data keys and paths are separate from schema identifiers.** Object keys
  are arbitrary strings. `get` is exact-key lookup, `get_path` uses RFC6901
  `ValuePath`, and `insert` returns a checked result. `FieldKey` and `FieldPath`
  remain declaration identifiers. Data diagnostics and pending obligations now
  use JSON Pointers, including empty keys and escaped slash/tilde segments.
- **Literal ingestion and authoring are distinct.** `from_data` never interprets
  templates or `$expr` objects. `from_template_json` explicitly enables authoring
  shorthand using AUTO compilation. Evaluator output is always literal data and
  is never reparsed. `Expression::template(source)` explicitly selects always-string
  interpolation; `Expression::with_syntax(source, ProgramSyntax)` selects AUTO,
  raw EXPRESSION, or TEMPLATE. `syntax()` preserves intent in retained programs.
- **Authored serde is wire v2.** The exact `{version, data, expressions}` envelope
  keeps `{path, syntax, source}` entries in a separate RFC6901 table targeting null
  placeholders, for example `{"path":"/message","syntax":"template","source":"{{ 7 }}"}`.
  Syntax is required (`auto`, `expression`, or `template`), without a default.
  Strict decoding rejects duplicate/unknown fields and data keys,
  unsupported versions, invalid/overlapping slots, and excess depth. Only authored
  values deserialize; serialization rejects explicit secrets in every phase.
  Redacted JSON views and schema output projections are not this wire format.
- **Canonical identities remain separately versioned.** Tree canonical bytes
  use version 2; expression equality, content IDs, and keyed commitments include
  authored syntax and exact source. `canonical_json_v1` preserves the existing raw-JSON v1 bytes for
  durable identities; it does not redact or interpret data. Historical schema
  encodings remain version 1; new scalar descriptors require explicit support
  in persisted plan envelopes. No durable v1 identity migration is implied.
- **Validation consumes and prepares authored input.**
  `ValidSchema::validate(AuthoredValue)` folds every read alias, applies transforms
  once, promotes declared string secrets before returning, and retains admitted
  compiled programs. `ValidValues` is bound to its schema snapshot and carries
  explicit `PendingValidation` obligations, not a complete runtime proof.
- **Resolution consumes proof and checks full runtime policy.**
  `ExpressionContext::evaluate` accepts `&CompiledProgram`, replacing
  `ExpressionAst`. Newly evaluated subtrees are prepared once without reapplying
  transforms to literal siblings. `resolve_data` completes data-only input with
  no engine, rejects programs, and still checks full rules and conditional policies.
- **Typed extraction has an explicit secret boundary.** `into_typed<T>` refuses
  secret-bearing values. `into_typed_exposing_secrets<T>` explicitly transfers
  plaintext to a trusted target while preserving union wire tagging, without an
  intermediate plaintext `serde_json::Value::String`; sensitive projection applies
  aliases and nested field/mode schemas recursively, and target decode failures retain
  only a redacted cause. Derived `#[field(secret)]` leaves must explicitly implement
  `SecretInput: DeserializeOwned + ZeroizeOnDrop`, including the inner leaf of
  `Option<T>`; bare `String` and unmarked wrappers fail to compile. Predicate and
  schema-bound loader contexts scrub nested secrets, alias inputs, unknown mode
  payloads, and expression sources; predicate arrays retain leaf opacity.
- **Schema discovery is fallible.** `HasSchema::schema` and `schema_of::<T>()`
  return `Result<ValidSchema, ValidationReport>`. Derived implementations cache
  success or construction failure instead of panicking on an invalid schema.
- **Regex transformers are checked configuration.** Use
  `Transformer::regex(pattern, group) -> Result<Transformer, ValidationError>` or
  `RegexCapture::new`; `Regex(RegexCapture)` replaces the public string/cache
  fields. Construction and serde reject invalid patterns and capture indices,
  with `transformer.invalid_pattern` and `transformer.invalid_capture_group`.
  String-only application, valid unmatched-input behavior, and serialized
  transformer metadata are preserved; invalid-pattern no-op warnings are removed.
- **Diagnostics retain private typed causes.** `ValidationError` has a boxed
  payload exposed through `code`, `path`, `severity`, `params`, and `message`
  getters. Expression, regex, and typed-decoding diagnostics redact payloads
  through the public source chain rather than exposing upstream error text.
- **Rules never validate a redaction marker as secret data.** Built-in value
  checks use a private zeroizing projection; predicates continue to receive
  scrubbed context. Protected composite errors retain codes and data paths but
  redact messages, params, and source chains. Secret uniqueness uses an
  ephemeral keyed index rather than pairwise comparisons.

### Historical Entries

The entries below describe earlier changes and their then-current APIs and
results. The data-foundation contract above supersedes earlier API guidance.

The 2026-04-28 quality-fixes pass (`refactor(schema)!:` + Phase 2-4 commits)
covers the full set of issues raised in the nebula-schema code review.

### ⚠ Breaking Changes

- **Assignability is direction-typed (ADR-0100 C15).**  `is_assignable_schema`
  and `explain_assignable` changed signature from `(&ValidSchema, &ValidSchema)`
  to `(&OutputSchema, &InputSchema)`, so transposing producer and consumer is now
  a compile error.  Callers wrap a `ValidSchema` with `OutputSchema::new` /
  `InputSchema::new` (or `.into()`).  Output-vs-output *evolution* moved to the
  new `OutputSchema::is_compatible_successor_of`.  `NodeIoSchemas` (in
  `nebula-workflow`) now carries `InputSchema` / `OutputSchema` instead of bare
  `ValidSchema`.  The newtypes are `#[repr(transparent)]` and serde-transparent —
  **the wire format is unchanged**.

- **KDF removed from the schema layer.**  `KdfParams`, `KdfError`, the
  `MIN_KDF_*` / `MAX_KDF_*` / `DEFAULT_KDF_OUTPUT_BYTES` constants,
  `KdfParams::hash_password`, and `SecretField::kdf(...)` are deleted, along
  with the `argon2` and `thiserror` dependencies.  Key derivation /
  password hashing is a credential-layer concern: `nebula-credential` already
  owns the AES-256-GCM + Argon2id pipeline (with per-record salting and AAD).
  The schema copy was a weaker (static-salt, no AEAD), zero-consumer duplicate
  whose synchronous Argon2 call ran on the async executor inside
  `ValidValues::resolve` (a worker-blocking hazard under load).  Secret string
  literals are still promoted to a zeroizing `SecretValue::String` during
  `resolve`; configure hashing on the credential side instead.
  `SecretValue::Bytes` / `SecretBytes` / `SecretWire` are unchanged (binary
  tokens and externally-hashed material still round-trip).

- **`ExpressionContext::evaluate` signature changed (T01).**  The trait no
  longer uses `#[async_trait::async_trait]`; impls now write
  ```rust
  fn evaluate<'a>(&'a self, ast: &'a ExpressionAst) -> EvalFuture<'a> {
      Box::pin(async move { ... })
  }
  ```
  where `EvalFuture<'a>` is a new public type alias for the boxed future.
  Drops the `async-trait` runtime dependency on the schema crate.

- **`Field::*::new` requires a pre-validated `FieldKey` (T02).**  Static
  keys: `Field::string(field_key!("name"))`.  Runtime keys: `Field::try_string(s)?`
  (new fallible alias, returns `ValidationError` instead of panicking).
  Same change applied to all 13 `Field::*` constructors plus the `FieldCollector`
  closure-DSL methods (`.string()`, `.secret()`, `.number()`, …) and the
  `ObjectBuilder::new` / `ListBuilder::new` constructors.  `~217` call sites
  migrated workspace-wide.

- **`FieldValues::set_raw` removed (T03).**  Use `try_set_raw(...)?` for
  fallible runtime input or `try_set_raw(...).expect("...")` in tests /
  migrations with literal keys.

- **`Loader<T>: PartialEq` removed (T05).**  The previous `always-true` impl
  violated the `PartialEq` contract.  Loaders are not value-comparable; if
  identity comparison is required, use `Arc::ptr_eq` on the inner handle.

### Added

- **`recursion_limit` STANDARD_CODE (T07).**  `FieldValues::from_json` and
  `try_set_raw` reject deeply-nested user JSON with the new
  `recursion_limit` code (`MAX_VALUE_DEPTH = 64`).  Closes a stack-overflow
  vector against adversarial wire payloads.

- **`MAX_SCHEMA_DEPTH` schema-tree depth cap.**  New public const
  (`= 64`, the schema-tree analogue of `MAX_VALUE_DEPTH`).
  `validate_index_limits` now rejects schemas nested beyond it with
  `schema.depth_limit` **before** the recursive lint passes run, so an
  over-deep schema (including one deserialized through `serde_json::from_value`,
  which has no streaming-parser recursion cap) cannot drive the lint/validate
  recursion into a stack overflow.  Replaces the previous implicit `u8::MAX`
  bound with an explicit, conservative one; schemas deeper than 64 were already
  unusable (their values cannot validate past `MAX_VALUE_DEPTH`).

- **`secret.default_forbidden` STANDARD_CODE (T18).**  Lint pass now
  hard-rejects `Field::Secret { default: Some(_) }`.  Symmetric with
  `SecretString::Deserialize` (which always errors) — secrets must
  originate from the resolve pipeline, never from wire JSON.

- **`audit-secret-expose` Cargo feature (T11).**  Off by default;
  `SecretString::expose` / `SecretBytes::expose` log at `tracing::trace!`
  by default (with `#[track_caller]` location), escalating to
  `tracing::debug!` when the feature is enabled.  Lets compliance/audit
  builds opt in to a per-call audit trail without flooding default logs.

- **`tracing::instrument` spans on every hot-path entry point (T10).**
  Covers `ValidSchema::validate`, `ValidValues::resolve`,
  `ValidSchema::json_schema`, `LoaderRegistry::load_options` /
  `load_records`, and `lint::lint_tree`.
  All emit structured fields (field counts, mode flags, error counts).

- **Per-crate `clippy.toml` (T17).**  `crates/schema/clippy.toml` bans
  `async_trait::async_trait` for this crate; `#![deny(clippy::disallowed_macros)]`
  in `lib.rs` escalates to error.  Other crates inherit the workspace
  default.

- **Compile-time `#[derive(Schema)]` conflict detection (T04).**  Three
  new compile-fail tests catch known invalid attribute combinations at
  expansion time instead of runtime: `secret + default`,
  `secret + multiline`, `no_expression + expression_required`.  Generated
  code also emits a `tracing::error!` event with the structured
  `ValidationReport` before any remaining runtime panic so failures are
  visible in logs even when the panic is caught.

### Fixed

- **`first_duplicate_index` no longer falls back to a Debug-formatted
  bucket key (T13).**  `serde_json::Value::to_string` is infallible for
  valid JSON; the previous fallback could produce false-positive
  `items.unique` reports for values whose Debug shapes happened to match.

- **`ExpressionMode` JSON Schema export is symmetric (T14).**  Every
  property now carries `x-nebula-resolved-value-schema` regardless of
  Forbidden / Allowed / Required mode.  UI consumers no longer need to
  branch on mode to find the post-resolution value schema.

- **`FieldKey` "mode"/"value" interned via `LazyLock` (T08).**  Removes ~6
  per-call `FieldKey::new("mode" / "value").expect(..)` allocations on
  every validate / resolve / promote-secrets recursion through a
  `Field::Mode` variant.

- **`SecretBytes::Drop` no longer double-zeroizes (T09).**  `Zeroizing<Vec<u8>>`
  already zeroes the heap buffer in its blanket Drop; the manual impl
  was redundant.

- **`resolve()` skips post-resolve revalidate when nothing changed (T12).**
  Schemas with no expression-bearing fields no longer pay the cost of
  a second full schema walk.  ~50 % wall-clock saving on the static-only
  fast path.

- **Doc rot.**  README pointers to deleted `docs/INTEGRATION_MODEL.md`,
  `docs/MATURITY.md`, `docs/PRODUCT_CANON.md`, the `docs/adr/0001-..0003-...`
  set, and `docs/adr/0034-..` (in `secret.rs`) replaced with inline
  pointers and references to the workspace `ARCHITECTURE.md`.
  `cargo doc -p nebula-schema --no-deps` is now warning-free.

### Deferred

- **T15 — moving `crates/schema/examples/` into a root-level `examples/`
  workspace member.**  Discovery during execution surfaced ~10 sibling
  crates with the same per-crate `examples/` shape; this is a
  workspace-wide migration that warrants its own plan rather than being
  done piecemeal in a `nebula-schema` PR.
