# Archived From "docs/archive/overview.md"

# Nebula Architecture Documentation

## Обзор системы

Nebula - высокопроизводительный workflow engine на Rust, состоящий из 30 модульных крейтов, организованных в четкие архитектурные слои.

---

### Архитектурные принципы

1. **Типовая безопасность** - максимальное использование системы типов Rust
2. **Модульность** - четкое разделение ответственностей между компонентами
3. **Гибкость разработки** - поддержка простого кода и derive макросов
4. **Atomic Actions** - фокус на переиспользуемые блоки
5. **Smart Resource Management** - различные lifecycle scopes
6. **Expression-driven** - мощная система выражений для динамической логики
7. **Event-Driven** - loose coupling через eventbus
8. **Security Isolation** - sandbox с capability-based доступом для выполнения Actions

---

### Слои архитектуры

```
┌─────────────────────────────────────────────────────────┐
│                 Presentation Layer                      │
│       (nebula-ui, nebula-api, nebula-cli, nebula-hub)   │
├─────────────────────────────────────────────────────────┤
│                 Developer Tools Layer                   │
│       (nebula-sdk, nebula-derive, nebula-testing)       │
├─────────────────────────────────────────────────────────┤
│            Multi-Tenancy & Clustering Layer             │
│            (nebula-cluster, nebula-tenant)              │
├─────────────────────────────────────────────────────────┤
│                 Business Logic Layer                    │
│         (nebula-resource, nebula-registry)              │
├─────────────────────────────────────────────────────────┤
│                   Execution Layer                       │
│  (nebula-engine, nebula-runtime, nebula-worker,         │
│              nebula-sandbox)                             │
├─────────────────────────────────────────────────────────┤
│                     Node Layer                          │
│  (nebula-node, nebula-action, nebula-parameter,         │
│              nebula-credential)                         │
├─────────────────────────────────────────────────────────┤
│                     Core Layer                          │
│  (nebula-core, nebula-workflow, nebula-execution,       │
│   nebula-memory, nebula-expression,                     │
│   nebula-eventbus, nebula-idempotency)                  │
├─────────────────────────────────────────────────────────┤
│              Cross-Cutting Concerns Layer               │
│  (nebula-config, nebula-log, nebula-metrics,            │
│    nebula-resilience, nebula-system,                    │
│   nebula-validator, nebula-locale)                      │
├─────────────────────────────────────────────────────────┤
│                Infrastructure Layer                     │
│         (nebula-storage, nebula-binary)                 │
└─────────────────────────────────────────────────────────┘
```

---

## Core Layer

---

### nebula-core
**Назначение:** Базовые типы и трейты, используемые всеми крейтами системы. Предотвращает циклические зависимости.

**Ключевые компоненты:**
- Базовые идентификаторы (ExecutionId, WorkflowId, NodeId)
- Концепция Scope для resource management
- Общие трейты для loose coupling

```rust
// Основные типы
pub struct ExecutionId(Uuid);
pub struct WorkflowId(String);
pub struct NodeId(String);
pub struct UserId(String);
pub struct TenantId(String);

// Универсальный Scope
pub enum ScopeLevel {
    Global,
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(ExecutionId, NodeId),
}

// Базовые трейты
pub trait Scoped {
    fn scope(&self) -> &ScopeLevel;
}

pub trait HasContext {
    fn execution_id(&self) -> Option<&ExecutionId>;
    fn workflow_id(&self) -> Option<&WorkflowId>;
    fn tenant_id(&self) -> Option<&TenantId>;
}

// Параметрическое значение — разделяет expressions от литеральных данных.
// Используется в WorkflowDefinition и ParameterCollection.
// После resolve на уровне Execution всё превращается в serde_json::Value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    /// Expression: "$nodes.user_lookup.result.email"
    Expression(Expression),
    /// Template: "Order #{order_id} for {name}"
    Template(TemplateString),
    /// Обычное JSON-значение
    Literal(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expression {
    pub raw: String,  // "$nodes.user_lookup.result.email"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateString {
    pub template: String,
    pub bindings: Vec<String>,
}
```

---

