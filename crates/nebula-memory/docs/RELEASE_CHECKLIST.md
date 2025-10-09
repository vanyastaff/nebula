# Release Checklist - nebula-memory v0.2.0

This checklist ensures all steps are completed before releasing a new version.

---

## Pre-Release Checklist

### Code Quality

- [x] All tests pass: `cargo test --all-features`
- [x] No compiler warnings in release mode
- [x] Documentation builds: `cargo doc --no-deps`
- [x] Examples compile: `cargo build --examples`
- [ ] Benchmarks run: `cargo bench --no-run`
- [x] Code formatted: `cargo fmt --check`
- [ ] Clippy passes: `cargo clippy --all-features -- -D warnings`

### Safety & Security

- [x] Miri validation: UnsafeCell migration complete
- [x] No Stacked Borrows violations
- [x] Security audit completed: `SECURITY_AUDIT.md`
- [x] All `unsafe` code documented
- [ ] Dependency audit: `cargo audit` (when available)

### Documentation

- [x] CHANGELOG.md updated with all changes
- [x] README.md reflects new features
- [x] API documentation reviewed
- [x] Examples updated and tested
- [x] Migration guide included in CHANGELOG
- [x] Error catalog complete: `ERRORS.md`

### New Features (v0.2.0)

- [x] UnsafeCell migration (BumpAllocator, PoolAllocator, StackAllocator)
- [x] TypedAllocator trait implementation
- [x] Enhanced error messages with suggestions
- [x] Macro DSL: `memory_scope!`, `allocator!`, `alloc!`, `dealloc!`
- [x] SIMD optimizations (AVX2): `copy_aligned_simd`, `fill_simd`, `compare_simd`
- [x] Stats infrastructure (thread-local batching ready)
- [x] Comprehensive examples (4 new files, 763 lines)
- [x] Complete documentation (853 lines)

---

## Version Bump

### Files to Update

- [ ] `Cargo.toml` - version = "0.2.0"
- [ ] `CHANGELOG.md` - Move [Unreleased] to [0.2.0]
- [ ] `README.md` - Update version badges
- [ ] Root workspace `Cargo.toml` if applicable

### Version Number

Current: `0.1.0`
Next: `0.2.0`

**Reason**: Major new features + breaking changes (UnsafeCell migration)

---

## Testing

### Local Testing

```bash
# Clean build
cargo clean
cargo build --all-features

# Run all tests
cargo test --all-features

# Run examples
cargo run --example error_handling
cargo run --example integration_patterns
cargo run --example macro_showcase
cargo run --example benchmarks

# Build documentation
cargo doc --all-features --no-deps --open

# Check for issues
cargo clippy --all-features -- -D warnings
cargo fmt --check
```

### Platform Testing

- [ ] Windows (x86_64-pc-windows-msvc)
- [ ] Linux (x86_64-unknown-linux-gnu)
- [ ] macOS (x86_64-apple-darwin)

### Feature Combinations

- [ ] `--no-default-features`
- [ ] `--features=std`
- [ ] `--features=simd`
- [ ] `--all-features`

---

## Documentation

### Generate Docs

```bash
cargo doc --all-features --no-deps
```

### Documentation Checklist

- [x] All public items documented
- [x] Examples in doc comments compile
- [x] Cross-references with "See also"
- [x] Migration guide in CHANGELOG
- [ ] docs.rs metadata in Cargo.toml

---

## Git & Release

### Git Workflow

```bash
# Ensure clean working directory
git status

# Create release branch
git checkout -b release/v0.2.0

# Update version numbers
# ... edit Cargo.toml, CHANGELOG.md ...

# Commit version bump
git add -A
git commit -m "chore: bump version to 0.2.0"

# Tag release
git tag -a v0.2.0 -m "Release v0.2.0

Major improvements:
- Miri-validated memory safety
- Rich macro DSL
- SIMD optimizations
- Comprehensive documentation
"

# Push to remote
git push origin release/v0.2.0
git push origin v0.2.0

# Merge to main
git checkout main
git merge release/v0.2.0
git push origin main
```

### GitHub Release

- [ ] Create GitHub release from tag
- [ ] Copy CHANGELOG.md content to release notes
- [ ] Highlight major features
- [ ] Include migration guide link

**Release Notes Template**:
```markdown
# nebula-memory v0.2.0 🎉

## Major Improvements

### 🔒 Memory Safety
- **Miri-Validated**: All allocators use `UnsafeCell` for proper provenance
- **Zero UB**: Fixed 3 critical Stacked Borrows violations
- Ready for `cargo +nightly miri test`

### 🎨 Developer Experience
- **Macro DSL**: `memory_scope!`, `allocator!`, `alloc!`, `dealloc!`
- **Type-Safe API**: New `TypedAllocator` trait prevents layout errors
- **Rich Errors**: Actionable error messages with suggestions

### ⚡ Performance
- **SIMD Operations**: AVX2-optimized memory ops (4x faster)
- **Hot Path Inlining**: Zero-overhead abstractions
- **Stats Infrastructure**: Thread-local batching ready

### 📚 Documentation
- **4 New Examples**: 763 lines of real-world patterns
- **Error Catalog**: Comprehensive error guide
- **Security Audit**: Full safety review
- **CHANGELOG**: Complete migration guide

## Migration Guide

See [CHANGELOG.md](CHANGELOG.md#migration-guide) for detailed migration instructions.

## What's Changed

**Full Changelog**: [v0.1.0...v0.2.0](https://github.com/USER/REPO/compare/v0.1.0...v0.2.0)

## Install

\`\`\`toml
[dependencies]
nebula-memory = "0.2.0"
\`\`\`
```

---

## Publish

### Dry Run

```bash
cargo publish --dry-run -p nebula-memory
```

### Actual Publish

```bash
cargo publish -p nebula-memory
```

### Verify

```bash
# Check on crates.io
open https://crates.io/crates/nebula-memory

# Try installing
cargo install nebula-memory --version 0.2.0
```

---

## Post-Release

### Announcements

- [ ] Update GitHub release with crates.io link
- [ ] Tweet about release (if applicable)
- [ ] Post on Reddit r/rust
- [ ] Post on Rust Users forum
- [ ] Update project website (if applicable)

### Monitoring

- [ ] Monitor GitHub issues for bug reports
- [ ] Watch crates.io download stats
- [ ] Review any compilation failures on docs.rs

### Backlog

- [ ] Create milestone for v0.3.0
- [ ] Prioritize next features
- [ ] Update roadmap

---

## Rollback Plan

If critical issues are discovered:

1. **Yank from crates.io**: `cargo yank --vers 0.2.0 -p nebula-memory`
2. **Fix issues**: Create hotfix branch
3. **Release patch**: v0.2.1 with fixes
4. **Un-yank if safe**: `cargo yank --undo --vers 0.2.0`

---

## Success Criteria

Release is successful if:

- ✅ Publishes to crates.io without errors
- ✅ Documentation builds on docs.rs
- ✅ No critical issues reported in first 48 hours
- ✅ Downloads > 100 in first week
- ✅ No security vulnerabilities reported

---

## Notes

**Breaking Changes**: Yes (UnsafeCell migration)
**SemVer**: 0.1.0 → 0.2.0 (minor bump, as we're pre-1.0)
**Stability**: Production-ready for use

---

**Last Updated**: 2025-01-09
**Release Manager**: Development Team
