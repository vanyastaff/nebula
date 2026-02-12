# Implementation Plan: Migrate to serde_json Value System

**Branch**: `008-serde-value-migration` | **Date**: 2026-02-11 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/008-serde-value-migration/spec.md`

## Summary

Replace the custom `nebula-value` type system with standard `serde_json::Value` and `RawValue` to eliminate conversion overhead when integrating with the Rust ecosystem. This migration affects three crates (nebula-config, nebula-resilience, nebula-expression) in a bottom-up approach, removing persistent data structures (`im` crate), using temporal types as strings with `chrono` parsing, and hiding RawValue optimizations behind a clean node developer API. Success is measured by zero test failures, zero compilation warnings, and confirmed elimination of conversion code at ecosystem boundaries.

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)
**Primary Dependencies**: serde_json, chrono, rust_decimal, bytes, thiserror
**Storage**: N/A (value type refactoring)
**Testing**: `cargo test --workspace` for all tests; validation requires 100% pass rate with zero test modifications
**Target Platform**: Cross-platform (Windows primary development, Linux/macOS support)
**Project Type**: Workspace (16 crates) - modifying 3 crates (nebula-config, nebula-resilience, nebula-expression), deleting 1 crate (nebula-value)
**Performance Goals**: Zero performance regression compared to nebula-value baseline; elimination of conversion overhead at serde ecosystem boundaries
**Constraints**: Migration order must be bottom-up (config → resilience → expression); all existing tests must pass without modification; no breaking API changes for end users
**Scale/Scope**: ~75 files use nebula-value across 4 workspace crates; typical payloads <10MB, system targets efficient handling up to 100MB

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with `.specify/memory/constitution.md` principles:

- [x] **Type Safety First**: Uses serde_json::Value enum (Null, Bool, Number, String, Array, Object); no custom newtype wrappers needed (simplification, not a violation)
- [x] **Isolated Error Handling**: Each migrated crate updates its own error type to include `#[from] serde_json::Error`; no shared error dependencies
- [x] **Test-Driven Development**: Existing tests validate behavior; migration strategy is test-first (run tests, fix compilation, run tests again)
- [x] **Async Discipline**: No async pattern changes - this is a value type refactoring only
- [x] **Modular Architecture**: Migration respects layer boundaries; bottom-up order prevents circular dependencies
- [x] **Observability**: No observability changes - migration is transparent to logging/tracing
- [x] **Simplicity**: Net reduction in complexity - deletes ~5000+ lines (nebula-value crate), removes `im` dependency, eliminates conversion code

**GATE RESULT**: PASS — No violations. This refactoring actively improves simplicity (Principle VII).

## Project Structure

### Documentation (this feature)

```text
specs/008-serde-value-migration/
├── plan.md              # This file
├── research.md          # Phase 0: serde_json best practices, migration patterns
├── data-model.md        # Phase 1: Value type mapping, temporal type handling
├── quickstart.md        # Phase 1: Migration guide for developers
├── contracts/           # Phase 1: API contracts for migrated crates (minimal - mostly internal refactoring)
└── tasks.md             # Phase 2: /speckit.tasks output (NOT created by this command)
```

### Source Code (repository root)

```text
crates/
├── nebula-config/       # System layer - MODIFY (Phase 1)
│   ├── src/lib.rs       # Update imports: nebula_value → serde_json
│   └── Cargo.toml       # Remove nebula-value dependency, ensure serde_json present
├── nebula-resilience/   # Cross-cutting layer - MODIFY (Phase 2)
│   ├── src/             # Update Value usage in policy configs
│   └── Cargo.toml       # Remove nebula-value dependency
├── nebula-expression/   # Core layer - MODIFY (Phase 3)
│   ├── src/             # Major refactor: builtins, template engine, context
│   └── Cargo.toml       # Remove nebula-value dependency
└── nebula-value/        # Core layer - DELETE (Phase 4)
    └── [entire crate]   # Remove after all dependents migrated

# Workspace Cargo.toml
# - Remove nebula-value from workspace members
# - Ensure serde_json version consistent across workspace
```

**Structure Decision**: No new crates created. This feature modifies 3 existing crates and deletes 1 crate. All affected crates are in appropriate architectural layers (System, Cross-Cutting, Core). Migration respects layer boundaries by proceeding bottom-up: System layer first (nebula-config has minimal Value usage), then Cross-Cutting (nebula-resilience), then Core (nebula-expression is most complex). This aligns with Principle V (Modular Architecture) — dependencies flow upward, preventing circular deps.

## Complexity Tracking

> **Not applicable** — Constitution Check shows no violations. This refactoring reduces complexity.

## Implementation Phases

### Phase 0: Research & Prerequisites

**Goal**: Resolve unknowns about serde_json ecosystem integration and establish migration patterns.

**Research Tasks**:
1. **serde_json Value API** - Review standard methods (`as_str()`, `as_i64()`, `get()`, `[]` indexing), error handling patterns
2. **RawValue best practices** - When to use `Box<RawValue>` vs `&RawValue`, serialization/deserialization patterns
3. **Temporal type handling** - ISO 8601/RFC 3339 string formats, `chrono` parsing patterns, common pitfalls
4. **Error conversion patterns** - How to wrap `serde_json::Error` in domain-specific errors with context
5. **Migration checklist** - Steps for each crate (update Cargo.toml, change imports, fix type mismatches, run tests)

**Output**: `research.md` documenting:
- serde_json::Value vs nebula_value::Value API differences
- RawValue usage patterns for deferred deserialization
- Temporal type serialization/deserialization examples
- Error handling template for each crate
- Step-by-step migration checklist

**Validation**: All NEEDS CLARIFICATION resolved; migration approach documented

---

### Phase 1: Design & Data Model

**Prerequisites**: research.md complete

**Goal**: Define type mappings, API contracts, and migration guide.

**Design Artifacts**:

1. **data-model.md** - Type mapping documentation:
   - nebula_value::Value → serde_json::Value (variant-by-variant)
   - Temporal types (Date, Time, DateTime, Duration) → string format specifications
   - Special types (Bytes, Decimal) → serde ecosystem equivalents
   - Collection types (Array, Object) → `Vec<Value>`, `Map<String, Value>`
   - Error types mapping for each crate

2. **contracts/** - API stability guarantees:
   - `nebula-config.md` - Public APIs remain unchanged (internal Value usage only)
   - `nebula-resilience.md` - Policy config serialization format unchanged
   - `nebula-expression.md` - Expression evaluation behavior unchanged (but Value type differs)

3. **quickstart.md** - Developer migration guide:
   - Before/after code examples for common patterns
   - Import statement changes
   - Type conversion updates
   - Error handling updates
   - Testing validation steps

**Agent Context Update**:
- Run `.specify/scripts/bash/update-agent-context.sh claude`
- Add technology: serde_json, chrono (if not already present)
- Document migration as active context

**Validation**: All design artifacts created and reviewed

---

### Phase 2: Implementation Planning (Output of this command)

**Goal**: Generate task breakdown for `/speckit.tasks` command.

**Implementation Order** (bottom-up):

1. **Phase 2.1: Migrate nebula-config** (~2-4 hours)
   - Update Cargo.toml dependencies
   - Replace `use nebula_value::Value` with `use serde_json::Value`
   - Fix type mismatches (method names, conversions)
   - Update error types: add `#[from] serde_json::Error`
   - Run tests: `cargo test -p nebula-config`
   - Quality gates: fmt, clippy, check, doc

2. **Phase 2.2: Migrate nebula-resilience** (~4-6 hours)
   - Same steps as Phase 2.1 for nebula-resilience
   - Additional: update policy config serialization/deserialization
   - Run tests: `cargo test -p nebula-resilience`
   - Quality gates: fmt, clippy, check, doc

3. **Phase 2.3: Migrate nebula-expression** (~8-12 hours - most complex)
   - Update builtin functions (math, string, datetime, array, object)
   - Update template engine and context handling
   - Temporal type handling: parse ISO 8601 strings to chrono types
   - Update error types extensively (many Value operations)
   - Run tests: `cargo test -p nebula-expression`
   - Quality gates: fmt, clippy, check, doc

4. **Phase 2.4: Delete nebula-value crate** (~1-2 hours)
   - Remove `crates/nebula-value/` directory
   - Update workspace `Cargo.toml` (remove from members)
   - Verify no remaining references: `rg "nebula_value" --type rust`
   - Run full workspace tests: `cargo test --workspace`
   - Final quality gates on entire workspace

5. **Phase 2.5: Validation & Cleanup** (~2-4 hours)
   - Run full test suite: 100% pass rate required
   - Verify zero compilation warnings
   - Code review: confirm no conversion code at ecosystem boundaries
   - Update CLAUDE.md to remove nebula-value from active technologies
   - Commit and create PR

**Output**: This plan.md file documents the implementation strategy. Next command is `/speckit.tasks` to generate granular task breakdown.

**Total Estimated Effort**: 17-28 hours

---

## Quality Gates

After EACH phase completion (2.1, 2.2, 2.3, 2.4), MUST run:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo check --workspace
cargo doc --no-deps --workspace
```

All warnings MUST be fixed before proceeding to next phase.

---

## Success Criteria (from spec.md)

- **SC-001**: All workspace tests pass (100% success rate)
- **SC-002**: Zero compilation errors and warnings
- **SC-003**: Node developers don't see RawValue in public APIs
- **SC-004**: Zero conversion code at serde ecosystem boundaries
- **SC-005**: Pass-through nodes avoid JSON parsing (RawValue optimization)
- **SC-006**: Codebase complexity reduces (~5000+ lines deleted)
- **SC-007**: Zero performance regression (validated by existing tests)

---

## Rollback Strategy

Not required - migration occurs entirely on feature branch `008-serde-value-migration`. Integration to main only after all tests pass and validation complete. (See spec.md Clarifications session)

---

## Next Steps

1. ✅ Phase 0: Generate `research.md` (THIS COMMAND)
2. ✅ Phase 1: Generate `data-model.md`, `contracts/`, `quickstart.md` (THIS COMMAND)
3. ⏭ Phase 2: Run `/speckit.tasks` to create granular task breakdown for implementation
4. ⏭ Implementation: Execute tasks from tasks.md
5. ⏭ Validation: Run all quality gates and verify success criteria
6. ⏭ Integration: Create PR and merge to main after approval
