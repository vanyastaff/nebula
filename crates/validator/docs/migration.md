# nebula-validator — Migration Guide

This document is the authority for all breaking changes, deprecation notices, and migration
paths in `nebula-validator`. It is referenced by the error code registry
(`tests/fixtures/compat/error_registry_v1.json`) and enforced by
`tests/contract/migration_requirements_test.rs`.

---

## Versioning Policy

**Minor releases** (`x.Y.z`) are additive only:
- New validators, combinators, and error helper constructors may be added.
- New error codes may be registered with `stability: "stable"`.
- No existing stable error code may be renamed, removed, or have its semantics changed.
- No field-path format contract may change.

**Major releases** (`X.0.0`) may introduce breaking changes, subject to the requirements below:
- Every breaking change must have an entry in the [Breaking Changes](#breaking-changes) table.
- Every removed or renamed API must have a migration action.
- No breaking change may ship without a migration mapping in this file.

**Deprecation window:** Deprecated items remain available for at least one minor release cycle
before removal. Removal happens in the next major release.

---

## Error Code Stability

Each code in `tests/fixtures/compat/error_registry_v1.json` carries a stability level:

| Level | Meaning |
|-------|---------|
| `stable` | Meaning and behavior will not change in minor releases. |
| `deprecated` | Scheduled for removal in the next major release. |

### Deprecation Process

1. **Mark** — add `#[deprecated(since = "x.y.z", note = "use X instead")]` to the item.
2. **Registry** — set `"stability": "deprecated"` in `error_registry_v1.json`.
3. **Document** — add an entry to the [Breaking Changes](#breaking-changes) table.
4. **Grace period** — the deprecated item remains for at least one major release cycle.
5. **Remove** — delete in the next major release and record the removal in this file.

---

## Field-Path Contract

All field paths stored in `ValidationError` are **RFC 6901 JSON Pointer** format:

| Input | Stored as |
|-------|-----------|
| `user.email` | `/user/email` |
| `items[0].name` | `/items/0/name` |
| `a/b` (literal slash) | `/a~1b` |
| `a~b` (literal tilde) | `/a~0b` |
| root | `""` (empty string) |

`ValidationError::with_field(path)` normalizes any dot/bracket input.
`ValidationError::with_pointer(pointer)` accepts an already-normalized pointer.
`field_pointer()` is the canonical accessor for downstream consumers.

The field-path format for a given validator/combinator pair is part of the minor-release
contract. Changing which format a combinator produces requires a major version bump.

---

## Breaking Changes

### Complete JSON Pointers

`FieldPath` now represents every RFC 6901 pointer, including root (`""`) and
empty-string object keys (`"/"`). `from_segments` is infallible and preserves all
segments: remove the old `unwrap`/fallback, and use `root()` for an empty path.
`parent()` of a single-segment path returns `Some(root())`; only root has no parent.
`starts_with` compares whole segments, with root a prefix of every path.

Serde uses strict `from_pointer(&str) -> Result<FieldPath, FieldPathError>`.
Dot/bracket aliases and URI fragments are rejected on the wire; explicitly use
`parse` for authored shorthand and serialize the resulting canonical pointer.
Pointer whitespace is significant and invalid tilde escapes are rejected.
The freeform diagnostic normalizer is unchanged: `with_field("")` still leaves
the field unspecified, while `with_field_path(FieldPath::root())` explicitly sets root.

### Staged Rule Evaluation

These active-development API breaks distinguish authored-value checks from complete
resolved-value evaluation. `serde_json::Value` remains the shared value model.

| Surface | Previous contract | Current contract |
|---------|-------------------|------------------|
| `Rule::validate`, `Logic::validate`, `validate_rules[_with_ctx]` | Unit success also meant skipped rules | `EvaluationOutcome::Satisfied` or `Deferred(Vec<DeferredReason>)` |
| `StaticOnly` | Silently skipped deferred rules and missing predicate context | Reports explicit remaining obligations through nested `All`, `Any`, `Not` |
| `Full` and `Validate<Value> for Rule` | Unavailable checks could pass and issue proofs | Require complete evaluation; unavailable checks produce diagnostics |
| `Rule::matches`, policy `resolve`, `resolve_field_policies` | Infallible conditions accepted wrong rule kinds | Return `Result`; value/deferred rules are invalid conditions |
| `Predicate::evaluate` | Infallible boolean over all contexts | `Result<bool, ValidationError>`; pending dependencies are unavailable |
| Authored policy dependencies | Unresolved paths looked missing | `Presence::Pending`, `Requiredness::Pending`, `FieldDirective::Deferred`; no early required failure |
| Pattern construction | Unchecked strings; separate `try_pattern` | `Rule::pattern(&str)` and `ValueRule::pattern(&str)` return `Result`; `RulePattern::new` checks direct construction |
| `ValueRule::Pattern`, `Predicate::Matches` | Raw pattern strings | Checked `RulePattern`; malformed serde is rejected before evaluation |
| Unit-rule map payloads | Arbitrary configuration silently discarded | Only `null` accepted, matching sub-enum serde; bare strings remain canonical |
| Empty `Rule::any` / `Rule::one_of` / `any_of` | Accepted every value | Reject every value |
| Mixed JSON number comparisons | Large integers rounded through `f64` | Exact comparison delegated to `num-cmp` |
| Diagnostics | All errors could be treated as input rejection | `kind()` and JSON `kind` distinguish `violation`, `invalid_rule`, `unavailable` |

An authored-stage caller handles `Deferred` as remaining work. Re-evaluate the
entire rule tree after resolution with the complete, appropriately scrubbed
`PredicateContext`; use `Full` and require `Satisfied` before issuing a complete
proof. `Custom` and `UniqueBy` currently have no runtime evaluator in this crate:
partial passes report obligations and full passes fail with `evaluation_unavailable`.

For authored root and field rules, attach unresolved expression roots with
`PredicateContext::with_pending_paths`. Pending overlaps both ancestors and
descendants, so a container containing unresolved data is unavailable too.
Other absent paths retain genuinely missing-value semantics. `None` is reserved
for an unavailable whole context. Build a fresh context without pending roots
after resolution.

Condition matching requires every dependency to be available, even beneath
`Any` or `Not`. Invalid condition configuration takes precedence over pending
dependencies. The policy resolver returns a deferred directive if visibility
or requiredness is pending, without emitting `required`; the caller records a
policy obligation and still validates any supplied literal value. Ordinary
rule validation retains partial logical evaluation and checks independent
static constraints while context-dependent branches are deferred.

`Any` needs a genuinely satisfied branch. `Not` preserves deferral and propagates
configuration/unavailable errors; it only inverts input violations. Programmatic
combinators retain short-circuiting and preserve non-violation errors when encountered,
including errors inside nested diagnostic aggregates.

```rust
use nebula_validator::{EvaluationOutcome, ExecutionMode, Rule, validate_rules};
use serde_json::json;

let rules = [Rule::pattern("^[a-z]+$")?];
match validate_rules(&json!("alice"), &rules, ExecutionMode::StaticOnly)? {
    EvaluationOutcome::Satisfied => {}
    EvaluationOutcome::Deferred(obligations) => {
        // The host retains these obligations until resolved-value evaluation.
        assert!(!obligations.is_empty());
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

### v0.x → Current

| Contract area | Old behavior | New behavior | Version | Consumer impact | Migration |
|---------------|-------------|-------------|---------|-----------------|-----------|
| Field-path format | Mixed dot/bracket notation stored verbatim | RFC 6901 JSON Pointer normalized by `with_field()` | v0.x | Consumers reading `field` as dot-notation must switch to `field_pointer()` or the `pointer` key in the JSON envelope | Replace `error.field` reads with `error.field_pointer()` |
| Error serialization | `field` key only in JSON envelope | Both `field` (raw) and `pointer` (RFC 6901) keys emitted | v0.x | Additive; consumers reading `field` are unaffected | Prefer `pointer` key for new consumers |
| API validation | `nebula-api` used a parallel local validation trait | Uses `nebula_validator::foundation::Validate<T>` directly | v0.x | Handlers implementing local `Validate` must migrate to the validator trait | Implement `nebula_validator::foundation::Validate<T>` |
| Regex errors in macros | `#[validate(regex = "...")]` panicked on invalid regex | Returns structured `ValidationError` with code `invalid_regex_pattern` | v0.x | Callers relying on panic behavior must handle a validation error | Match on `ValidatorError::ValidationFailed` |
| Combinator types | `ValidateExt::and()` returned `foundation::And`, free `and()` returned `combinators::And` | Both return `combinators::And` — single canonical type | v0.x | Code using both paths in the same generic bound would see a type mismatch | Remove `foundation::And` references; use `combinators::And` |

---

## Config Integration

Changes that affect `nebula-config` compatibility require an additional fixture update.

| Surface | Old | New | Impacted consumer | Required update |
|---------|-----|-----|-------------------|-----------------|
| Category constants | _(see registry)_ | _(see registry)_ | `nebula-config` | `tests/fixtures/compat/validator_contract_v*.json` |

---

## Rollback Guidance

If a release is rolled back due to contract breakage:

1. Revert to the previous stable tag.
2. Restore `error_registry_v1.json` and `minor_contract_v1.json` to their pre-release state.
3. Verify that `tests/contract/compatibility_fixtures_test.rs` passes against the reverted
   registry.
4. Ensure persisted validation error envelopes (e.g., stored in workflow execution logs)
   remain parseable by the reverted consumer code.

---

## Validation Checklist Before a Major Release

- [ ] Every breaking change has an entry in the table above.
- [ ] Every deprecated item has `#[deprecated]` in the source and `"stability": "deprecated"`
      in the registry.
- [ ] `tests/contract/migration_requirements_test.rs` passes.
- [ ] `tests/contract/governance_policy_test.rs` passes.
- [ ] `tests/contract/compatibility_fixtures_test.rs` passes against the new registry.
- [ ] Downstream crates (`nebula-config`, `nebula-api`, `nebula-parameter`) compile without
      deprecation warnings after applying the documented migrations.
