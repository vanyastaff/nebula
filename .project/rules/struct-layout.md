# Struct Layout & Cache Optimization Protocol

You are an expert in data-oriented design, cache-friendly memory layout, and Rust struct optimization. Your task is to analyze the provided Rust structs and identify all opportunities to improve cache performance, memory bandwidth, and execution speed through layout changes.

## Project Context
- Architecture: x86-64, 64-byte cache lines
- Rust edition: 2024, stable 1.94+
- Concurrency: parking_lot Mutex/RwLock, tokio async, std atomics
- Access patterns: [DESCRIBE HOW EACH STRUCT IS USED — hot path frequency, read vs write ratio, concurrent access patterns]

## Structs to analyze:
[PASTE STRUCTS HERE]

---

## Anti-Laziness & Accuracy Directives
1. You MUST process EVERY struct provided. Do not group them or skip any.
2. You MUST evaluate all 10 analysis points for every struct.
3. **CRITICAL RUST KNOWLEDGE:** Remember that `#[repr(Rust)]` (the default) automatically reorders fields to minimize padding (usually largest alignment to smallest). Do NOT hallucinate padding issues based on source-code order unless `#[repr(C)]` is present, or unless generics/lifetimes prevent optimal packing. Your layout analysis must reflect the *compiler-optimized* layout, not the raw source order.

---

## Analysis Protocol (Apply to EACH struct)

### 1. FIELD TEMPERATURE CLASSIFICATION
Classify every field based on the provided access patterns:
🔥 Hot (accessed every iteration / hot path) | 🌡 Warm | ❄ Cold (setup/error paths) | 💀 Dead

### 2. CURRENT LAYOUT ANALYSIS (Mental rustc simulation)
- Calculate sizes and offsets assuming `#[repr(Rust)]` reordering (or strict order if `#[repr(C)]`).
- Identify 64-byte cache line boundaries.
- Flag hot fields separated across multiple cache lines.
- Flag cold fields polluting the primary hot cache line.

### 3. FALSE SHARING DETECTION
- Identify fields written by different concurrent threads.
- Detect if these independently mutated fields share a 64-byte line.
- Estimate false sharing overhead: High / Medium / Low.

### 4. HOT/COLD SPLITTING OPPORTUNITIES
- If mixed temperatures exist, design a split (hot fields inline, cold fields behind `Box` or `Arc`).
- Evaluate the indirection cost vs. cache density benefit.

### 5. AoS → SoA OPPORTUNITIES
- Identify `Vec<Struct>` usage where hot loops only access a subset of fields.
- Propose `StructOfArrays` layout to maximize hardware prefetcher efficiency.

### 6. ATOMIC / MUTEX LAYOUT
- Recommend `#[repr(align(64))]` on contended atomics/locks.
- Check if `Mutex<T>` wraps a large type that bloats the cache line, forcing unnecessary cache evictions on lock acquisition.

### 7. ENUM REPR OPTIMIZATION
- Analyze discriminant sizes.
- Recommend `#[repr(u8)]`/`#[repr(u32)]` or niche optimizations (`NonZeroUsize`, pointer alignment).

### 8. PADDING ELIMINATION
- Even with `repr(Rust)`, padding exists. Find unavoidable padding bytes.
- Calculate bytes wasted per instance and at scale (e.g., 10,000 instances).

### 9. REFERENCE LOCALITY (Pointer Chasing)
- Flag `Arc<T>` / `Box<T>` / `String` / `Vec` fields adding pointer-chasing in hot paths.
- Suggest inlining small data (e.g., `ArrayVec`, `SmallVec`, or `InlineString`).

### 10. BRANCH PREDICTION HINTS
- Identify boolean/Option fields used in hot conditionals.
- Suggest `#[cold]`, `std::hint::unlikely`, or enum inversion if applicable.

---

## Output Format

For EACH struct, produce exactly this format:

### `[StructName]`
**Current Compiler-Optimized Layout (Estimated):**
```text
offset  size  field           temp  cache_line
0       8     field_a         🔥    0
8       4     field_b         ❄     0
...
total: N bytes, M bytes padding, spans K cache lines
```

**Findings:**

For every analysis point (1–10), output an H4 header. If nothing to report, write: *No issues.*

#### 1. Field Temperature Classification
...

#### 2. Current Layout Analysis
...

#### 3. False Sharing Detection
...

#### 4. Hot/Cold Splitting
...

#### 5. AoS → SoA
...

#### 6. Atomic / Mutex Layout
...

#### 7. Enum Repr Optimization
...

#### 8. Padding Elimination
...

#### 9. Reference Locality
...

#### 10. Branch Prediction Hints
...

**Proposed Optimized Layout:**
```rust
// Show the restructured struct with annotations
#[repr(align(64))]  // if applicable
struct [StructName] {
    // 🔥 hot cluster — cache line 0
    field_a: Type,

    // ❄ cold cluster
    field_b: Type,
}
// total: N bytes (was M bytes), spans K cache lines (was J)
```

**Summary Table:**

| Metric | Before | After | Delta |
|--------|--------|-------|-------|
| Size (bytes) | | | |
| Padding (bytes) | | | |
| Cache lines (hot path) | | | |
| False sharing risk | | | |
| Pointer chasing hops | | | |
