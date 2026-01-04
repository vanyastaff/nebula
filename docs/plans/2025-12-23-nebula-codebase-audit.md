# Nebula Codebase Audit Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Systematically audit the Nebula workflow automation codebase for common programming issues, anti-patterns, security vulnerabilities, and performance problems across all 16 crates.

**Architecture:** Multi-phase audit approach covering Rust-specific issues, concurrency patterns, error handling, resource management, security vulnerabilities, and performance bottlenecks. Each phase focuses on specific categories with automated tooling and manual code review.

**Tech Stack:** Rust 2024, Tokio async runtime, cargo clippy, cargo audit, cargo-geiger (unsafe code), cargo-deny, miri (for UB detection)

---

## Phase 1: Automated Static Analysis Setup

### Task 1: Configure Enhanced Linting Tools

**Files:**
- Create: `docs/audit/2025-12-23-audit-report.md`
- Create: `.cargo/audit.toml`
- Modify: `.github/workflows/ci.yml` (if exists)

**Step 1: Create audit configuration**

Create `.cargo/audit.toml`:
```toml
[advisories]
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]
ignore = []

[output]
format = "terminal"
quiet = false
```

**Step 2: Initialize audit report**

Create `docs/audit/2025-12-23-audit-report.md`:
```markdown
# Nebula Codebase Audit Report
Date: 2025-12-23

## Executive Summary
- Total Crates: 16
- Audit Categories: 11
- Status: In Progress

## Findings

### Critical Issues
(To be filled during audit)

### High Priority
(To be filled during audit)

### Medium Priority
(To be filled during audit)

### Low Priority / Recommendations
(To be filled during audit)

## Audit Progress
- [ ] Phase 1: Automated Analysis
- [ ] Phase 2: Memory Safety
- [ ] Phase 3: Concurrency Issues
- [ ] Phase 4: Rust-Specific Issues
- [ ] Phase 5: Error Handling
- [ ] Phase 6: Resource Management
- [ ] Phase 7: Security Vulnerabilities
- [ ] Phase 8: Performance Issues
- [ ] Phase 9: API Design
- [ ] Phase 10: Testing Quality
- [ ] Phase 11: Architecture Review
```

**Step 3: Run initial clippy audit**

Run:
```bash
cargo clippy --workspace --all-features --all-targets -- -D warnings -W clippy::all -W clippy::pedantic -W clippy::nursery
```

Expected: List of all clippy warnings/errors

**Step 4: Document clippy findings**

Append findings to audit report under "Phase 1: Automated Analysis"

**Step 5: Commit**

```bash
git add docs/audit/ .cargo/audit.toml
git commit -m "chore(audit): initialize codebase audit infrastructure"
```

---

### Task 2: Run Security Vulnerability Scan

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Install cargo-audit**

Run:
```bash
cargo install cargo-audit
```

Expected: cargo-audit installed successfully

**Step 2: Run security audit**

Run:
```bash
cargo audit --color never > docs/audit/cargo-audit-output.txt 2>&1
```

Expected: Security vulnerability report

**Step 3: Analyze audit results**

Review `docs/audit/cargo-audit-output.txt` for:
- Known vulnerabilities in dependencies
- Unmaintained crates
- Yanked crates

**Step 4: Document security findings**

Add findings to audit report:
```markdown
### Security Audit (cargo audit)
- Total advisories: X
- Critical: X
- High: X
- Medium: X
- Low: X

#### Details:
[List each advisory with CVE, affected crate, version, and remediation]
```

**Step 5: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): add cargo-audit security scan results"
```

---

### Task 3: Check for Unsafe Code Usage

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Install cargo-geiger**

Run:
```bash
cargo install cargo-geiger
```

Expected: cargo-geiger installed

**Step 2: Scan for unsafe code**

Run:
```bash
cargo geiger --output-format GitHubMarkdown > docs/audit/unsafe-code-report.md
```

Expected: Report of all unsafe code blocks

**Step 3: Review unsafe usage**

For each unsafe block found, verify:
- Is it necessary?
- Is it properly documented with safety invariants?
- Are safety requirements enforced?

**Step 4: Document unsafe code review**

Add to audit report:
```markdown
### Unsafe Code Analysis
- Total unsafe blocks: X
- Crates with unsafe: [list]
- Justified: X
- Needs review: X
- Should be removed: X

#### Critical unsafe blocks requiring review:
[List with file:line and reason]
```

**Step 5: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): analyze unsafe code usage with cargo-geiger"
```

---

## Phase 2: Memory Safety & Corruption Review

### Task 4: Integer Overflow/Underflow Audit

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Search for unchecked arithmetic**

Run:
```bash
rg "(\+|\-|\*|/|%)\s*(?!checked_|saturating_|wrapping_)" --type rust -g '!target/' -g '!tests/' > docs/audit/arithmetic-operations.txt
```

Expected: List of all arithmetic operations

**Step 2: Review arithmetic in critical paths**

Focus on:
- `nebula-value` (number operations)
- `nebula-memory` (size calculations)
- `nebula-resource` (capacity limits)
- Loop counters and index calculations

**Step 3: Identify risky patterns**

Look for:
```rust
// RISKY: Unchecked addition
let total = a + b;

// SAFE: Checked arithmetic
let total = a.checked_add(b).ok_or(Error::Overflow)?;

// SAFE: Saturating arithmetic
let total = a.saturating_add(b);
```

**Step 4: Document findings**

Add to audit report:
```markdown
### Integer Overflow/Underflow
#### High Risk:
- File: `path/to/file.rs:123`
  Issue: Unchecked addition in size calculation
  Severity: High
  Recommendation: Use `checked_add()` or `saturating_add()`

#### Medium Risk:
[List]

#### Low Risk / Acceptable:
[List with justification]
```

**Step 5: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review integer overflow/underflow risks"
```

---

### Task 5: Out-of-Bounds Access Review

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Search for direct indexing**

Run:
```bash
rg "\[.+\](?!\.get)" --type rust -g '!target/' -g '!tests/' > docs/audit/array-indexing.txt
```

Expected: List of all array/vec indexing operations

**Step 2: Review indexing patterns**

Look for:
```rust
// RISKY: Direct indexing (can panic)
let item = vec[index];

// SAFE: Bounds-checked access
let item = vec.get(index).ok_or(Error::IndexOutOfBounds)?;

// SAFE: Iterator usage
for item in vec.iter() { }
```

**Step 3: Check slice operations**

Review:
- `split_at()` calls
- `&slice[start..end]` ranges
- `chunks()` and `windows()` usage

**Step 4: Document findings**

Add to audit report:
```markdown
### Out-of-Bounds Access
#### Direct indexing requiring review:
[List with risk assessment]

#### Safe patterns:
[List approved usage]
```

**Step 5: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review array bounds and indexing safety"
```

---

## Phase 3: Concurrency & Parallelism Review

### Task 6: Data Race Detection

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Review Arc without Mutex**

Run:
```bash
rg "Arc<(?!Mutex|RwLock|AtomicU)" --type rust crates/
```

Expected: List of Arc usage without synchronization

**Step 2: Check for shared mutable state**

Look for patterns like:
```rust
// RISKY: Arc<RefCell<T>> across threads
let shared = Arc::new(RefCell::new(data));

// SAFE: Arc<Mutex<T>> or Arc<RwLock<T>>
let shared = Arc::new(Mutex::new(data));
```

**Step 3: Review Atomic operations**

Check:
- Atomic ordering (Relaxed vs Acquire/Release vs SeqCst)
- Proper memory barriers
- ABA problem in lock-free structures

**Step 4: Verify Send/Sync bounds**

Run:
```bash
rg "impl.*Send|impl.*Sync" --type rust crates/
```

Review each manual Send/Sync implementation for correctness

**Step 5: Document findings**

```markdown
### Data Race Analysis
#### Potential data races:
[List with explanation]

#### Atomic ordering review:
[List cases where ordering may be too weak]

#### Manual Send/Sync implementations:
[List with safety justification]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): analyze data race and synchronization patterns"
```

---

### Task 7: Deadlock Detection

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Map lock acquisition patterns**

Run:
```bash
rg "\.lock\(\)|\.write\(\)|\.read\(\)" --type rust crates/ -A 5 > docs/audit/lock-patterns.txt
```

**Step 2: Identify lock ordering**

Look for:
- Multiple locks acquired in same function
- Nested lock acquisition
- Different lock orders in different code paths

```rust
// DEADLOCK RISK: Inconsistent lock order
fn a() {
    let _x = lock1.lock();
    let _y = lock2.lock();
}

fn b() {
    let _y = lock2.lock(); // Different order!
    let _x = lock1.lock();
}
```

**Step 3: Review timeout mechanisms**

Check if locks use:
- `try_lock()` with timeout
- Deadlock detection/recovery
- Lock hierarchy documentation

**Step 4: Check for lock convoy**

Look for:
- Long critical sections
- Blocking I/O inside locks
- Complex computation inside locks

**Step 5: Document findings**

```markdown
### Deadlock Analysis
#### Potential deadlock scenarios:
[List lock ordering issues]

#### Lock convoy risks:
[List long critical sections]

#### Recommendations:
[List improvements]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): analyze deadlock and lock contention risks"
```

---

### Task 8: Async Runtime Blocking Review

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Find blocking operations in async**

Run:
```bash
rg "async fn.*\{" --type rust -A 20 crates/ | rg "(\.lock\(\)\.unwrap|thread::sleep|std::fs::|blocking)" > docs/audit/async-blocking.txt
```

**Step 2: Review blocking patterns**

Look for:
```rust
// BAD: Blocking in async
async fn process() {
    let data = std::fs::read("file.txt"); // Blocks executor!
    std::thread::sleep(Duration::from_secs(1)); // Blocks!
    let guard = mutex.lock().unwrap(); // Can block!
}

// GOOD: Proper async
async fn process() {
    let data = tokio::fs::read("file.txt").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let guard = mutex.lock().await;
}
```

**Step 3: Check spawn_blocking usage**

Verify:
- CPU-intensive work uses `spawn_blocking`
- Blocking I/O uses `spawn_blocking`
- Thread pool not exhausted

**Step 4: Review select! bias**

Check for:
```rust
// May starve some branches
tokio::select! {
    _ = fut1 => {},
    _ = fut2 => {},
}

// Needs biased if order matters
tokio::select! {
    biased;
    _ = fut1 => {},
    _ = fut2 => {},
}
```

**Step 5: Document findings**

```markdown
### Async Blocking Issues
#### Blocking operations in async:
[List with severity]

#### spawn_blocking usage:
[Verify correct usage]

#### Select bias issues:
[List where bias is needed]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review async runtime blocking issues"
```

---

## Phase 4: Rust-Specific Issues Review

### Task 9: Lifetime and Borrow Checker Issues

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Review complex lifetime bounds**

Run:
```bash
rg "<'[a-z].*:" --type rust crates/ | rg "where|for<" > docs/audit/complex-lifetimes.txt
```

**Step 2: Check for lifetime elision mistakes**

Look for functions where elision may hide bugs:
```rust
// May compile but be wrong
fn process(s: &str) -> &str { s }

// Explicit is better
fn process<'a>(s: &'a str) -> &'a str { s }
```

**Step 3: Review interior mutability**

Check:
- RefCell usage (can panic at runtime)
- Cell usage correctness
- UnsafeCell usage (if any)

```rust
// RISKY: RefCell can panic
let cell = RefCell::new(data);
let b1 = cell.borrow_mut();
let b2 = cell.borrow_mut(); // PANIC!

// SAFER: Check before borrowing
if let Ok(guard) = cell.try_borrow_mut() {
    // Use guard
}
```

**Step 4: Check Mutex poisoning handling**

Run:
```bash
rg "\.lock\(\)\.unwrap\(\)" --type rust crates/
```

Each `.unwrap()` on a lock can panic if poisoned. Consider:
```rust
// RISKY: Panics on poisoned mutex
let guard = mutex.lock().unwrap();

// SAFER: Handle poisoning
let guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
```

**Step 5: Document findings**

```markdown
### Lifetime & Borrow Checker Issues
#### Complex lifetime bounds:
[List overly complex signatures]

#### Interior mutability risks:
[List RefCell panic risks]

#### Mutex poisoning:
[List unhandled poisoning]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review lifetime and borrowing patterns"
```

---

### Task 10: Trait Object and PhantomData Review

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Review trait object safety**

Run:
```bash
rg "dyn\s+\w+" --type rust crates/ > docs/audit/trait-objects.txt
```

**Step 2: Check for object safety violations**

Look for:
- Generic methods in dyn Trait
- Sized requirements
- Associated type issues

**Step 3: Review PhantomData usage**

Run:
```bash
rg "PhantomData" --type rust crates/
```

Verify:
- Correct variance (covariant, contravariant, invariant)
- Drop check needs
- Proper Send/Sync implications

**Step 4: Check for Sized trait issues**

Look for:
```rust
// May need ?Sized
fn process<T>(value: T) { }

// Should be
fn process<T: ?Sized>(value: &T) { }
```

**Step 5: Document findings**

```markdown
### Trait Object & PhantomData
#### Trait object safety:
[List issues]

#### PhantomData usage:
[Verify correctness]

#### Sized trait issues:
[List restrictive bounds]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review trait objects and phantom data"
```

---

## Phase 5: Error Handling Review

### Task 11: Silent Failure Detection

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Find ignored Results**

Run:
```bash
rg "let\s+_\s+=.*;" --type rust crates/ | rg "Result|Option" > docs/audit/ignored-results.txt
```

**Step 2: Check for empty error handlers**

Run:
```bash
rg "\.unwrap_or\(\)|\.unwrap_or_default\(\)|\.ok\(\)" --type rust crates/
```

**Step 3: Review error logging**

Look for:
```rust
// BAD: Silent failure
let _ = operation();

// BAD: Lost context
operation().ok();

// GOOD: Logged failure
if let Err(e) = operation() {
    error!("Operation failed: {}", e);
}

// GOOD: Propagated
operation()?;
```

**Step 4: Check panic usage**

Run:
```bash
rg "panic!|unwrap\(\)|expect\(" --type rust crates/ -g '!tests/'
```

Verify each panic:
- Is it in library code? (bad)
- Is it in application code only? (acceptable)
- Could it be a Result instead?

**Step 5: Document findings**

```markdown
### Error Handling
#### Silent failures:
[List ignored errors]

#### Panic in library code:
[List with severity]

#### Missing error context:
[List where context is lost]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): analyze error handling patterns"
```

---

## Phase 6: Resource Management Review

### Task 12: Resource Leak Detection

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Review RAII patterns**

Check:
- File handles properly closed
- Network connections properly closed
- Locks properly released (RAII)
- Memory properly freed

**Step 2: Check for manual resource management**

Run:
```bash
rg "ManuallyDrop|forget|into_raw|from_raw" --type rust crates/
```

Each manual resource management needs careful review.

**Step 3: Review Drop implementations**

Run:
```bash
rg "impl.*Drop" --type rust crates/
```

Verify:
- Drop order is correct
- No panics in Drop
- Resources are cleaned up
- No deadlocks in Drop

**Step 4: Check unbounded growth**

Look for:
- Vec/HashMap without capacity limits
- Channel without bounds
- Cache without eviction
- Queue without size limits

```rust
// RISKY: Unbounded growth
let mut cache = HashMap::new();
cache.insert(key, value); // No limit!

// SAFE: Bounded cache
let mut cache = LruCache::new(1000);
```

**Step 5: Document findings**

```markdown
### Resource Management
#### Potential leaks:
[List resources not properly managed]

#### Unbounded growth:
[List collections without limits]

#### Drop order issues:
[List problematic Drop impls]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review resource management and RAII"
```

---

## Phase 7: Security Vulnerabilities Review

### Task 13: Input Validation Audit

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Review parameter validation**

Check `nebula-parameter` and `nebula-validator`:
- Are all inputs validated?
- Are bounds checked?
- Are types enforced?

**Step 2: Check for injection risks**

Run:
```bash
rg "format!|println!|execute|Command::new" --type rust crates/
```

Look for:
- String formatting with user input (format string injection)
- Shell command construction (command injection)
- SQL query construction (SQL injection - if applicable)

**Step 3: Review deserialization**

Run:
```bash
rg "serde|deserialize|from_str" --type rust crates/
```

Check for:
- Untrusted input deserialization
- Size limits on deserialized data
- Type confusion attacks

**Step 4: Check path traversal**

Run:
```bash
rg "Path::new|PathBuf::from|std::fs" --type rust crates/
```

Verify:
- User-provided paths are validated
- No `..` traversal allowed
- Paths are canonicalized

```rust
// RISKY: Path traversal
let path = PathBuf::from(user_input); // Could be ../../../etc/passwd

// SAFE: Validate and canonicalize
let path = sanitize_path(user_input)?;
let canonical = path.canonicalize()?;
if !canonical.starts_with(&base_dir) {
    return Err(Error::InvalidPath);
}
```

**Step 5: Document findings**

```markdown
### Security Vulnerabilities
#### Injection risks:
[List with severity]

#### Path traversal risks:
[List with mitigation]

#### Deserialization issues:
[List untrusted input handling]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): analyze input validation and injection risks"
```

---

### Task 14: Cryptography Review

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Review credential handling**

Check `nebula-credential`:
- Are secrets stored securely?
- Are secrets logged? (should not be)
- Are secrets in memory zeroed on drop?
- Is timing attack mitigation used?

**Step 2: Check for weak crypto**

Run:
```bash
rg "md5|sha1|rand::random|Random" --type rust crates/
```

Look for:
- Weak hash functions (MD5, SHA1)
- Insecure random (not cryptographically secure)
- Hardcoded keys/passwords

**Step 3: Review timing attack surface**

Check:
- String comparison (use constant-time comparison)
- Early returns on mismatch
- Cache timing issues

```rust
// VULNERABLE: Timing attack
if password == stored_password {
    return Ok(());
}

// SAFE: Constant-time comparison
use subtle::ConstantTimeEq;
if password.ct_eq(stored_password).into() {
    return Ok(());
}
```

**Step 4: Check secret logging**

Run:
```bash
rg "debug!|info!|warn!|error!|println!" --type rust crates/nebula-credential/
```

Verify no secrets are logged.

**Step 5: Document findings**

```markdown
### Cryptography & Secrets
#### Weak cryptography:
[List issues]

#### Timing attack vectors:
[List vulnerable comparisons]

#### Secret handling:
[List logging/storage issues]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review cryptography and secret handling"
```

---

## Phase 8: Performance Issues Review

### Task 15: Algorithmic Complexity Audit

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Find nested loops**

Run:
```bash
rg "for.*\{[\s\S]*for.*\{" --type rust crates/ -U > docs/audit/nested-loops.txt
```

**Step 2: Review O(n²) patterns**

Look for:
- Nested iteration over same collection
- Repeated linear searches
- Quadratic algorithms where linear exists

```rust
// BAD: O(n²)
for item in &vec1 {
    for other in &vec2 {
        if item == other { }
    }
}

// GOOD: O(n) with HashSet
let set: HashSet<_> = vec2.iter().collect();
for item in &vec1 {
    if set.contains(item) { }
}
```

**Step 3: Check excessive cloning**

Run:
```bash
rg "\.clone\(\)" --type rust crates/ -g '!tests/' > docs/audit/cloning.txt
```

Review each clone:
- Is it necessary?
- Could we use references?
- Could we use Arc/Rc?

**Step 4: Review string concatenation**

Run:
```bash
rg "\+.*&str|format!.*\{.*\}" --type rust crates/
```

Look for loops concatenating strings:
```rust
// BAD: O(n²) in loop
let mut result = String::new();
for s in strings {
    result = result + s; // Allocates each time!
}

// GOOD: O(n)
let result = strings.join("");
// or
let mut result = String::with_capacity(total_len);
for s in strings {
    result.push_str(s);
}
```

**Step 5: Document findings**

```markdown
### Performance Issues
#### Algorithmic complexity:
[List O(n²) or worse patterns]

#### Excessive cloning:
[List hot paths with clones]

#### String concatenation:
[List inefficient patterns]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): analyze algorithmic complexity and allocations"
```

---

### Task 16: Memory Usage Audit

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Review nebula-memory crate**

Focus on:
- Pool sizes and limits
- Cache eviction policies
- Arena allocator usage
- Memory budget enforcement

**Step 2: Check for unnecessary boxing**

Run:
```bash
rg "Box::new" --type rust crates/ -g '!tests/'
```

Review each Box:
- Is heap allocation necessary?
- Could we use stack allocation?
- Is it for trait objects? (justified)

**Step 3: Review collection capacity**

Run:
```bash
rg "Vec::new\(\)|HashMap::new\(\)" --type rust crates/
```

Look for:
- Preallocated capacity where size is known
- Reserve calls for growing collections

```rust
// GOOD: Preallocate
let mut vec = Vec::with_capacity(known_size);

// BAD: Multiple allocations
let mut vec = Vec::new();
for item in items {
    vec.push(item); // May reallocate many times
}
```

**Step 4: Check for memory leaks in tests**

Run a sampling of tests with Valgrind or similar:
```bash
cargo test --test integration_test
# Check for leaks
```

**Step 5: Document findings**

```markdown
### Memory Usage
#### Unnecessary boxing:
[List cases]

#### Missing capacity hints:
[List collections that should preallocate]

#### Memory leaks:
[List any detected leaks]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review memory usage and allocations"
```

---

## Phase 9: API Design Review

### Task 17: Public API Audit

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Review public exports**

For each crate, check:
```bash
rg "^pub (fn|struct|enum|trait|type|const|static|mod)" --type rust crates/*/src/lib.rs
```

**Step 2: Check for breaking changes**

Look for:
- Public struct fields (should be private with accessors)
- Missing `#[non_exhaustive]` on enums/structs
- Missing stability attributes
- Inconsistent naming

**Step 3: Review builder patterns**

Check if builders use:
- `#[must_use]` attributes
- Typestate pattern for safety
- Consuming methods vs borrowing

**Step 4: Check for leaky abstractions**

Look for:
- Implementation details in public API
- Platform-specific types exposed
- Internal error types leaked

**Step 5: Document findings**

```markdown
### API Design
#### Breaking change risks:
[List public items that could break]

#### Missing must_use:
[List builders/constructors without must_use]

#### Leaky abstractions:
[List internal details exposed]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review public API design"
```

---

## Phase 10: Testing Quality Review

### Task 18: Test Coverage and Quality

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Generate coverage report**

Run:
```bash
cargo tarpaulin --workspace --out Html --output-dir docs/audit/coverage
```

Expected: HTML coverage report

**Step 2: Review test organization**

Check:
- Unit tests in each module
- Integration tests in `tests/`
- Property-based tests where appropriate
- Benchmark tests for performance-critical code

**Step 3: Identify flaky tests**

Run tests multiple times:
```bash
for i in {1..10}; do cargo test --workspace || echo "FAIL $i"; done
```

Document any failures.

**Step 4: Review test quality**

Look for:
- Tests testing implementation, not behavior
- Hardcoded values instead of property tests
- Missing edge cases (empty, single, max values)
- Missing error cases

**Step 5: Check mock usage**

Review:
- Are mocks overused?
- Are integration tests preferred?
- Are mocks properly reset between tests?

**Step 6: Document findings**

```markdown
### Testing Quality
#### Coverage:
- Overall: X%
- Critical paths: X%
- Low coverage areas: [list]

#### Flaky tests:
[List unstable tests]

#### Test quality issues:
[List improvement areas]
```

**Step 7: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): analyze test coverage and quality"
```

---

## Phase 11: Architecture Review

### Task 19: Dependency and Modularity Audit

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Generate dependency graph**

Run:
```bash
cargo tree --workspace --charset ascii > docs/audit/dependency-tree.txt
```

**Step 2: Check for circular dependencies**

Review dependency tree for cycles at the crate level.

**Step 3: Review crate organization**

Check:
- Is each crate focused? (SRP)
- Are dependencies minimal?
- Is there unnecessary coupling?

**Step 4: Check for god objects**

Run:
```bash
rg "^impl.*\{$" --type rust -A 1000 crates/ | rg "pub fn" -c | sort -rn | head -20
```

Look for structs with too many methods.

**Step 5: Document findings**

```markdown
### Architecture
#### Circular dependencies:
[List any cycles]

#### Crate organization:
[List recommendations]

#### God objects:
[List overly large types]
```

**Step 6: Commit**

```bash
git add docs/audit/
git commit -m "chore(audit): review architecture and modularity"
```

---

## Final Task: Audit Summary and Recommendations

### Task 20: Compile Audit Results

**Files:**
- Modify: `docs/audit/2025-12-23-audit-report.md`

**Step 1: Aggregate all findings**

Compile all issues from phases 1-11 into summary tables:

```markdown
## Summary of Findings

| Category | Critical | High | Medium | Low | Total |
|----------|----------|------|--------|-----|-------|
| Memory Safety | X | X | X | X | X |
| Concurrency | X | X | X | X | X |
| Rust Issues | X | X | X | X | X |
| Security | X | X | X | X | X |
| Performance | X | X | X | X | X |
| Error Handling | X | X | X | X | X |
| Resources | X | X | X | X | X |
| API Design | X | X | X | X | X |
| Testing | X | X | X | X | X |
| Architecture | X | X | X | X | X |
| **Total** | **X** | **X** | **X** | **X** | **X** |
```

**Step 2: Prioritize issues**

Create action items:
```markdown
## Action Items

### Immediate (Critical)
1. [Issue with file:line reference]
2. [Issue with file:line reference]

### Short-term (High)
1. [Issue]
2. [Issue]

### Medium-term (Medium)
[List]

### Long-term / Nice-to-have (Low)
[List]
```

**Step 3: Create remediation tracking**

```markdown
## Remediation Tracking

- [ ] Fix all critical issues
- [ ] Fix all high-priority issues
- [ ] Create tickets for medium-priority issues
- [ ] Document low-priority items for future consideration
```

**Step 4: Write executive summary**

Update the executive summary with:
- Total issues found
- Risk assessment
- Key recommendations
- Estimated effort

**Step 5: Final commit**

```bash
git add docs/audit/
git commit -m "chore(audit): complete codebase audit with summary"
```

---

## Notes

- This audit is systematic but not exhaustive
- Focus on high-risk areas first
- Use automated tools but verify with manual review
- Security and concurrency issues are highest priority
- Performance issues should be validated with profiling
- Document all findings with file:line references
- Create separate issues/tickets for fixes
- Re-audit after fixes to verify resolution

