//! Workflow-level definition types.

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::{ActionKey, NodeKey, WorkflowId};
use serde::{Deserialize, Serialize};

use crate::{Version, connection::Connection, node::NodeDefinition};

/// Current schema version of the workflow definition format.
///
/// v1 → v2: replaced `trigger: Option<TriggerDefinition>` with
/// `trigger_bindings: Vec<TriggerBinding>` (pluggable-trigger reframe, ADR-0095).
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// A complete workflow definition: nodes, connections, metadata, and config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Triggers that can start this workflow. Empty = manual / API-only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_bindings: Vec<TriggerBinding>,
    /// Free-form tags for filtering and grouping.
    #[serde(default)]
    pub tags: Vec<String>,
    /// When this definition was first created.
    pub created_at: DateTime<Utc>,
    /// When this definition was last modified.
    pub updated_at: DateTime<Utc>,
    /// Who owns this workflow (user/team/org ID for multi-tenant).
    /// Required for storage with Row-Level Security.
    #[serde(default)]
    pub owner_id: Option<String>,
    /// UI metadata: node positions, viewport, annotations.
    /// Opaque to the engine — only desktop/web app reads this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_metadata: Option<UiMetadata>,
    /// Schema version of the definition format itself.
    /// Used for forward/backward compatibility detection.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
}

impl WorkflowDefinition {
    /// Check if this definition's schema version is supported.
    #[must_use]
    pub fn is_schema_supported(&self) -> bool {
        self.schema_version <= CURRENT_SCHEMA_VERSION
    }
}

/// A trigger that can start this workflow: a reference to a plugin-provided
/// trigger action by key, plus its author-supplied configuration.
///
/// Structurally parallel to [`NodeDefinition`] (`action_key` + config), but a
/// trigger lives outside the execution graph — it is a workflow *starter*, not
/// a node. The referenced action must resolve to a `TriggerAction` in the
/// plugin registry at load time; this type carries no transport knowledge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerBinding {
    /// Stable identity of this trigger within the workflow (dedup scope key,
    /// routing diagnostics). Author-defined, like `NodeDefinition::id`.
    pub id: NodeKey,
    /// The plugin-provided trigger action this binding instantiates,
    /// e.g. `"cron.schedule"`, `"http.webhook"`.
    pub action_key: ActionKey,
    /// Optional pinned interface version (mirrors `NodeDefinition`).
    #[serde(default)]
    pub interface_version: Option<Version>,
    /// Opaque, action-specific configuration (cron expression, webhook path,
    /// event-type filter). Validated by the trigger action, not by `workflow`.
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Strategy for handling node failures without explicit error edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorStrategy {
    /// Fail the entire workflow immediately on first node failure.
    #[default]
    FailFast,
    /// Continue executing unaffected branches; fail the workflow only after
    /// all reachable nodes have completed or failed.
    ContinueOnError,
    /// Ignore node failures entirely — the workflow always completes.
    IgnoreErrors,
}

/// Runtime configuration for a workflow execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// What to do when a node fails and has no error edge.
    #[serde(default)]
    pub error_strategy: ErrorStrategy,
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
            error_strategy: ErrorStrategy::default(),
        }
    }
}

/// Settings that control how often execution progress is persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

/// Visual metadata for the workflow editor. Engine ignores this entirely.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UiMetadata {
    /// Per-node visual properties (position, color, collapsed state).
    #[serde(default)]
    pub node_positions: HashMap<NodeKey, NodePosition>,
    /// Editor viewport (zoom, scroll position).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Viewport>,
    /// Free-form annotations (sticky notes, comments).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
}

/// Position of a node in the visual editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePosition {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

/// Editor viewport state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    /// Horizontal scroll offset.
    pub x: f64,
    /// Vertical scroll offset.
    pub y: f64,
    /// Zoom level (1.0 = 100%).
    pub zoom: f64,
}

/// Free-form annotation (sticky note, comment) in the editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    /// Unique identifier for this annotation.
    pub id: String,
    /// Annotation text content.
    pub text: String,
    /// Position in the editor canvas.
    pub position: NodePosition,
    /// Optional color (CSS hex string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

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
            timeout: Some(Duration::from_secs(30)),
            max_parallel_nodes: 5,
            checkpointing: CheckpointingConfig {
                enabled: false,
                interval: Some(Duration::from_secs(1)),
            },
            retry_policy: Some(RetryConfig::fixed(3, 500)),
            error_strategy: ErrorStrategy::ContinueOnError,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: WorkflowConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.timeout, Some(Duration::from_secs(30)));
        assert_eq!(back.max_parallel_nodes, 5);
        assert!(!back.checkpointing.enabled);
        assert_eq!(back.checkpointing.interval, Some(Duration::from_secs(1)));
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

    #[test]
    fn schema_version_defaults_to_current() {
        let wf_id = WorkflowId::new();
        let json = format!(
            "{{\
            \"id\": \"{wf_id}\",\
            \"name\": \"test\",\
            \"version\": {{\"major\": 1, \"minor\": 0, \"patch\": 0}},\
            \"nodes\": [],\
            \"connections\": [],\
            \"created_at\": \"2026-01-01T00:00:00Z\",\
            \"updated_at\": \"2026-01-01T00:00:00Z\"\
            }}"
        );
        let def: WorkflowDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(def.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(def.is_schema_supported());
    }

    #[test]
    fn trigger_binding_serde_roundtrip() {
        use nebula_core::ActionKey;
        let binding = TriggerBinding {
            id: node_key!("every-5-min"),
            action_key: "cron.schedule".parse::<ActionKey>().unwrap(),
            interface_version: None,
            config: serde_json::json!({"expression": "0 */5 * * *"}),
        };
        let json = serde_json::to_string(&binding).unwrap();
        let back: TriggerBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, binding.id);
        assert_eq!(back.action_key, binding.action_key);
        assert_eq!(back.config, binding.config);
    }

    #[test]
    fn workflow_definition_with_trigger_bindings_roundtrips() {
        use nebula_core::ActionKey;
        let wf_id = WorkflowId::new();
        let binding = TriggerBinding {
            id: node_key!("webhook-in"),
            action_key: "http.webhook".parse::<ActionKey>().unwrap(),
            interface_version: None,
            config: serde_json::json!({"path": "/hooks/incoming"}),
        };
        let now = Utc::now();
        let def = WorkflowDefinition {
            id: wf_id,
            name: "wf-with-trigger".into(),
            description: None,
            version: Version::new(1, 0, 0),
            nodes: Vec::new(),
            connections: Vec::new(),
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            trigger_bindings: vec![binding],
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: CURRENT_SCHEMA_VERSION,
        };
        let json = serde_json::to_string(&def).unwrap();
        let back: WorkflowDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(back.trigger_bindings.len(), 1);
        assert_eq!(back.trigger_bindings[0].id, node_key!("webhook-in"));
    }

    #[test]
    fn future_schema_version_not_supported() {
        let wf_id = WorkflowId::new();
        let json = format!(
            "{{\
            \"id\": \"{wf_id}\",\
            \"name\": \"test\",\
            \"version\": {{\"major\": 1, \"minor\": 0, \"patch\": 0}},\
            \"nodes\": [],\
            \"connections\": [],\
            \"created_at\": \"2026-01-01T00:00:00Z\",\
            \"updated_at\": \"2026-01-01T00:00:00Z\",\
            \"schema_version\": 99\
            }}"
        );
        let def: WorkflowDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(def.schema_version, 99);
        assert!(!def.is_schema_supported());
    }

    #[test]
    fn ui_metadata_round_trips() {
        let mut ui = UiMetadata::default();
        ui.node_positions
            .insert(node_key!("test"), NodePosition { x: 100.0, y: 200.0 });
        ui.viewport = Some(Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.5,
        });
        let json = serde_json::to_string(&ui).unwrap();
        let parsed: UiMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(ui, parsed);
    }

    #[test]
    fn ui_metadata_empty_annotations_omitted() {
        let ui = UiMetadata::default();
        let json = serde_json::to_value(&ui).unwrap();
        assert!(json.get("annotations").is_none());
    }

    #[test]
    fn owner_id_defaults_to_none() {
        let wf_id = WorkflowId::new();
        let json = format!(
            "{{\
            \"id\": \"{wf_id}\",\
            \"name\": \"test\",\
            \"version\": {{\"major\": 1, \"minor\": 0, \"patch\": 0}},\
            \"nodes\": [],\
            \"connections\": [],\
            \"created_at\": \"2026-01-01T00:00:00Z\",\
            \"updated_at\": \"2026-01-01T00:00:00Z\"\
            }}"
        );
        let def: WorkflowDefinition = serde_json::from_str(&json).unwrap();
        assert!(def.owner_id.is_none());
    }

    #[test]
    fn ui_metadata_skipped_when_none() {
        let wf_id = WorkflowId::new();
        let json = format!(
            "{{\
            \"id\": \"{wf_id}\",\
            \"name\": \"test\",\
            \"version\": {{\"major\": 1, \"minor\": 0, \"patch\": 0}},\
            \"nodes\": [],\
            \"connections\": [],\
            \"created_at\": \"2026-01-01T00:00:00Z\",\
            \"updated_at\": \"2026-01-01T00:00:00Z\"\
            }}"
        );
        let def: WorkflowDefinition = serde_json::from_str(&json).unwrap();
        assert!(def.ui_metadata.is_none());

        // Roundtrip: ui_metadata should be absent from serialized output
        let serialized = serde_json::to_value(&def).unwrap();
        assert!(serialized.get("ui_metadata").is_none());
    }
}
