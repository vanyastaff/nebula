# Test Strategy

## Test Pyramid

- **Unit:** Plugin trait impls; metadata builder; registry CRUD; PluginVersions add/get; error display
- **Integration:** Plugin + action handler registration (when restored); plugin + credential description
- **Contract:** PluginKey normalization; metadata serialization roundtrip
- **End-to-end:** N/A in plugin crate; runtime/engine own E2E

## Critical Invariants

- `Plugin::key()` equals `metadata().key()`
- `PluginRegistry::register()` fails if key exists
- `PluginVersions::add()` rejects key mismatch and duplicate version
- `PluginMetadata::build()` rejects empty/invalid key
- `Plugin` is object-safe (`Arc<dyn Plugin>`)

## Scenario Matrix

- **Happy path:** Build metadata → create plugin → register → get by key → get plugin
- **Retry path:** N/A
- **Cancellation path:** N/A
- **Timeout path:** N/A
- **Upgrade/migration path:** Add version to PluginVersions; get by new version

## Tooling

- **Property testing:** Key normalization idempotence (optional)
- **Fuzzing:** N/A
- **Benchmarks:** Registry lookup latency (optional)
- **CI quality gates:** `cargo test -p nebula-plugin`; `cargo clippy -p nebula-plugin`

## Exit Criteria

- **Coverage goals:** All public APIs exercised; error paths covered
- **Flaky test budget:** Zero
- **Performance regression thresholds:** Registry lookup < 1µs (if benchmarked)
