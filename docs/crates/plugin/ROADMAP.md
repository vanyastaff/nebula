# Roadmap

## Phase 1: Action Integration

- **Deliverables:**
  - Restore `process_action` and `stateful_action` in `PluginComponents`
  - Integrate `ProcessActionAdapter`, `StatefulActionAdapter` from action crate
  - Remove or deprecate `InternalHandler` placeholder
- **Risks:** Action adapter API may change; compatibility with existing handlers
- **Exit criteria:** Plugin can register typed actions; runtime executes them

## Phase 2: Resource Registration

- **Deliverables:**
  - Add `ResourceDescription` (or equivalent) to credential/resource crate
  - Add `resource()` to `PluginComponents`
  - Document resource registration in plugin docs
- **Risks:** Resource crate design not finalized
- **Exit criteria:** Plugin can declare resource requirements; runtime resolves them

## Phase 3: Dynamic Loading Hardening

- **Deliverables:**
  - Validate `PluginLoader` on Windows/Linux/macOS
  - Sandbox/validation for loaded libraries
  - Document dynamic loading in README and API
- **Risks:** FFI safety; platform-specific behavior
- **Exit criteria:** PluginLoader loads from `.so`/`.dll`/`.dylib`; CI passes

## Phase 4: Metadata and DX

- **Deliverables:**
  - Serialized metadata schema for API/UI consumption
  - SDK builders for plugin authoring
  - Macros for `#[derive(Plugin)]` if beneficial
- **Risks:** Schema drift; macro complexity
- **Exit criteria:** Plugin metadata serializable; SDK examples work

## Metrics of Readiness

- **Correctness:** All tests pass; no UB in loader
- **Latency:** Registry lookup < 1µs (in-memory)
- **Stability:** No breaking changes without MIGRATION.md
- **Operability:** Clear error messages; logging for load failures
