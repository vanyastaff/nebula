# Cargo Optimization Guide

## 📦 Cargo Build Optimization

### Release Profile Configuration

```toml
# Cargo.toml

[profile.release]
opt-level = 3              # Maximum optimization
lto = true                 # Link-time optimization
codegen-units = 1          # Better optimization, slower compile
strip = true               # Strip symbols from binary
panic = 'abort'            # Smaller binary size

[profile.release.build-override]
opt-level = 0              # Don't optimize build scripts

[profile.dev]
opt-level = 0              # Fast compilation
debug = true
incremental = true
```

### Advanced Optimization Profiles

```toml
# Ultra-optimized release
[profile.ultra]
inherits = "release"
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true
panic = 'abort'

# Size-optimized (for embedded/wasm)
[profile.min-size]
inherits = "release"
opt-level = "z"            # Optimize for size
lto = true
codegen-units = 1
strip = true
panic = 'abort'

# Fast compile for development
[profile.dev-fast]
inherits = "dev"
opt-level = 1
incremental = true
```

## ⚡ Compilation Speed Optimization

### .cargo/config.toml

```toml
# .cargo/config.toml

[build]
# Use faster linker (mold on Linux, lld on others)
rustflags = ["-C", "link-arg=-fuse-ld=lld"]

# Parallel compilation
jobs = 8                   # Or number of CPU cores

[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]

[target.x86_64-pc-windows-msvc]
linker = "lld-link.exe"

[target.x86_64-apple-darwin]
rustflags = ["-C", "link-arg=-fuse-ld=/usr/local/opt/llvm/bin/ld64.lld"]
```

### Incremental Compilation

```bash
# Enable incremental compilation (default in dev)
export CARGO_INCREMENTAL=1

# Sccache for compilation caching
cargo install sccache
export RUSTC_WRAPPER=sccache

# Check sccache statistics
sccache --show-stats
```

## 🔧 Dependency Optimization

### Features Management

```toml
# Cargo.toml

[features]
default = ["std"]
std = []
full = ["std", "async", "caching", "performance"]
async = ["tokio"]
caching = ["lru"]
performance = ["simd", "rayon"]
minimal = []

# Make large dependencies optional
[dependencies]
tokio = { version = "1.0", optional = true, features = ["full"] }
rayon = { version = "1.7", optional = true }
```

### Workspace Optimization

```toml
# Workspace Cargo.toml

[workspace]
members = [
    "crates/nebula-memory",
    "crates/nebula-validator",
    "crates/nebula-expression",
]

# Share dependencies across workspace
[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.0", features = ["full"] }

# Then in member crates:
[dependencies]
serde = { workspace = true }
tokio = { workspace = true }
```

## 🚀 Runtime Performance

### Profile-Guided Optimization (PGO)

```bash
# Step 1: Build instrumented binary
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" \
    cargo build --release

# Step 2: Run typical workload
./target/release/my-app benchmark

# Step 3: Merge profile data
llvm-profdata merge -o /tmp/pgo-data/merged.profdata /tmp/pgo-data

# Step 4: Build with PGO
RUSTFLAGS="-Cprofile-use=/tmp/pgo-data/merged.profdata" \
    cargo build --release
```

### CPU-Specific Optimization

```bash
# Build for native CPU architecture
RUSTFLAGS="-C target-cpu=native" cargo build --release

# Or in .cargo/config.toml
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "target-cpu=native"]
```

## 📊 Binary Size Optimization

### Minimize Binary Size

```toml
# Cargo.toml

[profile.release]
opt-level = "z"            # Optimize for size
lto = true
codegen-units = 1
strip = true
panic = 'abort'

[dependencies]
# Use smaller allocators
[target.'cfg(not(target_env = "msvc"))'.dependencies]
jemallocator = "0.5"
```

### Strip Debug Info

```bash
# Manual stripping
strip target/release/my-app

# Or use strip = true in Cargo.toml
cargo build --release

# Check binary size
ls -lh target/release/my-app
```

### UPX Compression (for executables)

```bash
# Install UPX
# Ubuntu/Debian: apt install upx-ucl
# macOS: brew install upx

# Compress binary
upx --best --lzma target/release/my-app

# Result: 50-70% size reduction
```

## 🧪 Test Optimization

### Parallel Testing

```bash
# Use all CPU cores
cargo test --release -- --test-threads=8

# Or let cargo decide
cargo test --release

# Skip doc tests for speed
cargo test --lib --bins --tests
```

### Test Compilation Cache

```bash
# Build tests without running
cargo test --no-run

# Run pre-built tests
cargo test --release

# Cache test builds
sccache --show-stats
```

## 🔍 Profiling and Analysis

### Compilation Time Analysis

```bash
# Show compilation timing
cargo build --timings

# Detailed build graph
cargo build -Z timings=html

# Check what's being rebuilt
cargo build -vv
```

### Runtime Profiling

```bash
# Linux perf
perf record --call-graph dwarf cargo run --release
perf report

# Flamegraph
cargo install flamegraph
cargo flamegraph

# Valgrind
valgrind --tool=callgrind target/release/my-app
kcachegrind callgrind.out.*
```

## 🎯 Nebula-Specific Optimizations

### Workspace Build Strategy

```bash
# Build only changed crates
cargo check -p nebula-memory
cargo check -p nebula-validator

# Use workspace-level caching
cargo build --workspace

# Parallel crate compilation
cargo build --workspace -j 8
```

### Feature Flag Strategy

```toml
# nebula-memory/Cargo.toml

[features]
default = ["std", "lru"]
std = []
full = ["std", "lru", "ttl", "lfu"]
lru = []
ttl = ["std"]
lfu = ["std"]

# Only build needed features
[dependencies]
parking_lot = { version = "0.12", optional = true, features = ["serde"] }
```

### Test Optimization Matrix

```bash
# Quick smoke test (no features)
cargo test --workspace --lib --no-default-features

# Full test with all features
cargo test --workspace --all-features

# Per-crate parallel testing
cargo test -p nebula-memory & \
cargo test -p nebula-validator & \
cargo test -p nebula-expression & \
wait
```

## 📈 Benchmarking

### Criterion Benchmarks

```rust
// benches/my_benchmark.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn fibonacci_benchmark(c: &mut Criterion) {
    c.bench_function("fib 20", |b| {
        b.iter(|| fibonacci(black_box(20)))
    });
}

criterion_group!(benches, fibonacci_benchmark);
criterion_main!(benches);
```

```bash
# Run benchmarks
cargo bench

# Compare with baseline
cargo bench --bench my_benchmark -- --save-baseline my_baseline
# ... make changes ...
cargo bench --bench my_benchmark -- --baseline my_baseline
```

## 🔐 Security Scanning Optimization

### Cargo Audit

```bash
# Install cargo-audit
cargo install cargo-audit

# Audit dependencies
cargo audit

# Audit with JSON output
cargo audit --json

# Auto-fix vulnerabilities
cargo audit fix
```

### Cargo Deny

```toml
# deny.toml

[advisories]
vulnerability = "deny"
unmaintained = "warn"
unsound = "warn"
notice = "warn"

[licenses]
unlicensed = "deny"
allow = ["MIT", "Apache-2.0", "BSD-3-Clause"]
deny = ["GPL-3.0"]

[bans]
multiple-versions = "warn"
wildcards = "deny"
```

```bash
# Install cargo-deny
cargo install cargo-deny

# Run checks
cargo deny check
```

## 💡 Quick Optimization Checklist

### Development

- [ ] Enable incremental compilation
- [ ] Use fast linker (mold/lld)
- [ ] Setup sccache
- [ ] Use `cargo check` instead of `cargo build`
- [ ] Minimize dependencies
- [ ] Use workspace dependencies

### Release

- [ ] Configure release profile (opt-level=3, lto=true)
- [ ] Enable profile-guided optimization
- [ ] Use target-cpu=native for local builds
- [ ] Strip debug symbols
- [ ] Consider UPX compression
- [ ] Run benchmarks

### Testing

- [ ] Parallel test execution
- [ ] Cache test builds
- [ ] Skip doc tests when not needed
- [ ] Use `cargo nextest` for faster tests
- [ ] Minimize test dependencies

### CI/CD

- [ ] Cache cargo registry
- [ ] Cache target/ directory
- [ ] Use sccache in CI
- [ ] Parallel job matrix
- [ ] Only build what changed

## 🛠️ Useful Cargo Tools

```bash
# Fast test runner
cargo install cargo-nextest
cargo nextest run

# Dependency tree
cargo tree

# Outdated dependencies
cargo install cargo-outdated
cargo outdated

# Unused dependencies
cargo install cargo-udeps
cargo +nightly udeps

# Expand macros
cargo expand

# Assembly output
cargo rustc -- --emit asm

# LLVM IR output
cargo rustc -- --emit llvm-ir
```

## 📚 Resources

- [Cargo Book - Profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Optimizing Rust Build Times](https://matklad.github.io/2021/09/04/fast-rust-builds.html)
- [Profile-Guided Optimization](https://doc.rust-lang.org/rustc/profile-guided-optimization.html)
