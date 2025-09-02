//! Validation complexity types and estimation

use serde::{Serialize, Deserialize};
use std::fmt::{self, Display};

/// Complexity level of a validation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ValidationComplexity {
    /// O(1) - Constant time
    Trivial = 0,
    /// O(1) to O(n) - Simple operations
    Simple = 1,
    /// O(n log n) - Moderate complexity
    Moderate = 2,
    /// O(n²) - Complex operations
    Complex = 3,
    /// O(n³) or worse - Very complex
    VeryComplex = 4,
}

impl ValidationComplexity {
    /// Get all complexity levels
    pub fn all() -> Vec<Self> {
        vec![
            Self::Trivial,
            Self::Simple,
            Self::Moderate,
            Self::Complex,
            Self::VeryComplex,
        ]
    }
    
    /// Get a human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            Self::Trivial => "Trivial - Constant time operation",
            Self::Simple => "Simple - Linear time operation",
            Self::Moderate => "Moderate - Logarithmic complexity",
            Self::Complex => "Complex - Quadratic complexity",
            Self::VeryComplex => "Very Complex - Cubic or worse complexity",
        }
    }
    
    /// Get the big-O notation
    pub fn big_o(&self) -> &'static str {
        match self {
            Self::Trivial => "O(1)",
            Self::Simple => "O(n)",
            Self::Moderate => "O(n log n)",
            Self::Complex => "O(n²)",
            Self::VeryComplex => "O(n³) or worse",
        }
    }
    
    /// Estimate time for n items (in microseconds)
    pub fn estimate_time_us(&self, n: usize) -> u64 {
        match self {
            Self::Trivial => 1,
            Self::Simple => n as u64,
            Self::Moderate => {
                let n = n as f64;
                (n * n.log2()) as u64
            },
            Self::Complex => (n * n) as u64,
            Self::VeryComplex => {
                let n = n as u64;
                n * n * n
            }
        }
    }
    
    /// Combine two complexities (take the maximum)
    pub fn combine(self, other: Self) -> Self {
        std::cmp::max(self, other)
    }
    
    /// Add complexities (for sequential operations)
    pub fn add(self, other: Self) -> Self {
        // For sequential operations, take the dominant complexity
        std::cmp::max(self, other)
    }
    
    /// Multiply complexity (for nested operations)
    pub fn multiply(self, other: Self) -> Self {
        let combined = (self as u8) + (other as u8);
        match combined {
            0 => Self::Trivial,
            1 => Self::Simple,
            2 => Self::Moderate,
            3 => Self::Complex,
            _ => Self::VeryComplex,
        }
    }
}

impl Default for ValidationComplexity {
    fn default() -> Self {
        Self::Simple
    }
}

impl Display for ValidationComplexity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.big_o())
    }
}

/// Complexity estimator for validation operations
#[derive(Debug, Clone)]
pub struct ComplexityEstimator {
    /// Base complexity
    base: ValidationComplexity,
    /// Additional factors
    factors: Vec<ComplexityFactor>,
}

/// Factors that affect complexity
#[derive(Debug, Clone)]
pub enum ComplexityFactor {
    /// Size of input data
    InputSize(usize),
    /// Number of rules to evaluate
    RuleCount(usize),
    /// Depth of nested structures
    NestingDepth(usize),
    /// Network I/O operations
    NetworkIO(usize),
    /// Database queries
    DatabaseQueries(usize),
    /// Regex patterns
    RegexPatterns(usize),
}

impl ComplexityEstimator {
    /// Create a new estimator
    pub fn new(base: ValidationComplexity) -> Self {
        Self {
            base,
            factors: Vec::new(),
        }
    }
    
    /// Add a complexity factor
    pub fn with_factor(mut self, factor: ComplexityFactor) -> Self {
        self.factors.push(factor);
        self
    }
    
    /// Estimate the total complexity
    pub fn estimate(&self) -> ValidationComplexity {
        let mut complexity = self.base;
        
        for factor in &self.factors {
            complexity = match factor {
                ComplexityFactor::InputSize(n) if *n > 1000 => {
                    complexity.combine(ValidationComplexity::Moderate)
                },
                ComplexityFactor::RuleCount(n) if *n > 10 => {
                    complexity.combine(ValidationComplexity::Moderate)
                },
                ComplexityFactor::NestingDepth(n) if *n > 3 => {
                    complexity.multiply(ValidationComplexity::Simple)
                },
                ComplexityFactor::NetworkIO(_) => {
                    complexity.combine(ValidationComplexity::Complex)
                },
                ComplexityFactor::DatabaseQueries(n) if *n > 1 => {
                    complexity.combine(ValidationComplexity::Complex)
                },
                ComplexityFactor::RegexPatterns(n) if *n > 5 => {
                    complexity.combine(ValidationComplexity::Moderate)
                },
                _ => complexity,
            };
        }
        
        complexity
    }
}

/// Metrics for complexity analysis
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComplexityMetrics {
    /// Estimated complexity
    pub estimated_complexity: Option<ValidationComplexity>,
    /// Actual time taken (microseconds)
    pub actual_time_us: Option<u64>,
    /// Number of operations performed
    pub operations_count: u64,
    /// Memory used (bytes)
    pub memory_used: Option<u64>,
    /// Complexity factors encountered
    pub factors: Vec<String>,
}

impl ComplexityMetrics {
    /// Record an operation
    pub fn record_operation(&mut self) {
        self.operations_count += 1;
    }
    
    /// Set actual time
    pub fn set_actual_time(&mut self, time_us: u64) {
        self.actual_time_us = Some(time_us);
    }
    
    /// Add a complexity factor
    pub fn add_factor(&mut self, factor: impl Into<String>) {
        self.factors.push(factor.into());
    }
    
    /// Calculate efficiency (estimated vs actual)
    pub fn efficiency(&self) -> Option<f64> {
        match (self.estimated_complexity, self.actual_time_us) {
            (Some(complexity), Some(actual)) => {
                let estimated = complexity.estimate_time_us(self.operations_count as usize);
                Some(estimated as f64 / actual as f64)
            },
            _ => None,
        }
    }
}