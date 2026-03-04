# Decisions

## D-001: Single Prelude

**Status:** Adopt

**Context:** Authors need one import for common types and macros.

**Decision:** One prelude re-exports core, Value, and common authoring types/macros (CONSTITUTION).

**Alternatives considered:** Multiple preludes per layer — rejected to avoid confusion.

**Trade-offs:** Prelude content is stable; changes require careful versioning.

**Consequences:** Breaking prelude = major; document re-exports in API.md.

**Migration impact:** None; current design.

**Validation plan:** Compatibility tests: prelude compiles and exports documented types.

---

## D-002: Optional Macros and Builders

**Status:** Adopt

**Context:** Some authors want minimal deps; others want full DX.

**Decision:** Macros (nebula-macros) and builders are optional (feature or optional deps) so minimal authors can depend only on what they need.

**Alternatives considered:** Always include all — rejected to keep binary and dep tree smaller for minimal users.

**Trade-offs:** Feature matrix to maintain; default features should cover most users.

**Consequences:** Document default vs optional features; release builds may disable testing/builders.

**Migration impact:** None.

**Validation plan:** CI with default-features and full features.

---

## D-003: No Orchestration or Runtime in SDK

**Status:** Adopt

**Context:** Engine and runtime own execution.

**Decision:** SDK provides types and authoring tools only; does not run workflows or schedule nodes.

**Alternatives considered:** SDK embedding test engine — rejected; engine stays in engine crate.

**Trade-offs:** Authors use TestContext/MockExecution for tests, not full engine (unless they add engine dep).

**Consequences:** TestContext must match runtime context shape; documented in TEST_STRATEGY.

**Migration impact:** None.

**Validation plan:** TestContext and MockExecution contract tests with action crate.
