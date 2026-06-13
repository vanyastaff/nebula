# nebula-error — design

| Field | Value |
|-------|-------|
| **Status** | Stable — foundation crate (zero workspace deps) |
| **Layer** | Cross-cutting (leaf) |
| **Redesign role** | **Not structurally touched.** Stable vocabulary that credential/crypto/resource depend on; supplies the `Classify` / `RetryHint` words used to classify rotation/teardown transient-vs-permanent. |
| **Related** | PRODUCT_CANON §4.2, invariant L2-§12.4; in-flight `refactor/error-unify-validation` branch adds the canonical `ValidationError` (not in this worktree) |

---

## 1. Purpose & boundaries

The workspace's **error taxonomy**: one classification vocabulary
(category / code / severity / retryability) via the `Classify` trait, a generic
`NebulaError<E>` wrapper with `TypeId`-keyed details + a context chain, and
`RetryHint` as **data** consumed by `nebula-resilience`. It makes
transient-vs-permanent an **explicit decision** (ErrorClassifier pattern).

**Owns:** `Classify`, `NebulaError<E>`, `ErrorCategory`, `ErrorCode`,
`ErrorSeverity`, `RetryHint`, the `ErrorDetails` container + 13 Google/AWS-style
detail structs, `ErrorCollection`/`BatchResult`, and the `#[derive(Classify)]`
macro.

**Explicitly does NOT own:** retry *execution* (that is `nebula-resilience` — this
crate only carries the `RetryHint` data), domain error *variants* (each crate
defines its own and impls `Classify`), and — today — `ValidationError` (it lands
here only when the error-unify branch merges).

## 2. Public surface

| Item | Where |
|------|-------|
| `Classify` (trait; `category`/`code` required, rest defaulted) | `src/traits.rs:45` |
| `ErrorClassifier` (predicate over `ErrorCategory`) | `src/traits.rs:98` |
| `NebulaError<E: Classify>` (`new`/`with_*`/`context`/`map_inner`) | `src/error.rs:61` |
| `NebulaError::context_chain` / `details` / `source` | `src/error.rs:268,256,273` |
| `ErrorCategory` (14 variants, `#[non_exhaustive]`) + `is_default_retryable` | `src/category.rs:24,73` |
| `ErrorCategory::http_status_code` / `from_http_status` | `src/convert.rs:39,76` |
| `ErrorCode` (newtype `Cow<'static,str>`, `const fn new`) + `codes::*` | `src/code.rs:23,129` |
| `RetryHint` (`after` / `max_attempts` — data) | `src/retry.rs:23` |
| `ErrorDetail` marker + `ErrorDetails` (`TypeId`-keyed map) | `src/details.rs:31,52` |
| `ErrorCollection<E>` / `BatchResult<T,E>` | `src/collection.rs:47,269` |

## 3. Dependencies & dependents

- **Deps:** none from the workspace. Optional `serde`, optional
  `nebula-error-macros` (`derive` feature) — both off by default.
- **Dependents:** effectively the whole workspace (16 crates: action, api, core,
  credential, crypto, engine, execution, expression, log, metadata, metrics,
  plugin, resilience, resource, validator, workflow).

## 4. Invariants & contracts

- **[L2-§12.4]** Transient-vs-permanent is an explicit classification, never an
  inferred guess. `Classify::is_retryable` + `ErrorCategory::is_default_retryable`
  (Timeout | Exhausted | External | RateLimit | Unavailable) are the seam.
- **Display prints the full context chain** (regression-fixed #405) — a dropped
  chain link is a contract violation.
- `RetryHint` is **data, not behavior** — `nebula-resilience` decides whether and
  how to act on it.

## 5. Known tensions / debt (honest)

1. **Derive-macro doc drift.** `nebula-error-macros` docs list 12 categories
   ("…`unsupported`") but the parser also accepts `unavailable` and
   `data_too_large` (`macros/src/lib.rs:258-259`).
2. **Aspirational claim.** `convert.rs:4` promises "protocol bridges (gRPC, etc.)
   behind feature flags" — no such feature exists.
3. **`ValidationError` is not here yet** — only on the unmerged
   `refactor/error-unify-validation` branch; do not reference it as present.

## 6. Forward design

- When error-unify merges, the canonical `nebula_error::ValidationError` becomes
  the single validation-error type; credential's local `ValidationError`
  (ADR-0088 D7) is deleted onto it.
- API is stable; no breaking changes planned. Fix the two doc-drift items above as
  hygiene.
