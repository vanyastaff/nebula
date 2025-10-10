//! Combinator Chain Optimizer
//!
//! This module provides optimization for validator chains to improve
//! performance by reordering validators and applying fusion rules.
//!
//! # Optimization Strategies
//!
//! 1. **Complexity-based reordering**: Run cheap validators before expensive ones
//! 2. **Short-circuit optimization**: Fail fast with high-selectivity validators
//! 3. **Metadata-driven decisions**: Use validator metadata for smart ordering
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::optimizer::ValidatorChainOptimizer;
//!
//! let optimizer = ValidatorChainOptimizer::new();
//! let optimized = optimizer.optimize(validator_chain);
//! ```

use crate::core::{TypedValidator, ValidationComplexity, ValidatorMetadata};

// ============================================================================
// VALIDATOR CHAIN OPTIMIZER
// ============================================================================

/// Optimizer for validator chains.
///
/// Analyzes validator metadata and reorders/optimizes the validation chain
/// for better performance.
#[derive(Debug, Clone)]
pub struct ValidatorChainOptimizer {
    /// Enable complexity-based reordering.
    reorder_by_complexity: bool,

    /// Enable short-circuit optimization.
    short_circuit: bool,

    /// Minimum complexity difference to trigger reordering.
    min_complexity_diff: u32,
}

impl ValidatorChainOptimizer {
    /// Creates a new optimizer with default settings.
    #[must_use] 
    pub fn new() -> Self {
        Self {
            reorder_by_complexity: true,
            short_circuit: true,
            min_complexity_diff: 1,
        }
    }

    /// Enables or disables complexity-based reordering.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_reorder_by_complexity(mut self, enabled: bool) -> Self {
        self.reorder_by_complexity = enabled;
        self
    }

    /// Enables or disables short-circuit optimization.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_short_circuit(mut self, enabled: bool) -> Self {
        self.short_circuit = enabled;
        self
    }

    /// Sets minimum complexity difference for reordering.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_min_complexity_diff(mut self, diff: u32) -> Self {
        self.min_complexity_diff = diff;
        self
    }

    /// Checks if one validator should run before another.
    ///
    /// Returns true if `a` should run before `b`.
    #[must_use] 
    pub fn should_run_first(&self, meta_a: &ValidatorMetadata, meta_b: &ValidatorMetadata) -> bool {
        if !self.reorder_by_complexity {
            return false; // No reordering
        }

        let complexity_diff = i32::from(meta_b.complexity.score()) - i32::from(meta_a.complexity.score());

        // Only reorder if difference is significant
        if complexity_diff.unsigned_abs() < self.min_complexity_diff {
            return false;
        }

        // Run cheaper validators first
        meta_a.complexity < meta_b.complexity
    }

    /// Analyzes a validator chain and suggests optimizations.
    pub fn analyze<V: TypedValidator>(&self, validator: &V) -> OptimizationReport {
        let metadata = validator.metadata();

        OptimizationReport {
            original_complexity: metadata.complexity,
            cacheable: metadata.cacheable,
            estimated_speedup: self.estimate_speedup(&metadata),
            recommendations: self.generate_recommendations(&metadata),
        }
    }

    /// Estimates potential speedup from optimization.
    fn estimate_speedup(&self, metadata: &ValidatorMetadata) -> f64 {
        match metadata.complexity {
            ValidationComplexity::Constant => 1.0,
            ValidationComplexity::Logarithmic => 1.1,
            ValidationComplexity::Linear => 1.2,
            ValidationComplexity::Expensive => 2.0,
        }
    }

    /// Generates optimization recommendations.
    fn generate_recommendations(&self, metadata: &ValidatorMetadata) -> Vec<String> {
        let mut recommendations = Vec::new();

        if metadata.cacheable && metadata.complexity >= ValidationComplexity::Linear {
            recommendations.push("Consider adding .cached() combinator".to_string());
        }

        if metadata.complexity == ValidationComplexity::Expensive {
            recommendations.push(
                "Expensive validator detected. Consider running cheap validators first".to_string(),
            );
        }

        if metadata.tags.contains(&"async".to_string()) {
            recommendations
                .push("Async validator. Consider batching or parallelization".to_string());
        }

        recommendations
    }

    /// Analyzes a validator with runtime statistics.
    ///
    /// Uses both static metadata and runtime statistics to provide
    /// more accurate recommendations.
    pub fn analyze_with_stats<V: TypedValidator>(
        &self,
        validator: &V,
        stats: &ValidatorStats,
    ) -> OptimizationReport {
        let metadata = validator.metadata();
        let mut recommendations = self.generate_recommendations(&metadata);

        // Add stats-based recommendations
        if stats.failure_rate() > 0.7 {
            recommendations.push(
                "High failure rate detected. Consider running this validator early in the chain"
                    .to_string(),
            );
        }

        if stats.average_time_ns() > 1_000_000.0 {
            // > 1ms
            recommendations.push(
                "Slow validator detected (avg > 1ms). Consider caching or optimization".to_string(),
            );
        }

        if stats.selectivity_score() > 0.8 {
            recommendations.push(
                "High selectivity score. This validator rejects most inputs - run it early"
                    .to_string(),
            );
        }

        OptimizationReport {
            original_complexity: metadata.complexity,
            cacheable: metadata.cacheable,
            estimated_speedup: self.estimate_speedup_with_stats(&metadata, stats),
            recommendations,
        }
    }

    /// Estimates speedup considering runtime statistics.
    fn estimate_speedup_with_stats(
        &self,
        metadata: &ValidatorMetadata,
        stats: &ValidatorStats,
    ) -> f64 {
        let base_speedup = self.estimate_speedup(metadata);

        // Boost for high failure rate (early rejection is better)
        let failure_boost = if stats.failure_rate() > 0.5 { 1.5 } else { 1.0 };

        base_speedup * failure_boost
    }

    /// Optimizes a chain of validators by reordering them.
    ///
    /// Returns a new vector with validators in optimal order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let optimizer = ValidatorChainOptimizer::new();
    /// let validators = vec![expensive_validator, cheap_validator, medium_validator];
    /// let optimized = optimizer.optimize_chain(validators);
    /// // Result: [cheap_validator, medium_validator, expensive_validator]
    /// ```
    #[must_use] 
    pub fn optimize_chain<V>(&self, validators: Vec<V>) -> Vec<V>
    where
        V: TypedValidator,
    {
        if !self.reorder_by_complexity || validators.is_empty() {
            return validators;
        }

        let mut with_meta: Vec<(V, ValidatorMetadata)> = validators
            .into_iter()
            .map(|v| {
                let meta = v.metadata();
                (v, meta)
            })
            .collect();

        // Sort by complexity (cheap first)
        with_meta.sort_by(|(_, meta_a), (_, meta_b)| meta_a.complexity.cmp(&meta_b.complexity));

        with_meta.into_iter().map(|(v, _)| v).collect()
    }

    /// Optimizes a chain using runtime statistics.
    ///
    /// Considers both complexity and selectivity scores from stats.
    #[must_use] 
    pub fn optimize_chain_with_stats<V>(&self, validators: Vec<(V, ValidatorStats)>) -> Vec<V>
    where
        V: TypedValidator,
    {
        if validators.is_empty() {
            return Vec::new();
        }

        let mut with_scores: Vec<(V, f64)> = validators
            .into_iter()
            .map(|(v, stats)| {
                let meta = v.metadata();
                // Combined score: lower complexity + higher selectivity = run first
                let complexity_score = f64::from(meta.complexity.score());
                let selectivity_score = stats.selectivity_score();

                // Lower score = run first
                let combined_score = complexity_score - (selectivity_score * 50.0);
                (v, combined_score)
            })
            .collect();

        // Sort by combined score
        with_scores.sort_by(|(_, score_a), (_, score_b)| {
            score_a
                .partial_cmp(score_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        with_scores.into_iter().map(|(v, _)| v).collect()
    }
}

impl Default for ValidatorChainOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// OPTIMIZATION REPORT
// ============================================================================

/// Report of optimization analysis.
#[derive(Debug, Clone)]
pub struct OptimizationReport {
    /// The original complexity level of the validator.
    pub original_complexity: ValidationComplexity,

    /// Whether the validator supports caching of results.
    pub cacheable: bool,

    /// The estimated performance improvement multiplier from optimization.
    pub estimated_speedup: f64,

    /// A list of optimization recommendations for the validator.
    pub recommendations: Vec<String>,
}

impl OptimizationReport {
    /// Checks if optimization is recommended.
    #[must_use] 
    pub fn is_optimization_recommended(&self) -> bool {
        !self.recommendations.is_empty() || self.estimated_speedup > 1.1
    }

    /// Returns a human-readable summary.
    #[must_use] 
    pub fn summary(&self) -> String {
        let mut lines = vec![
            format!("Complexity: {:?}", self.original_complexity),
            format!("Cacheable: {}", self.cacheable),
            format!("Estimated speedup: {:.2}x", self.estimated_speedup),
        ];

        if !self.recommendations.is_empty() {
            lines.push("Recommendations:".to_string());
            for rec in &self.recommendations {
                lines.push(format!("  - {rec}"));
            }
        }

        lines.join("\n")
    }
}

// ============================================================================
// VALIDATOR ORDERING HELPERS
// ============================================================================

/// Helper trait for comparing validators based on optimization criteria.
pub trait ValidatorOrdering: TypedValidator {
    /// Returns the optimization priority score (lower = run earlier).
    fn optimization_priority(&self) -> i32 {
        let metadata = self.metadata();
        i32::from(metadata.complexity.score())
    }

    /// Checks if this validator should run before another.
    fn should_run_before<V: TypedValidator>(&self, other: &V) -> bool {
        self.optimization_priority() < other.optimization_priority()
    }
}

// Blanket implementation for all TypedValidators
impl<T: TypedValidator> ValidatorOrdering for T {}

// ============================================================================
// OPTIMIZATION STRATEGIES
// ============================================================================

/// Different optimization strategies for validator chains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationStrategy {
    /// No optimization.
    None,

    /// Reorder by complexity (cheap validators first).
    ComplexityBased,

    /// Optimize for fast failure (high selectivity first).
    FailFast,

    /// Balance between complexity and selectivity.
    Balanced,
}

impl OptimizationStrategy {
    /// Returns a description of the strategy.
    #[must_use] 
    pub fn description(&self) -> &str {
        match self {
            Self::None => "No optimization applied",
            Self::ComplexityBased => "Reorder validators by complexity (O(1) before O(n))",
            Self::FailFast => "Run high-selectivity validators first to fail early",
            Self::Balanced => "Balance complexity and selectivity",
        }
    }

    /// Applies the strategy to determine run order.
    #[must_use] 
    pub fn compare_validators(
        &self,
        meta_a: &ValidatorMetadata,
        meta_b: &ValidatorMetadata,
    ) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        match self {
            Self::None => Ordering::Equal,

            Self::ComplexityBased => meta_a.complexity.cmp(&meta_b.complexity),

            Self::FailFast => {
                // Prefer validators that are likely to fail
                // Use selectivity_score from custom metadata if available
                let selectivity_a = meta_a
                    .custom
                    .get("selectivity_score")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);

                let selectivity_b = meta_b
                    .custom
                    .get("selectivity_score")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);

                // Higher selectivity = more likely to fail = run first
                // If no stats, fall back to complexity
                if selectivity_a == selectivity_b {
                    meta_a.complexity.cmp(&meta_b.complexity)
                } else {
                    selectivity_b
                        .partial_cmp(&selectivity_a)
                        .unwrap_or(Ordering::Equal)
                }
            }

            Self::Balanced => {
                // First compare complexity
                match meta_a.complexity.cmp(&meta_b.complexity) {
                    Ordering::Equal => {
                        // If same complexity, prefer cacheable ones
                        match (meta_a.cacheable, meta_b.cacheable) {
                            (true, false) => Ordering::Less,
                            (false, true) => Ordering::Greater,
                            _ => Ordering::Equal,
                        }
                    }
                    other => other,
                }
            }
        }
    }
}

// ============================================================================
// STATISTICS AND PROFILING
// ============================================================================

/// Statistics collector for validator performance.
#[derive(Debug, Clone, Default)]
pub struct ValidatorStats {
    /// The total number of times the validator was called.
    pub call_count: u64,

    /// The number of times the validator passed validation.
    pub pass_count: u64,

    /// The number of times the validator failed validation.
    pub fail_count: u64,

    /// The total time spent in validation, measured in nanoseconds.
    pub total_time_ns: u64,
}

impl ValidatorStats {
    /// Creates new empty statistics.
    #[must_use] 
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a validation call with result and duration.
    pub fn record(&mut self, passed: bool, duration_ns: u64) {
        self.call_count += 1;
        if passed {
            self.pass_count += 1;
        } else {
            self.fail_count += 1;
        }
        self.total_time_ns += duration_ns;
    }

    /// Returns the failure rate (0.0 to 1.0).
    #[must_use] 
    pub fn failure_rate(&self) -> f64 {
        if self.call_count == 0 {
            return 0.0;
        }
        self.fail_count as f64 / self.call_count as f64
    }

    /// Returns the average time per call (nanoseconds).
    #[must_use] 
    pub fn average_time_ns(&self) -> f64 {
        if self.call_count == 0 {
            return 0.0;
        }
        self.total_time_ns as f64 / self.call_count as f64
    }

    /// Returns a selectivity score (higher = more selective/likely to fail).
    #[must_use] 
    pub fn selectivity_score(&self) -> f64 {
        self.failure_rate()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimizer_new() {
        let optimizer = ValidatorChainOptimizer::new();
        assert!(optimizer.reorder_by_complexity);
        assert!(optimizer.short_circuit);
    }

    #[test]
    fn test_should_run_first() {
        let optimizer = ValidatorChainOptimizer::new();

        let cheap = ValidatorMetadata {
            name: "Cheap".to_string(),
            description: None,
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec![],
            version: None,
            custom: Default::default(),
        };

        let expensive = ValidatorMetadata {
            name: "Expensive".to_string(),
            description: None,
            complexity: ValidationComplexity::Expensive,
            cacheable: true,
            estimated_time: None,
            tags: vec![],
            version: None,
            custom: Default::default(),
        };

        assert!(optimizer.should_run_first(&cheap, &expensive));
        assert!(!optimizer.should_run_first(&expensive, &cheap));
    }

    #[test]
    fn test_optimization_strategy_complexity_based() {
        let strategy = OptimizationStrategy::ComplexityBased;

        let cheap = ValidatorMetadata {
            name: "Cheap".to_string(),
            description: None,
            complexity: ValidationComplexity::Constant,
            cacheable: false,
            estimated_time: None,
            tags: vec![],
            version: None,
            custom: Default::default(),
        };

        let expensive = ValidatorMetadata {
            name: "Expensive".to_string(),
            description: None,
            complexity: ValidationComplexity::Linear,
            cacheable: false,
            estimated_time: None,
            tags: vec![],
            version: None,
            custom: Default::default(),
        };

        use std::cmp::Ordering;
        assert_eq!(
            strategy.compare_validators(&cheap, &expensive),
            Ordering::Less
        );
    }

    #[test]
    fn test_validator_stats() {
        let mut stats = ValidatorStats::new();

        stats.record(true, 100);
        stats.record(true, 200);
        stats.record(false, 150);

        assert_eq!(stats.call_count, 3);
        assert_eq!(stats.pass_count, 2);
        assert_eq!(stats.fail_count, 1);
        assert_eq!(stats.failure_rate(), 1.0 / 3.0);
        assert_eq!(stats.average_time_ns(), 150.0);
    }

    #[test]
    fn test_optimization_report() {
        let report = OptimizationReport {
            original_complexity: ValidationComplexity::Expensive,
            cacheable: true,
            estimated_speedup: 1.5,
            recommendations: vec!["Consider caching".to_string()],
        };

        assert!(report.is_optimization_recommended());
        let summary = report.summary();
        assert!(summary.contains("Expensive"));
        assert!(summary.contains("Recommendations"));
    }
}
