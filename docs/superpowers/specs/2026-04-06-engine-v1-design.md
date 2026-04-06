# nebula-engine v1 — Design Spec

## Goal

Complete the workflow execution engine for production v1: wire storage persistence, credential/resource DI, enforce execution budgets, support error strategies, and enable crash recovery via checkpoints.

## Current State

Engine is a **frontier-based DAG orchestrator** (1515 LOC, 48 tests). Works for simple workflows: linear pipelines, fan-out, diamond merge, error routing. Dispatches nodes via tokio JoinSet with semaphore-bounded concurrency. Parameter resolution (expressions, templates, references) works. Metrics wired.

**NOT wired:** storage (in-memory only), credentials, resources, execution budget enforcement, trigger lifecycle, support inputs.

---

## 1. Storage Integration (v1 BLOCKER)

Engine must persist state to survive crashes:

```rust
pub struct WorkflowEngine {
    runtime: Arc<ActionRuntime>,
    metrics: MetricsRegistry,
    expression_engine: Arc<ExpressionEngine>,
    resolver: ParamResolver,
    // NEW:
    workflow_repo: Arc<dyn WorkflowRepo>,
    execution_repo: Arc<dyn ExecutionRepo>,
}
```

### Checkpoint Contract (RT7 + RT12)

After EACH node completion:
1. Serialize node output to storage (`execution_repo.save_node_output(exec_id, node_id, output)`)
2. Update `ExecutionState` with node terminal status
3. Persist `ExecutionState` to storage (`execution_repo.save_state(state)`)
4. THEN dispatch next ready nodes

On crash recovery:
1. Query `execution_repo.list_running()` → find incomplete executions
2. Load `ExecutionState` → identify last completed node
3. Skip terminal nodes, resume from first non-terminal
4. Re-resolve inputs from persisted predecessor outputs

```rust
impl WorkflowEngine {
    /// Resume an incomplete execution after process restart.
    pub async fn resume_execution(
        &self,
        execution_id: &ExecutionId,
    ) -> Result<ExecutionResult, EngineError> {
        let state = self.execution_repo.load_state(execution_id).await?;
        let workflow = self.workflow_repo.get(state.workflow_id()).await?;
        let persisted_outputs = self.execution_repo.load_all_outputs(execution_id).await?;

        // Build ready queue from non-terminal nodes whose predecessors are all terminal
        let ready = self.compute_resume_frontier(&workflow, &state, &persisted_outputs);

        self.execute_from_frontier(workflow, state, persisted_outputs, ready).await
    }
}
```

### Node Output Storage

Per-node outputs stored individually (RT12):

```rust
pub trait ExecutionRepo {
    // Existing:
    async fn save_state(&self, state: &ExecutionState) -> Result<()>;
    async fn load_state(&self, id: &ExecutionId) -> Result<ExecutionState>;

    // NEW:
    async fn save_node_output(&self, exec_id: &ExecutionId, node_id: &NodeId, output: &Value) -> Result<()>;
    async fn load_node_output(&self, exec_id: &ExecutionId, node_id: &NodeId) -> Result<Option<Value>>;
    async fn load_all_outputs(&self, exec_id: &ExecutionId) -> Result<HashMap<NodeId, Value>>;

    // For resume:
    async fn list_running(&self) -> Result<Vec<ExecutionId>>;
}
```

---

## 2. Credential DI (v1 BLOCKER)

Wire `CredentialResolver` into `ActionContext` during node dispatch:

```rust
// In engine's spawn_node():
let ctx = ActionContext::builder()
    .execution_id(exec_id)
    .node_id(node_id)
    .workflow_id(workflow_id)
    .cancellation(cancel_token.child_token())
    .with_credentials(Arc::new(EngineCredentialAccessor::new(
        credential_resolver.clone(),
        action_dependencies.credential_keys(),
    )))
    .with_resources(Arc::new(EngineResourceAccessor::new(
        resource_manager.clone(),
        action_dependencies.resource_keys(),
    )))
    .with_logger(Arc::new(ExecutionLogger::new(exec_id, node_id)))
    .build();
```

`EngineCredentialAccessor` implements `CredentialAccessor`:
```rust
struct EngineCredentialAccessor {
    resolver: Arc<CredentialResolver>,
    allowed_keys: HashSet<CredentialKey>,
}

impl CredentialAccessor for EngineCredentialAccessor {
    async fn resolve_typed<S: AuthScheme>(&self, key: &str) -> Result<S, ActionError> {
        // Validate key is declared
        // Resolve via CredentialResolver
        // Project to typed scheme
    }
}
```

---

## 3. Execution Budget Enforcement

`ExecutionBudget` (from execution crate) enforced by engine loop:

```rust
struct ExecutionTracker {
    budget: ExecutionBudget,
    started_at: Instant,
    total_output_bytes: AtomicU64,
    total_retries: AtomicU32,
}

impl ExecutionTracker {
    /// Check budget before dispatching next node.
    fn check_budget(&self) -> Result<(), EngineError> {
        // max_duration
        if let Some(max) = self.budget.max_duration {
            if self.started_at.elapsed() > max {
                return Err(EngineError::BudgetExceeded("max_duration exceeded"));
            }
        }
        // max_output_bytes (RT1)
        if let Some(max) = self.budget.max_output_bytes {
            if self.total_output_bytes.load(Relaxed) > max {
                return Err(EngineError::BudgetExceeded("max_output_bytes exceeded"));
            }
        }
        // max_total_retries
        if let Some(max) = self.budget.max_total_retries {
            if self.total_retries.load(Relaxed) > max {
                return Err(EngineError::BudgetExceeded("max_total_retries exceeded"));
            }
        }
        Ok(())
    }
}
```

---

## 4. Error Strategy Enforcement

Engine respects `WorkflowConfig.error_strategy` + per-node `ErrorStrategy`:

```rust
fn handle_node_failure(
    &self,
    node_id: NodeId,
    error: ActionError,
    state: &mut ExecutionState,
    config: &WorkflowConfig,
) -> FailureAction {
    // Check if node has OnError edges
    let has_error_handler = self.graph.has_error_edges(node_id);

    if has_error_handler {
        // Route to error handler nodes
        return FailureAction::RouteToErrorHandler;
    }

    match config.error_strategy {
        ErrorStrategy::FailFast => FailureAction::CancelExecution,
        ErrorStrategy::ContinueOnError => FailureAction::SkipDependents,
        ErrorStrategy::IgnoreErrors => FailureAction::Continue,
    }
}
```

---

## 5. NodeDefinition.enabled Check

Engine skips disabled nodes:

```rust
// In frontier processing:
if !node_def.enabled {
    state.set_node_state(node_id, NodeState::Skipped);
    self.resolve_outgoing_edges(node_id, None, &mut ready_queue);
    continue;
}
```

---

## 6. Proactive Credential Refresh (C5)

Before dispatching a node that uses credentials:

```rust
for cred_key in action_deps.credential_keys() {
    if let Some(expires_at) = self.credential_resolver.expires_at(cred_key).await? {
        let threshold = Duration::from_secs(300); // 5 min
        if expires_at - Utc::now() < threshold {
            self.credential_resolver.refresh(cred_key).await?;
        }
    }
}
```

---

## 7. IdempotencyManager — Durable (B1)

Engine uses durable idempotency backed by storage:

```rust
let idem_key = IdempotencyKey::generate(exec_id, node_id, attempt);
if self.execution_repo.check_idempotency(&idem_key).await? {
    // Already executed — load persisted output
    let output = self.execution_repo.load_node_output(exec_id, node_id).await?;
    return Ok(output);
}
// Execute, then mark as done
let result = runtime.execute_action(...).await?;
self.execution_repo.mark_idempotent(&idem_key).await?;
```

---

## 8. Action Version Pinning (B6)

Engine uses pinned version from NodeDefinition, never `get_latest()`:

```rust
let handler = match &node_def.action_version {
    Some(version) => registry.get(&node_def.action_key, version),
    None => registry.get_latest(&node_def.action_key),
};
// In production: action_version should always be Some.
// get_latest() is for editor/preview only.
```

---

## 9. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Storage | In-memory only | Persistent via ExecutionRepo |
| Credentials | Not wired | EngineCredentialAccessor → ActionContext |
| Resources | Stub | EngineResourceAccessor → ActionContext |
| Budget | Semaphore only | Full budget tracking (duration, bytes, retries) |
| Error strategy | FailFast hardcoded | Configurable per workflow |
| Enabled check | Not checked | Disabled nodes skipped |
| Idempotency | In-memory HashSet | Durable via storage |
| Version pinning | get_latest() always | Pinned from NodeDefinition |
| Crash recovery | None | Resume from checkpoint |

---

## 10. Implementation Phases

| Phase | What | Depends on |
|-------|------|------------|
| 1 | Storage integration (save/load state + outputs) | PgExecutionRepo |
| 2 | Credential DI (EngineCredentialAccessor) | Credential v3 resolver |
| 3 | Budget enforcement + error strategy | Phase 1 |
| 4 | Crash recovery (resume_execution) | Phase 1 |
| 5 | Durable idempotency | Phase 1 |
| 6 | Resource DI | Resource v2 Manager |
| 7 | Action version pinning | ActionRegistry versioned |
| 8 | Proactive credential refresh | Phase 2 |

**Phase 1-3 = minimum viable engine.** Phase 4-5 = production-ready.

---

## 11. Not In Scope

- Trigger lifecycle (runtime concern)
- Dynamic fan-out / ForEach (v2)
- Sub-workflows (v2)
- ConcurrencyPolicy per-key (v2, RT10)
- WaitForEvent / suspended execution (v2)
- Multi-instance distributed scheduling (v2+)
- CachePolicy / skip-if-cached (v1.1)
