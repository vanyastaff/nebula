# Test Strategy

## Test Pyramid

- **Unit:** Format functions (`format_bytes`, `format_duration`); pressure threshold logic; error constructors
- **Integration:** `SystemInfo::get()`, `memory::current()`, `init()` with default features
- **Contract:** nebula-memory integration (reads `memory::current()`)
- **End-to-end:** Examples run successfully; cross-platform CI

## Critical Invariants

- `MemoryPressure::is_concerning()` true iff `>= High`
- `SystemInfo::get()` never panics; returns valid struct (possibly with "Unknown" fields)
- `init()` idempotent; safe to call multiple times
- All public types `Send + Sync`

## Scenario Matrix

- **Happy path:** init → get info → check pressure
- **Retry path:** N/A (no retries in crate)
- **Cancellation path:** N/A (sync only)
- **Timeout path:** N/A
- **Upgrade/migration path:** Feature flag combinations; deprecated API removal

## Tooling

- **Property testing:** proptest for `format_bytes`, `format_duration` (valid ranges)
- **Fuzzing:** Not yet; consider for path/parse functions
- **Benchmarks:** criterion for `SystemInfo::get()`, `memory::current()`, `process::list()`
- **CI quality gates:** `cargo test -p nebula-system`, `cargo test -p nebula-system --all-features`

## Exit Criteria

- **Coverage goals:** Core paths; format/utils; error handling
- **Flaky test budget:** Zero; platform-specific tests may be `#[ignore]` on unsupported platforms
- **Performance regression thresholds:** Document baseline; alert on >20% regression
