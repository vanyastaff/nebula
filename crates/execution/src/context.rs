//! Execution context for workflow runs.

use std::time::Duration;

use nebula_core::ExecutionId;
use serde::{Deserialize, Deserializer};

fn default_max_concurrent_nodes() -> usize {
    10
}

fn deserialize_min_concurrency<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    let n = usize::deserialize(deserializer)?;
    if n == 0 {
        return Err(serde::de::Error::custom(
            "max_concurrent_nodes must be >= 1 (0 deadlocks the workflow scheduler — zero permits on the node semaphore)",
        ));
    }
    Ok(n)
}

/// Resource budget for a single workflow execution.
///
/// Controls concurrency, wall-clock timeout, output size, and retry limits.
/// All `Option` fields default to `None` (unlimited).
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use nebula_execution::context::ExecutionBudget;
///
/// let budget = ExecutionBudget::default()
///     .with_max_duration(Duration::from_secs(300))
///     .with_max_output_bytes(10 * 1024 * 1024);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionBudget {
    /// Maximum nodes executing in parallel.
    ///
    /// Must be at least **1**. The workflow engine uses a [`tokio::sync::Semaphore`]
    /// with this many permits; `0` leaves no permits and deadlocks scheduling.
    #[serde(
        default = "default_max_concurrent_nodes",
        deserialize_with = "deserialize_min_concurrency"
    )]
    pub max_concurrent_nodes: usize,

    /// Wall-clock timeout for the entire execution. `None` = unlimited.
    #[serde(default, with = "crate::serde_duration_opt")]
    pub max_duration: Option<Duration>,

    /// Maximum total bytes across all node outputs. `None` = unlimited.
    #[serde(default)]
    pub max_output_bytes: Option<u64>,

    /// Maximum retry attempts summed across all nodes. `None` = unlimited.
    #[serde(default)]
    pub max_total_retries: Option<u32>,
}

impl Default for ExecutionBudget {
    fn default() -> Self {
        Self {
            max_concurrent_nodes: 10,
            max_duration: None,
            max_output_bytes: None,
            max_total_retries: None,
        }
    }
}

impl ExecutionBudget {
    /// Validates fields that affect the workflow scheduler.
    ///
    /// Returns an error if [`Self::max_concurrent_nodes`] is zero — the engine
    /// would otherwise construct a `Semaphore::new(0)` with no permits and
    /// hang forever waiting for concurrency slots.
    pub fn validate_for_execution(&self) -> Result<(), &'static str> {
        if self.max_concurrent_nodes == 0 {
            return Err("max_concurrent_nodes must be >= 1 (0 deadlocks the workflow scheduler)");
        }
        Ok(())
    }

    /// Set the maximum number of concurrent nodes.
    ///
    /// # Panics
    ///
    /// Panics if `n == 0`. A zero semaphore deadlocks the scheduler
    /// silently, so the builder rejects the value loudly.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_max_concurrent_nodes(mut self, n: usize) -> Self {
        assert!(
            n > 0,
            "with_max_concurrent_nodes(0) would deadlock the scheduler; \
             use a positive limit"
        );
        self.max_concurrent_nodes = n;
        self
    }

    /// Set the wall-clock timeout for the entire execution.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_max_duration(mut self, duration: Duration) -> Self {
        self.max_duration = Some(duration);
        self
    }

    /// Set the maximum total bytes across all node outputs.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_max_output_bytes(mut self, bytes: u64) -> Self {
        self.max_output_bytes = Some(bytes);
        self
    }

    /// Set the maximum retry attempts summed across all nodes.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_max_total_retries(mut self, retries: u32) -> Self {
        self.max_total_retries = Some(retries);
        self
    }
}

/// Lightweight execution context.
///
/// This is a minimal placeholder until execution context is properly designed.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Unique identifier for this execution.
    pub execution_id: ExecutionId,
    /// Resource budget for this execution.
    pub budget: ExecutionBudget,
}

impl ExecutionContext {
    /// Create a new execution context.
    pub fn new(execution_id: ExecutionId, budget: ExecutionBudget) -> Self {
        Self {
            execution_id,
            budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget_has_sensible_values() {
        let budget = ExecutionBudget::default();
        assert_eq!(budget.max_concurrent_nodes, 10);
        assert_eq!(budget.max_duration, None);
        assert_eq!(budget.max_output_bytes, None);
        assert_eq!(budget.max_total_retries, None);
    }

    #[test]
    fn builder_sets_all_fields() {
        let budget = ExecutionBudget::default()
            .with_max_concurrent_nodes(4)
            .with_max_duration(Duration::from_secs(300))
            .with_max_output_bytes(1024 * 1024)
            .with_max_total_retries(50);

        assert_eq!(budget.max_concurrent_nodes, 4);
        assert_eq!(budget.max_duration, Some(Duration::from_secs(300)));
        assert_eq!(budget.max_output_bytes, Some(1024 * 1024));
        assert_eq!(budget.max_total_retries, Some(50));
    }

    #[test]
    fn serde_roundtrip_full() {
        let budget = ExecutionBudget::default()
            .with_max_duration(Duration::from_millis(5000))
            .with_max_output_bytes(999)
            .with_max_total_retries(3);

        let json = serde_json::to_string(&budget).unwrap();
        let restored: ExecutionBudget = serde_json::from_str(&json).unwrap();
        assert_eq!(budget, restored);
    }

    #[test]
    fn serde_roundtrip_defaults() {
        let budget = ExecutionBudget::default();
        let json = serde_json::to_string(&budget).unwrap();
        let restored: ExecutionBudget = serde_json::from_str(&json).unwrap();
        assert_eq!(budget, restored);
    }

    #[test]
    fn deserialize_minimal_json() {
        let json = r#"{"max_concurrent_nodes":5}"#;
        let budget: ExecutionBudget = serde_json::from_str(json).unwrap();
        assert_eq!(budget.max_concurrent_nodes, 5);
        assert_eq!(budget.max_duration, None);
        assert_eq!(budget.max_output_bytes, None);
        assert_eq!(budget.max_total_retries, None);
    }

    #[test]
    fn deserialize_rejects_zero_max_concurrent_nodes() {
        let json = r#"{"max_concurrent_nodes":0}"#;
        let err = serde_json::from_str::<ExecutionBudget>(json).unwrap_err();
        assert!(
            err.to_string().contains("max_concurrent_nodes"),
            "unexpected serde error: {err}"
        );
    }

    #[test]
    fn validate_for_execution_rejects_zero() {
        let budget = ExecutionBudget {
            max_concurrent_nodes: 0,
            ..ExecutionBudget::default()
        };
        assert!(budget.validate_for_execution().is_err());
    }
}
