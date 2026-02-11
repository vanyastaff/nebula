# Quickstart: nebula-memory Pre-Release Implementation

**Date**: 2026-02-11
**Branch**: `007-memory-prerelease`

## Prerequisites

- Rust 1.92+ with Edition 2024
- Windows, Linux, or macOS
- Repository cloned and on branch `007-memory-prerelease`

## Implementation Order

Work through phases sequentially. Each phase has a validation gate.

### Phase 1: Fix Compilation Errors

```bash
# Current state: 9 errors, 32 warnings
cargo check -p nebula-memory --all-features

# Fix in this order:
# 1. monitoring.rs:11 — remove nebula_error imports, use MemoryError
# 2. allocator/monitored.rs — remove AllocErrorCode, fix with_layout calls
# 3. stats/config.rs, stats/mod.rs — fix crate::core::error → crate::error paths

# Gate:
cargo check -p nebula-memory --all-features  # 0 errors
```

### Phase 2: Remove Compression + no_std

```bash
# 1. Delete src/compression/ directory
# 2. Remove compression, alloc, streaming features from Cargo.toml
# 3. Remove lz4_flex dependency
# 4. Simplify all #[cfg(not(feature = "std"))] blocks
# 5. Delete src/lockfree/mod.rs
# 6. Update lib.rs module declarations

# Gate:
cargo check -p nebula-memory --all-features  # Still compiles
```

### Phase 3: Fix Warnings

```bash
# Fix all 32 warnings (many may vanish after Phase 2)
# Key: add unsafe {} inside unsafe fn bodies (Rust 2024)

# Gate:
cargo check -p nebula-memory --all-features 2>&1 | grep "warning" | wc -l  # 0
```

### Phase 4: Replace Panic Stubs

```bash
# Fix remaining panic!() calls in production code
# Add tests for new error paths

# Gate:
grep -rn "panic!" crates/nebula-memory/src/ --include="*.rs" | grep -v test  # empty
cargo test -p nebula-memory --all-features
```

### Phase 5: Process Backup Files

```bash
# Review .old files for patterns, then delete all .bak/.old files

# Gate:
find crates/nebula-memory -name "*.bak" -o -name "*.old"  # empty
```

### Phase 6: Cross-Platform Validation

```bash
# Review syscalls/ for platform coverage
# Ensure fallbacks exist for all platform-specific code

# Gate:
cargo check -p nebula-memory --all-features  # on your platform
cargo check --workspace  # no breakage
```

### Phase 7: Fix Tests + Examples

```bash
# Gate:
cargo test -p nebula-memory --all-features
for example in $(ls crates/nebula-memory/examples/*.rs | xargs -n1 basename | sed 's/.rs//'); do
  cargo run -p nebula-memory --example "$example"
done
```

### Phase 8: Documentation

```bash
# Gate:
cargo doc -p nebula-memory --no-deps --all-features  # 0 warnings
cargo test --doc -p nebula-memory --all-features
```

### Phase 9: Final Quality Gates

```bash
cargo fmt --all -- --check
cargo clippy -p nebula-memory --all-features -- -D warnings
cargo check -p nebula-memory --all-features --all-targets
cargo test -p nebula-memory --all-features
cargo doc -p nebula-memory --no-deps --all-features
cargo check --workspace  # no workspace breakage
```

## Key Files to Modify

| File | Phase | Change |
| ---- | ----- | ------ |
| `src/monitoring.rs` | 1 | Remove nebula_error imports |
| `src/allocator/monitored.rs` | 1 | Fix AllocErrorCode + with_layout |
| `src/stats/config.rs` | 1 | Fix error path |
| `src/stats/mod.rs` | 1 | Fix error path |
| `Cargo.toml` | 2 | Remove features + deps |
| `src/lib.rs` | 2 | Remove module declarations |
| `src/compression/` | 2 | DELETE entirely |
| `src/lockfree/` | 2 | DELETE entirely |
| ~20 files with `cfg(not(feature = "std"))` | 2 | Simplify conditionals |
| ~15 files with unsafe fn bodies | 3 | Add unsafe {} blocks |
| `src/allocator/manager.rs` | 4 | Replace panic with Result |
| `src/pool/hierarchical.rs` | 4 | Replace panic with Result |
| 11 .bak/.old files | 5 | DELETE after review |
| `src/syscalls/` | 6 | Verify cross-platform |
| `README.md` | 8 | Update MSRV, features, platforms |
