# Roadmap

Phased path to a stable, production-ready developer toolkit for action and plugin authors. Aligned with [CONSTITUTION.md](./CONSTITUTION.md): SDK is a facade; prelude and re-exports are versioned; no orchestration in SDK.

## Phase 1: Contract and Prelude Stability

- **Deliverables:**
  - Formal prelude stability policy: document re-exports; minor = additive only; removal or signature change = major.
  - Compatibility tests: macro output (nebula-macros) and builder output conform to nebula-action contract; CI enforces.
  - API.md and README aligned with actual prelude, builders, and testing modules.
- **Risks:**
  - Downstream crates (action, runtime) changing contract without SDK update; author breakage.
- **Exit criteria:**
  - Prelude content documented and test-covered; no undocumented breaking re-exports.
  - NodeBuilder/TriggerBuilder and derive output work with engine/runtime.

## Phase 2: Testing and Authoring DX

- **Deliverables:**
  - TestContext and MockExecution stay in sync with action/runtime expectations; documented contracts.
  - Optional deps / feature flags: minimal (types only) vs full (builders, testing, codegen); document in README.
  - Assertion helpers and ExecutionHarness stable; additive only in minor.
- **Risks:**
  - Test utilities drifting from real runtime behavior; false confidence in tests.
- **Exit criteria:**
  - Authors can write and test nodes with SDK only; test utilities documented with compatibility guarantees.
  - Feature matrix (minimal vs full) clear for release builds and CI.

## Phase 3: Codegen and Tooling

- **Deliverables:**
  - Optional codegen feature: OpenAPI spec generation, type generators (if adopted); behind feature flag.
  - Dev server with hot-reload (if adopted); documented as optional DX.
  - No new domain logic in SDK; only re-exports and authoring conveniences.
- **Risks:**
  - Codegen or dev-server adding heavy deps or unstable behavior.
- **Exit criteria:**
  - Optional features documented; default features keep SDK lean; no orchestration or runtime in SDK.

## Phase 4: Ecosystem and Compatibility

- **Deliverables:**
  - Version compatibility matrix: SDK X works with core/action/runtime Y; document in README or MIGRATION.
  - Migration guide for prelude or builder breaking changes (major).
  - Cookbook or examples for authoring nodes and testing with TestContext/MockExecution.
- **Risks:**
  - Platform version drift leaving SDK behind or forcing unnecessary major bumps.
- **Exit criteria:**
  - Clear compatibility story; external action authors can depend on SDK with predictable upgrades.

## Metrics of Readiness

- **Correctness:** Prelude and builders produce engine-compatible nodes; tests pass against real runtime contract.
- **Stability:** Prelude and re-exports stable in patch/minor; breaking = major + MIGRATION.
- **Operability:** Authors can onboard with one prelude import; optional features documented and gated.
