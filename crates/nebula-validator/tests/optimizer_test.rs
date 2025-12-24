//! Integration tests for the validator chain optimizer
//!
//! Tests optimization strategies, performance profiling, and chain analysis.

use nebula_validator::combinators::{
    OptimizationReport, OptimizationStrategy, ValidatorChainOptimizer, ValidatorOrdering,
    ValidatorStats,
};
use nebula_validator::core::{ValidationComplexity, ValidationError, Validator, ValidatorMetadata};
use std::time::Duration;

// ============================================================================
// TEST HELPERS
// ============================================================================

struct CheapValidator;
impl Validator for CheapValidator {
    type Input = str;

    fn validate(&self, _input: &Self::Input) -> Result<(), ValidationError> {
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "CheapValidator".to_string(),
            description: Some("O(1) validator".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: Some(Duration::from_micros(10)),
            tags: vec![],
            version: None,
            custom: Default::default(),
        }
    }
}

struct ExpensiveValidator;
impl Validator for ExpensiveValidator {
    type Input = str;

    fn validate(&self, _input: &Self::Input) -> Result<(), ValidationError> {
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "ExpensiveValidator".to_string(),
            description: Some("O(n²) validator".to_string()),
            complexity: ValidationComplexity::Expensive,
            cacheable: false,
            estimated_time: Some(Duration::from_micros(1000)),
            tags: vec![],
            version: None,
            custom: Default::default(),
        }
    }
}

struct LinearValidator;
impl Validator for LinearValidator {
    type Input = str;

    fn validate(&self, _input: &Self::Input) -> Result<(), ValidationError> {
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "LinearValidator".to_string(),
            description: Some("O(n) validator".to_string()),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(Duration::from_micros(100)),
            tags: vec![],
            version: None,
            custom: Default::default(),
        }
    }
}

struct AsyncValidator;
impl Validator for AsyncValidator {
    type Input = str;

    fn validate(&self, _input: &Self::Input) -> Result<(), ValidationError> {
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "AsyncValidator".to_string(),
            description: Some("Async I/O validator".to_string()),
            complexity: ValidationComplexity::Expensive,
            cacheable: false,
            estimated_time: Some(Duration::from_micros(5000)),
            tags: vec!["async".to_string()],
            version: None,
            custom: Default::default(),
        }
    }
}

// ============================================================================
// OPTIMIZER TESTS
// ============================================================================

#[test]
fn test_optimizer_creation() {
    let optimizer = ValidatorChainOptimizer::new();
    assert!(optimizer.should_run_first(&CheapValidator.metadata(), &ExpensiveValidator.metadata()));
}

#[test]
fn test_optimizer_with_settings() {
    let optimizer = ValidatorChainOptimizer::new()
        .with_reorder_by_complexity(false)
        .with_short_circuit(false)
        .with_min_complexity_diff(5);

    // With reordering disabled, should not reorder
    assert!(
        !optimizer.should_run_first(&CheapValidator.metadata(), &ExpensiveValidator.metadata())
    );
}

#[test]
fn test_should_run_first_by_complexity() {
    let optimizer = ValidatorChainOptimizer::new();

    let cheap = CheapValidator.metadata();
    let linear = LinearValidator.metadata();
    let expensive = ExpensiveValidator.metadata();

    // Cheap should run before linear
    assert!(optimizer.should_run_first(&cheap, &linear));

    // Cheap should run before expensive
    assert!(optimizer.should_run_first(&cheap, &expensive));

    // Linear should run before expensive
    assert!(optimizer.should_run_first(&linear, &expensive));

    // Expensive should NOT run before cheap
    assert!(!optimizer.should_run_first(&expensive, &cheap));
}

#[test]
fn test_min_complexity_diff_threshold() {
    let optimizer = ValidatorChainOptimizer::new().with_min_complexity_diff(10);

    let cheap = CheapValidator.metadata();
    let linear = LinearValidator.metadata();

    // Complexity diff between Constant(1) and Linear(10) is 9, which is < 10
    // So no reordering should occur
    assert!(!optimizer.should_run_first(&cheap, &linear));
}

#[test]
fn test_optimization_report_generation() {
    let optimizer = ValidatorChainOptimizer::new();
    let report = optimizer.analyze(&ExpensiveValidator);

    assert_eq!(report.original_complexity, ValidationComplexity::Expensive);
    assert!(!report.cacheable);
    assert!(report.estimated_speedup > 1.0);
    assert!(!report.recommendations.is_empty());
}

#[test]
fn test_optimization_report_for_cheap_validator() {
    let optimizer = ValidatorChainOptimizer::new();
    let report = optimizer.analyze(&CheapValidator);

    assert_eq!(report.original_complexity, ValidationComplexity::Constant);
    assert!(report.cacheable);
    assert_eq!(report.estimated_speedup, 1.0);
}

#[test]
fn test_optimization_recommendations_for_expensive() {
    let optimizer = ValidatorChainOptimizer::new();
    let report = optimizer.analyze(&ExpensiveValidator);

    assert!(report.is_optimization_recommended());
    let summary = report.summary();
    assert!(summary.contains("Expensive"));
    assert!(summary.contains("Recommendations"));
}

#[test]
fn test_optimization_recommendations_for_async() {
    let optimizer = ValidatorChainOptimizer::new();
    let report = optimizer.analyze(&AsyncValidator);

    assert!(report.is_optimization_recommended());
    let recommendations = report.recommendations;
    assert!(
        recommendations.iter().any(|r| r.contains("async")
            || r.contains("batching")
            || r.contains("parallelization"))
    );
}

#[test]
fn test_optimization_recommendations_for_linear_cacheable() {
    let optimizer = ValidatorChainOptimizer::new();
    let report = optimizer.analyze(&LinearValidator);

    // Linear + cacheable should recommend caching
    let recommendations = report.recommendations;
    assert!(recommendations.iter().any(|r| r.contains("cached")));
}

// ============================================================================
// VALIDATOR ORDERING TESTS
// ============================================================================

#[test]
fn test_validator_ordering_trait() {
    let cheap = CheapValidator;
    let expensive = ExpensiveValidator;

    assert!(cheap.should_run_before(&expensive));
    assert!(!expensive.should_run_before(&cheap));
}

#[test]
fn test_optimization_priority() {
    let cheap = CheapValidator;
    let linear = LinearValidator;
    let expensive = ExpensiveValidator;

    assert!(cheap.optimization_priority() < linear.optimization_priority());
    assert!(linear.optimization_priority() < expensive.optimization_priority());
}

// ============================================================================
// OPTIMIZATION STRATEGY TESTS
// ============================================================================

#[test]
fn test_optimization_strategy_descriptions() {
    assert!(!OptimizationStrategy::None.description().is_empty());
    assert!(
        !OptimizationStrategy::ComplexityBased
            .description()
            .is_empty()
    );
    assert!(!OptimizationStrategy::FailFast.description().is_empty());
    assert!(!OptimizationStrategy::Balanced.description().is_empty());
}

#[test]
fn test_optimization_strategy_none() {
    let strategy = OptimizationStrategy::None;
    let cheap = CheapValidator.metadata();
    let expensive = ExpensiveValidator.metadata();

    use std::cmp::Ordering;
    assert_eq!(
        strategy.compare_validators(&cheap, &expensive),
        Ordering::Equal
    );
}

#[test]
fn test_optimization_strategy_complexity_based() {
    let strategy = OptimizationStrategy::ComplexityBased;
    let cheap = CheapValidator.metadata();
    let expensive = ExpensiveValidator.metadata();

    use std::cmp::Ordering;
    assert_eq!(
        strategy.compare_validators(&cheap, &expensive),
        Ordering::Less
    );
    assert_eq!(
        strategy.compare_validators(&expensive, &cheap),
        Ordering::Greater
    );
}

#[test]
fn test_optimization_strategy_fail_fast() {
    let strategy = OptimizationStrategy::FailFast;
    let cheap = CheapValidator.metadata();
    let expensive = ExpensiveValidator.metadata();

    use std::cmp::Ordering;
    // FailFast currently uses complexity as heuristic
    assert_eq!(
        strategy.compare_validators(&cheap, &expensive),
        Ordering::Less
    );
}

#[test]
fn test_optimization_strategy_balanced() {
    let strategy = OptimizationStrategy::Balanced;

    let cheap_cacheable = CheapValidator.metadata();
    let cheap_non_cacheable = ValidatorMetadata {
        name: "CheapNonCacheable".to_string(),
        description: None,
        complexity: ValidationComplexity::Constant,
        cacheable: false,
        estimated_time: None,
        tags: vec![],
        version: None,
        custom: Default::default(),
    };

    use std::cmp::Ordering;
    // Same complexity, but cacheable should come first
    assert_eq!(
        strategy.compare_validators(&cheap_cacheable, &cheap_non_cacheable),
        Ordering::Less
    );
}

// ============================================================================
// VALIDATOR STATS TESTS
// ============================================================================

#[test]
fn test_validator_stats_creation() {
    let stats = ValidatorStats::new();
    assert_eq!(stats.call_count, 0);
    assert_eq!(stats.pass_count, 0);
    assert_eq!(stats.fail_count, 0);
    assert_eq!(stats.total_time_ns, 0);
}

#[test]
fn test_validator_stats_recording() {
    let mut stats = ValidatorStats::new();

    stats.record(true, 100);
    assert_eq!(stats.call_count, 1);
    assert_eq!(stats.pass_count, 1);
    assert_eq!(stats.fail_count, 0);
    assert_eq!(stats.total_time_ns, 100);

    stats.record(false, 200);
    assert_eq!(stats.call_count, 2);
    assert_eq!(stats.pass_count, 1);
    assert_eq!(stats.fail_count, 1);
    assert_eq!(stats.total_time_ns, 300);
}

#[test]
fn test_validator_stats_failure_rate() {
    let mut stats = ValidatorStats::new();

    stats.record(true, 100);
    stats.record(true, 100);
    stats.record(false, 100);

    assert_eq!(stats.failure_rate(), 1.0 / 3.0);
}

#[test]
fn test_validator_stats_average_time() {
    let mut stats = ValidatorStats::new();

    stats.record(true, 100);
    stats.record(true, 200);
    stats.record(false, 300);

    assert_eq!(stats.average_time_ns(), 200.0);
}

#[test]
fn test_validator_stats_selectivity_score() {
    let mut stats = ValidatorStats::new();

    stats.record(false, 100);
    stats.record(false, 100);
    stats.record(true, 100);

    // Selectivity = failure_rate = 2/3
    assert_eq!(stats.selectivity_score(), 2.0 / 3.0);
}

#[test]
fn test_validator_stats_empty() {
    let stats = ValidatorStats::new();

    // Empty stats should return 0.0 for rates and averages
    assert_eq!(stats.failure_rate(), 0.0);
    assert_eq!(stats.average_time_ns(), 0.0);
    assert_eq!(stats.selectivity_score(), 0.0);
}

// ============================================================================
// OPTIMIZATION REPORT TESTS
// ============================================================================

#[test]
fn test_optimization_report_is_recommended() {
    let report = OptimizationReport {
        original_complexity: ValidationComplexity::Constant,
        cacheable: false,
        estimated_speedup: 1.0,
        recommendations: vec![],
    };

    assert!(!report.is_optimization_recommended());

    let report_with_speedup = OptimizationReport {
        original_complexity: ValidationComplexity::Expensive,
        cacheable: true,
        estimated_speedup: 1.5,
        recommendations: vec![],
    };

    assert!(report_with_speedup.is_optimization_recommended());

    let report_with_recommendations = OptimizationReport {
        original_complexity: ValidationComplexity::Constant,
        cacheable: false,
        estimated_speedup: 1.0,
        recommendations: vec!["Use caching".to_string()],
    };

    assert!(report_with_recommendations.is_optimization_recommended());
}

#[test]
fn test_optimization_report_summary() {
    let report = OptimizationReport {
        original_complexity: ValidationComplexity::Expensive,
        cacheable: true,
        estimated_speedup: 2.0,
        recommendations: vec![
            "Consider caching".to_string(),
            "Run cheap validators first".to_string(),
        ],
    };

    let summary = report.summary();

    assert!(summary.contains("Expensive"));
    assert!(summary.contains("true"));
    assert!(summary.contains("2.00"));
    assert!(summary.contains("Consider caching"));
    assert!(summary.contains("Run cheap validators first"));
}
