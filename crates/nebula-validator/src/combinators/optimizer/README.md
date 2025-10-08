# Validator Chain Optimizer

The optimizer module provides tools to analyze and optimize validator chains for better performance by reordering validators and applying smart execution strategies.

## Table of Contents

- [Overview](#overview)
- [Core Components](#core-components)
- [Optimization Strategies](#optimization-strategies)
- [Usage Examples](#usage-examples)
- [Performance Profiling](#performance-profiling)
- [Best Practices](#best-practices)

## Overview

The optimizer analyzes validator metadata (complexity, cacheability, estimated time) and determines optimal execution order. By running cheap validators before expensive ones, chains can fail fast and minimize wasted computation.

### Key Benefits

- **Fail Fast**: Run high-selectivity validators first to reject invalid input early
- **Complexity-Based Ordering**: Execute O(1) validators before O(n) or O(n²) validators
- **Smart Caching Recommendations**: Identify validators that would benefit from memoization
- **Performance Profiling**: Track validator statistics to make data-driven optimization decisions

## Core Components

### `ValidatorChainOptimizer`

The main optimizer that analyzes validator chains and determines optimal ordering.

```rust
use nebula_validator::combinators::ValidatorChainOptimizer;

let optimizer = ValidatorChainOptimizer::new()
    .with_reorder_by_complexity(true)
    .with_short_circuit(true)
    .with_min_complexity_diff(1);
```

**Configuration Options:**
- `reorder_by_complexity`: Enable/disable automatic reordering (default: true)
- `short_circuit`: Enable early exit on first failure (default: true)
- `min_complexity_diff`: Minimum complexity difference to trigger reordering (default: 1)

### `OptimizationReport`

Provides analysis and recommendations for a validator.

```rust
let report = optimizer.analyze(&my_validator);

println!("Complexity: {:?}", report.original_complexity);
println!("Cacheable: {}", report.cacheable);
println!("Estimated speedup: {:.2}x", report.estimated_speedup);

if report.is_optimization_recommended() {
    for recommendation in &report.recommendations {
        println!("  - {}", recommendation);
    }
}
```

### `ValidatorOrdering` Trait

Helper trait for comparing validators (blanket implemented for all `TypedValidator` types).

```rust
use nebula_validator::combinators::ValidatorOrdering;

if cheap_validator.should_run_before(&expensive_validator) {
    // Run cheap_validator first
}

let priority = validator.optimization_priority(); // Lower = run earlier
```

### `ValidatorStats`

Tracks runtime statistics for performance profiling.

```rust
use nebula_validator::combinators::ValidatorStats;

let mut stats = ValidatorStats::new();

// Record validation results
stats.record(true, 100_000); // passed, took 100µs
stats.record(false, 50_000); // failed, took 50µs

// Analyze performance
println!("Failure rate: {:.2}%", stats.failure_rate() * 100.0);
println!("Average time: {:.2}µs", stats.average_time_ns() / 1000.0);
println!("Selectivity: {:.2}", stats.selectivity_score());
```

## Optimization Strategies

The optimizer supports four different strategies for determining validator execution order:

### 1. `OptimizationStrategy::None`

No optimization applied - validators run in their original order.

```rust
use nebula_validator::combinators::OptimizationStrategy;

let strategy = OptimizationStrategy::None;
```

**Use when:**
- Order matters for semantic reasons
- Validators have side effects
- Debugging/testing specific orderings

### 2. `OptimizationStrategy::ComplexityBased`

Reorder validators by computational complexity (O(1) → O(log n) → O(n) → O(n²)).

```rust
let strategy = OptimizationStrategy::ComplexityBased;
```

**Use when:**
- All validators are pure (no side effects)
- Minimizing CPU time is priority
- Input size varies significantly

**Example:**
```
Original: [O(n²) regex, O(1) length check, O(n) contains]
Optimized: [O(1) length check, O(n) contains, O(n²) regex]
```

### 3. `OptimizationStrategy::FailFast`

Run validators with high failure rates first to reject invalid input early.

```rust
let strategy = OptimizationStrategy::FailFast;
```

**Use when:**
- Most inputs are invalid
- Want to minimize average-case latency
- Have runtime statistics available

**Example:**
```
Original: [rarely fails A, often fails B, sometimes fails C]
Optimized: [often fails B, sometimes fails C, rarely fails A]
```

### 4. `OptimizationStrategy::Balanced`

Balance between complexity and other factors (e.g., prefer cacheable validators).

```rust
let strategy = OptimizationStrategy::Balanced;
```

**Use when:**
- Want both performance and reliability
- Have varied validator characteristics
- General-purpose optimization

**Decision logic:**
1. Compare complexity first
2. If equal complexity, prefer cacheable validators
3. Otherwise maintain original order

## Usage Examples

### Basic Optimization Analysis

```rust
use nebula_validator::combinators::ValidatorChainOptimizer;
use nebula_validator::validators::MinLength;

let optimizer = ValidatorChainOptimizer::new();
let validator = MinLength { min: 5 };

let report = optimizer.analyze(&validator);
println!("{}", report.summary());
```

### Comparing Two Validators

```rust
use nebula_validator::combinators::ValidatorChainOptimizer;

let optimizer = ValidatorChainOptimizer::new();

let cheap = CheapValidator;
let expensive = ExpensiveValidator;

if optimizer.should_run_first(&cheap.metadata(), &expensive.metadata()) {
    // Run cheap validator first, then expensive
    cheap.validate(input)?;
    expensive.validate(input)?;
} else {
    // Run in original order
    expensive.validate(input)?;
    cheap.validate(input)?;
}
```

### Building an Optimized Chain

```rust
use nebula_validator::combinators::{ValidatorChainOptimizer, ValidatorOrdering};

// Collect validators with their priorities
let mut validators = vec![
    (expensive_regex, expensive_regex.optimization_priority()),
    (cheap_length, cheap_length.optimization_priority()),
    (medium_format, medium_format.optimization_priority()),
];

// Sort by priority (ascending = run earlier)
validators.sort_by_key(|(_, priority)| *priority);

// Run validators in optimized order
for (validator, _) in validators {
    validator.validate(input)?;
}
```

### Runtime Performance Tracking

```rust
use nebula_validator::combinators::ValidatorStats;
use std::time::Instant;

let mut stats = ValidatorStats::new();

for input in test_inputs {
    let start = Instant::now();
    let result = validator.validate(input);
    let duration = start.elapsed().as_nanos() as u64;

    stats.record(result.is_ok(), duration);
}

// Use stats to make optimization decisions
if stats.failure_rate() > 0.8 {
    println!("This validator fails often - move it earlier!");
}

if stats.average_time_ns() > 1_000_000 {
    println!("This validator is slow - consider caching!");
}
```

### Strategy Comparison

```rust
use nebula_validator::combinators::OptimizationStrategy;
use std::cmp::Ordering;

let strategies = [
    OptimizationStrategy::None,
    OptimizationStrategy::ComplexityBased,
    OptimizationStrategy::FailFast,
    OptimizationStrategy::Balanced,
];

for strategy in strategies {
    println!("{}: {}",
        format!("{:?}", strategy),
        strategy.description()
    );

    match strategy.compare_validators(&validator_a.metadata(), &validator_b.metadata()) {
        Ordering::Less => println!("  → Run A before B"),
        Ordering::Greater => println!("  → Run B before A"),
        Ordering::Equal => println!("  → Order doesn't matter"),
    }
}
```

## Performance Profiling

### Measuring Validator Performance

```rust
use std::time::Instant;
use nebula_validator::combinators::ValidatorStats;

fn profile_validator<V>(validator: &V, inputs: &[&V::Input]) -> ValidatorStats
where
    V: TypedValidator,
{
    let mut stats = ValidatorStats::new();

    for input in inputs {
        let start = Instant::now();
        let result = validator.validate(input);
        let duration = start.elapsed().as_nanos() as u64;

        stats.record(result.is_ok(), duration);
    }

    stats
}

// Usage
let stats = profile_validator(&my_validator, &test_cases);
println!("Average time: {:.2}ms", stats.average_time_ns() / 1_000_000.0);
println!("Failure rate: {:.1}%", stats.failure_rate() * 100.0);
```

### Comparing Chain Performance

```rust
use std::time::Instant;

// Original chain
let original_chain = vec![expensive, medium, cheap];
let start = Instant::now();
for validator in &original_chain {
    if validator.validate(input).is_err() {
        break;
    }
}
let original_time = start.elapsed();

// Optimized chain
let optimized_chain = vec![cheap, medium, expensive];
let start = Instant::now();
for validator in &optimized_chain {
    if validator.validate(input).is_err() {
        break;
    }
}
let optimized_time = start.elapsed();

let speedup = original_time.as_micros() as f64 / optimized_time.as_micros() as f64;
println!("Speedup: {:.2}x", speedup);
```

## Best Practices

### 1. Set Validator Metadata

Always provide accurate metadata for your validators:

```rust
fn metadata(&self) -> ValidatorMetadata {
    ValidatorMetadata {
        name: "EmailFormat".to_string(),
        description: Some("Validates email format using regex".to_string()),
        complexity: ValidationComplexity::Linear, // Regex is O(n)
        cacheable: true, // No side effects, safe to cache
        estimated_time: Some(Duration::from_micros(50)),
        tags: vec!["format".to_string(), "email".to_string()],
        version: Some("1.0.0".to_string()),
        custom: Default::default(),
    }
}
```

### 2. Use Complexity-Based Ordering for Pure Validators

If your validators have no side effects, reorder them by complexity:

```rust
let optimizer = ValidatorChainOptimizer::new()
    .with_reorder_by_complexity(true);

// Cheap validators run first automatically
```

### 3. Profile Before Optimizing

Collect runtime statistics before making optimization decisions:

```rust
// Run with original order, collect stats
let stats_a = profile_validator(&validator_a, test_inputs);
let stats_b = profile_validator(&validator_b, test_inputs);

// Compare performance
if stats_a.average_time_ns() < stats_b.average_time_ns() {
    // A is faster, run it first
}
```

### 4. Consider Selectivity

Validators that fail often should run early:

```rust
if stats.failure_rate() > 0.5 {
    // This validator rejects >50% of inputs
    // Move it earlier in the chain
}
```

### 5. Cache Expensive Validators

If a validator is expensive but cacheable, add caching:

```rust
let report = optimizer.analyze(&expensive_validator);

if report.recommendations.iter().any(|r| r.contains("cached")) {
    let optimized = expensive_validator.cached();
}
```

### 6. Set Minimum Complexity Difference

Avoid unnecessary reordering for minor complexity differences:

```rust
let optimizer = ValidatorChainOptimizer::new()
    .with_min_complexity_diff(5); // Only reorder if difference >= 5

// Won't reorder if complexities are Constant(1) vs Logarithmic(5)
// Will reorder if Constant(1) vs Linear(10)
```

### 7. Document Optimization Decisions

Use optimization reports to document why validators run in a certain order:

```rust
let report = optimizer.analyze(&validator);
println!("Running validator: {}", validator.metadata().name);
println!("Reason: {}", report.summary());
```

## Advanced Topics

### Custom Complexity Scores

Complexity scores determine ordering:
- `Constant` = 1
- `Logarithmic` = 5
- `Linear` = 10
- `Expensive` = 100

You can use these for custom comparisons:

```rust
use nebula_validator::core::ValidationComplexity;

let complexity_a = ValidationComplexity::Linear;
let complexity_b = ValidationComplexity::Expensive;

if complexity_a.score() < complexity_b.score() {
    println!("A is cheaper than B");
}
```

### Integrating with Custom Validators

The optimizer works with any `TypedValidator`:

```rust
impl TypedValidator for CustomValidator {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        // Custom validation logic
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            // Provide accurate metadata for optimization
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            ..Default::default()
        }
    }
}

// Optimizer automatically works with CustomValidator
let optimizer = ValidatorChainOptimizer::new();
let report = optimizer.analyze(&CustomValidator);
```

## See Also

- [Cached Combinator](../cached/README.md) - Memoization for expensive validators
- [Validator Metadata](../../core/metadata.rs) - Introspection and analysis
- [Core Traits](../../core/traits.rs) - TypedValidator trait
