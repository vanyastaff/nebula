# Archived From "docs/archive/overview.md"

### nebula-execution
**Назначение:** Runtime выполнение workflow - управляет "как выполняется".

**Ключевые компоненты:**
- ExecutionContext - контекст выполнения
- ExecutionState - состояние выполнения
- NodeOutput - результаты узлов
- Expression integration

```rust
// Контекст выполнения с интеграцией всех систем
pub struct ExecutionContext {
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub workflow_definition: Arc<WorkflowDefinition>,
    pub node_outputs: Arc<RwLock<HashMap<NodeId, NodeOutput>>>,
    pub resource_manager: Arc<ResourceManager>,
    pub credential_manager: Arc<CredentialManager>,
    pub expression_engine: Arc<ExpressionEngine>,
}

// NodeOutput хранит результат как serde_json::Value
pub struct NodeOutput {
    pub result: serde_json::Value,
    pub status: NodeStatus,
    pub duration: Duration,
}

// Использование
let context = ExecutionContext::new(workflow_id, execution_id);

// Вычисление expressions — результат serde_json::Value
let user_email: serde_json::Value = context
    .evaluate_expression("$nodes.create_user.result.email")
    .await?;

// Resolve ParamValue перед передачей в Action
let resolved: serde_json::Value = context
    .resolve_param_value(&param_value)
    .await?;

// Получение ресурсов с правильным scope
let database = context.get_resource::<DatabaseResource>().await?;
```

---

