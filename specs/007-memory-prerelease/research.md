# Research: nebula-memory Pre-Release Readiness

**Date**: 2026-02-11
**Branch**: `007-memory-prerelease`

## R1: Compilation Error Root Causes

### Decision: Fix strategy for each of 9 errors

**Analysis**: All 9 errors trace to 3 root causes:

| Root Cause | Errors | Fix |
| ---------- | ------ | --- |
| References to non-existent `nebula_error` crate | E0433 x2, E0432 x1 (monitoring.rs:11) | Remove `nebula_error` imports, use local `MemoryError`. Per CLAUDE.md: "do NOT use `nebula-error` dependency" |
| References to non-existent `AllocErrorCode` type and `MemoryError::with_layout` method | E0432 x1, E0599 x2, E0061 x2 (allocator/monitored.rs) | Remove `AllocErrorCode` import. Replace `AllocError::with_layout(0, layout)` with `AllocError::allocation_failed_with_layout(layout)` |
| Wrong path `crate::core::error::MemoryResult` | E0433 x2 (stats/config.rs, stats/mod.rs) | Change to `crate::error::MemoryResult` / `crate::error::MemoryError` |

**Rationale**: All errors are leftover references to a previous error system that was refactored. The `.old` backup files contain the old error system. The current `error.rs` defines `MemoryError` with different method names.

**Alternatives considered**: Adding `nebula_error` dependency — rejected per constitution (Principle II: Isolated Error Handling).

## R2: Rust 2024 Edition `unsafe` Changes

### Decision: Add explicit `unsafe {}` blocks inside `unsafe fn` bodies

**Analysis**: Rust 2024 (Edition 2024, stabilized in 1.85+) changed the behavior of `unsafe fn`: the function body is no longer implicitly an unsafe block. All unsafe operations inside `unsafe fn` must now be wrapped in explicit `unsafe {}` blocks.

This affects 11 locations in nebula-memory, primarily in:
- `allocator/traits.rs` — blanket implementations calling `allocate`/`deallocate`/`grow`/`shrink`
- `utils.rs` — `copy_aligned_simd` calling `ptr::copy_nonoverlapping`

**Rationale**: This is a mechanical fix required by the edition. No design decision needed.

**Alternatives considered**: None — this is mandatory for Rust 2024.

## R3: heapless Dependency

### Decision: Keep heapless, it's not related to no_std

**Analysis**: `heapless` is used in exactly one place: `allocator/manager.rs:131` for a stack-allocated `FnvIndexMap<AllocatorId, &'static dyn ManagedAllocator, 16>`. This is a performance optimization (fixed-size, no heap allocation for the allocator registry), not a no_std requirement.

**Rationale**: Removing heapless would require replacing with `HashMap` which adds heap allocation in the allocator manager hot path. Not worth changing.

**Alternatives considered**: Replace with `HashMap` — rejected for performance reasons.

## R4: Workspace Impact of Removing Compression

### Decision: Safe to remove — no downstream consumers

**Analysis**: 
- `nebula-expression` depends on `nebula-memory` with `features = ["cache"]` — no compression dependency
- `nebula-value` has nebula-memory commented out (disabled, pending docs)
- No other crate in the workspace depends on `nebula-memory`'s compression feature

**Rationale**: Zero downstream impact.

## R5: Cross-Platform syscall Implementation Review

### Decision: Current implementation is adequate, needs minor fixes

**Analysis**: The `syscalls/` module provides three tiers:
1. **Unix**: `libc::mmap`, `munmap`, `mprotect`, `madvise`, `msync` — full featured
2. **Windows**: `winapi::VirtualAlloc`, `VirtualFree`, `VirtualProtect`, `FlushViewOfFile` — full featured
3. **Fallback**: `std::alloc::alloc`/`dealloc` — basic but functional

Memory pressure monitoring uses `nebula_system::memory` which already handles cross-platform detection.

**Issues found**:
- `memory_prefetch()`: Linux uses `MADV_WILLNEED`, Windows/other uses volatile reads — acceptable fallback
- `get_memory_page_info()`: Linux reads `/proc/[pid]/maps`, Windows uses `VirtualQuery` — no macOS implementation, falls through to empty result. Needs macOS path or graceful empty return.

**Rationale**: The current three-tier approach is sound. Minor macOS gap in `get_memory_page_info` needs documentation or a basic implementation.

## R6: Backup File Value Assessment

### Decision: Extract error patterns from .old files, delete all .bak files

**Analysis**: 

**Valuable (.old files)**:
- `allocator/error.rs.old` (~600 lines): Contains `AllocErrorCode` enum with 11 variants, `Severity` levels, `ErrorStats` with atomic counters. The error code categorization is worth considering for `MemoryError` — but the current `MemoryError` already covers most cases with different naming.
- `core/error.rs.old` (~400 lines): Contains `MemoryErrorCode` with 22 variants and category organization. Again, current `MemoryError` already covers these.

**Conclusion**: The current `MemoryError` in `error.rs` already has 15+ variants covering allocation, pool, arena, cache, budget, and system errors. The `.old` files represent an older approach with separate code enums that was intentionally refactored away. No patterns need to be extracted — the current system is more idiomatic (using `thiserror` directly).

**Not valuable (.bak files)**:
- All 9 `.bak` files are snapshots before refactoring. The current versions of these files are the improved versions. No extraction needed.

**Rationale**: The refactoring from the old error system to the current one was intentional. Reverting would violate the constitution (Principle II — isolated error handling with thiserror).

## R7: no_std Path Audit

### Decision: 135 conditional blocks, mostly trivial to remove

**Analysis**: The `#[cfg(not(feature = "std"))]` blocks fall into categories:
1. **Import switching** (~60%): `use core::X` vs `use std::X` — remove, keep only `std` path
2. **Stub types** (~20%): Empty or panic-ing alternatives for std-only types — delete entirely
3. **Feature-gated modules** (~15%): Entire module conditionals — simplify to always-available
4. **Misc** (~5%): Conditional derives, test cfgs — simplify

The `alloc` and `streaming` features exist only for no_std and can be removed from Cargo.toml.

**Rationale**: With `std` required, all these conditionals are dead code and add cognitive overhead without value.
