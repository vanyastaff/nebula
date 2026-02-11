# Feature Specification: nebula-memory Pre-Release Readiness

**Feature Branch**: `007-memory-prerelease`  
**Created**: 2026-02-11  
**Status**: Draft  
**Input**: User description: "Bring the incomplete nebula-memory crate to pre-release quality. Address memory isolation and resource management problems present in n8n. Prepare code for Rust 1.92+ (Edition 2024). Dead code should be studied before deletion — it may represent incomplete features worth finishing. The solution must be cross-platform (Windows, Linux, macOS)."

## Clarifications

### Session 2026-02-11

- Q: Поддерживать `no_std` в пре-релизе или убрать? → A: Убрать поддержку `no_std`. Сделать `std` обязательным, убрать `alloc`-only пути и `panic!`-заглушки для `no_std`.
- Q: Доработать модуль компрессии (`compression/`) для пре-релиза или убрать? → A: Убрать модуль компрессии. Удалить модуль и feature-флаг, нет необходимости для текущего пре-релиза.

## Problem Context: Why This Matters

Workflow automation platforms like n8n suffer from fundamental memory isolation and resource management problems:

1. **No per-workflow resource isolation** — a single poorly-designed workflow can spike memory or CPU and crash the entire instance, taking down all running automations ([n8n community request](https://community.n8n.io/t/request-for-per-workflow-resource-isolation-in-n8n/151198)).
2. **Memory bloat from retained execution data** — self-looping workflows retain execution data across iterations, causing exponential slowdowns (from processing 13 days/minute down to 2 days/minute within 6 minutes) ([n8n community report](https://community.n8n.io/t/n8n-workflow-memory-bloat-processing-daily-sales-data-causes-exponential-slowdown-and-stalls/114385)).
3. **Weak sandboxing of expressions** — insufficient isolation in expression evaluation engines has led to critical RCE vulnerabilities (CVE-2025-68613, CVSS 9.9/10.0; CVE-2026-25049) where authenticated users could escape the sandbox ([Orca Security](https://orca.security/resources/blog/cve-2025-68613-n8n-rce-vulnerability/), [The Register](https://www.theregister.com/2026/02/05/n8n_security_woes_roll_on)).
4. **No graceful degradation** — when memory is exhausted, there is no throttling or per-workflow budget; the entire system fails.

nebula-memory addresses problems 1, 2, and 4 at the allocator/budget level. Problem 3 (expression sandboxing) belongs to a future nebula-sandbox crate.

## Dead Code Strategy

The crate contains significant amounts of code that appears unused but may represent **incomplete features worth completing**. The approach for dead code must be:

1. **Study before deleting** — every seemingly-dead module, struct, or function must be evaluated: does it represent a partially-implemented feature that should be finished, or is it truly obsolete?
2. **Backup files (.bak, .old)** — 13 backup files exist in the source tree. The `.old` error files (~1000 lines) contain a sophisticated error system with severity levels, error codes, and statistics that may be worth integrating. These must be reviewed, valuable patterns extracted, and the backup files then removed.
3. **Stub implementations** — several locations use `panic!()` or `unimplemented!()` as placeholders (hierarchical pool factory, multi-level cache cleanup). These must be completed with proper error-returning implementations or removed.
4. **Empty modules** — the `lockfree` module is declared but empty. Either implement it or remove the declaration.
5. **Feature-gated disabled code** — NUMA support is commented out. It should remain feature-gated but with a clear "experimental/unsupported" status documented.
6. **Removed modules** — the `compression/` module and its feature flag are removed from the pre-release (no current need). The `no_std` / `alloc`-only code paths are removed — `std` is now required.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Workflow Developer Uses Memory-Safe Allocators (Priority: P1)

A workflow automation developer integrates nebula-memory to manage memory for workflow node execution. They select an appropriate allocator (bump, pool, or stack), allocate and deallocate memory without triggering undefined behavior, and observe correct resource cleanup when a workflow step completes. The crate compiles cleanly on Rust 1.92+ (Edition 2024) with no warnings on Windows, Linux, and macOS.

**Why this priority**: Without correct, warning-free, cross-platform compilation and safe allocation/deallocation, nothing else works. This is the foundation of the entire crate.

**Independent Test**: Can be fully tested by building the crate with `cargo build`, running `cargo clippy -- -D warnings`, running `cargo test`, and verifying all allocator round-trips (allocate -> use -> deallocate) succeed without Miri violations.

**Acceptance Scenarios**:

1. **Given** a fresh Rust 1.92+ toolchain on any supported platform (Windows, Linux, macOS), **When** the developer runs `cargo check --all-features` on nebula-memory, **Then** the build succeeds with zero errors and zero warnings.
2. **Given** a BumpAllocator, PoolAllocator, or StackAllocator, **When** memory is allocated, written to, read from, and deallocated, **Then** no undefined behavior occurs (validated by Miri) and all memory is properly reclaimed.
3. **Given** the crate compiled with `cargo clippy --all-features -- -D warnings`, **When** the full clippy audit runs, **Then** zero warnings are produced.

---

### User Story 2 - Cross-Platform Memory Management (Priority: P2)

A development team builds their Nebula-based automation platform and deploys it on Windows servers, Linux containers, and macOS developer machines. The memory management layer works identically across all three platforms. Platform-specific optimizations (huge pages, madvise hints) are available where supported but never required — the system gracefully falls back to portable behavior.

**Why this priority**: Cross-platform support is essential for a Rust crate targeting diverse deployment environments. Platform-specific code that only works on Linux blocks adoption.

**Independent Test**: Can be tested by running the full test suite on each target platform and verifying identical behavior for core operations.

**Acceptance Scenarios**:

1. **Given** the crate built on Windows, **When** all allocators are used (bump, pool, stack, system), **Then** allocation, deallocation, and statistics work identically to Linux and macOS.
2. **Given** a platform without huge page support (e.g., macOS), **When** an allocator is configured with huge page hints, **Then** the system silently falls back to standard pages without errors.
3. **Given** the system call abstraction layer, **When** compiled on an unsupported platform (e.g., WebAssembly), **Then** a safe fallback implementation using standard library allocation is used instead of failing to compile.
4. **Given** the memory pressure monitoring system, **When** run on Windows, **Then** it correctly detects system memory levels using platform-appropriate mechanisms (not Linux-only `/proc` parsing).

---

### User Story 3 - Memory Isolation Between Workflow Nodes (Priority: P3)

A platform operator runs multiple workflow nodes concurrently. Each node has its own memory scope (arena/budget). One node cannot access or corrupt memory belonging to another node. If one node exceeds its memory budget, only that node is affected — other nodes continue operating normally. This directly solves n8n's "one rogue workflow crashes everything" problem.

**Why this priority**: Memory isolation is the core value proposition over n8n's shared-memory model. Without isolation, workflows can corrupt each other's state.

**Independent Test**: Can be fully tested by creating two independent memory budgets, allocating under both concurrently, exhausting one budget, and verifying the other remains unaffected.

**Acceptance Scenarios**:

1. **Given** two independent memory budgets (A and B), **When** budget A is exhausted, **Then** allocations against budget B still succeed.
2. **Given** two arena-scoped allocators running on separate threads, **When** both allocate and mutate data concurrently, **Then** no data races or cross-contamination occur (validated by thread-safety tests and Miri).
3. **Given** a hierarchical budget (parent -> child), **When** the child budget is released, **Then** the parent budget reflects the freed capacity.
4. **Given** a workflow node that enters an allocation loop (simulating n8n's memory bloat pattern), **When** its budget limit is reached, **Then** further allocations fail gracefully with a descriptive error, and other workflow nodes are not impacted.

---

### User Story 4 - Observable Memory Usage and Pressure Monitoring (Priority: P4)

An operations engineer monitors memory usage of a running Nebula instance. They can query per-allocator statistics (bytes allocated, peak usage, allocation count) and receive pressure-based signals when system memory is running low, allowing the system to gracefully degrade.

**Why this priority**: Observability is essential for production operation but is secondary to correct allocation and isolation.

**Independent Test**: Can be fully tested by enabling statistics tracking, performing known allocations, querying statistics, and verifying reported values match expectations.

**Acceptance Scenarios**:

1. **Given** an allocator with `track_stats: true`, **When** 10 allocations of 64 bytes each are performed, **Then** statistics report exactly 10 allocations and at least 640 bytes allocated.
2. **Given** the memory monitoring system is active, **When** system memory pressure exceeds a configured threshold, **Then** a pressure event is raised with the appropriate severity level.
3. **Given** statistics are enabled on multiple allocators, **When** global statistics are queried, **Then** they reflect the aggregate of all individual allocators.

---

### User Story 5 - Clean, Complete Public API (Priority: P5)

A library consumer adds nebula-memory as a dependency. The public API is well-organized: incomplete features are either finished or cleanly removed, backup files are absent, stub implementations are replaced with proper error handling, and documentation accurately reflects the current API. Feature flags are clearly documented and work correctly.

**Why this priority**: A clean, complete API surface is required for a credible pre-release. Dead code and panicking stubs undermine trust.

**Independent Test**: Can be tested by running `cargo doc --no-deps`, inspecting generated docs, and verifying no `.bak`/`.old` files or panicking stubs exist.

**Acceptance Scenarios**:

1. **Given** the crate source tree, **When** inspected for backup files (`.bak`, `.old`, `.tmp`), **Then** none are found.
2. **Given** a consumer adds `nebula-memory` with default features, **When** they explore the public API, **Then** all public types, traits, and functions have doc comments.
3. **Given** any error condition in the crate (invalid config, exhausted pool, unsupported operation), **When** triggered, **Then** the system returns a `Result::Err` with actionable context — never panics.
4. **Given** the crate is built with `cargo doc --no-deps`, **When** documentation is generated, **Then** zero documentation warnings are produced and all links resolve.

---

### User Story 6 - All Tests and Examples Pass (Priority: P6)

A contributor clones the repository and runs the full test suite. All unit tests, integration tests, and examples compile and pass on all supported platforms. The README examples accurately reflect the current API.

**Why this priority**: Full test coverage is the final gate for pre-release confidence.

**Independent Test**: Can be tested by running `cargo test -p nebula-memory --all-features` and `cargo run -p nebula-memory --example <each_example>`.

**Acceptance Scenarios**:

1. **Given** a clean checkout on Rust 1.92+, **When** `cargo test -p nebula-memory --all-features` is run, **Then** all tests pass (zero failures).
2. **Given** each example in `examples/`, **When** compiled and run, **Then** each exits successfully with no panics or errors.
3. **Given** the README code snippets, **When** compared against the actual API, **Then** all snippets are accurate and would compile.

### Edge Cases

- What happens when an allocator is created with a zero-size buffer? The system must return a clear error, not panic or allocate successfully.
- What happens when an allocation request exceeds the remaining capacity of a bump allocator? A descriptive `MemoryError` must be returned with actionable suggestions.
- What happens when a pool allocator runs out of free blocks and growth is disabled? The system must return `MemoryError::PoolExhausted` rather than blocking indefinitely.
- How does the system behave under concurrent allocation from 100+ threads? Thread-safe variants must not deadlock, corrupt data, or panic.
- What happens when a child budget exceeds its parent's remaining capacity? The child allocation must fail with a budget error; the parent's state must remain consistent.
- What happens when the system runs out of physical memory (OS-level)? The system allocator wrapper must propagate the OS error as a `MemoryError`, not trigger an abort.
- What happens when platform-specific features (huge pages, NUMA) are requested on an unsupported platform? The system must silently fall back to standard behavior, not fail.
- What happens when a removed feature (e.g., compression) is referenced in user code after upgrading? The crate must fail at compile time with a clear feature-gate error, not at runtime.

## Requirements *(mandatory)*

### Functional Requirements

**Compilation & Code Quality:**
- **FR-001**: The crate MUST compile on Rust 1.92+ (Edition 2024) with zero errors and zero warnings under `cargo check --all-features`.
- **FR-002**: The crate MUST pass `cargo clippy --all-features -- -D warnings` with zero warnings.
- **FR-003**: The crate MUST pass `cargo fmt --all -- --check` with no formatting violations.

**Cross-Platform:**
- **FR-004**: The crate MUST compile and pass all tests on Windows (x86_64), Linux (x86_64), and macOS (x86_64 and aarch64).
- **FR-005**: Platform-specific optimizations (huge pages, memory advise hints, direct system calls) MUST be feature-gated and MUST fall back to portable standard-library implementations on unsupported platforms.
- **FR-006**: The system call abstraction layer MUST provide implementations for Windows (`VirtualAlloc`/`VirtualFree`), Unix (`mmap`/`munmap`), and a fallback using `std::alloc` for other platforms.
- **FR-007**: Memory pressure monitoring MUST work on Windows, Linux, and macOS using platform-appropriate mechanisms.

**Core Allocation:**
- **FR-008**: All allocator types (Bump, Pool, Stack, System) MUST correctly allocate, reallocate, and deallocate memory without undefined behavior.
- **FR-009**: Zero-size allocations MUST be handled consistently across all allocator types (either succeed with a valid sentinel or fail with a clear error).
- **FR-010**: Thread-safe allocator variants MUST be safe for concurrent use from multiple threads without data races, deadlocks, or corruption.

**Memory Isolation & Budgets:**
- **FR-011**: Memory arena scopes MUST provide complete isolation — memory allocated in one scope MUST NOT be accessible from another scope.
- **FR-012**: Memory budgets MUST enforce hard limits — when a budget is exhausted, further allocations MUST fail with a descriptive error rather than silently succeeding or panicking.
- **FR-013**: Hierarchical budgets MUST correctly propagate capacity changes — releasing a child budget MUST restore capacity to the parent.

**Observability:**
- **FR-014**: Statistics tracking MUST accurately report allocation counts, byte counts, and peak usage when enabled.
- **FR-015**: Memory pressure monitoring MUST detect and signal when system memory usage exceeds configured thresholds.

**Code Completeness (Dead Code Strategy):**
- **FR-016**: Every partially-implemented feature MUST be evaluated: complete the implementation if it adds value for pre-release, or remove it cleanly with documented rationale — no panicking stubs may remain in the crate.
- **FR-017**: All backup files (`.bak`, `.old`, `.tmp`) MUST be removed from the source tree after valuable patterns are extracted.
- **FR-018**: Empty module declarations (e.g., `lockfree`) MUST be either implemented or removed.
- **FR-019**: All `panic!()` and `unimplemented!()` in non-test code MUST be replaced with proper `Result::Err` returns or removed.

**API Surface & Documentation:**
- **FR-020**: All public types, traits, and functions MUST have documentation comments.
- **FR-021**: All examples in the `examples/` directory MUST compile and run successfully.
- **FR-022**: All integration tests MUST pass.
- **FR-023**: The README MUST accurately reflect the current MSRV (1.92), API surface, feature flags, and supported platforms.
- **FR-024**: Feature flags MUST work correctly in isolation — enabling any single feature MUST NOT cause compilation failures.
- **FR-025**: Error types MUST provide actionable context — each error variant MUST include enough information for the caller to diagnose and recover from the failure.

### Key Entities

- **Allocator**: A memory provider that offers allocate/deallocate/reallocate operations. Multiple specialized implementations (Bump, Pool, Stack, System) each optimized for different access patterns.
- **Arena**: A region-scoped memory container that allocates sequentially and deallocates all memory at once when the arena is dropped. Provides memory isolation boundaries between workflow nodes.
- **Memory Budget**: A capacity constraint applied to a set of allocations. Supports hierarchical parent-child relationships where child budgets are bounded by their parent. Prevents one workflow from consuming all system memory.
- **Object Pool**: A pre-allocated collection of reusable memory blocks of fixed size. Returns blocks to the pool on deallocation rather than freeing them.
- **Memory Statistics**: Counters and metrics describing allocator behavior — allocation count, byte count, peak usage, fragmentation ratio.
- **Memory Monitor**: A system-level observer that tracks OS memory pressure and signals the application when thresholds are crossed. Must work cross-platform.
- **System Call Abstraction**: A platform abstraction layer providing memory mapping, protection, and advise operations with implementations for Windows, Unix, and a fallback.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The crate compiles with zero warnings on Rust 1.92+ across all feature flag combinations on Windows, Linux, and macOS.
- **SC-002**: 100% of unit tests and integration tests pass on all three supported platforms.
- **SC-003**: 100% of examples compile and run without errors.
- **SC-004**: Concurrent allocation from 8+ threads completes without data races, verified by thread-safety tests.
- **SC-005**: Memory budget enforcement correctly rejects allocations that would exceed the budget in 100% of test cases.
- **SC-006**: Per-allocator statistics are accurate to within 0% deviation from expected values in controlled test scenarios.
- **SC-007**: No backup or temporary files (`.bak`, `.old`, `.tmp`) remain in the source tree.
- **SC-008**: All public API items have documentation, verified by `cargo doc --no-deps` with zero warnings.
- **SC-009**: Clippy produces zero warnings under `cargo clippy --all-features -- -D warnings`.
- **SC-010**: Zero `panic!()` or `unimplemented!()` calls exist in non-test production code paths.
- **SC-011**: Platform-specific features gracefully fall back on unsupported platforms without compilation or runtime failures.

## Assumptions

- The sandbox/isolation crate (nebula-sandbox) is explicitly out of scope for this feature. nebula-memory focuses on allocator-level memory isolation (arenas, budgets, scopes), not process-level or OS-level sandboxing (which is where n8n's expression injection CVEs would be addressed).
- The MSRV is updated from 1.70 (stated in the current README) to 1.92 to match the workspace-wide Rust 2024 Edition requirement.
- NUMA-aware allocation remains feature-gated and documented as "experimental/unsupported" — it is not blocking for the pre-release.
- The crate will not yet be published to crates.io — "pre-release" means the code is clean, tested, and ready for internal use within the Nebula workspace.
- Dead code / backup files will be studied before deletion. Valuable patterns (especially from the ~1000 lines in `.old` error files) will be integrated into the current codebase before the backup files are removed.
- "Cross-platform" means Windows (x86_64), Linux (x86_64), and macOS (x86_64 + aarch64). Other targets (WebAssembly, embedded) are not required but should not be blocked by the code structure.
- The hierarchical pool factory (`create_child_static`) and multi-level cache cleanup thread are known incomplete implementations that must be finished or removed as part of this work.
- `no_std` support is dropped for the pre-release. The `std` feature is required. `alloc`-only code paths and `no_std` panic stubs are removed. This can be revisited in a future release if there is demand.
- The `compression/` module is removed from the pre-release. There is no current use case for in-allocator compression in the workflow engine. The module and its feature flag (`compression`) are deleted entirely.

## Scope Boundaries

**In scope:**
- Fix all compilation warnings for Rust 1.92+ / Edition 2024
- Ensure cross-platform compilation and behavior (Windows, Linux, macOS)
- Review and complete or remove partially-implemented features (dead code strategy)
- Extract valuable patterns from `.bak`/`.old` files, then delete them
- Replace all `panic!()` stubs with proper error handling
- Remove `no_std` / `alloc`-only code paths — make `std` required
- Remove `compression/` module and its feature flag
- Fix or remove failing tests
- Clean up empty modules, unused imports, dead code
- Ensure all feature flags compile correctly in isolation
- Complete documentation for public API
- Fix README to reflect current state (MSRV, API, features, platforms)
- Ensure memory isolation between arenas/budgets works correctly
- Ensure all examples compile and run
- Ensure platform-specific code has proper fallbacks

**Out of scope:**
- `no_std` support (deferred — `std` is required for pre-release)
- Compression module (removed — no current need, can be re-added later)
- Process-level or OS-level sandboxing (future nebula-sandbox crate)
- Expression sandboxing (relates to n8n CVEs but belongs in nebula-sandbox/nebula-expression)
- Publishing to crates.io
- New allocator implementations not already started
- Performance benchmarking campaigns (existing benchmarks should pass, but no new targets)
- WebAssembly or embedded platform support (should not be blocked, but not tested)
- NUMA-aware allocation implementation (keep feature-gated, document as experimental)

## References

- [n8n: Request for Per-Workflow Resource Isolation](https://community.n8n.io/t/request-for-per-workflow-resource-isolation-in-n8n/151198) — the core problem nebula-memory solves
- [n8n: Memory Bloat from Workflow Loops](https://community.n8n.io/t/n8n-workflow-memory-bloat-processing-daily-sales-data-causes-exponential-slowdown-and-stalls/114385) — execution data retained across iterations
- [CVE-2025-68613: n8n RCE via Expression Injection](https://orca.security/resources/blog/cve-2025-68613-n8n-rce-vulnerability/) — weak sandbox isolation (out of scope, future nebula-sandbox)
- [n8n Security Advisory 2026-01-08](https://blog.n8n.io/security-advisory-20260108/) — systemic isolation weaknesses
- [SandCell: Sandboxing Rust Beyond Unsafe Code](https://arxiv.org/html/2509.24032v1) — relevant research for future nebula-sandbox
- [n8n Memory-Related Errors Documentation](https://docs.n8n.io/hosting/scaling/memory-errors/) — official n8n memory troubleshooting
