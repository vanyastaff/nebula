# Nebula Observability - Пересмотренная Архитектура для Workflow Engine

**Created**: 2025-10-12
**Context**: После анализа Nebula как workflow engine (n8n-подобная система)
**Status**: 🎯 Recommended Architecture

---

## Проблема с Предыдущим Планом

Предыдущий анализ (TECHNICAL_DEBT.md) рассматривал nebula-log как **обычную logging библиотеку**, где observability - это **опциональная фича**.

**Это НЕВЕРНО для workflow engine!**

### Почему Observability - это Core для Workflow Engine:

1. **Execution Tracking** - пользователь должен видеть, где сейчас execution
2. **Node Metrics** - каждый node должен отчитываться о своей работе
3. **Workflow Analytics** - статистика по workflows критична для продукта
4. **Debugging Workflows** - без observability невозможно понять, почему workflow упал
5. **UI Real-time Updates** - frontend должен получать события о прогрессе
6. **Multi-tenant Monitoring** - разные tenants должны видеть свои метрики

### Примеры из n8n, Temporal, Prefect:

**n8n**:
- Event tracking для каждого node execution
- Workflow execution history в UI
- Metrics по успешности/провалам
- Real-time status updates

**Temporal**:
- Built-in observability (не опционально!)
- Metrics, traces, logs для всех workflows
- UI показывает live execution state

**Prefect**:
- Observability как core feature
- Automatic instrumentation всех tasks
- Dashboard с метриками

---

## Правильная Архитектура

### Принцип: Observability - это Core Infrastructure, не Feature

```
┌─────────────────────────────────────────────────────────┐
│                 nebula-log (Core Infrastructure)        │
│                                                         │
│  ┌───────────────┐  ┌───────────────┐  ┌───────────┐  │
│  │   Logging     │  │   Tracing     │  │  Events   │  │
│  │  (tracing)    │  │    (spans)    │  │  (hooks)  │  │
│  └───────────────┘  └───────────────┘  └───────────┘  │
│                                                         │
│  Observability Module (ALWAYS AVAILABLE):              │
│  ┌─────────────────────────────────────────────────┐  │
│  │  • ObservabilityEvent (trait)                   │  │
│  │  • ObservabilityHook (trait)                    │  │
│  │  • Event Registry (thread-safe)                 │  │
│  │  • Common Events (OperationStarted, etc.)       │  │
│  └─────────────────────────────────────────────────┘  │
│                                                         │
│  Metrics Integration (OPTIONAL - feature gated):       │
│  ┌─────────────────────────────────────────────────┐  │
│  │  • MetricsHook (records to metrics crate)       │  │
│  │  • Helpers (counter!, gauge!, histogram!)       │  │
│  │  • Re-exports from `metrics` crate              │  │
│  └─────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### Module Structure

```rust
// crates/nebula-log/src/lib.rs

// ALWAYS PUBLIC - Core Infrastructure
pub mod observability {
    pub mod hooks;      // ObservabilityEvent, ObservabilityHook traits
    pub mod events;     // OperationStarted, OperationCompleted, etc.
    pub mod registry;   // register_hook(), emit_event()

    // Feature-gated built-in hooks
    #[cfg(feature = "metrics-integration")]
    pub use hooks::MetricsHook;
}

// FEATURE GATED - Optional Metrics Backend
#[cfg(feature = "metrics-integration")]
pub mod metrics {
    pub use metrics::{counter, gauge, histogram};  // re-export
    pub mod helpers;  // convenience wrappers
}

// Prelude
pub mod prelude {
    // Core logging - always available
    pub use crate::{info, debug, error, warn, trace};

    // Core observability - always available
    pub use crate::observability::{
        ObservabilityEvent, ObservabilityHook,
        emit_event, register_hook,
        OperationStarted, OperationCompleted, OperationFailed,
    };

    // Metrics - only with feature
    #[cfg(feature = "metrics-integration")]
    pub use crate::metrics::{counter, gauge, histogram};

    #[cfg(feature = "metrics-integration")]
    pub use crate::observability::MetricsHook;
}
```

---

## Feature Flags Strategy

### Recommended Flags:

```toml
[features]
default = ["ansi", "async", "observability"]

# Core features
ansi = []
async = ["tokio"]

# Observability is ALWAYS included in default
# (users can opt-out with default-features = false)
observability = []  # Just a marker, module is always compiled

# Optional backends
metrics-integration = ["metrics"]
telemetry = ["opentelemetry", "opentelemetry_sdk", "opentelemetry-otlp"]
sentry = ["dep:sentry", "sentry-tracing"]

# File output
file = ["tracing-appender", "flate2"]

# Compatibility
log-compat = ["tracing-log"]

# Everything
full = [
    "ansi", "async", "observability",
    "metrics-integration", "telemetry", "sentry",
    "file", "log-compat"
]
```

### Rationale:

1. **`observability` всегда в default** - это core для workflow engine
2. **`metrics-integration`** - опциональный backend (нужен для Prometheus/OTEL)
3. Пользователи могут opt-out: `nebula-log = { default-features = false }`
4. Четкое разделение: abstractions (core) vs implementations (optional)

---

## Why This Architecture?

### ✅ Advantages:

1. **Core Abstractions Always Available**
   - Любой nebula crate может emit events без зависимости от `metrics`
   - Workflow engine может track execution без external dependencies
   - Hooks можно зарегистрировать независимо от backend

2. **Flexible Backend Integration**
   - Хочешь Prometheus? Включи `metrics-integration`
   - Хочешь только events в UI? Используй core observability
   - Можешь написать custom hook без зависимостей

3. **Zero-Cost for Minimal Users**
   - `default-features = false` → нет observability overhead
   - Но для workflow engine это не нужно - observability критична

4. **Clear Dependencies**
   ```
   nebula-log (core)
   ├── observability (always)  ← NO DEPS
   └── metrics (optional)      ← depends on `metrics` crate
   ```

5. **Production-Ready**
   - Следует паттернам Temporal, Prefect, n8n
   - Observability как first-class citizen
   - Extensible через hooks

---

## How Other Crates Use It

### nebula-engine (Workflow Execution)

```rust
// Cargo.toml
[dependencies]
nebula-log = { path = "../nebula-log" }  # Gets observability by default

# Optional: enable metrics backend
[features]
metrics = ["nebula-log/metrics-integration"]
```

```rust
// src/executor.rs
use nebula_log::prelude::*;

pub struct WorkflowExecutor;

impl WorkflowExecutor {
    pub async fn execute(&self, workflow_id: WorkflowId) -> Result<()> {
        // Core observability - always works, no feature flags needed
        let tracker = OperationTracker::new(
            format!("workflow.{}", workflow_id),
            "execution"
        );

        // ... execute workflow ...

        tracker.success();
        Ok(())
    }
}
```

### nebula-action (Node Execution)

```rust
use nebula_log::observability::{emit_event, ObservabilityEvent};

pub struct ActionExecutor;

// Custom domain event
struct ActionExecutionEvent {
    action_id: String,
    duration_ms: u64,
    success: bool,
}

impl ObservabilityEvent for ActionExecutionEvent {
    fn name(&self) -> &str { "action.execution" }
    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "action_id": self.action_id,
            "duration_ms": self.duration_ms,
            "success": self.success,
        }))
    }
}

impl ActionExecutor {
    pub fn execute(&self, action: &Action) {
        let start = Instant::now();
        let result = self.run_action(action);

        // Emit domain event - always available!
        emit_event(&ActionExecutionEvent {
            action_id: action.id.clone(),
            duration_ms: start.elapsed().as_millis() as u64,
            success: result.is_ok(),
        });
    }
}
```

### Application Setup (with Metrics)

```rust
// main.rs
use nebula_log::prelude::*;

#[cfg(feature = "metrics-integration")]
fn setup_metrics() {
    // Register MetricsHook to send events to Prometheus
    register_hook(Arc::new(MetricsHook::new()));

    // Setup Prometheus exporter
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .install()
        .expect("failed to install Prometheus recorder");
}

fn main() {
    // Initialize logging
    nebula_log::auto_init().expect("failed to init logger");

    // Register observability hooks
    register_hook(Arc::new(LoggingHook::new(Level::INFO)));

    #[cfg(feature = "metrics-integration")]
    setup_metrics();

    // Custom hook for UI updates
    register_hook(Arc::new(WebSocketHook::new()));

    // Start application
    run_workflow_engine().await;
}
```

---

## What About TECHNICAL_DEBT.md?

### Items to Revise:

#### ❌ REMOVE: Section 1.1 "Module Organization & Visibility"
**Reason**: Observability SHOULD be always public - это не bug, это feature!

**Decision**: Keep `pub mod observability` without feature gate

---

#### ✅ KEEP: Section 1.2 "Builder Complexity"
**Reason**: Это реальная проблема - 537 строк с дублированием

**Priority**: Medium (не критично, но улучшит maintainability)

---

#### ✅ KEEP: Section 1.3 "Config Module Size"
**Reason**: 334 строки - тоже реальная проблема

**Priority**: Medium

---

#### ✅ KEEP: Section 2.2 "Test Coverage Gaps"
**Reason**: Тесты важны

**Priority**: High (но не блокер)

---

#### ❌ REVISE: Section 3.1 "Feature Flag Dependencies"
**Old**: Criticized `telemetry = [..., "observability"]`
**New**: This is CORRECT! Telemetry should include observability

**Revised Recommendation**:
```toml
# CORRECT approach:
observability = []  # Core abstractions
metrics-integration = ["metrics", "observability"]
telemetry = ["opentelemetry", "...", "metrics-integration"]
```

---

#### ✅ KEEP: Section 3.3 "Error Handling Inconsistency"
**Reason**: Hooks должны быть panic-safe

**Priority**: High for production workflow engine

---

#### ⚠️ REVISE: Section 4.1 "Allocation in Hot Paths"
**Old**: Called it "premature optimization"
**New**: For workflow engine with 1000s of events/sec, this matters!

**Priority**: Medium → High (profile first, then optimize)

---

## Updated Sprint Plan

### Sprint 1: Fix Real Issues (Not Visibility) ⚡ HIGH
**Duration**: 2 days

**Tasks**:
- [ ] Add panic safety to hooks (catch_unwind)
- [ ] Document observability as core feature (update README)
- [ ] Add error hook for debugging
- [ ] Verify feature flags are correct (observability in default)

**Rationale**: For production workflow engine, hooks MUST be robust

---

### Sprint 2: Improve Testing 🧪 HIGH
**Duration**: 2-3 days

**Tasks**:
- [ ] Integration tests (observability + metrics)
- [ ] Concurrent hook registration tests
- [ ] Memory leak tests
- [ ] Performance benchmarks (event emission rate)

**Rationale**: Workflow engine needs confidence in observability

---

### Sprint 3: Builder Refactoring 🔧 MEDIUM
**Duration**: 3-4 days

**Tasks**:
- [ ] Extract format layer builders
- [ ] Reduce duplication
- [ ] Simplify builder.rs (537 → 300 lines)

**Rationale**: Nice to have, but not blocking

---

### Sprint 4: Config Restructuring 📁 MEDIUM
**Duration**: 2 days

**Tasks**:
- [ ] Split config.rs into submodules
- [ ] Add Config builder pattern

**Rationale**: Improves maintainability

---

### Sprint 5: Performance & Polish ⚡ MEDIUM
**Duration**: 2-3 days

**Tasks**:
- [ ] Profile hot paths (event emission)
- [ ] Optimize allocations in MetricsHook
- [ ] Consider lock-free registry if needed
- [ ] Complete documentation

**Rationale**: Workflow engine needs good performance

---

## Comparison with Other Systems

### Temporal (Go)

```go
// Observability is CORE - not optional
import "go.temporal.io/sdk/workflow"

func MyWorkflow(ctx workflow.Context) error {
    // Metrics are automatic - built into SDK
    workflow.GetMetricsHandler(ctx).Counter("my_counter").Inc(1)
    return nil
}
```

### n8n (TypeScript)

```typescript
// EventBus is CORE infrastructure
import { EventBus } from 'n8n-core';

class NodeExecutor {
  execute(node: INode) {
    // Events always emitted - not optional
    this.eventBus.emit('node.execution.started', {
      nodeId: node.id,
      timestamp: Date.now()
    });
  }
}
```

### Prefect (Python)

```python
# Observability built-in - not optional
from prefect import flow, task

@task  # Automatically instrumented
def my_task():
    pass  # Metrics, logs, traces collected automatically

@flow  # Same here
def my_flow():
    my_task()
```

**Nebula должна следовать этому паттерну!**

---

## Recommendation

### ✅ DO:

1. **Keep observability module always public** - это core infrastructure
2. **Feature-gate only MetricsHook** - это optional backend
3. **Include observability in default features** - критично для workflow engine
4. **Focus on robustness first** - panic safety, error handling
5. **Profile before optimizing** - measure hot paths
6. **Document clearly** - observability - это feature, не debt

### ❌ DON'T:

1. **Don't feature-gate core abstractions** - hooks/events нужны всегда
2. **Don't treat observability as optional** - для workflow engine это core
3. **Don't over-optimize prematurely** - но будь готов к оптимизации
4. **Don't focus on builder refactoring first** - это косметика

---

## Final Decision Matrix

| Issue | Old Priority | New Priority | Reason |
|-------|-------------|--------------|---------|
| Module visibility | 🔴 High | ⚪ Not an issue | Observability SHOULD be public |
| Panic safety | 🟡 Medium | 🔴 High | Critical for production |
| Test coverage | 🟡 Medium | 🔴 High | Need confidence |
| Performance | 🟢 Low | 🟡 Medium | Workflow engine needs it |
| Builder complexity | 🟡 Medium | 🟢 Low | Nice to have |
| Config size | 🟡 Medium | 🟢 Low | Nice to have |
| Documentation | 🟢 Low | 🟡 Medium | Users need clarity |

---

## Next Steps

1. **Update TECHNICAL_DEBT.md** - remove incorrect section 1.1
2. **Update GitHub Issue #40** - change scope to panic safety instead
3. **Re-prioritize sprints** - robustness & testing first
4. **Document decision** - why observability is always-on
5. **Start Sprint 1 (Revised)** - panic safety & error handling

---

**Status**: ✅ Ready for Implementation
**Confidence Level**: 🎯 High - based on workflow engine requirements and industry patterns

---

## Trait-Based Extensibility Pattern

### Core Concept: Implement Traits from nebula-log in Domain Crates

nebula-log предоставляет **traits**, а domain crates (nebula-action, nebula-workflow, etc.) их **реализуют** для своих типов.

### Example 1: Domain Events Implement ObservabilityEvent

```rust
// ========================================
// nebula-log (defines the trait)
// ========================================
pub trait ObservabilityEvent: Send + Sync {
    fn name(&self) -> &str;
    fn timestamp(&self) -> SystemTime { SystemTime::now() }
    fn data(&self) -> Option<serde_json::Value> { None }
}

// ========================================
// nebula-workflow (implements for domain types)
// ========================================
use nebula_log::observability::ObservabilityEvent;

pub struct WorkflowStartedEvent {
    pub workflow_id: WorkflowId,
    pub execution_id: ExecutionId,
    pub tenant_id: TenantId,
}

impl ObservabilityEvent for WorkflowStartedEvent {
    fn name(&self) -> &str { "workflow.started" }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "workflow_id": self.workflow_id.to_string(),
            "execution_id": self.execution_id.to_string(),
            "tenant_id": self.tenant_id.to_string(),
        }))
    }
}

// Usage in workflow executor
use nebula_log::observability::emit_event;

impl WorkflowExecutor {
    pub async fn execute(&self, workflow: Workflow) -> Result<()> {
        emit_event(&WorkflowStartedEvent {
            workflow_id: workflow.id.clone(),
            execution_id: self.execution_id,
            tenant_id: self.tenant_id,
        });

        // ... execute workflow ...
        Ok(())
    }
}
```

### Example 2: Custom Hooks for WebSocket/Database

```rust
// ========================================
// nebula-ui (WebSocket broadcast hook)
// ========================================
use nebula_log::observability::{ObservabilityHook, ObservabilityEvent};

pub struct WebSocketBroadcastHook {
    tx: broadcast::Sender<EventMessage>,
}

impl ObservabilityHook for WebSocketBroadcastHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        let message = EventMessage {
            name: event.name().to_string(),
            data: event.data(),
        };
        let _ = self.tx.send(message);
    }
}

// ========================================
// nebula-storage (Database audit hook)
// ========================================
pub struct DatabaseAuditHook {
    pool: PgPool,
}

impl ObservabilityHook for DatabaseAuditHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // Store events in audit_log table
        tokio::spawn(async move {
            sqlx::query!(
                "INSERT INTO audit_log (event_name, data) VALUES ($1, $2)",
                event.name(),
                event.data()
            ).execute(&pool).await;
        });
    }
}
```

### Benefits of Trait-Based Design

1. **Loose Coupling** - nebula-log не зависит от domain crates
2. **Domain-Specific Events** - каждый crate определяет свои события
3. **Custom Hooks** - пользователи могут добавлять hooks без изменения nebula-log
4. **Zero Dependencies** - domain crates нужен только trait
5. **Easy Testing** - mock hooks для тестирования

### Event Naming Convention

```
{crate}.{resource}.{action}

Examples:
- workflow.execution.started
- workflow.execution.completed
- action.http_request.started
- memory.allocation.succeeded
- resilience.circuit_breaker.opened
```

