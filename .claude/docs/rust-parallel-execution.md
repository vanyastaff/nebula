# Rust Parallel Execution Patterns

## 🚨 CRITICAL: CONCURRENT EXECUTION RULE

**ABSOLUTE RULE**: ALL Rust operations MUST be concurrent/parallel in a single message.

> ⚡ **RUST GOLDEN RULE**: "1 MESSAGE = ALL MEMORY-SAFE OPERATIONS"

## 🔴 MANDATORY CONCURRENT PATTERNS

### Cargo Operations
```bash
# ✅ CORRECT: Batch ALL cargo commands in single message
Bash("cargo check -p nebula-memory")
Bash("cargo check -p nebula-validator")
Bash("cargo check -p nebula-expression")
Bash("cargo test --workspace")

# ❌ WRONG: Sequential messages
[Message 1] Bash("cargo check -p nebula-memory")
[Message 2] Bash("cargo check -p nebula-validator")  # Too slow!
```

### Crate Management
```bash
# ✅ CORRECT: Batch ALL dependency operations
Bash("cargo add serde serde_json")
Bash("cargo add tokio --features full")
Bash("cargo add --dev proptest criterion")
Bash("cargo build --all-features")

# ❌ WRONG: One dependency at a time
```

### Testing
```bash
# ✅ CORRECT: Run ALL test suites in parallel
Bash("cargo test -p nebula-memory --lib")
Bash("cargo test -p nebula-validator --lib")
Bash("cargo test -p nebula-expression --lib")
Bash("cargo test --workspace --doc")

# ❌ WRONG: Wait for each test suite
```

## 📦 Correct Concurrent Execution Examples

### Example 1: Full Crate Setup
```
[Single Message]:
  - TodoWrite { todos: [all setup tasks] }
  - Bash("cargo new my-rust-app --bin")
  - Bash("cd my-rust-app && cargo add serde tokio reqwest")
  - Bash("cd my-rust-app && cargo add --dev proptest criterion")
  - Write("Cargo.toml", cargoConfiguration)
  - Write("src/main.rs", mainApplication)
  - Write("src/lib.rs", libraryModule)
  - Write("tests/integration_test.rs", integrationTests)
  - Bash("cd my-rust-app && cargo build && cargo test")
```

### Example 2: Fixing Multiple Issues
```
[Single Message]:
  - TodoWrite { todos: [all fix tasks] }
  - Read("crates/nebula-memory/src/cache/policies/lfu.rs")
  - Read("crates/nebula-memory/src/cache/policies/ttl.rs")
  - Read("crates/nebula-memory/src/cache/policies/mod.rs")
  - Edit("lfu.rs", fix1)
  - Edit("ttl.rs", fix2)
  - Edit("mod.rs", fix3)
  - Bash("cargo check -p nebula-memory")
  - Bash("cargo test -p nebula-memory --lib policies")
```

### Example 3: Testing All Crates
```
[Single Message]:
  - Bash("cargo test -p nebula-memory --lib")
  - Bash("cargo test -p nebula-validator --lib")
  - Bash("cargo test -p nebula-parameter --lib")
  - Bash("cargo test -p nebula-expression --lib")
  - Bash("cargo test -p nebula-derive --lib")
  - Bash("cargo test --workspace --doc")
```

## 🎯 Nebula-Specific Patterns

### Issue Fixing Pattern
```
[Single Message]:
  - TodoWrite([
      "Analyze Issue #N",
      "Read all affected files",
      "Apply architectural fix",
      "Test all changes",
      "Document solution"
    ])
  - Bash("gh issue view N --json title,body")
  - Read("affected/file1.rs")
  - Read("affected/file2.rs")
  - Read("affected/file3.rs")
  - Edit("file1.rs", architecturalFix1)
  - Edit("file2.rs", architecturalFix2)
  - Bash("cargo check -p affected-crate")
  - Bash("cargo test -p affected-crate --lib")
  - Bash("gh issue comment N --body '✅ Fixed!'")
```

### Multiple Crate Refactoring
```
[Single Message]:
  - TodoWrite([all refactoring tasks])
  - Glob("crates/nebula-*/src/**/*.rs")
  - Read("crates/nebula-memory/src/mod.rs")
  - Read("crates/nebula-validator/src/mod.rs")
  - Edit("nebula-memory/src/cache.rs", applyExtensionTrait)
  - Edit("nebula-validator/src/traits.rs", applyTypeErasure)
  - Bash("cargo check --workspace")
  - Bash("cargo test --workspace --lib")
  - Bash("cargo clippy --all-features")
```

## 🔧 Memory Safety Coordination

### Ownership Pattern Batch
```
[Single Message]:
  - Write("src/ownership/smart_pointers.rs", smartPointers)
  - Write("src/ownership/lifetimes.rs", lifetimePatterns)
  - Write("src/ownership/borrowing.rs", borrowingExamples)
  - Write("tests/memory_safety.rs", memorySafetyTests)
  - Bash("cargo build")
  - Bash("cargo miri test")  # If miri is available
```

## ⚡ Async/Concurrency Coordination

### Tokio Async Batch
```
[Single Message]:
  - Write("src/async/runtime.rs", tokioRuntimeConfig)
  - Write("src/async/tasks.rs", asyncTaskHandling)
  - Write("src/async/channels.rs", channelCommunication)
  - Write("tests/async_tests.rs", asyncTestCases)
  - Bash("cargo add tokio --features full")
  - Bash("cargo test --features async")
```

## 🧪 Testing Coordination

### Comprehensive Testing Batch
```
[Single Message]:
  - Write("tests/integration_test.rs", integrationTests)
  - Write("tests/common/mod.rs", testUtilities)
  - Write("benches/benchmark.rs", criterionBenchmarks)
  - Write("tests/property_tests.rs", proptestCases)
  - Bash("cargo test --all-features")
  - Bash("cargo bench --no-run")
  - Bash("cargo test --doc")
```

## 🚀 Performance Optimization Batch

### Performance Enhancement
```
[Single Message]:
  - Write("src/performance/simd.rs", simdOptimizations)
  - Write("src/performance/zero_copy.rs", zeroCopyPatterns)
  - Write("benches/performance_bench.rs", performanceBenchmarks)
  - Edit("Cargo.toml", addReleaseOptimizations)
  - Bash("cargo build --release")
  - Bash("cargo bench --all-features")
```

## 📊 Code Quality Coordination

### Quality Toolchain Batch
```
[Single Message]:
  - Write("rustfmt.toml", rustfmtConfiguration)
  - Write("clippy.toml", clippyConfiguration)
  - Bash("cargo fmt --all")
  - Bash("cargo clippy --all-targets --all-features -- -D warnings")
  - Bash("cargo test --workspace")
```

## 🎯 Performance Tips

### Why Parallel Execution Matters

1. **Cargo Lock Contention**: Cargo can handle parallel builds internally
2. **I/O Parallelism**: Network, disk operations benefit from concurrency
3. **Token Efficiency**: Single message = single API call
4. **User Experience**: Faster results, less waiting

### Benchmarks

```
Sequential (bad):
  cargo check crate1  [30s]
  cargo check crate2  [30s]
  cargo check crate3  [30s]
  Total: 90s

Parallel (good):
  cargo check crate1 & crate2 & crate3  [35s]
  Total: 35s (2.5x faster!)
```

## 🔄 CI/CD Integration

### GitHub Actions Batch
```
[Single Message]:
  - Write(".github/workflows/ci.yml", rustCI)
  - Write(".github/workflows/security.yml", securityWorkflow)
  - Write("scripts/ci-test.sh", ciTestScript)
  - Bash("cargo test --all-features")
  - Bash("cargo clippy --all-targets -- -D warnings")
  - Bash("cargo audit")
```

## 💡 Best Practices Summary

### DO ✅
- Batch ALL cargo operations in single message
- Run tests in parallel across all crates
- Read multiple files concurrently
- Edit multiple files before testing
- Use TodoWrite for complex task tracking

### DON'T ❌
- Wait for one cargo command before starting next
- Test crates sequentially
- Make multiple messages for related operations
- Read files one by one when you need multiple
- Skip TodoWrite for multi-step operations

## 📚 Nebula Project Examples

### Closing Multiple Issues
```
[Single Message]:
  - TodoWrite([issue3, issue53, issue2 tasks])
  - Bash("gh issue view 3 --json title,body")
  - Bash("gh issue view 53 --json title,body")
  - Read("crates/nebula-memory/src/cache/policies/*.rs")
  - Read("crates/nebula-validator/src/combinators/*.rs")
  - Edit("policies/lfu.rs", enableLFU)
  - Edit("policies/ttl.rs", addClearMethod)
  - Edit("combinators/optional.rs", fixRust2024)
  - Bash("cargo test -p nebula-memory --lib policies")
  - Bash("cargo test -p nebula-validator --lib")
  - Bash("gh issue close 3 --comment 'Fixed LFU!'")
```

### Full Workspace Check
```
[Single Message]:
  - Bash("cargo check -p nebula-memory")
  - Bash("cargo check -p nebula-validator")
  - Bash("cargo check -p nebula-parameter")
  - Bash("cargo check -p nebula-expression")
  - Bash("cargo check -p nebula-derive")
  - Bash("cargo check -p nebula-log")
  - Bash("cargo check -p nebula-resilience")
  - Bash("cargo clippy --workspace --all-features")
```

## 🎓 Learning Resources

- [Rust Async Book](https://rust-lang.github.io/async-book/)
- [Cargo Parallel Execution](https://doc.rust-lang.org/cargo/reference/build-scripts.html)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)
