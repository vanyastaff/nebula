# Test Strategy

## Test Pyramid

- **Unit:** PluginMetadata builder success/failure; PluginRegistry register, get, contains, list; duplicate key returns AlreadyExists; PluginType::single and versioned. Plugin impl (e.g. EchoPlugin in doc test) registers and key/name/version match.
- **Integration:** Engine (or test harness) creates registry, registers plugin, resolves by key; assert engine can get plugin and components. Optional: loader loads test lib and registers (with dynamic-loading).
- **Contract:** Engine depends on registry get/list; contract test: register plugin, engine resolves action or plugin by key. No execution in plugin crate.
- **E2E:** Out of scope for plugin (engine/API own E2E).

## Critical Invariants

- After register(plugin_type), get(key) returns the same plugin type; contains(key) is true. Duplicate key register fails.
- Plugin::register() only mutates PluginComponents; no I/O or execution.
- Default build (no dynamic-loading) has no unsafe.

## Scenario Matrix

- **Happy path:** Register one or more plugins; get and list; engine resolves by key.
- **Duplicate path:** Register same key twice; second fails with AlreadyExists.
- **Load path:** With feature, load from path; register; get returns loaded plugin. Load invalid path fails with clear error.

## Tooling

- **CI:** cargo test; with and without dynamic-loading if feasible. Optional: contract test with engine.
- **Benchmarks:** N/A unless registry scale (many keys) is critical.

## Exit Criteria

- All registry and metadata paths covered; duplicate key and load failure tested. No flaky tests. Contract test with engine (when added) in CI.
