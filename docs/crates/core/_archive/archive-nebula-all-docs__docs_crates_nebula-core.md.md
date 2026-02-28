# Archived From "docs/archive/nebula-all-docs.md"

## FILE: docs/crates/nebula-core.md
---

# nebula-core

## Назначение

`nebula-core` - это фундаментальный crate, содержащий базовые trait'ы, типы и абстракции, используемые во всей системе Nebula.

## Ответственность

- Определение основных trait'ов (Action, TriggerAction, etc.)
- Базовые типы данных (WorkflowId, NodeId, ExecutionId)
- Error types
- Common utilities

## Архитектура

### Основные компоненты

```rust
// Traits
pub trait Action { }
pub trait TriggerAction: Action { }
pub trait PollingAction: TriggerAction { }
pub trait SupplyAction: Action { }

// Types
pub struct Workflow { }
pub struct Node { }
pub struct Execution { }

// Identifiers  
pub struct WorkflowId(Uuid);
pub struct NodeId(String);
pub struct ExecutionId(Uuid);
```

### Зависимости

- Минимальные внешние зависимости
- Только стабильные, широко используемые crates

## Roadmap

### Milestone 1: Basic Types (Week 1)
- [x] Проектирование типов
- [ ] WorkflowId, NodeId, ExecutionId
- [ ] Error types
- [ ] Basic traits

### Milestone 2: Action System (Week 1-2)
- [ ] Action trait
- [ ] TriggerAction trait
- [ ] Metadata types
- [ ] Tests

### Milestone 3: Workflow Types (Week 2)
- [ ] Workflow struct
- [ ] Node struct
- [ ] Connection types
- [ ] Validation

### Milestone 4: Documentation (Week 2-3)
- [ ] API documentation
- [ ] Examples
- [ ] Integration guide

## API Design

```rust
// Example usage
use nebula_core::prelude::*;

#[derive(Debug)]
pub struct MyAction;

impl Action for MyAction {
    type Input = String;
    type Output = String;
    
    async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<Self::Output> {
        Ok(input.to_uppercase())
    }
}
```

## Testing Strategy

- Unit tests для каждого компонента
- Property-based testing для ID types
- Doc tests для всех примеров

## Performance Considerations

- Zero-cost abstractions
- No allocations в hot paths
- Efficient serialization

---

