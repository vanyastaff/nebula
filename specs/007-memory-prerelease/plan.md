# Implementation Plan: nebula-memory Pre-Release Readiness

**Branch**: `007-memory-prerelease` | **Date**: 2026-02-11 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/007-memory-prerelease/spec.md`

## Summary

Bring the nebula-memory crate (~38,000 lines across 80+ source files) to pre-release quality for Rust 1.92+ (Edition 2024). The work involves: fixing 9 compilation errors and 32 warnings, removing the compression module (~1,300 lines) and no_std paths (~135 conditional blocks), cleaning up 11 backup files, replacing 6 panic stubs with proper error handling, removing the empty lockfree module, ensuring cross-platform compilation (Windows/Linux/macOS), and completing documentation. No new features are added — this is a stabilization and cleanup effort.

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)
**Primary Dependencies**: nebula-core, nebula-system, nebula-log (optional), thiserror, parking_lot, crossbeam-queue, hashbrown, dashmap, tokio (optional), winapi (Windows)
**Storage**: N/A (in-memory allocators)
**Testing**: `cargo test -p nebula-memory --all-features`, integration tests (6 files), examples (14 files), benchmarks (3 files)
**Target Platform**: Cross-platform (Windows x86_64, Linux x86_64, macOS x86_64/aarch64)
**Project Type**: Single crate within 16-crate workspace
**Performance Goals**: No regression from current state. Existing benchmarks must pass, no new targets.
**Constraints**: Must not break other workspace crates that depend on nebula-memory. `std` is now required (no_std dropped).
**Scale/Scope**: ~38,000 lines of source code. 9 errors, 32 warnings, 11 backup files, 6 panic stubs, 1 empty module, ~1,300 lines of compression code to remove, ~135 no_std conditional blocks to simplify.

### Current Compilation State

**Errors (9)**:
- `E0432`: Unresolved import `crate::allocator::AllocErrorCode` (1)
- `E0433`: Unresolved module `nebula_error` (2), missing `core::error` (2)
- `E0599`: No variant `with_layout` on `MemoryError` (2)
- `E0061`: Wrong argument count (2)

**Warnings (32)**:
- `E0133`: Unsafe function calls need `unsafe {}` blocks in Rust 2024 (11)
- Unused imports (9)
- Unnecessary `unsafe` blocks (3)
- Mutable variables that don't need `mut` (4)
- Other (5)

### Modules to Remove

| Module | Lines | Reason |
| ------ | ----- | ------ |
| `src/compression/` | ~1,300 | No current use case, clarification decision |
| `src/lockfree/mod.rs` | 1 | Empty, no implementation |
| no_std conditional paths | ~135 blocks | `std` now required, clarification decision |

### Panic Stubs to Fix

| Location | Current Behavior | Required Action |
| -------- | ---------------- | --------------- |
| `allocator/manager.rs:398` | `panic!("Global allocator manager not initialized")` | Return `Result::Err(MemoryError::NotInitialized)` |
| `cache/simple.rs:407` | `panic!("Should not compute!")` | Review context — likely test-only, move to test |
| `compression/mod.rs:137,139` | `panic!` on missing compressor | Removed with compression module |
| `pool/hierarchical.rs:105` | `panic!("Child pool factory not implemented")` | Implement or return `Result::Err` |
| `pool/ttl.rs:346` | `panic!("TTL pool requires std feature")` | Removed with no_std paths |

### Backup Files to Process (11)

| File | Action |
| ---- | ------ |
| `src/allocator/error.rs.old` | Extract valuable error patterns, then delete |
| `src/core/error.rs.old` | Extract valuable error patterns, then delete |
| `src/arena/cross_thread.rs.bak` | Compare with current, delete |
| `src/arena/thread_safe.rs.bak` | Compare with current, delete |
| `src/budget/budget.rs.bak` | Compare with current, delete |
| `src/budget/manager.rs.bak` | Compare with current, delete |
| `src/cache/compute.rs.bak` | Compare with current, delete |
| `src/cache/multi_level.rs.bak` | Compare with current, delete |
| `src/cache/partitioned.rs.bak` | Compare with current, delete |
| `src/cache/scheduled.rs.bak` | Compare with current, delete |
| `src/pool/hierarchical.rs.bak` | Compare with current, delete |

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- [x] **Type Safety First**: Existing crate uses newtype patterns, enums, sized types. No changes to type design — cleanup only.
- [x] **Isolated Error Handling**: Crate defines its own `MemoryError` with `thiserror`. Fix: remove lingering references to `nebula_error` (compilation error E0433). No cross-crate error dependency.
- [x] **Test-Driven Development**: Existing tests (6 integration test files, unit tests in modules). Fix failing tests; no new features require new TDD cycles. New error variants added for panic replacements get tests.
- [x] **Async Discipline**: Existing async module uses tokio correctly. No changes to async patterns — cleanup only.
- [x] **Modular Architecture**: Single crate, no new cross-crate dependencies. Removing compression reduces dependency surface (lz4_flex removed).
- [x] **Observability**: Statistics and monitoring modules preserved. nebula-log integration maintained via feature flag.
- [x] **Simplicity**: This work reduces complexity — removes compression, no_std, empty modules, backup files. Net reduction of ~1,500+ lines.
- [x] **Rust API Guidelines**: Quality gates (fmt, clippy, check, doc) are the core deliverable of this feature.

**GATE RESULT**: PASS — no violations, no complexity tracking needed.

## Project Structure

### Documentation (this feature)

```text
specs/007-memory-prerelease/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output (trait API contracts)
└── tasks.md             # Phase 2 output (/speckit.tasks)
```

### Source Code (repository root)

```text
crates/nebula-memory/
├── src/
│   ├── allocator/       # Bump, Pool, Stack, System allocators + traits
│   ├── arena/           # Arena variants (basic, thread-safe, typed, local, cross-thread, streaming)
│   ├── async_support/   # Async arena/pool wrappers
│   ├── budget/          # Memory budgeting with hierarchical support
│   ├── cache/           # Compute cache, multi-level, partitioned, scheduled, async
│   ├── compression/     # ← TO BE REMOVED
│   ├── core/            # Core traits, types, config
│   ├── extensions/      # Integration extensions
│   ├── lockfree/        # ← TO BE REMOVED (empty)
│   ├── pool/            # Object pools (basic, thread-safe, lock-free, TTL, priority, hierarchical)
│   ├── stats/           # Statistics, monitoring, profiling, predictive analytics
│   ├── syscalls/        # Platform abstraction (mmap/VirtualAlloc/fallback)
│   ├── error.rs         # MemoryError enum
│   ├── lib.rs           # Crate root, module declarations, re-exports
│   ├── macros.rs         # memory_scope!, allocator!, alloc!, dealloc!
│   ├── monitoring.rs    # System memory pressure monitoring
│   └── utils.rs         # Alignment, formatting, backoff, prefetch, SIMD ops
├── tests/               # 6 integration test files
├── examples/            # 14 example files
├── benches/             # 3 benchmark files
└── Cargo.toml
```

**Structure Decision**: No new crates. This feature modifies only `nebula-memory`. Net reduction in module count (remove compression/, lockfree/).

## Implementation Phases

### Phase 1: Fix Compilation Errors (Critical Path)

Fix the 9 compilation errors that prevent the crate from building. These must be resolved first as all other work depends on a compiling crate.

**Work items**:
1. Remove references to `nebula_error` crate (E0433) — replace with local `MemoryError`
2. Remove references to `AllocErrorCode` (E0432) — either define locally or use `MemoryError` variants
3. Fix `MemoryError::with_layout` calls (E0599) — update to current API or add the variant
4. Fix function argument mismatches (E0061)
5. Fix `core::error` references (E0433) — Rust 2024 moved `Error` trait, update paths

**Dependency**: None (first step)
**Validation**: `cargo check -p nebula-memory --all-features` succeeds with 0 errors

### Phase 2: Remove Compression Module and no_std Paths

Remove code that was decided out-of-scope during clarification.

**Work items**:
1. Delete `src/compression/` directory entirely (~1,300 lines)
2. Remove `compression` feature flag from `Cargo.toml`
3. Remove `lz4_flex` dependency from `Cargo.toml`
4. Remove all `#[cfg(feature = "compression")]` gates in other modules
5. Remove `compression` from `full` feature set
6. Remove `no-std` from package categories in `Cargo.toml`
7. Remove `alloc` feature flag from `Cargo.toml`
8. Remove `streaming` feature (depends on `alloc` only)
9. Simplify all `#[cfg(not(feature = "std"))]` blocks — remove conditional, keep the `std` path
10. Remove no_std panic stubs (pool/ttl.rs:346)
11. Remove `heapless` dependency (no_std collections)
12. Delete `src/lockfree/mod.rs` and remove from `lib.rs`
13. Remove `CompressedArena`, `CompressedBump`, `CompressedPool` types that depend on compression
14. Update `lib.rs` module declarations

**Dependency**: Phase 1 (need compiling crate to verify removals don't break anything)
**Validation**: `cargo check -p nebula-memory --all-features` succeeds, feature set is smaller and cleaner

### Phase 3: Fix Warnings (Rust 2024 Compliance)

Fix all 32 warnings to achieve zero-warning compilation.

**Work items**:
1. Add `unsafe {}` blocks inside `unsafe fn` bodies (Rust 2024 requirement, 11 warnings)
2. Remove unused imports (9 warnings)
3. Remove unnecessary `unsafe` blocks (3 warnings)
4. Remove unused `mut` qualifiers (4 warnings)
5. Fix remaining miscellaneous warnings (5)

**Dependency**: Phase 2 (some warnings may disappear after module removal)
**Validation**: `cargo check -p nebula-memory --all-features` with 0 errors AND 0 warnings

### Phase 4: Replace Panic Stubs with Error Handling

Replace remaining `panic!()` calls in production code with proper `Result::Err` returns.

**Work items**:
1. `allocator/manager.rs:398` — Replace `panic!("not initialized")` with `Result::Err(MemoryError::NotInitialized)` or equivalent. May need to add a new error variant.
2. `pool/hierarchical.rs:105` — Replace `panic!("Child pool factory not implemented")` with proper factory propagation or `Result::Err(MemoryError::Unsupported)`.
3. `cache/simple.rs:407` — Review context: if test-only assertion, move to test; if production code, return error.
4. Add tests for each new error path.

**Dependency**: Phase 3 (clean compilation baseline)
**Validation**: `grep -rn "panic!" src/ --include="*.rs"` returns only test code. `cargo test` passes.

### Phase 5: Study and Remove Backup Files

Review backup files for valuable patterns, integrate what's useful, then delete all backups.

**Work items**:
1. Read `src/allocator/error.rs.old` (~600 lines) — extract error code system, severity levels, error statistics if valuable for FR-025 (actionable error context)
2. Read `src/core/error.rs.old` (~400 lines) — extract error categorization patterns if useful
3. For each `.bak` file: diff against current version, identify any lost functionality
4. Integrate valuable patterns into current codebase
5. Delete all 11 backup files
6. Verify no references remain to deleted files

**Dependency**: Phase 4 (error handling must be established before integrating old error patterns)
**Validation**: `find . -name "*.bak" -o -name "*.old" -o -name "*.tmp"` returns empty

### Phase 6: Cross-Platform Validation

Ensure platform-specific code works correctly on all targets.

**Work items**:
1. Review `src/syscalls/direct.rs` — verify Windows (`VirtualAlloc`), Unix (`mmap`), and fallback paths all compile
2. Review `src/syscalls/info.rs` — verify `get_page_size()` works on all platforms
3. Review `src/syscalls/mod.rs` — verify `AllocatorCapabilities::detect()` works cross-platform
4. Review `src/monitoring.rs` — verify memory pressure detection uses `nebula_system` cross-platform API
5. Ensure no `#[cfg(target_os = "linux")]` code without corresponding Windows/macOS fallback
6. Run `cargo check --target x86_64-pc-windows-msvc --all-features` (native)
7. Verify NUMA code is properly feature-gated and documented as experimental

**Dependency**: Phase 3 (need clean compilation)
**Validation**: Compilation succeeds for all three target platforms

### Phase 7: Fix Tests and Examples

Ensure all tests and examples pass.

**Work items**:
1. Run `cargo test -p nebula-memory --all-features` and fix failures
2. Fix the previously reported 2 failing integration tests (from 21/23)
3. Run each example: `cargo run -p nebula-memory --example <name>` for all 14 examples
4. Fix examples that reference removed modules (compression, no_std)
5. Remove examples that are entirely about removed features
6. Update test assertions that reference removed types/features
7. Fix `tests/allocator_basic.rs:75` unnecessary `unsafe` block warning

**Dependency**: Phase 6 (platform validation may affect test behavior)
**Validation**: `cargo test -p nebula-memory --all-features` — 100% pass. All examples run without errors.

### Phase 8: Documentation and README

Complete documentation for all public APIs and update README.

**Work items**:
1. Run `cargo doc -p nebula-memory --no-deps --all-features` and fix all warnings
2. Add doc comments to all public types/traits/functions missing them
3. Update README.md:
   - MSRV: 1.70 → 1.92
   - Remove compression feature from docs
   - Remove no_std references
   - Update feature flags list
   - Update supported platforms section
   - Fix code examples to match current API
   - Remove "no-std" from categories
4. Update CHANGELOG.md with pre-release changes
5. Verify all doc links resolve
6. Verify doc examples compile (`cargo test --doc -p nebula-memory`)

**Dependency**: Phase 7 (API must be stable before documenting)
**Validation**: `cargo doc -p nebula-memory --no-deps --all-features` with 0 warnings. README reflects actual state.

### Phase 9: Final Quality Gates

Run the full CI pipeline to validate pre-release readiness.

**Work items**:
1. `cargo fmt --all -- --check`
2. `cargo clippy -p nebula-memory --all-features -- -D warnings`
3. `cargo check -p nebula-memory --all-features --all-targets`
4. `cargo test -p nebula-memory --all-features`
5. `cargo doc -p nebula-memory --no-deps --all-features`
6. `cargo publish -p nebula-memory --dry-run` (verify publishability)
7. Verify no backup files remain
8. Verify no panic stubs remain in production code
9. Run `cargo check --workspace` to ensure no other crates are broken

**Dependency**: Phase 8 (all work complete)
**Validation**: All 9 checks pass. Crate is pre-release ready.

## Risk Assessment

| Risk | Impact | Mitigation |
| ---- | ------ | ---------- |
| Removing compression breaks downstream code | Medium | Search workspace for `compression` feature usage before removal |
| no_std removal breaks heapless-dependent code | Medium | Audit all heapless usages — they may be used independently of no_std |
| Backup files contain patterns not in current code | Low | Read and diff each backup before deletion |
| Cross-platform syscall fallbacks have bugs | Medium | Test on Windows (native dev environment) |
| Fixing panic stubs changes API signatures | Low | Existing callers likely already handle Result — panics were stubs |
| Other workspace crates depend on removed features | High | Run `cargo check --workspace` after each phase |

## Complexity Tracking

No complexity violations. This feature is a net complexity reduction:
- Removes ~1,500+ lines (compression + lockfree + no_std paths)
- Removes 2 optional dependencies (lz4_flex, heapless potentially)
- Removes 3+ feature flags (compression, alloc, streaming)
- Reduces conditional compilation surface (~135 cfg blocks simplified)
