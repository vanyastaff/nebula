# Fuzzing Tests for nebula-value

This directory contains fuzzing tests using [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) and [libFuzzer](https://llvm.org/docs/LibFuzzer.html).

## Overview

Fuzzing is a technique for finding bugs by feeding random/mutated inputs to the code and checking for crashes, panics, or undefined behavior. It's particularly effective at finding:

- Panics and crashes
- Integer overflows
- Out-of-bounds access
- Infinite loops
- Memory safety issues
- Logic errors with edge cases

## Fuzz Targets

### 1. `fuzz_serde` - JSON Serialization/Deserialization

Tests JSON parsing and serialization with arbitrary UTF-8 input:

```rust
// Fuzzes:
- serde_json::from_str (arbitrary JSON)
- serde_json::to_string (roundtrip)
- serde_json::to_string_pretty
```

**What it finds:**
- JSON parsing edge cases
- Serialization panics
- Roundtrip inconsistencies
- UTF-8 handling issues

### 2. `fuzz_operations` - Value Operations

Tests all Value arithmetic, comparison, and logical operations with arbitrary values:

```rust
// Fuzzes:
- add, sub, mul, div
- eq, and, or, not
- merge, clone
```

**What it finds:**
- Integer overflow in arithmetic
- Type coercion bugs
- Division by zero handling
- NaN/Infinity edge cases
- Unexpected panics in operations

### 3. `fuzz_text` - Text Operations

Tests Text type with arbitrary UTF-8 strings:

```rust
// Fuzzes:
- Text::from_str (arbitrary UTF-8)
- concat, substring
- len, is_empty
```

**What it finds:**
- Unicode edge cases
- String boundary issues
- Substring out-of-bounds
- UTF-8 validation bugs

### 4. `fuzz_bytes` - Bytes Operations

Tests Bytes type with arbitrary binary data:

```rust
// Fuzzes:
- Bytes::new (arbitrary data)
- Base64 encoding/decoding
- slice operations
```

**What it finds:**
- Base64 encoding bugs
- Slice boundary issues
- Binary data edge cases
- Clone/equality bugs

### 5. `fuzz_collections` - Array/Object Operations

Tests persistent collections with arbitrary data:

```rust
// Fuzzes:
- Array push, concat, get
- Object insert, get, merge
- Iterator operations
```

**What it finds:**
- Collection mutation bugs
- Structural sharing issues
- Out-of-bounds access
- Merge logic errors

## Prerequisites

Install cargo-fuzz:

```bash
cargo install cargo-fuzz
```

**Note**: Fuzzing only works on Linux and macOS with nightly Rust. On Windows, use WSL.

**Status**: All fuzz targets have been migrated to native `Value` type (Sprint 7).
Fuzzing code compiles successfully on Linux/macOS.

## Running Fuzz Tests

### Quick Test (for development)

Run a fuzz target for a short time:

```bash
cd crates/nebula-value
cargo +nightly fuzz run fuzz_serde -- -max_total_time=60
```

### Full Fuzzing Session

Run continuously until stopped:

```bash
cargo +nightly fuzz run fuzz_serde
```

### Run All Targets

```bash
#!/bin/bash
for target in fuzz_serde fuzz_operations fuzz_text fuzz_bytes fuzz_collections; do
    echo "Fuzzing $target..."
    cargo +nightly fuzz run $target -- -max_total_time=300
done
```

### List All Targets

```bash
cargo +nightly fuzz list
```

## Fuzzing Options

### Time-Limited Fuzzing

```bash
# Run for 5 minutes
cargo +nightly fuzz run fuzz_serde -- -max_total_time=300

# Run for 1 hour
cargo +nightly fuzz run fuzz_serde -- -max_total_time=3600
```

### Corpus-Based Fuzzing

```bash
# Use existing corpus
cargo +nightly fuzz run fuzz_serde fuzz/corpus/fuzz_serde

# Minimize corpus
cargo +nightly fuzz cmin fuzz_serde
```

### Memory Limit

```bash
# Limit to 2GB RAM
cargo +nightly fuzz run fuzz_serde -- -rss_limit_mb=2048
```

### Parallel Fuzzing

```bash
# Run 4 workers in parallel
cargo +nightly fuzz run fuzz_serde -- -workers=4 -jobs=4
```

## Interpreting Results

### No Crashes

```
#1234567: cov: 1234 ft: 5678 corp: 42 exec/s: 1234 rss: 128Mb
```

- `cov`: Code coverage (more is better)
- `ft`: Features found
- `corp`: Corpus size
- `exec/s`: Executions per second
- `rss`: Memory usage

### Crash Found

```
==12345==ERROR: AddressSanitizer: heap-buffer-overflow
SUMMARY: AddressSanitizer: heap-buffer-overflow
```

Crash artifacts are saved to `fuzz/artifacts/fuzz_serde/`:

```bash
# Re-run the crash
cargo +nightly fuzz run fuzz_serde fuzz/artifacts/fuzz_serde/crash-abc123

# Debug with gdb
rust-gdb target/x86_64-unknown-linux-gnu/release/fuzz_serde \
  fuzz/artifacts/fuzz_serde/crash-abc123
```

## Corpus Management

### Seed Corpus

Add interesting test cases to `fuzz/corpus/fuzz_serde/`:

```bash
echo '{"key":"value"}' > fuzz/corpus/fuzz_serde/simple_json
echo '{"nested":{"array":[1,2,3]}}' > fuzz/corpus/fuzz_serde/nested
```

### Merge Corpora

```bash
# Merge fuzz_operations corpus into fuzz_serde
cargo +nightly fuzz run fuzz_serde fuzz/corpus/fuzz_serde fuzz/corpus/fuzz_operations
```

### Minimize Corpus

```bash
# Remove redundant test cases
cargo +nightly fuzz cmin fuzz_serde
```

## Continuous Fuzzing

### GitHub Actions (Example)

```yaml
name: Fuzz Testing

on:
  schedule:
    - cron: '0 2 * * *'  # Run nightly

jobs:
  fuzz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@nightly

      - name: Install cargo-fuzz
        run: cargo install cargo-fuzz

      - name: Fuzz for 1 hour
        run: |
          cd crates/nebula-value
          timeout 3600 cargo +nightly fuzz run fuzz_serde || true

      - name: Upload artifacts
        if: failure()
        uses: actions/upload-artifact@v3
        with:
          name: fuzz-artifacts
          path: crates/nebula-value/fuzz/artifacts/
```

### OSS-Fuzz Integration

For continuous fuzzing infrastructure, consider integrating with [OSS-Fuzz](https://github.com/google/oss-fuzz).

## Troubleshooting

### "command not found: cargo-fuzz"

```bash
cargo install cargo-fuzz
```

### "nightly toolchain not installed"

```bash
rustup install nightly
```

### Windows Support

Fuzzing requires Linux/macOS. On Windows, use WSL:

```bash
wsl --install Ubuntu
# Inside WSL:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup install nightly
cargo install cargo-fuzz
```

### Out of Memory

Reduce RSS limit:

```bash
cargo +nightly fuzz run fuzz_serde -- -rss_limit_mb=1024
```

## Coverage Report

Generate coverage from fuzzing:

```bash
# Run with coverage tracking
cargo +nightly fuzz coverage fuzz_serde

# Generate HTML report
cargo +nightly cov -- show \
  target/x86_64-unknown-linux-gnu/release/fuzz_serde \
  --format=html \
  -instr-profile=fuzz/coverage/fuzz_serde/coverage.profdata \
  > coverage.html
```

## Best Practices

1. **Start with short runs** during development (60-300 seconds)
2. **Run longer sessions** (1-24 hours) before releases
3. **Seed the corpus** with interesting test cases
4. **Monitor coverage** to ensure fuzzing is effective
5. **Fix crashes immediately** before they compound
6. **Minimize corpus** regularly to reduce redundancy
7. **Run in CI** for continuous validation

## Integration with Testing

Fuzzing complements other testing approaches:

| Test Type | Purpose | Coverage |
|-----------|---------|----------|
| **Unit tests** | Specific scenarios | Known cases |
| **Property tests** | Mathematical properties | Random valid inputs |
| **Fuzz tests** | Find crashes | Random any inputs |
| **Integration tests** | End-to-end flows | Real-world usage |

Use all four for comprehensive testing.

## Performance

Typical fuzzing performance:

- **fuzz_serde**: ~10,000 exec/s
- **fuzz_operations**: ~50,000 exec/s
- **fuzz_text**: ~30,000 exec/s
- **fuzz_bytes**: ~40,000 exec/s
- **fuzz_collections**: ~20,000 exec/s

To find bugs, aim for:
- **1 million** executions minimum
- **10 million** for thorough testing
- **100 million+** for production systems

## Resources

- [cargo-fuzz book](https://rust-fuzz.github.io/book/cargo-fuzz.html)
- [libFuzzer documentation](https://llvm.org/docs/LibFuzzer.html)
- [Rust Fuzz organization](https://github.com/rust-fuzz)
- [Fuzzing Rust code](https://blog.rust-lang.org/2021/03/18/Rust-1.51.0.html#fuzzing)