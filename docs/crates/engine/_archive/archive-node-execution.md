# Archived From "docs/archive/node-execution.md"

### nebula-engine
**Назначение:** Главный orchestrator выполнения workflows.

**Ключевые компоненты:**
- WorkflowEngine - основной движок
- Scheduler - планирование выполнения
- Executor - выполнение узлов
- State management

```rust
pub struct WorkflowEngine {
    scheduler: Arc<Scheduler>,
    executor: Arc<Executor>,
    state_store: Arc<StateStore>,
    resource_manager: Arc<ResourceManager>,
    event_bus: Arc<EventBus>,
}

impl WorkflowEngine {
    pub async fn execute_workflow(
        &self,
        workflow_id: WorkflowId,
        input: serde_json::Value,
        options: ExecutionOptions,
    ) -> Result<ExecutionHandle> {
        // Создаем execution
        let execution_id = ExecutionId::new();
        let workflow = self.load_workflow(&workflow_id).await?;
        
        // Инициализируем контекст
        let context = ExecutionContext::new(
            execution_id.clone(),
            workflow_id.clone(),
            workflow,
        );
        
        // Планируем выполнение
        let handle = self.scheduler.schedule(
            context,
            input,
            options.priority,
        ).await?;
        
        // Отправляем событие
        self.event_bus.emit(ExecutionEvent::Started {
            execution_id,
            workflow_id,
        }).await?;
        
        Ok(handle)
    }
}
```

---

---

### Полный flow выполнения

```rust
// 1. Определяем workflow
let workflow = WorkflowBuilder::new("order-processing")
    .add_node("validate", "validation.order")
    .add_node("payment", "payment.process")
    .add_node("notification", "notification.send")
    .connect("validate", "payment", "$nodes.validate.success")
    .connect("payment", "notification", "$nodes.payment.success")
    .build();

// 2. Регистрируем в engine
engine.register_workflow(workflow).await?;

// 3. Запускаем выполнение
let handle = engine.execute_workflow(
    "order-processing",
    json!({ "order_id": 12345, "amount": 99.99 }),
    ExecutionOptions::default(),
).await?;

// 4. Engine создает ExecutionContext
// 5. Scheduler планирует выполнение узлов
// 6. Worker'ы выполняют Actions
// 7. Results сохраняются в context
// 8. Events публикуются в eventbus

// Мониторинг выполнения
handle.on_node_complete(|node_id, result| {
    println!("Node {} completed: {:?}", node_id, result);
});

let final_result = handle.await?;
```

