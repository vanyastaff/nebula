# Common Types

`nebula-core::types` provides reusable domain types shared across the workspace.

---

## Version

Semver version struct with pre-release and build metadata:

```rust
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre: Option<String>,    // e.g. "alpha.1"
    pub build: Option<String>,  // e.g. "20260228"
}
```

**Compatibility:** `a.is_compatible_with(b)` → same major AND same minor.

**Display:** `"1.2.3"`, `"1.2.3-alpha.1"`, `"1.2.3+20260228"`.

---

## InterfaceVersion

Schema compatibility version for action interfaces and ports:

```rust
pub struct InterfaceVersion {
    pub major: u32,
    pub minor: u32,
}
```

**Compatibility:** `required.is_compatible_with(provided)` → same major AND
`provided.minor >= required.minor` (forward-compatible: new minor is acceptable).

Used by `nebula-action` to ensure the engine's interface contract is met before loading
a plugin.

---

## Status

Nine-variant lifecycle status:

```rust
pub enum Status {
    Active,
    Inactive,
    InProgress,
    Completed,
    Failed,
    Pending,
    Cancelled,
    Suspended,
    Error,
}
```

| Method | Returns `true` for |
|---|---|
| `is_success()` | `Completed` |
| `is_failure()` | `Failed`, `Error` |
| `is_completed()` | `Completed`, `Failed`, `Cancelled`, `Error` |
| `is_active()` | `Active`, `InProgress` |

Used by workflow executions, resource states, and operation results.

---

## Priority

Five-level priority for scheduling and queuing:

```rust
pub enum Priority {
    Low = 1,
    Normal = 2,
    High = 3,
    Critical = 4,
    Emergency = 5,
}
```

| Method | Returns `true` for |
|---|---|
| `is_urgent()` | `High`, `Critical`, `Emergency` |
| `is_critical()` | `Critical`, `Emergency` |

---

## ProjectType

```rust
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    Personal,
    Team,
}
```

---

## RoleScope

RBAC role scope — where a role applies:

```rust
pub enum RoleScope {
    Global,
    Project,
    Credential,
    Workflow,
}
```

---

## OperationResult\<T\>

```rust
pub struct OperationResult<T> {
    pub status: Status,
    pub data: Option<T>,
    pub error: Option<String>,
    pub completed_at: DateTime<Utc>,
    pub duration: Duration,
}
```

**Constructors:**

```rust
OperationResult::success(data, duration)
OperationResult::failure(error_message, duration)
```

---

## OperationContext

Rich context for tracking an in-progress operation across layers:

```rust
pub struct OperationContext {
    pub operation_id: String,
    pub execution_id: Option<ExecutionId>,
    pub workflow_id: Option<WorkflowId>,
    pub node_id: Option<NodeId>,
    pub user_id: Option<UserId>,
    pub tenant_id: Option<TenantId>,
    pub priority: Priority,
    pub metadata: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
}
```

Builder pattern:

```rust
let ctx = OperationContext::builder()
    .execution_id(exec_id)
    .workflow_id(wf_id)
    .priority(Priority::High)
    .metadata("source", "api")
    .build();
```

---

## Utility Functions

```rust
// Generate a unique operation ID string
let id = nebula_core::types::utils::generate_operation_id();

// Format a Duration as human-readable string
let s = nebula_core::types::utils::format_duration(dur); // "1.23s", "45ms"

// Parse version string
let v: Version = nebula_core::types::utils::parse_version("1.2.3-beta")?;

// Validate identifier (matches IDENTIFIER_PATTERN: ^[a-zA-Z_][a-zA-Z0-9_-]*$)
let ok = nebula_core::types::utils::is_valid_identifier("my_action");
```
