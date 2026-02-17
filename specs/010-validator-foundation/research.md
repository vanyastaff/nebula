# Research: Validator Foundation Restructuring

**Branch**: `010-validator-foundation` | **Date**: 2026-02-16

## R1: Cow<'static, str> Simplification in ValidationError

**Decision**: Keep `Cow<'static, str>` for all ValidationError fields.

**Rationale**: Analysis of actual usage shows that every field CAN receive runtime-constructed values:
- `code` — the `WithCode` combinator allows users to set dynamic codes
- `message` — commonly uses `format!()` for parameterized messages like "must be at least {min} characters"
- `field` — always runtime-constructed (e.g., "user.email", "items[0]")
- `params` — values are runtime (e.g., ("min", "5") where "5" comes from the validator state)
- `help` — sometimes dynamic

Changing to `&'static str` would break the `WithCode`/`WithMessage` combinators and make parameterized errors impossible. The current `Cow` approach is already optimal: static literals have zero allocation, dynamic strings use owned `String`.

**Alternatives considered**:
- `&'static str` for `code` only — breaks `WithCode` combinator
- `String` everywhere — worse performance for the common case (static error codes)
- `Arc<str>` — more complex, no benefit over Cow for this use case

## R2: GAT Simplification in AsValidatable

**Decision**: Keep current GAT pattern unchanged.

**Rationale**: The current `AsValidatable` trait using GAT is already the idiomatic Rust 2024 pattern:

```rust
pub trait AsValidatable<T: ?Sized> {
    type Output<'a>: Borrow<T> where Self: 'a;
    fn as_validatable(&self) -> Result<Self::Output<'_>, ValidationError>;
}
```

GATs (Generic Associated Types) are stable since Rust 1.65 and this pattern is the canonical way to express zero-copy type conversions. There is no simpler alternative in Rust 2024/2025 that achieves the same zero-copy semantics. The `for<'a>` bound in `validate_any()` is necessary and correct.

**Alternatives considered**:
- `impl Borrow<T>` return — can't express lifetime relationship
- Remove GAT, return `Box<dyn Borrow<T>>` — adds allocation, defeats purpose
- Trait alias — syntactic sugar only, doesn't simplify

## R3: Rust 2024 Edition Features Applicable

**Decision**: Practical modernization is limited to code patterns, not trait redesign.

**Rationale**: Analysis of Rust 2024 edition features vs. current crate code:

| Feature | Applicable? | Where |
|---------|------------|-------|
| `let chains` (if-let chains) | Already used | `evaluate_rule()` in parameter crate uses them |
| `async fn in traits` | Not applicable | AsyncValidate is being REMOVED; no async code left |
| GATs | Already used | AsValidatable already uses stable GATs |
| `impl Trait` in more positions | Minor | Return positions in some factory functions |
| Improved const generics | Not applicable | Validators use runtime values, not const generics |
| Precise capturing (`use<>`) | Maybe | Could simplify some `+ '_` lifetime bounds |

**Practical modernization**: Focus on structural cleanup (flat modules, dead code removal, feature flags, prelude) rather than language-level trait changes. The existing trait design is already modern and idiomatic.

## R4: Flattening Strategy — File Mapping

**Decision**: Move files from subcategory folders to flat `validators/` directory. Merge small `mod.rs` files into parent.

**Current → Target file mapping** (31 files → 28 files):

| Current Path | Target Path | Notes |
|-------------|------------|-------|
| `validators/string/length.rs` | `validators/length.rs` | Direct move |
| `validators/string/pattern.rs` | `validators/pattern.rs` | Direct move |
| `validators/string/content.rs` | `validators/content.rs` | Direct move |
| `validators/string/uuid.rs` | `validators/uuid.rs` | Direct move |
| `validators/string/datetime.rs` | `validators/datetime.rs` | Add `date_only()` |
| `validators/string/json.rs` | `validators/json_string.rs` | Rename to avoid conflict with `json` module |
| `validators/string/password.rs` | `validators/password.rs` | Direct move |
| `validators/string/phone.rs` | `validators/phone.rs` | Direct move |
| `validators/string/credit_card.rs` | `validators/credit_card.rs` | Direct move |
| `validators/string/iban.rs` | `validators/iban.rs` | Direct move |
| `validators/string/semver.rs` | `validators/semver.rs` | Direct move |
| `validators/string/slug.rs` | `validators/slug.rs` | Direct move |
| `validators/string/hex.rs` | `validators/hex.rs` | Direct move |
| `validators/string/base64.rs` | `validators/base64.rs` | Direct move |
| `validators/numeric/range.rs` | `validators/range.rs` | Direct move |
| `validators/numeric/properties.rs` | `validators/properties.rs` | Direct move |
| `validators/numeric/divisibility.rs` | `validators/divisibility.rs` | Direct move |
| `validators/numeric/float.rs` | `validators/float.rs` | Direct move |
| `validators/numeric/percentage.rs` | `validators/percentage.rs` | Direct move |
| `validators/collection/size.rs` | `validators/size.rs` | Direct move |
| `validators/collection/elements.rs` | `validators/elements.rs` | Direct move |
| `validators/collection/structure.rs` | `validators/structure.rs` | Direct move |
| `validators/network/ip_address.rs` | `validators/ip_address.rs` | Direct move |
| `validators/network/port.rs` | `validators/port.rs` | Direct move |
| `validators/network/mac_address.rs` | `validators/mac_address.rs` | Direct move |
| `validators/logical/boolean.rs` | `validators/boolean.rs` | Direct move |
| `validators/logical/nullable.rs` | `validators/nullable.rs` | Direct move |
| — | `validators/hostname.rs` | NEW: RFC 1123 |
| — | `validators/time.rs` | NEW: TimeOnly |

**Deleted files**: 5 `mod.rs` files from subcategory folders (`string/mod.rs`, `numeric/mod.rs`, `collection/mod.rs`, `network/mod.rs`, `logical/mod.rs`). Their re-exports merge into `validators/mod.rs`.

## R5: Dead Code Inventory

**Files to DELETE entirely:**

| File | LOC (approx) | Reason |
|------|-------------|--------|
| `core/refined.rs` | ~200 | 0 consumers, Phase 7 reimplementation |
| `core/state.rs` | ~300 | 0 consumers, Phase 7 reimplementation |
| `combinators/map.rs` | ~80 | Deprecated, mapper function never used |
| `tests/refined_test.rs` | ~60 | Tests deleted code |

**Code to REMOVE from existing files:**

| File | What to remove |
|------|---------------|
| `core/mod.rs` (→ `foundation/mod.rs`) | `pub mod refined`, `pub mod state`, `pub use refined::Refined`, `pub use state::*`, `pub use traits::AsyncValidate` |
| `core/traits.rs` (→ `foundation/traits.rs`) | `AsyncValidate` trait definition (~15 LOC) |
| `core/metadata.rs` (→ `foundation/metadata.rs`) | `ValidatorStatistics`, `RegisteredValidatorMetadata` — move behind `#[cfg(feature = "optimizer")]` |
| `combinators/mod.rs` | `pub mod map`, all map re-exports |
| `lib.rs` | `pub mod core` → `pub mod foundation` |

**Estimated dead code removed**: ~650 LOC

## R6: Feature Flag Design

**Decision**: Three optional features, one default.

```toml
[features]
default = ["serde"]
serde = []                          # JSON/Value support
caching = ["dep:moka"]              # Cached combinator
optimizer = []                       # ValidatorChainOptimizer, ValidatorStatistics
full = ["serde", "caching", "optimizer"]
```

**What each flag gates:**

| Feature | Gated items |
|---------|------------|
| `serde` | `json` module, `AsValidatable<_> for Value` impls, `JsonField` combinator, serde derives on `ValidationError` |
| `caching` | `Cached<V>`, `CacheStats`, `cached()` factory, `.cached()` / `.cached_with_capacity()` on `ValidateExt` |
| `optimizer` | `ValidatorChainOptimizer`, `ValidatorStats`, `ValidatorStatistics`, `RegisteredValidatorMetadata`, `OptimizationReport` |

**CI matrix**: 4 combinations — `--no-default-features`, default, `--features caching`, `--all-features`.

## R7: Hostname Validator Specification

**Decision**: Implement RFC 1123 hostname validation (Section 2.1).

**Rules**:
- Total length: 1..=253 characters
- Split by `.` into labels
- Each label: 1..=63 characters
- Each label: `[a-zA-Z0-9-]` only
- Labels must NOT start or end with hyphen
- At least one label required
- Trailing dot optional (FQDN)

**Edge cases**: empty string → error, single dot → error, double dot → error, 254+ chars → error.

## R8: TimeOnly Validator Specification

**Decision**: Validate ISO 8601 time strings.

**Accepted formats**:
- `HH:MM:SS` (e.g., "14:30:00")
- `HH:MM:SS.sss` (e.g., "14:30:00.123") — optional milliseconds
- `HH:MM:SSZ` or `HH:MM:SS+HH:MM` — optional timezone

**Validation rules**:
- Hours: 0..=23
- Minutes: 0..=59
- Seconds: 0..=60 (60 for leap second)
- Milliseconds: 0..=999

**Builder**: `TimeOnly::new()`, `.require_timezone()`, `.allow_milliseconds(true/false)`.

## R9: DateTime::date_only() Specification

**Decision**: Add builder method to existing `DateTime` validator.

**Accepted format**: `YYYY-MM-DD` only.
**Rejected**: Any string containing `T`, time components, or timezone.

**Implementation**: Set internal flags `allow_date_only: true, date_only_mode: true` on existing `DateTime` struct. Parse with `chrono::NaiveDate::parse_from_str("%Y-%m-%d")`. Reject if input contains 'T' or 'Z'.
