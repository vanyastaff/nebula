# Value Transformers

Guide to automatic value transformation (coercion).

---

## What are Transformers?

**Transformers** automatically adjust invalid values to the nearest valid state, providing better UX than outright rejection.

### Transformer vs Validation

| Aspect | Validation | Transformer |
|--------|------------|-------------|
| **Purpose** | Check if valid | Make valid |
| **Result** | `Result<(), Error>` | Always succeeds |
| **Behavior** | Reject invalid | Fix invalid |
| **User Impact** | Shows error | Silently corrects |
| **Pipeline Order** | After transform | Before validate |

**Example:**
```rust
// WITHOUT transformer: User types "105" in 0-100 field
// -> Error: "Value must be <= 100"
// -> User frustrated

// WITH transformer: User types "105" in 0-100 field
// -> Value becomes 100
// -> No error, slider snaps to max
```

---

## Processing Pipeline

```
User Input: 105
    │
    ▼
┌──────────────────┐
│   1. TRANSFORM   │  Fix invalid → 100  (clamp to 0-100)
└──────────────────┘
    │
    ▼
┌──────────────────┐
│  2. VALIDATE     │  Check valid → OK ✅ (100 is valid)
└──────────────────┘
    │
    ▼
┌──────────────────┐
│   3. STORAGE     │  Store: 100
└──────────────────┘
    │
    ▼
┌──────────────────┐
│   4. NOTIFY      │  Event: ValueChanged { old: 90, new: 100 }
└──────────────────┘
```

---

## Decision Tree: Transformer vs Validation

```
Is the fix obvious?
    │
    ├─ YES → Use TRANSFORMER
    │        Examples:
    │        • 105 → 100 (clamp to max)
    │        • -10° → 350° (wrap angle)
    │        • "  hello  " → "hello" (trim)
    │
    └─ NO → Use VALIDATION
             Examples:
             • Start date > end date (which to change?)
             • Invalid email format (can't auto-fix)
             • Duplicate username (user must choose new)
```

---

## Built-in Transformers

### 1. ClampTransformer

Limits value to a range.

```rust
pub struct ClampTransformer<T> {
    pub min: T,
    pub max: T,
}
```

**Visual:**
```
Input:  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        -50   0   25   50   75   100  125  150

Clamp(0, 100):
Output: ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        0     0   25   50   75   100  100  100
        ▲                                 ▲
      clamped                          clamped
```

**Example:**
```rust
NumberParameter::builder::<f64>("opacity")
    .transformer(ClampTransformer { min: 0.0, max: 1.0 })
    .build()

// Input: -0.5  -> Output: 0.0
// Input: 0.5   -> Output: 0.5
// Input: 1.5   -> Output: 1.0
```

---

### 2. RoundTransformer

Rounds to step increments.

```rust
pub struct RoundTransformer<T> {
    pub step: T,
}
```

**Visual:**
```
Input:  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        0    7   15   22   30   37   45   52

Round(step=15):
Output: ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        0    0   15   15   30   30   45   45
             ▼        ▼         ▼         ▼
           down      down      down      down
```

**Example:**
```rust
NumberParameter::builder::<f64>("angle")
    .transformer(RoundTransformer { step: 15.0 })
    .build()

// Input: 0.0   -> Output: 0.0
// Input: 7.0   -> Output: 0.0
// Input: 8.0   -> Output: 15.0
// Input: 22.5  -> Output: 15.0
// Input: 23.0  -> Output: 30.0
```

---

### 3. ModuloTransformer

Wraps value cyclically (ideal for angles).

```rust
pub struct ModuloTransformer<T> {
    pub modulo: T,
}
```

**Visual:**
```
Angles (modulo 360):

Input:  -180° -90°  0°   90°  180° 270° 360° 450° 540°
         │     │    │    │    │    │    │    │    │
Output:  180° 270°  0°   90°  180° 270°  0°  90°  180°
         wrap  wrap                       wrap wrap wrap

        360°/0°
            │
    270° ───┼─── 90°
            │
         180°
```

**Example:**
```rust
NumberParameter::builder::<f64>("rotation")
    .transformer(ModuloTransformer { modulo: 360.0 })
    .build()

// Input: 0.0    -> Output: 0.0
// Input: 180.0  -> Output: 180.0
// Input: 360.0  -> Output: 0.0
// Input: 370.0  -> Output: 10.0
// Input: -10.0  -> Output: 350.0
// Input: -370.0 -> Output: 350.0
```

---

### 4. SnapTransformer

Snaps to nearest value from a list.

```rust
pub struct SnapTransformer<T> {
    pub values: Vec<T>,
    pub threshold: Option<T>,  // Max distance to snap
}
```

**Visual:**
```
Snap to: [0, 25, 50, 75, 100], threshold=10

Input:  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        0    12   25   38   50   62   75   88  100

Output: ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        0    12   25   38   50   62   75   88  100
        ▼         ▼         ▼         ▼         ▼
      snap      snap      snap      snap      snap
             ×         ×         ×         ×
           no snap   no snap   no snap   no snap
        (>10 away)
```

**Example:**
```rust
NumberParameter::builder::<f64>("quality")
    .transformer(SnapTransformer {
        values: vec![0.0, 0.25, 0.5, 0.75, 1.0],
        threshold: Some(0.1),
    })
    .build()

// Input: 0.0   -> Output: 0.0
// Input: 0.05  -> Output: 0.0   (within threshold)
// Input: 0.12  -> Output: 0.12  (outside threshold, no snap)
// Input: 0.23  -> Output: 0.25  (within threshold)
// Input: 0.48  -> Output: 0.5   (within threshold)
```

---

### 5. NormalizeTransformer

Normalizes vectors to unit length.

```rust
pub struct NormalizeTransformer;
```

**Visual:**
```
Input:  [3, 4, 0]
        Length = √(3² + 4² + 0²) = 5

Output: [0.6, 0.8, 0]
        Length = √(0.6² + 0.8² + 0²) = 1 ✅

   Y
   │   (3,4)
   4   ╱│
   │  ╱ │
   │ ╱  │         After normalize:
   │╱   │
   └────┴─── X          (0.6, 0.8)
   0    3              ╱│
                      ╱ │ Length = 1
                     ╱  │
                    ╱___│
                   0
```

**Example:**
```rust
VectorParameter::<f64, 3>::direction("forward")
    .transformer(NormalizeTransformer)
    .build()

// Input: [3.0, 0.0, 0.0]   -> Output: [1.0, 0.0, 0.0]
// Input: [0.0, 5.0, 0.0]   -> Output: [0.0, 1.0, 0.0]
// Input: [3.0, 4.0, 0.0]   -> Output: [0.6, 0.8, 0.0]
// Input: [0.0, 0.0, 0.0]   -> Output: [0.0, 0.0, 1.0]  (default)
```

---

### 6. AspectRatioTransformer

Maintains proportions when one component changes.

```rust
pub struct AspectRatioTransformer {
    pub locked: bool,
    pub reference_ratio: [f64; 2],
}
```

**Visual:**
```
Original: [1920, 1080] = 16:9 ratio

User changes width to 3840:

Input:  [3840, 1080]  ← height unchanged
         ▲
      doubled width

Output: [3840, 2160]
         ▲     ▲
      doubled  doubled (to maintain 16:9)

   Before:          After:
   ┌────────┐      ┌────────────────┐
   │        │      │                │
   │ 16:9   │  →   │     16:9       │
   │        │      │                │
   └────────┘      └────────────────┘
   1920x1080        3840x2160
```

**Example:**
```rust
VectorParameter::<f64, 2>::builder("size")
    .transformer(AspectRatioTransformer {
        locked: true,
        reference_ratio: [16.0, 9.0],
    })
    .build()

// ratio = 16:9
// Input: [1920.0, 1080.0] -> Output: [1920.0, 1080.0]
// Input: [3840.0, 1080.0] -> Output: [3840.0, 2160.0]  (width doubled -> height doubled)
```

---

### 7. CustomTransformer

Arbitrary transformation logic.

```rust
pub struct CustomTransformer<T, F>
where
    F: Fn(T) -> T + Send + Sync,
{
    pub transform_fn: Arc<F>,
    pub description: String,
}
```

**Visual:**
```
Example: Force even numbers

Input:  0   1   2   3   4   5   6   7   8   9  10
        │   │   │   │   │   │   │   │   │   │   │
Output: 0   2   2   4   4   6   6   8   8  10  10
            ▲       ▲       ▲       ▲
           +1      +1      +1      +1
```

**Example:**
```rust
NumberParameter::builder::<i64>("even_port")
    .transformer(CustomTransformer {
        transform_fn: Arc::new(|v: i64| if v % 2 == 0 { v } else { v + 1 }),
        description: "Round to even number".into(),
    })
    .build()

// Input: 8080  -> Output: 8080
// Input: 8081  -> Output: 8082
```

---

## Transformer Chains

Multiple transformers apply **in order**:

```
Input: 373.0

    │
    ▼
┌──────────────────┐
│ Round(step=15)   │  373.0 → 375.0
└──────────────────┘
    │
    ▼
┌──────────────────┐
│ Modulo(360)      │  375.0 → 15.0
└──────────────────┘
    │
    ▼
Output: 15.0 ✅
```

**Code:**
```rust
NumberParameter::builder::<f64>("angle")
    .transformer(RoundTransformer { step: 15.0 })      // 1. Round to 15°
    .transformer(ModuloTransformer { modulo: 360.0 })  // 2. Wrap to 0-360°
    .build()
```

### Order Matters!

```rust
// Version A: Round -> Clamp
.transformer(RoundTransformer { step: 10.0 })
.transformer(ClampTransformer { min: 0.0, max: 100.0 })

// Version B: Clamp -> Round
.transformer(ClampTransformer { min: 0.0, max: 100.0 })
.transformer(RoundTransformer { step: 10.0 })

// For input 105.0:
// Version A: 110.0 -> 100.0
// Version B: 100.0 -> 100.0
```

---

## Real-World Use Cases

### ✅ Good Use of Transformers

```rust
// 1. Opacity slider
NumberParameter::builder("opacity")
    .range(0.0, 1.0)
    .transformer(ClampTransformer { min: 0.0, max: 1.0 })
    // User types "1.5" → transformed to "1.0" ✅

// 2. Rotation angle
NumberParameter::builder("rotation")
    .transformer(ModuloTransformer { modulo: 360.0 })
    // User types "450°" → transformed to "90°" ✅

// 3. Direction vector
VectorParameter::direction("forward")
    .transformer(NormalizeTransformer)
    // User enters [10, 0, 0] → transformed to [1, 0, 0] ✅

// 4. Discount percentage
NumberParameter::builder("discount")
    .transformer(RoundTransformer { step: 5.0 })
    .transformer(ClampTransformer { min: 0.0, max: 100.0 })
    // User types "47.3%" → transformed to "45%" ✅
```

### ❌ Bad Use of Transformers

```rust
// ❌ Don't transform when user intent unclear
NumberParameter::builder("port")
    .transformer(CustomTransformer {
        // User types 99999
        // Should we use 8080? 80? 443? 3000?
        // Intent unclear → use VALIDATION instead!
        transform_fn: Arc::new(|_| 8080),
    })

// ✅ Better: Clamp to valid range
NumberParameter::builder("port")
    .range(1024, 65535)
    .transformer(ClampTransformer { min: 1024, max: 65535 })
    // User types 99999 → transformed to 65535 ✅
    // Intent clear: use maximum allowed
```

---

## Use Cases Gallery

### Slider with Overshoot Protection

```rust
NumberParameter::builder::<f64>("quality")
    .soft_max(100.0)       // Slider ends at 100
    .hard_max(10000.0)     // API allows up to 10000
    .transformer(ClampTransformer { min: 0.0, max: 10000.0 })
    .build()
```

### Angle with Wrapping

```rust
NumberParameter::builder::<f64>("rotation")
    .subtype(NumberSubtype::Angle)
    .transformer(ModuloTransformer { modulo: 360.0 })
    .build()
```

### Percentage with Rounding

```rust
NumberParameter::builder::<f64>("discount")
    .subtype(NumberSubtype::Percentage)
    .transformer(RoundTransformer { step: 5.0 })
    .transformer(ClampTransformer { min: 0.0, max: 100.0 })
    .build()

// Input: 47% -> 45%
// Input: 99% -> 100%
```

### Direction Vector (Always Normalized)

```rust
VectorParameter::<f64, 3>::direction("look_at")
    .transformer(NormalizeTransformer)
    .default([0.0, 0.0, 1.0])
    .build()
```

### Color Channels (0-1 Clamped)

```rust
VectorParameter::<f64, 3>::color_rgb("color")
    .component_transformer(ClampTransformer { min: 0.0, max: 1.0 })
    .build()

// Input: [1.5, 0.5, -0.1]  -> Output: [1.0, 0.5, 0.0]
```

### Image Size with Aspect Lock

```rust
VectorParameter::<f64, 2>::builder("size")
    .subtype(VectorSubtype::Size2D)
    .default([1920.0, 1080.0])
    .transformer(AspectRatioTransformer {
        locked: true,
        reference_ratio: [16.0, 9.0],
    })
    .build()
```

---

## When to Use Transformers vs Validation

### Use Transformers When:

- ✅ Range limits (clamp to min/max)
- ✅ Rounding to increments
- ✅ Cyclic wrapping (angles)
- ✅ Vector normalization
- ✅ Format fixing (trim, lowercase)
- ✅ User typos that are obvious fixes
- ✅ Slider overrun

### Use Validation When:

- ✅ Complex business rules
- ✅ External constraints (API limits)
- ✅ Data integrity (foreign keys)
- ✅ Security checks
- ✅ Semantic errors (invalid date combinations)
- ✅ User confirmation needed

**Rule of thumb:** If the "correct" value is obvious, use transformer. If user intent is unclear, use validation and show error.

---

## Performance

### Fast Path

```rust
impl<T: Numeric> NumberParameter<T> {
    pub fn transform(&self, value: T) -> T {
        // Fast path: no transformers
        if self.transformers.is_empty() {
            return value;
        }
        
        // Apply transformers
        let mut result = value;
        for t in &self.transformers {
            result = t.transform(result);
        }
        result
    }
}
```

### Optimization Tips

1. **Order transformers cheaply first** - Put clamp before normalize
2. **Combine when possible** - One custom transformer vs many small ones
3. **Skip when valid** - Check if value already valid before transforming

```
❌ Slow: .transformer(A).transformer(B).transformer(C)
✅ Fast: .transformer(Combined(A, B, C))
```

---

## UI Feedback

Optionally show users when transformation occurred:

```rust
pub struct TransformFeedback {
    pub original_value: Value,
    pub final_value: Value,
    pub was_transformed: bool,
    pub description: Option<String>,
}

// Context method with feedback
let feedback = context.set_with_feedback(OPACITY, 1.5)?;
if feedback.was_transformed {
    // Show tooltip for 2 seconds:
    // "Value clamped to 1.0 (maximum)"
    show_hint(feedback.description.unwrap());
}
```

---

## Summary

**Transformers = Automatic fixing of invalid values**

**Benefits:**
- ✅ Better UX (no errors for fixable mistakes)
- ✅ Robustness (API can't break system)
- ✅ Consistency (slider/keyboard/API behave same)

**When to use:**
- ✅ Range limits, rounding, wrapping
- ✅ Format fixing, normalization
- ✅ Obvious corrections

**When NOT to use:**
- ❌ Ambiguous fixes (unclear user intent)
- ❌ Business logic (need user decision)
- ❌ Validation that needs confirmation

**Remember:**
1. Transform first, validate second
2. Always succeeds (never errors)
3. Chain multiple transformers
4. Order matters!
