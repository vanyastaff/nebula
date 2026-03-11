# Stable Core vs Optional Extension Boundary

Defines which parts of `nebula-validator` are **contractual** (stable, depended
upon by other crates) versus **extension** (may change, experimental, or
domain-specific).

---

## Boundary Summary

```
┌─────────────────────────────────────────────────────────┐
│                    STABLE CORE                          │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Traits:  Validate<T>, ValidateExt<T>,            │  │
│  │           Validatable                             │  │
│  │  Errors:  ValidationError, ValidationErrors,      │  │
│  │           ErrorSeverity, ValidatorError            │  │
│  │  Proof:   Validated<T>                            │  │
│  │  Type-erasure: AnyValidator<T>                    │  │
│  └───────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Combinators (core):                              │  │
│  │    And, Or, Not, When, Unless, Optional,          │  │
│  │    Cached, Lazy, Each, JsonField                  │  │
│  │    WithMessage, WithCode                          │  │
│  │    AllOf, AnyOf (from factories)                  │  │
│  └───────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Validators (all built-in):                       │  │
│  │    length, content, pattern, range, size,         │  │
│  │    boolean, nullable, network, temporal            │  │
│  └───────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Macros: validator!, compose!, any_of!            │  │
│  └───────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────┤
│                    EXTENSION ZONE                       │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Experimental:                                    │  │
│  │    MultiField, NestedValidate,                    │  │
│  │    CollectionNested, OptionalNested               │  │
│  └───────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Internal (hidden):                               │  │
│  │    ErasedValidator, AsValidatable,                │  │
│  │    macro @-arms plumbing                          │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

---

## Stability Tiers

### Tier 1: Stable Core (contract)

Items in this tier are covered by backward compatibility guarantees. Changes
require migration docs ([MIGRATION.md](MIGRATION.md)) and are major-version
candidates.

| Module | Items | Guarantees |
|--------|-------|------------|
| `foundation` | `Validate<T>` | Trait signature frozen |
| `foundation` | `ValidateExt<T>` | `.and()`, `.or()`, `.not()`, `.when()` frozen |
| `foundation` | `Validatable` | `.validate_with()` frozen |
| `foundation` | `ValidationError` | Fields: `code`, `message`, `field` frozen. 80-byte layout enforced by CI |
| `foundation` | `ValidationErrors` | `.push()`, `.into_result()`, `.is_empty()` frozen |
| `foundation` | `ErrorSeverity` | Enum variants frozen (`#[non_exhaustive]` allows additions) |
| `foundation` | `ValidationMode` | `FailFast` / `CollectAll` variants frozen (`#[non_exhaustive]`) |
| `foundation` | `FieldPath` | Typed RFC 6901 JSON Pointer — `.parse()`, `.segments()`, `.push()`, `.append()` frozen |
| `foundation` | `AnyValidator<T>` | `.into_any()` conversion frozen |
| `proof` | `Validated<T>` | `.into_inner()`, `.as_ref()`, construction via `validate_into()` only |
| `error` | `ValidatorError` | Enum variants frozen (`#[non_exhaustive]`) |
| `validators::*` | All 60+ validators | Error codes frozen (enforced by `error_registry_v1.json`) |
| `combinators::*` | Core 12 types | Composition semantics frozen (laws tested) |
| `combinators::field` | `MultiField`, `Field` | Stabilized — `with_mode()` for ValidationMode support |
| `combinators::nested` | `NestedValidate`, `OptionalNested`, `CollectionNested` | Stabilized — `with_mode()` for ValidationMode support |
| `combinators::nested` | `SelfValidating` | Trait for self-validating types — `check()` method frozen |
| `combinators::each` | `Each` | `with_mode()` for ValidationMode support |
| `combinators::factories` | `AllOf` | `with_mode()` for ValidationMode support |
| macros | `validator!`, `compose!`, `any_of!` | All 5 variants stable |

**Consumer contract**: if you depend on items from this tier, your code will not
break on minor version upgrades.

### Tier 2: Extension (experimental)

Items in this tier may change without migration docs. They are behind the
`pub mod` boundary but not re-exported in the prelude.

| Module | Items | Status |
|--------|-------|--------|
| (none currently) | — | All previously experimental items promoted to Tier 1 in Phase 5 |

**Consumer contract**: use these at your own risk. Pin to exact versions if
depending on their API. Report bugs to help stabilize.

### Tier 3: Internal (hidden)

Items in this tier are implementation details. Do not depend on them.

| Module | Items | Purpose |
|--------|-------|---------|
| `foundation` | `ErasedValidator` (sealed trait) | Enables type-erased `AnyValidator<T>` |
| `foundation` | `AsValidatable` | JSON Value → &str bridge for `validate_any()` |
| `macros.rs` | All `@`-prefixed arms | Internal code generation helpers |

**Consumer contract**: these items may be renamed, removed, or restructured
at any time without notice.

---

## What Each Downstream Crate Depends On

| Consumer | Depends on (Tier 1) | Notes |
|----------|---------------------|-------|
| `nebula-api` | `Validate<T>`, `ValidationError` | `ValidatedJson<T>` extractor |
| `nebula-config` | `Validate<T>`, `min_length`, `email`, `matches_regex` | Config schema validation |
| `nebula-sdk` | `Validate<T>`, `ValidateExt<T>`, `ValidationError`, `ValidationErrors` | Re-export for plugin authors |
| `nebula-parameter` | `matches_regex` (pattern matching) | Conditional parameter validation |

All downstream consumers depend exclusively on Tier 1 items.

---

## Adding New Items

### To Stable Core (Tier 1)

New validators are added directly to Tier 1 because they follow the established
pattern:

1. Define via `validator!` macro
2. Register error code in `error_registry_v1.json`
3. Add to crate prelude
4. Add compatibility fixture test
5. Declare benchmark budget if in hot path

### To Extension (Tier 2)

Experimental features start in Tier 2:

1. Implement in appropriate module
2. Do NOT add to prelude
3. Document as "Experimental" in the module doc comment
4. Add tests but no compatibility fixtures

### Promotion: Tier 2 → Tier 1

Requirements for promotion to stable core:

- [ ] At least 2 consumers use the item in production code
- [ ] API has been stable for at least 1 minor release cycle
- [ ] Full test coverage including compatibility fixtures
- [ ] Error codes registered (if applicable)
- [ ] Benchmark budget declared (if in hot path)
- [ ] Added to prelude
- [ ] Documented in [API.md](API.md) stable surface

---

## Feature Flags

Currently no feature flags gate functionality. The full crate is always available.

If a schema/policy layer is added in the future (see [SCHEMA_EVALUATION.md](SCHEMA_EVALUATION.md)),
it should be behind an optional feature flag to keep the core dependency footprint minimal:

```toml
[features]
default = []
schema = ["schemars"]  # hypothetical future feature
```

---

## References

- [API.md — Stability Tiers](API.md)
- [ARCHITECTURE.md — Module Map](ARCHITECTURE.md)
- [MIGRATION.md — Breaking Change History](MIGRATION.md)
- [SCHEMA_EVALUATION.md — Schema Layer Decision](SCHEMA_EVALUATION.md)
- [PATTERNS.md — Consumer Usage Patterns](PATTERNS.md)
