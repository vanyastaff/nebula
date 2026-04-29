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
/// Controls concurrency, wall-clock timeout, total output size, and
/// the global retry cap (sum of retry attempts across all nodes).
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
///     .with_max_output_bytes(10 * 1024 * 1024)
///     .with_max_total_retries(50);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionBudget {
    /// Maximum nodes executing in parallel.
    ///
    /// Must be at least **1**. The workflow engine uses a `tokio::sync::Semaphore`
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

    /// Global cap on retry attempts summed across **all** nodes in
    /// the execution (ADR-0042 §Consequences "Out of scope" + §M2.1
    /// T4 acceptance). Complements per-node
    /// `RetryConfig::max_attempts`: the engine consults both on every
    /// failure and whichever caps first wins (canon §11.2).
    ///
    /// `None` = no global cap; per-node policy is the only retry
    /// gate. A `Some(0)` value disables engine-level retry entirely
    /// for this execution, regardless of the per-node `RetryConfig`.
    ///
    /// Forward-compat: legacy persisted budgets that predate this
    /// field deserialize as `None`.
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

    /// Set the global retry cap (ADR-0042 §M2.1 T4).
    ///
    /// `0` disables engine-level retry entirely for this execution
    /// even when nodes declare a `RetryConfig`. Useful for tests and
    /// for "execute once, no retries" SLAs.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_max_total_retries(mut self, n: u32) -> Self {
        self.max_total_retries = Some(n);
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
    }

    #[test]
    fn builder_sets_all_fields() {
        let budget = ExecutionBudget::default()
            .with_max_concurrent_nodes(4)
            .with_max_duration(Duration::from_mins(5))
            .with_max_output_bytes(1024 * 1024);

        assert_eq!(budget.max_concurrent_nodes, 4);
        assert_eq!(budget.max_duration, Some(Duration::from_mins(5)));
        assert_eq!(budget.max_output_bytes, Some(1024 * 1024));
    }

    #[test]
    fn serde_roundtrip_full() {
        let budget = ExecutionBudget::default()
            .with_max_duration(Duration::from_secs(5))
            .with_max_output_bytes(999);

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

    /// ADR-0042 §M2.1 T2 — `max_total_retries` must round-trip
    /// through serde so `resume_execution` carries the same global
    /// cap the original run was started with.
    #[test]
    fn max_total_retries_roundtrip() {
        let budget = ExecutionBudget::default().with_max_total_retries(7);
        assert_eq!(budget.max_total_retries, Some(7));
        let json = serde_json::to_string(&budget).unwrap();
        let back: ExecutionBudget = serde_json::from_str(&json).unwrap();
        assert_eq!(back.max_total_retries, Some(7));
    }

    /// Forward-compat: legacy budgets that predate `max_total_retries`
    /// deserialize as `None` (no global cap), so a resumed legacy
    /// execution does not crash on the missing field.
    #[test]
    fn max_total_retries_missing_field_deserializes_as_none() {
        let legacy = r#"{"max_concurrent_nodes":4}"#;
        let budget: ExecutionBudget = serde_json::from_str(legacy).unwrap();
        assert_eq!(budget.max_total_retries, None);
    }

    /// `Some(0)` is a meaningful "disable retry" signal — distinct
    /// from `None` (no cap). The engine must observe both states.
    #[test]
    fn max_total_retries_zero_is_distinct_from_none() {
        let disabled = ExecutionBudget::default().with_max_total_retries(0);
        let unlimited = ExecutionBudget::default();
        assert_eq!(disabled.max_total_retries, Some(0));
        assert_eq!(unlimited.max_total_retries, None);
        assert_ne!(disabled, unlimited);
    }
}
