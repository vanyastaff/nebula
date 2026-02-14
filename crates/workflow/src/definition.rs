//! Workflow-level definition types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use nebula_core::{Version, WorkflowId};

use crate::connection::Connection;
use crate::node::NodeDefinition;

/// A complete workflow definition: nodes, connections, metadata, and config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    /// Unique identifier for this workflow.
    pub id: WorkflowId,
    /// Human-readable name.
    pub name: String,
    /// Optional longer description.
    #[serde(default)]
    pub description: Option<String>,
    /// Semantic version of the workflow definition.
    pub version: Version,
    /// The nodes (action steps) in this workflow.
    pub nodes: Vec<NodeDefinition>,
    /// Edges connecting the nodes.
    pub connections: Vec<Connection>,
    /// Workflow-level variables available to all nodes.
    #[serde(default)]
    pub variables: HashMap<String, serde_json::Value>,
    /// Runtime configuration.
    #[serde(default)]
    pub config: WorkflowConfig,
    /// Free-form tags for filtering and grouping.
    #[serde(default)]
    pub tags: Vec<String>,
    /// When this definition was first created.
    pub created_at: DateTime<Utc>,
    /// When this definition was last modified.
    pub updated_at: DateTime<Utc>,
}

/// Runtime configuration for a workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    /// Maximum wall-clock time for the entire workflow run.
    #[serde(default, with = "crate::serde_duration_opt")]
    pub timeout: Option<Duration>,
    /// Maximum number of nodes that may execute concurrently.
    #[serde(default = "default_max_parallel")]
    pub max_parallel_nodes: usize,
    /// Checkpointing (durable progress) settings.
    #[serde(default)]
    pub checkpointing: CheckpointingConfig,
    /// Default retry policy applied to nodes that do not declare their own.
    #[serde(default)]
    pub retry_policy: Option<RetryConfig>,
}

fn default_max_parallel() -> usize {
    10
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            timeout: None,
            max_parallel_nodes: default_max_parallel(),
            checkpointing: CheckpointingConfig::default(),
            retry_policy: None,
        }
    }
}

/// Settings that control how often execution progress is persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointingConfig {
    /// Whether checkpointing is enabled at all.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minimum interval between checkpoints.
    #[serde(default, with = "crate::serde_duration_opt")]
    pub interval: Option<Duration>,
}

fn default_true() -> bool {
    true
}

impl Default for CheckpointingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval: None,
        }
    }
}

/// Retry policy with configurable backoff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Total number of attempts (including the first).
    pub max_attempts: u32,
    /// Delay before the first retry, in milliseconds.
    pub initial_delay_ms: u64,
    /// Upper bound on delay, in milliseconds.
    pub max_delay_ms: u64,
    /// Multiplier applied to the delay after each attempt.
    pub backoff_multiplier: f64,
}

impl RetryConfig {
    /// Create a fixed-delay retry policy.
    #[must_use]
    pub fn fixed(max_attempts: u32, delay_ms: u64) -> Self {
        Self {
            max_attempts,
            initial_delay_ms: delay_ms,
            max_delay_ms: delay_ms,
            backoff_multiplier: 1.0,
        }
    }

    /// Create an exponential-backoff retry policy (multiplier = 2.0).
    #[must_use]
    pub fn exponential(max_attempts: u32, initial_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            max_attempts,
            initial_delay_ms,
            max_delay_ms,
            backoff_multiplier: 2.0,
        }
    }

    /// Calculate the delay for a given attempt (0-indexed).
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let delay_ms = self.initial_delay_ms as f64 * self.backoff_multiplier.powi(attempt as i32);
        let capped = delay_ms.min(self.max_delay_ms as f64) as u64;
        Duration::from_millis(capped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_config_fixed() {
        let cfg = RetryConfig::fixed(3, 500);
        assert_eq!(cfg.max_attempts, 3);
        assert_eq!(cfg.initial_delay_ms, 500);
        assert_eq!(cfg.max_delay_ms, 500);
        assert!((cfg.backoff_multiplier - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn retry_config_exponential() {
        let cfg = RetryConfig::exponential(5, 100, 10_000);
        assert_eq!(cfg.max_attempts, 5);
        assert_eq!(cfg.initial_delay_ms, 100);
        assert_eq!(cfg.max_delay_ms, 10_000);
        assert!((cfg.backoff_multiplier - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn delay_for_attempt_exponential_backoff() {
        let cfg = RetryConfig::exponential(5, 100, 10_000);
        assert_eq!(cfg.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(cfg.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(cfg.delay_for_attempt(2), Duration::from_millis(400));
        assert_eq!(cfg.delay_for_attempt(3), Duration::from_millis(800));
    }

    #[test]
    fn delay_for_attempt_caps_at_max() {
        let cfg = RetryConfig::exponential(10, 100, 500);
        // attempt 3 => 100 * 2^3 = 800, but capped to 500
        assert_eq!(cfg.delay_for_attempt(3), Duration::from_millis(500));
        assert_eq!(cfg.delay_for_attempt(10), Duration::from_millis(500));
    }

    #[test]
    fn delay_for_attempt_fixed_is_constant() {
        let cfg = RetryConfig::fixed(3, 250);
        assert_eq!(cfg.delay_for_attempt(0), Duration::from_millis(250));
        assert_eq!(cfg.delay_for_attempt(1), Duration::from_millis(250));
        assert_eq!(cfg.delay_for_attempt(2), Duration::from_millis(250));
    }

    #[test]
    fn workflow_config_default_values() {
        let cfg = WorkflowConfig::default();
        assert!(cfg.timeout.is_none());
        assert_eq!(cfg.max_parallel_nodes, 10);
        assert!(cfg.checkpointing.enabled);
        assert!(cfg.checkpointing.interval.is_none());
        assert!(cfg.retry_policy.is_none());
    }

    #[test]
    fn checkpointing_config_default_values() {
        let cfg = CheckpointingConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.interval.is_none());
    }

    #[test]
    fn workflow_config_serde_roundtrip() {
        let cfg = WorkflowConfig {
            timeout: Some(Duration::from_millis(30_000)),
            max_parallel_nodes: 5,
            checkpointing: CheckpointingConfig {
                enabled: false,
                interval: Some(Duration::from_millis(1_000)),
            },
            retry_policy: Some(RetryConfig::fixed(3, 500)),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: WorkflowConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.timeout, Some(Duration::from_millis(30_000)));
        assert_eq!(back.max_parallel_nodes, 5);
        assert!(!back.checkpointing.enabled);
        assert_eq!(
            back.checkpointing.interval,
            Some(Duration::from_millis(1_000))
        );
        assert!(back.retry_policy.is_some());
    }

    #[test]
    fn retry_config_serde_roundtrip() {
        let cfg = RetryConfig::exponential(5, 100, 10_000);
        let json = serde_json::to_string(&cfg).unwrap();
        let back: RetryConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.max_attempts, cfg.max_attempts);
        assert_eq!(back.initial_delay_ms, cfg.initial_delay_ms);
        assert_eq!(back.max_delay_ms, cfg.max_delay_ms);
        assert!((back.backoff_multiplier - cfg.backoff_multiplier).abs() < f64::EPSILON);
    }
}
