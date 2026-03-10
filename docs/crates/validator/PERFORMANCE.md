# Performance: nebula-validator

Performance budgets, optimization notes, and cache strategy for the validation framework.

## Benchmark Budgets

All budgets are defined in `tests/fixtures/perf/benchmark_budgets_v1.json`.
Values represent **upper bounds in nanoseconds** measured on a release build.
Each budget has ~2x safety margin above measured baselines.

### Summary

| Category | Operation | Budget |
|----------|-----------|--------|
| **Validators (success)** | Length/pattern check | ≤ 15 ns |
| | Email/URL validation | ≤ 50 ns |
| | Regex match | ≤ 500 ns |
| **Validators (error)** | Simple error (code + message) | ≤ 500 ns |
| | Error with 2 params | ≤ 600 ns |
| | Error with nested child | ≤ 1 µs |
| **Combinators** | AND (2 validators) | ≤ 15 ns |
| | AND (5 validators) | ≤ 30 ns |
| | AND (10 validators) | ≤ 50 ns |
| | OR (first succeeds) | ≤ 15 ns |
| | NOT | ≤ 10 ns |
| | When (skipped) | ≤ 10 ns |
| | Cached (hit) | ≤ 100 ns |
| **Real-world** | Username validation | ≤ 30 ns |
| | 3-field form (all valid) | ≤ 150 ns |
| **Memory** | `ValidationError` size | = 80 bytes |

### Threshold Policy

1. **Hard budgets**: Regressions beyond stated `max_ns` values block the PR.
2. **Exceptions**: A budget may be relaxed with justification in the PR description
   and explicit approval. The budget JSON must be updated in the same PR.
3. **New validators**: Every new validator or combinator must declare a budget
   entry before merge.
4. **Tightening**: After a confirmed optimization, budgets should be tightened
   to lock in the improvement.

### Benchmark Profiles

Two profiles are available via `scripts/bench-validator.{sh,ps1}`:

| Profile | Mode | Use Case | Sample Size |
|---------|------|----------|-------------|
| **Quick** | `quick` | PR checks, local dev | ~10 samples |
| **Full** | `full` | Release validation, regression hunting | ~100 samples |
| **Baseline** | `baseline` | Save a named baseline for comparison | ~100 samples |
| **Compare** | `compare` | Compare current results against baseline | ~100 samples |

```bash
# PR quick check (default)
./scripts/bench-validator.sh quick

# Full release profile
./scripts/bench-validator.sh full

# Save baseline before a change
./scripts/bench-validator.sh baseline before-optimization

# Compare after a change
./scripts/bench-validator.sh compare before-optimization

# Run only specific benches
./scripts/bench-validator.sh quick main string_validators cache
```

Results are stored in `target/criterion/` with HTML reports at
`target/criterion/<group>/report/index.html`.

## Allocation Profile

### Success Path (zero-alloc)

The happy path allocates **zero bytes**. Validators return `Ok(())` which is a
zero-sized value. The `Validate<T>` trait takes `&T` and returns
`Result<(), ValidationError>` — no heap allocation on success.

### Error Path

Error construction is the primary allocator. The `ValidationError` struct is
optimized to minimize allocations:

```
ValidationError (80 bytes, stack)
├── code:    Cow<'static, str>  — 24B (Borrowed for built-in codes = 0 alloc)
├── message: Cow<'static, str>  — 24B (Owned when using format! = 1 alloc)
├── field:   Option<Cow<…>>     — 24B (Owned if field path conversion = 1 alloc)
└── extras:  Option<Box<…>>     —  8B (Box allocated only if params/nested used)
    └── ErrorExtras
        ├── params:   SmallVec<[_;2]>  — inline for 0–2 params (95% of cases)
        ├── nested:   Vec<…>           — heap-allocated per nested error
        ├── severity: ErrorSeverity    — 1 byte copy
        └── help:     Option<Cow<…>>   — rare, lazy
```

**Allocation count by error type:**

| Error Type | Heap Allocs | Notes |
|------------|-------------|-------|
| Static code + static message | 0 | `Cow::Borrowed` for both |
| Static code + `format!` message | 1 | Message string |
| With field (dot → pointer) | +1 | JSON Pointer conversion |
| With 1–2 params | +1 | Box<ErrorExtras> only |
| With 3+ params | +2 | Box + SmallVec spill |
| With nested errors | +1 per child | Vec<ValidationError> |

### Optimization Guidance

**For validator authors:**

1. **Prefer static messages** — use `Cow::Borrowed` (string literals) over `format!()`.
   ```rust
   // Good: zero-alloc error
   ValidationError::new("required", "This field is required")

   // Avoid: allocates for message
   ValidationError::new("required", format!("Field {} is required", name))
   ```

2. **Keep params ≤ 2** — the `SmallVec<[_;2]>` inlines up to 2 key-value pairs
   without heap allocation. A third param triggers a spill.

3. **Avoid nested errors in leaf validators** — nested errors are for combinators
   (Or, Each) that aggregate results from multiple validators.

## Cache Strategy (moka)

The `Cached<V>` combinator wraps any validator with a moka LRU cache for
memoizing results. Use it when:

### When to Cache

| Scenario | Recommendation |
|----------|----------------|
| Regex validation | **Cache** — regex matching is ~10–50x slower than length checks |
| Email/URL parsing | **Maybe** — ~20–35 ns is fast; cache only if called repeatedly |
| Simple length/pattern | **Don't cache** — ~5 ns is faster than cache lookup (~50 ns) |
| Composed chain (5+ validators) | **Maybe** — if total > 100 ns and inputs repeat |
| External service check | **Cache** — network I/O is orders of magnitude slower |

### Rule of Thumb

> Cache when `validator_cost > cache_lookup_cost` (~50–100 ns) **and**
> the same inputs are validated repeatedly.

### Capacity Tuning

Default capacity is **1000 entries** with LRU eviction.

```rust
use nebula_validator::combinators::cached::Cached;

// Default: 1000 entries
let v = cached(expensive_validator());

// Custom capacity for high-cardinality inputs
let v = Cached::with_capacity(expensive_validator(), 10_000);

// Check utilization
let stats = v.cache_stats();
if stats.utilization() > 0.9 {
    // Consider increasing capacity
}
```

**Sizing guidance:**

| Input cardinality | Suggested capacity | Notes |
|-------------------|--------------------|-------|
| < 100 unique | 100 | Small fixed set |
| 100–1000 | 1000 (default) | Typical form fields |
| 1000–10000 | 5000–10000 | Batch processing |
| > 10000 | Consider no cache | LRU thrashing likely |

### Memory Cost

Each cache entry stores:
- `u64` hash key (8 bytes)
- `Arc<Result<(), ValidationError>>` — pointer (8 bytes) + Arc header (16 bytes)
- For Ok: ~0 extra bytes
- For Err: ~80+ bytes (ValidationError)

At default capacity (1000), worst case ~112 KB for all-error cache.

### Thread Safety

The moka cache is lock-free for reads and uses fine-grained locks for writes.
It is safe to share `Cached<V>` across threads via `Arc`. No external
synchronization is needed.

## CI Enforcement

Benchmark threshold tests in `tests/contract/benchmark_budget_test.rs` enforce:

1. **Budget file integrity** — the JSON fixture must parse and contain all
   required categories.
2. **Memory layout assertions** — `size_of::<ValidationError>() == 80` is checked
   at compile time.
3. **Budget value validation** — all budgets must be positive and within sane
   bounds (no accidental 0 ns or 1 ms budgets).

Runtime benchmark enforcement (comparing actual Criterion output against budgets)
is performed in CI via the bench scripts. The contract tests validate the budget
artifact itself, not runtime performance (which is machine-dependent).
