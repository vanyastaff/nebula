#### 6. nebula-engine (Week 4-5)
- [ ] 6.1 **Core Engine**
  - [ ] 6.1.1 Create WorkflowEngine struct
  - [ ] 6.1.2 Implement event loop
  - [ ] 6.1.3 Add state management
  - [ ] 6.1.4 Add execution context
  - [ ] 6.1.5 Implement scheduling logic
  - [ ] 6.1.6 Add graceful shutdown

- [ ] 6.2 **DAG Processing**
  - [ ] 6.2.1 Implement topological sort
  - [ ] 6.2.2 Add cycle detection
  - [ ] 6.2.3 Implement parallel execution
  - [ ] 6.2.4 Add conditional branching
  - [ ] 6.2.5 Add loop support
  - [ ] 6.2.6 Add subworkflow support

- [ ] 6.3 **Event System**
  - [ ] 6.3.1 Define WorkflowEvent types
  - [ ] 6.3.2 Implement event bus abstraction
  - [ ] 6.3.3 Add Kafka integration
  - [ ] 6.3.4 Add event persistence
  - [ ] 6.3.5 Add event replay
  - [ ] 6.3.6 Add dead letter queue

- [ ] 6.4 **Execution Control**
  - [ ] 6.4.1 Implement pause/resume
  - [ ] 6.4.2 Add cancellation
  - [ ] 6.4.3 Add timeout handling
  - [ ] 6.4.4 Add retry logic
  - [ ] 6.4.5 Add error propagation
  - [ ] 6.4.6 Add compensation logic


---

## nebula-engine

### Purpose
Движок выполнения workflows, управляющий жизненным циклом executions.

### Responsibilities
- Orchestration workflows
- Event processing
- State management
- Scheduling

### Architecture
```rust
pub struct WorkflowEngine {
    event_bus: Arc<dyn EventBus>,
    state_store: Arc<dyn StateStore>,
    scheduler: Arc<Scheduler>,
    executor: Arc<Executor>,
}
```

### Event Flow
```
Trigger → Event → Engine → Scheduler → Worker
                    ↓
                State Store
```

---
