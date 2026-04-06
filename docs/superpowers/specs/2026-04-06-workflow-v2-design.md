# nebula-workflow v2 — Design Spec

## Goal

Complete workflow definition model for production use: schema versioning, validation improvements, action registry integration, UI metadata, multi-tenant support.

## Current State

2,193 LOC, 69 tests. Today's session added: TriggerDefinition, ErrorStrategy, `#[non_exhaustive]` on 5 enums, NodeDefinition.enabled, NodeDefinition::new returns Result. Core DAG model (nodes, connections, conditions, graph validation) is solid.

---

## 1. WorkflowDefinition — Missing Fields

```rust
pub struct WorkflowDefinition {
    // Existing fields (unchanged)
    pub id: WorkflowId,
    pub name: String,
    pub description: Option<String>,
    pub version: Version,
    pub nodes: Vec<NodeDefinition>,
    pub connections: Vec<Connection>,
    pub variables: HashMap<String, Value>,
    pub config: WorkflowConfig,
    pub trigger: Option<TriggerDefinition>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // NEW fields
    /// Who owns this workflow (user/team/org ID for multi-tenant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

fn default_schema_version() -> u32 { 1 }
```

### UiMetadata (opaque to engine)

```rust
/// Visual metadata for the workflow editor. Engine ignores this entirely.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UiMetadata {
    /// Per-node visual properties (position, color, collapsed state).
    #[serde(default)]
    pub node_positions: HashMap<NodeId, NodePosition>,
    /// Editor viewport (zoom, scroll position).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Viewport>,
    /// Free-form annotations (sticky notes, comments).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    pub id: String,
    pub text: String,
    pub position: NodePosition,
    pub color: Option<String>,
}
```

---

## 2. Schema Versioning

```rust
/// Schema version history:
/// v1: Initial format (current)
/// v2: (future) Added when breaking changes to serialization format happen

impl WorkflowDefinition {
    /// Check if this definition's schema version is supported.
    pub fn is_schema_supported(&self) -> bool {
        self.schema_version <= CURRENT_SCHEMA_VERSION
    }
}

const CURRENT_SCHEMA_VERSION: u32 = 1;
```

CI guard: snapshot tests that serialize/deserialize a reference workflow and compare against committed JSON fixture. Any change to serialization = test failure = explicit review.

---

## 3. Validation Improvements

### 3.1 Trigger Validation

```rust
fn validate_trigger(trigger: &TriggerDefinition) -> Vec<WorkflowError> {
    match trigger {
        TriggerDefinition::Cron { expression } => {
            // Validate cron expression syntax
            if cron::Schedule::from_str(expression).is_err() {
                vec![WorkflowError::InvalidTrigger {
                    reason: format!("invalid cron expression: {expression}"),
                }]
            } else { vec![] }
        }
        TriggerDefinition::Webhook { path, .. } => {
            if !path.starts_with('/') {
                vec![WorkflowError::InvalidTrigger {
                    reason: "webhook path must start with '/'".into(),
                }]
            } else { vec![] }
        }
        _ => vec![],
    }
}
```

### 3.2 New WorkflowError Variants

```rust
pub enum WorkflowError {
    // ... existing variants ...

    /// Invalid trigger configuration.
    #[classify(category = "validation", code = "WORKFLOW:INVALID_TRIGGER")]
    #[error("invalid trigger: {reason}")]
    InvalidTrigger { reason: String },

    /// Workflow schema version not supported.
    #[classify(category = "validation", code = "WORKFLOW:UNSUPPORTED_SCHEMA")]
    #[error("unsupported schema version {version}, max supported: {max}")]
    UnsupportedSchema { version: u32, max: u32 },
}
```

### 3.3 Action Registry Validation (future, when wired to engine)

```rust
/// Validate that all action_keys in the workflow exist in the registry.
pub fn validate_against_registry(
    definition: &WorkflowDefinition,
    registry: &ActionRegistry,
) -> Vec<WorkflowError> {
    definition.nodes.iter().filter_map(|node| {
        if registry.get_latest(&node.action_key).is_none() {
            Some(WorkflowError::UnknownAction {
                node_id: node.id,
                action_key: node.action_key.clone(),
            })
        } else { None }
    }).collect()
}
```

---

## 4. Builder Improvements

```rust
impl WorkflowBuilder {
    /// Set owner for multi-tenant workflows.
    pub fn owner(mut self, owner_id: impl Into<String>) -> Self {
        self.owner_id = Some(owner_id.into());
        self
    }

    /// Set trigger.
    pub fn trigger(mut self, trigger: TriggerDefinition) -> Self {
        self.trigger = Some(trigger);
        self
    }

    /// Set UI metadata.
    pub fn ui_metadata(mut self, metadata: UiMetadata) -> Self {
        self.ui_metadata = Some(metadata);
        self
    }

    /// Build with ALL errors collected (not fail-fast).
    pub fn build(self) -> Result<WorkflowDefinition, Vec<WorkflowError>> {
        let mut errors = vec![];
        // ... collect all validation errors ...
        if errors.is_empty() {
            Ok(definition)
        } else {
            Err(errors)
        }
    }
}
```

---

## 5. PartialEq on WorkflowDefinition

Add `PartialEq` derive (flagged in audit — missing, makes testing awkward):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDefinition { ... }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowConfig { ... }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryConfig { ... }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckpointingConfig { ... }
```

---

## 6. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Owner/tenant | None | `owner_id: Option<String>` |
| UI metadata | None | `UiMetadata` (positions, viewport, annotations) |
| Schema version | None | `schema_version: u32` with compatibility check |
| Trigger validation | None | Cron syntax + webhook path validation |
| Builder errors | Fail-fast | Collect all errors |
| PartialEq | Missing on WorkflowDefinition | Derived |
| Action validation | None | `validate_against_registry()` (future) |

---

## 7. Not In Scope

- Sub-workflows / node grouping (Phase 2)
- Workflow migration between schema versions (Phase 2)
- Soft delete (storage concern, not definition)
- Tag validation/allowlists (API layer concern)
- Node-to-node type checking (DataTag enforcement, editor concern)

---

## Post-Conference Round 2 Amendments

### W1. owner_id becomes required OwnerId (Meta)
`owner_id: Option<String>` becomes `owner_id: OwnerId` (required newtype). All storage queries filter by OwnerId. V1 blocker for multi-tenant. `PostgresStorage` must set `app.current_owner` session variable for Row Level Security policies — enforcement at the database layer, not just application code (Supabase feedback).

### W2. Durable webhook inbound queue (Telegram)
Webhook events written to Postgres BEFORE HTTP 200 ack. At-least-once delivery with dedup by event fingerprint.
