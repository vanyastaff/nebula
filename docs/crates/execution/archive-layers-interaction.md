# Archived From "docs/archive/layers-interaction.md"

### 3. nebula-expression ↔ nebula-execution

**Цепочка:** Expression вычисляется в контексте Execution и возвращает serde_json::Value

```rust
// nebula-execution предоставляет контекст
pub struct ExecutionContext {
    pub node_outputs: Arc<RwLock<HashMap<NodeId, NodeOutput>>>,
    pub variables: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    expression_engine: Arc<ExpressionEngine>,
}

impl ExecutionContext {
    pub async fn evaluate_expression(&self, expr: &str) -> Result<serde_json::Value, ExpressionError> {
        // Создаем контекст для expression engine
        let eval_context = ExpressionContext {
            get_node_output: Box::new(|node_id| {
                self.node_outputs.read().await
                    .get(&node_id)
                    .map(|output| output.result.clone())
            }),
            get_variable: Box::new(|var_name| {
                self.variables.read().await
                    .get(&var_name)
                    .cloned()
            }),
            get_user_context: Box::new(|| {
                json!({
                    "id": self.user_id,
                    "account": self.account_id,
                })
            }),
        };
        
        // Expression engine парсит и вычисляет
        self.expression_engine.evaluate(expr, eval_context).await
    }
    
    /// Резолвит ParamValue в serde_json::Value.
    /// Expressions и templates вычисляются, литералы возвращаются as-is.
    pub async fn resolve_param_value(&self, pv: &ParamValue) -> Result<serde_json::Value> {
        match pv {
            ParamValue::Literal(v) => Ok(v.clone()),
            ParamValue::Expression(expr) => self.evaluate_expression(&expr.raw).await,
            ParamValue::Template(tmpl) => self.resolve_template(tmpl).await,
        }
    }
}

// nebula-expression использует данные из контекста
impl ExpressionEngine {
    pub async fn evaluate(
        &self, 
        expr: &str, 
        context: ExpressionContext
    ) -> Result<serde_json::Value, ExpressionError> {
        let ast = self.parse(expr)?;
        self.eval_ast(&ast, &context).await
    }
    
    async fn eval_ast(&self, ast: &Ast, ctx: &ExpressionContext) -> Result<serde_json::Value> {
        match ast {
            Ast::NodeReference { node_id, field_path } => {
                // Получаем данные через контекст
                let node_output = (ctx.get_node_output)(node_id).await?;
                self.extract_field(&node_output, field_path)
            }
            Ast::Variable(name) => {
                (ctx.get_variable)(name).await
                    .ok_or_else(|| ExpressionError::VariableNotFound(name.clone()))
            }
            Ast::BinaryOp { left, op, right } => {
                let left_val = self.eval_ast(left, ctx).await?;
                let right_val = self.eval_ast(right, ctx).await?;
                self.apply_operator(op, &left_val, &right_val)
            }
            // ...
        }
    }
}

// Пример полного flow
let context = ExecutionContext::new(/* ... */);

// Сохраняем результат узла — обычный serde_json::Value
context.node_outputs.write().await.insert(
    NodeId::new("fetch_user"),
    NodeOutput {
        result: json!({
            "email": "user@example.com",
            "age": 25,
        }),
        status: NodeStatus::Completed,
        duration: Duration::from_millis(42),
    }
);

// Вычисляем expression — результат serde_json::Value
let email = context.evaluate_expression("$nodes.fetch_user.result.email").await?;
assert_eq!(email, json!("user@example.com"));
```

---

### 5. nebula-eventbus ↔ nebula-execution ↔ nebula-log

**Event flow:** Execution генерирует события, Log их записывает

```rust
// nebula-execution генерирует события
impl ExecutionContext {
    pub async fn start_node(&self, node_id: NodeId) -> Result<()> {
        // Emit event через eventbus
        self.event_bus.publish(NodeEvent::Started {
            execution_id: self.execution_id.clone(),
            workflow_id: self.workflow_id.clone(),
            node_id: node_id.clone(),
            timestamp: SystemTime::now(),
        }).await?;
        
        // Также логируем
        self.logger.info(&format!("Starting node {}", node_id));
        
        Ok(())
    }
}

// nebula-log подписывается на события
pub struct EventLogger {
    logger: Logger,
}

impl EventLogger {
    pub fn subscribe_to_events(event_bus: &EventBus) {
        // Подписываемся на все execution события
        event_bus.subscribe(|event: ExecutionEvent| async move {
            match event {
                ExecutionEvent::Started { execution_id, workflow_id, .. } => {
                    log::info!(
                        target: "execution",
                        execution_id = %execution_id,
                        workflow_id = %workflow_id,
                        "Execution started"
                    );
                }
                ExecutionEvent::Failed { execution_id, error, .. } => {
                    log::error!(
                        target: "execution", 
                        execution_id = %execution_id,
                        error = %error,
                        "Execution failed"
                    );
                }
                // ...
            }
        });
        
        // Подписываемся на node события
        event_bus.subscribe(|event: NodeEvent| async move {
            match event {
                NodeEvent::Started { node_id, .. } => {
                    log::debug!("Node {} started", node_id);
                }
                NodeEvent::Completed { node_id, duration, .. } => {
                    log::info!("Node {} completed in {:?}", node_id, duration);
                }
                // ...
            }
        });
    }
}

// nebula-metrics тоже слушает события
pub struct MetricsCollector;

impl MetricsCollector {
    pub fn subscribe_to_events(event_bus: &EventBus) {
        event_bus.subscribe(|event: NodeEvent| async move {
            match event {
                NodeEvent::Completed { duration, .. } => {
                    metrics::histogram!("node_duration_seconds", duration.as_secs_f64());
                    metrics::increment_counter!("nodes_completed_total");
                }
                NodeEvent::Failed { .. } => {
                    metrics::increment_counter!("nodes_failed_total");
                }
                // ...
            }
        });
    }
}
```

