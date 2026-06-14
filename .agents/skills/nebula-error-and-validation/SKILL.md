---
name: nebula-error-and-validation
description: Use when adding or changing error types, returning Results, classifying retryability, handling validation errors, or replacing unwrap/expect/panic in library code.
---

# nebula-error-and-validation

**When to use:** Defining a new error variant, deciding error vs panic, adding
retry classification, surfacing validation failures, or anytime the no-unwrap
guard hook fires. Load before writing error-handling code in any lib crate.

Procedure — follow top to bottom. Every rule below is grounded in a real file;
paths are cited inline. Verify against the cited file if anything looks stale.

## 0. Pick the error vocabulary (lib vs bin)

- **Library crate → `thiserror` + `Classify` + `NebulaError`.** Never
  stringly-typed errors, never `anyhow`. (`crates/error/README.md` §Contract
  `[L2-§12.4]`; `crates/error/AGENTS.md`.)
- **Binary → `anyhow` is allowed**, and only there. (Same contract.)
- A typed variant per failure mode, not a single `Other(String)` catch-all.
  `nebula-error` ships prebuilt detail structs (`BadRequest`, `FieldViolation`,
  `ResourceInfo`, `RequestInfo`, `ExecutionContext`, `DebugInfo`) in
  `crates/error/src/detail_types.rs` — attach those instead of formatting prose
  into a message.

## 1. No `unwrap()/expect()/panic!()/todo!()/unimplemented!()/unreachable!()` in library code

Enforced by `edit-guard.sh` (root `AGENTS.md` §Enforced Discipline) — not
advisory. If the guard fires, do not work around it; fix the code.

Exempt contexts, per `clippy.toml`:

- **Tests** — `allow-unwrap-in-tests`, `allow-expect-in-tests`,
  `allow-panic-in-tests`, `allow-dbg-in-tests`, `allow-print-in-tests`,
  `allow-indexing-slicing-in-tests` (lines 53-59).
- **`const` context** — `allow-unwrap-in-consts`, `allow-expect-in-consts`
  (lines 64-65).
- **Binaries** — use `anyhow` (`?` + `.context(...)`), not `unwrap`.

Replacement order for a fallible expression in lib code:

1. Return a typed error and propagate with `?` (preferred).
2. `let Some(x) = opt else { return Err(...) }` / `match` — `clippy.toml`
   `matches-for-let-else = "WellKnownTypes"` (line 124) nudges this.
3. Last resort, with a justification: put
   `// guard-justified: <reason>` on the line directly above the construct.
   This is the **only** escape for the discretionary edit rules; there is no
   escape for lefthook-bypass, lint-suppression, or no-unwrap-as-a-class. The
   same `// guard-justified:` line is also required above any `#[allow]` /
   `todo!` / `unimplemented!` / `unreachable!` you must keep.

## 2. Make retryability explicit via `Classify`

Do not let transient-vs-permanent become folklore scattered per crate. Each
error type implements `Classify` (`crates/error/src/traits.rs`):

- `category()` → `ErrorCategory` (`Transient` / `Permanent` / `Internal` / …,
  `crates/error/src/category.rs`).
- `code()` → machine-readable `ErrorCode` (`crates/error/src/code.rs`).
- `severity()` → `ErrorSeverity` (`Error` / `Warning` / `Info`).
- `retry_hint()` → `RetryHint` — **the single transient-vs-permanent decision
  surface** (`crates/error/README.md` `[L1-§4.2]`, `crates/error/src/retry.rs`).

`RetryHint` is **data**, not execution. `nebula-resilience` consumes it; do not
re-implement classification inside resilience or any consumer. Use
`#[derive(Classify)]` (feature `derive`, from `nebula-error-macros`) where the
mapping is mechanical.

## 3. Wrap, don't log-and-discard

Wrap a `Classify` error in `NebulaError<E>` (`crates/error/src/error.rs`) to
attach typed details + a context chain, instead of logging the error and
returning a bare one:

- `Result<T, E>` is aliased to `std::result::Result<T, NebulaError<E>>`
  (`crates/error/src/lib.rs`) — use the alias.
- `Display` for `NebulaError<E>` **must** emit the full context chain
  (regression fixed in `0f047d32`, #405 — do not regress;
  `crates/error/README.md` `[L3-§12.4]`).
- Details are TypeId-keyed and extensible (`crates/error/src/details.rs`).
- `nebula-error` is **not** an API formatter (`nebula-api` maps to RFC 9457
  `problem+json`) and **not** a logger (`nebula-log`). Stay in lane
  (`crates/error/AGENTS.md` §Conventions).

## 4. Don't swallow errors on state mutations

`let _ = fallible(...)` on a state transition silently hides invalid-transition
bugs (e.g. `let _ = transition_node(...)` discarding an invalid-transition
error). Always propagate with `?` or handle the variant explicitly. A discarded
`Result` on a mutation is a latent durability bug, not a cleanup.

## 5. Validation: one canonical `ValidationError`, validator owns `required`

Validation is unified on a single structured type:

- `ValidationError` is the canonical structured error (80 bytes, `Cow`-based,
  RFC 6901 field paths), defined in
  `crates/validator/src/foundation/error/validation_error.rs` and re-exported
  through `nebula_schema::error` (`crates/schema/src/validated.rs` imports it).
  Do not define a second per-crate validation error type.
- **`nebula-validator` is the sole emitter of `required`** — the `REQUIRED`
  code (`crates/validator/src/foundation/error/codes.rs`,
  `pub const REQUIRED: &str = "required"`). Locked by
  `crates/schema/tests/seam_required_emitter.rs`; do not emit a `required`
  failure from schema or any other crate.
- **Schema maps serde-stable modes at validate time.** `nebula-schema` keeps
  serde-stable `VisibilityMode` / `RequiredMode` (`crates/schema/src/mode.rs`)
  and maps them at validate time. The condition-evaluation seam lives in
  `nebula-validator` (`policy` module / `resolve_field_policies`); the single
  schema↔validator crossing is `validate_rules_with_ctx` + `resolve_field_policies`
  (`crates/schema/AGENTS.md`).
- **No `Rule::evaluate` shim.** `Rule::evaluate` / `RuleContext` were removed,
  not shimmed (ADR-0080 §"Condition evaluation seam (absorbs 0052)").
- Rule-failure codes surface validator-native verbatim (`min_length`, `min`,
  `invalid_format`) — **no namespace remap** at the schema boundary
  (`crates/schema/AGENTS.md`).
- `Validated<T>` is a proof-token: never construct one without calling
  `validate`, and never add `Deserialize` to it — deserialized data must be
  re-validated (`crates/validator/README.md` `[L1-§4.5]`).

## 6. Pick the right retry layer (ADR-0068)

ADR-0068 — two disjoint layers, chosen by *who can
recover*:

- **Layer 1 — action-internal recovery → `nebula-resilience::retry_with`**
  (`crates/resilience/src/retry.rs`). Wrap an outbound call the action itself
  understands and can re-issue (transient `5xx`, `Retry-After`, DB reconnect,
  rate-limit cooldown). Stays in the action's source.
- **Layer 2 — whole-action failure → `NodeDefinition.retry_policy`**
  (engine node-level, `crates/workflow/src/node.rs`). For sandbox
  crash/panic/OOM, param-resolution failure, third-party plugin actions, or
  operator-declared policy — the engine re-dispatches.
- They compose by **timing**: Layer 1 runs first inside the action; if it
  exhausts its budget the action returns an error, and the engine then consults
  `retry_policy` (Layer 2). `ActionResult::Retry` was removed — do not
  reintroduce a third surface.

## 7. Observability is Definition of Done

Every new error variant ships with a tracing event, and every new state error
ships with an invariant check — not as follow-up (root `AGENTS.md` §Agent
Rules: "New state / error / hot path must ship with a typed error variant +
tracing span + invariant check"). See the observability skill for the span /
metric / log boundaries (`docs/OBSERVABILITY.md`).

## Quick checklist before you finish

- [ ] Lib crate uses `thiserror` + `Classify` + `NebulaError`; no `anyhow`, no
      stringly-typed errors.
- [ ] No `unwrap/expect/panic/todo/unimplemented/unreachable` in lib code
      (or each is `// guard-justified:`).
- [ ] New error type implements `Classify` with a real `retry_hint()`.
- [ ] No `let _ =` discarding a `Result` from a state mutation.
- [ ] Validation goes through the canonical `ValidationError`; only validator
      emits `required`; no `Rule::evaluate` shim; no code namespace remap.
- [ ] Retry placed in the correct layer (resilience vs node `retry_policy`).
- [ ] New variant has a tracing event; new state error has an invariant check.
