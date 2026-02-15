# nebula-resource: Long-Term Roadmap

> **Статус:** Living document
> **Создан:** 2026-02-15
> **Горизонт:** 12-18 месяцев
> **Принцип:** Лучше меньше, но качественно. Каждая фаза — завершённый, production-ready результат.

---

## 1. Видение финального продукта

`nebula-resource` — фреймворк управления жизненным циклом внешних ресурсов (БД, HTTP-клиенты, очереди, кэши) для workflow-движка Nebula.

### Что это НЕ является

- **Не коллекция драйверов.** Крейт определяет трейты и менеджер, а не реализации для PostgreSQL/Redis/Kafka. Драйверы живут в отдельных крейтах (`nebula-resource-postgres`, `nebula-resource-redis`, и т.д.) — по аналогии с `nebula-sandbox-inprocess` и `nebula-queue-memory`.
- **Не ORM.** Ресурс — это управляемое соединение, а не слой абстракции над данными.
- **Не замена OpenTelemetry.** Трейсинг и метрики интегрируются через экосистему `tracing`/`metrics`, а не через собственную реализацию.

### Что это является

```
┌─────────────────────────────────────────────────────────┐
│                    nebula-engine                         │
│  Владеет ResourceManager, создаёт ResourceProvider      │
└──────────────────────┬──────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│                   nebula-runtime                         │
│  Передаёт ResourceProvider в ActionContext               │
└──────────────────────┬──────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│                   nebula-action                          │
│  trait ResourceProvider: Send + Sync                     │
│    async fn acquire<T>(&self, key) -> Result<T>         │
│    async fn release<T>(&self, key, instance) -> Result  │
│                                                         │
│  ActionContext::resource::<T>("main_db").await?          │
└─────────────────────────────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│                  nebula-resource                         │
│                                                         │
│  ┌─────────────┐ ┌──────────────┐ ┌──────────────────┐ │
│  │  Resource    │ │ ResourcePool │ │  HealthChecker   │ │
│  │  trait       │ │ <T>          │ │  (background)    │ │
│  └──────┬──────┘ └──────┬───────┘ └────────┬─────────┘ │
│         │               │                  │           │
│  ┌──────▼───────────────▼──────────────────▼─────────┐ │
│  │              ResourceManager                       │ │
│  │  - Registry (TypeId → Factory)                     │ │
│  │  - Pools (TypeId → Pool<T>)                        │ │
│  │  - Dependencies (DependencyGraph)                  │ │
│  │  - Scoping (ResourceScope + access control)        │ │
│  │  - Lifecycle (state machine per instance)          │ │
│  └───────────────────────────────────────────────────┘ │
│                                                         │
│  ┌───────────┐ ┌──────────┐ ┌─────────┐ ┌───────────┐ │
│  │ Events    │ │ Hooks    │ │ Metrics │ │ Quarantine│ │
│  │ (Phase 3) │ │ (Phs. 5) │ │ (Phs. 3)│ │ (Phase 8) │ │
│  └───────────┘ └──────────┘ └─────────┘ └───────────┘ │
└─────────────────────────────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│              Driver Crates (Phase 6)                     │
│                                                         │
│  nebula-resource-http      (reqwest)                    │
│  nebula-resource-postgres  (sqlx)                       │
│  nebula-resource-redis     (redis-rs)                   │
│  nebula-resource-kafka     (rdkafka)                    │
│  nebula-resource-mongodb   (mongodb)                    │
└─────────────────────────────────────────────────────────┘
```

---

## 2. Финальная архитектура (target state)

### 2.1 Структура файлов

```
crates/resource/
├── Cargo.toml
├── ROADMAP.md
├── docs/
│   ├── Architecture.md          # Архитектура (этот уровень детализации)
│   ├── ResourceTrait.md         # Как реализовать Resource
│   ├── Pooling.md               # Стратегии пулинга
│   ├── HealthChecks.md          # Система health-проверок
│   └── Integration.md           # Интеграция с action/engine
├── src/
│   ├── lib.rs                   # Публичный API + prelude
│   │
│   ├── resource.rs              # Resource trait, ResourceId
│   ├── config.rs                # ResourceConfig trait
│   ├── context.rs               # ResourceContext (плоский)
│   ├── error.rs                 # ResourceError
│   ├── scope.rs                 # ResourceScope с parent chain
│   ├── lifecycle.rs             # LifecycleState машина
│   │
│   ├── manager/
│   │   ├── mod.rs               # ResourceManager — центральный координатор
│   │   ├── registry.rs          # TypeId → Factory mapping
│   │   └── dependency.rs        # DependencyGraph (string keys)
│   │
│   ├── pool/
│   │   ├── mod.rs               # Pool<T> (generic, async)
│   │   ├── strategy.rs          # FIFO, LIFO, LRU
│   │   ├── entry.rs             # PoolEntry с метаданными
│   │   └── config.rs            # PoolConfig
│   │
│   ├── health/
│   │   ├── mod.rs               # HealthChecker (фоновый мониторинг)
│   │   ├── status.rs            # HealthStatus, HealthState
│   │   ├── pipeline.rs          # Multi-stage health checks [Phase 4]
│   │   └── degraded.rs          # Degraded state handling [Phase 4]
│   │
│   ├── events/
│   │   ├── mod.rs               # EventBus (broadcast channel) [Phase 3]
│   │   └── types.rs             # ResourceEvent enum
│   │
│   ├── hooks/
│   │   ├── mod.rs               # HookRegistry [Phase 5]
│   │   └── builtin.rs           # Audit, metrics, credential refresh
│   │
│   ├── metrics/
│   │   ├── mod.rs               # MetricsCollector [Phase 3]
│   │   └── pool.rs              # Pool-specific метрики
│   │
│   ├── quarantine/
│   │   ├── mod.rs               # QuarantineManager [Phase 8]
│   │   └── recovery.rs          # Стратегии восстановления
│   │
│   └── testing/
│       ├── mod.rs               # MockResource, TestPool
│       └── fixtures.rs          # Готовые фикстуры для тестов
│
├── tests/
│   ├── lifecycle_property.rs    # Property tests (proptest)
│   ├── scope_isolation.rs       # Security тесты
│   ├── pool_integration.rs      # Pool интеграционные
│   ├── manager_concurrent.rs    # Тесты конкурентности
│   └── serde_roundtrip.rs       # Serde property тесты
│
├── benches/
│   └── pool_throughput.rs       # Criterion бенчмарки
│
└── examples/
    ├── basic_resource.rs        # Простой ресурс
    └── pooled_resource.rs       # Ресурс с пулингом
```

### 2.2 Ecosystem crates (отдельные крейты)

```
crates/
├── resource/                    # Фреймворк (этот крейт)
├── resource-derive/             # Процедурные макросы [Phase 7]
├── resource-http/               # HTTP клиент [Phase 6]
├── resource-postgres/           # PostgreSQL [Phase 6]
├── resource-redis/              # Redis [Phase 6]
├── resource-kafka/              # Kafka [Phase 6]
└── resource-mongodb/            # MongoDB [Phase 6]
```

### 2.3 Ключевые трейты (финальное состояние)

```rust
/// Основной трейт ресурса.
/// По аналогии с bb8::ManageConnection — минимальный контракт.
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    /// Тип конфигурации (десериализуемый из JSON/YAML).
    type Config: ResourceConfig;

    /// Тип экземпляра (то, что получает action).
    type Instance: Send + Sync + 'static;

    /// Уникальный идентификатор типа ресурса.
    fn id(&self) -> &str;

    /// Создать новый экземпляр ресурса.
    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError>;

    /// Проверить, что экземпляр всё ещё рабочий.
    /// По умолчанию — всегда Ok(true).
    async fn is_valid(&self, instance: &Self::Instance) -> Result<bool, ResourceError> {
        let _ = instance;
        Ok(true)
    }

    /// Подготовить экземпляр к переиспользованию (после возврата в пул).
    /// По умолчанию — no-op.
    async fn recycle(&self, instance: &mut Self::Instance) -> Result<(), ResourceError> {
        let _ = instance;
        Ok(())
    }

    /// Очистить экземпляр при удалении из пула / shutdown.
    async fn cleanup(&self, instance: Self::Instance) -> Result<(), ResourceError> {
        drop(instance);
        Ok(())
    }

    /// Зависимости — какие ресурсы должны быть инициализированы до этого.
    fn dependencies(&self) -> Vec<&str> {
        Vec::new()
    }
}

/// Конфигурация ресурса. Каждый ресурс определяет свой тип конфига.
pub trait ResourceConfig: Send + Sync + serde::de::DeserializeOwned + 'static {
    /// Валидация конфигурации. Вызывается перед create().
    fn validate(&self) -> Result<(), ResourceError>;
}

/// Расширение для ресурсов с health check (opt-in).
/// Не навязывается — ресурс может не реализовывать.
#[async_trait]
pub trait HealthCheckable: Send + Sync {
    /// Проверка здоровья. Возвращает детализированный статус.
    async fn health_check(&self) -> HealthStatus;

    /// Рекомендуемый интервал проверки.
    fn check_interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    /// Таймаут на одну проверку.
    fn check_timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
}
```

### 2.4 ResourceContext (финальное состояние — плоский)

```rust
/// Контекст выполнения для операций с ресурсами.
/// Плоская структура — без вложенных объектов.
pub struct ResourceContext {
    /// Идентификатор scope'а запроса.
    pub scope: ResourceScope,

    /// ID текущего выполнения workflow.
    pub execution_id: String,

    /// ID workflow-определения.
    pub workflow_id: String,

    /// ID tenant'а (для multi-tenancy изоляции).
    pub tenant_id: Option<String>,

    /// Токен отмены (CancellationToken из tokio_util).
    pub cancellation: CancellationToken,

    /// Произвольные метаданные (string → string).
    pub metadata: HashMap<String, String>,
}
```

### 2.5 ResourceScope (с parent chain для безопасной изоляции)

```rust
/// Уровень видимости ресурса с полной цепочкой родителей.
/// Это позволяет проверять contains() корректно.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceScope {
    /// Доступен всем.
    Global,

    /// Принадлежит конкретному tenant'у.
    Tenant {
        tenant_id: String,
    },

    /// Принадлежит конкретному workflow внутри tenant'а.
    Workflow {
        workflow_id: String,
        tenant_id: Option<String>,
    },

    /// Принадлежит конкретному выполнению.
    Execution {
        execution_id: String,
        workflow_id: Option<String>,
        tenant_id: Option<String>,
    },

    /// Принадлежит конкретному действию.
    Action {
        action_id: String,
        execution_id: Option<String>,
        workflow_id: Option<String>,
        tenant_id: Option<String>,
    },
}
```

### 2.6 Интеграция с action (порт)

```rust
// В crates/action/src/provider.rs — рядом с CredentialProvider

/// Порт для получения ресурсов из action'ов.
/// Runtime предоставляет реализацию, backed by ResourceManager.
#[async_trait]
pub trait ResourceProvider: Send + Sync {
    /// Получить типизированный ресурс по ключу.
    async fn acquire<T: Send + Sync + 'static>(
        &self,
        key: &str,
    ) -> Result<ResourceHandle<T>, ActionError>;
}

/// RAII-guard: ресурс автоматически возвращается в пул при drop.
pub struct ResourceHandle<T> {
    instance: Option<T>,
    release_tx: Option<oneshot::Sender<T>>,
}

impl<T> Deref for ResourceHandle<T> { ... }
impl<T> Drop for ResourceHandle<T> { ... } // Возвращает в пул
```

---

## 3. Roadmap по фазам

### Соглашения

- **Каждая фаза — законченный результат.** После каждой фазы крейт можно использовать.
- **Exit criteria** — что должно быть выполнено, чтобы считать фазу завершённой.
- **Quality gate** — автоматические проверки (CI), которые должны проходить.

---

### Phase 0: Foundation Reset

> **Цель:** Убрать мёртвый код, исправить security-дыры, сделать крейт компилируемым и тестируемым.
> **Длительность:** 2-4 недели
> **Результат:** Чистая кодовая база ~2,500-3,000 LOC вместо ~8,500

#### 0.1 Удалить мёртвый код

| Удалить | Причина |
|---------|---------|
| `src/resources/` (все 16 файлов) | Stub-ы без логики. Драйверы будут в отдельных крейтах |
| `src/context/` модуль (3 файла, ~1,100 LOC) | Дублирует `core/context.rs`, использует необъявленный `rand` |
| `core/traits/resource.rs`, `instance.rs`, `cloneable.rs` | Мёртвый код, дублирует `core/resource.rs` |
| Трейты: `Observable`, `GracefulShutdown`, `Metrics`, `Resettable`, `Configurable` | 0 реализаций |
| `Stateful` трейт + `src/stateful/` | Преждевременная абстракция |
| `src/observability/` модуль | No-op без feature, дублирует `tracing` экосистему |
| Все driver-зависимости из Cargo.toml | sqlx, mongodb, rdkafka, reqwest — не нужны framework'у |
| 19 aspirational docs из `docs/` | Описывают несуществующий продукт |

#### 0.2 Исправить security

| # | Проблема | Исправление |
|---|----------|-------------|
| S1 | `ResourceScope::contains()` всегда `true` для cross-level | Добавить parent chain в scope variants, проверять принадлежность |
| S2 | `validate_scope_access()` пропускает cross-tenant | Реализовать полную валидацию через parent chain |
| S3 | Config structs с `#[derive(Debug)]` утекают URL с паролями | Custom Debug impl с `[REDACTED]` для credential-полей |

#### 0.3 Исправить баги

| # | Баг | Исправление |
|---|-----|-------------|
| B1 | `PooledResource::Drop` — пустой, ресурсы утекают | Реализовать async cleanup через channel (как CleanupMessage) |
| B2 | `find_resource_id_for_type` — string matching по type_name | `HashMap<TypeId, String>` при `register()` |
| B3 | `ResourceFactory` всегда передаёт пустой JSON | Передавать реальный конфиг через register() |
| B4 | `rand` не объявлен в Cargo.toml | Удаляется вместе с `context/` модулем |

#### 0.4 Убрать лишние зависимости

- Удалить `nebula-resilience` как обязательную зависимость (resilience — забота вызывающего кода)
- Удалить все driver-specific features (postgres, mysql, mongodb, redis, kafka, http-client)
- Оставить: `nebula-log`, `tokio`, `async-trait`, `serde`, `thiserror`, `dashmap`, `parking_lot`, `uuid`
- `nebula-credential` остаётся как optional feature

#### 0.5 Добавить P0-тесты

- Property test: lifecycle state machine (все пары состояний)
- Unit tests: `validate_scope_access()` — каждая комбинация scope'ов
- Unit test: pool exhaustion (max_size+1)
- Serde roundtrip для всех сериализуемых типов
- `ResourceGuard::Drop` callback verification

#### Exit criteria
- `cargo check --workspace` — OK
- `cargo test -p nebula-resource` — все проходят
- `cargo clippy -p nebula-resource -- -D warnings` — OK
- LOC < 3,500 (src/ без тестов)
- 0 security findings severity >= High
- `PooledResource::Drop` корректно возвращает ресурс

---

### Phase 1: Core Framework

> **Цель:** Рабочий resource framework, интегрированный с action pipeline.
> **Длительность:** 4-6 недель
> **Результат:** Actions могут объявлять и использовать ресурсы в workflow'ах.

#### 1.1 Переделать Resource trait

Текущий трейт слишком сложный (2 associated types + 7 методов + отдельные ResourceConfig и ResourceInstance). Новый дизайн — по модели bb8::ManageConnection:

- `Resource` — create, is_valid, recycle, cleanup, dependencies (см. секцию 2.3)
- `ResourceConfig` — validate + DeserializeOwned
- Убрать `ResourceInstance` как отдельный трейт
- Убрать `ResourceMetadata` как обязательный — id() возвращает &str, остальное в config

#### 1.2 Переделать ResourceManager

- Proper `HashMap<TypeId, String>` mapping (TypeId → resource key) при register()
- `register<R: Resource>(&self, resource: R, config: R::Config)` — типизированная регистрация
- `acquire<T>(&self, key: &str, context: &ResourceContext) -> Result<PooledHandle<T>>`
- `release<T>(&self, key: &str, instance: T)` — возврат в пул
- `shutdown()` — graceful: drain pools, cancel health checks, cleanup
- Config передаётся при регистрации, а не пустой JSON

#### 1.3 Pool ↔ Resource интеграция

Текущая проблема: Pool не использует Poolable трейт. Решение — Pool работает напрямую с Resource:

- `Pool<R: Resource>` вместо `Pool<T: Send + Sync>`
- Pool вызывает `resource.create()`, `resource.is_valid()`, `resource.recycle()`, `resource.cleanup()`
- Strategies: FIFO (default), LIFO (для connection locality)
- LRU — отложить до Phase 4, когда будут реальные use cases
- `PoolConfig`: min_idle, max_size, acquire_timeout, max_lifetime, idle_timeout

#### 1.4 DependencyGraph — упростить

- Keys: `&str` вместо `ResourceId` (ресурсу достаточно имени)
- Сохранить: topological sort, cycle detection
- Удалить: версионирование, namespace, compatibility checking

#### 1.5 ResourceProvider port

- Добавить `ResourceProvider` trait в `crates/action/src/provider.rs`
- Добавить `with_resources()` и `resource::<T>()` в `ActionContext`
- В runtime: `ResourceProviderAdapter` обёртка над `ResourceManager`

#### 1.6 Reference implementation

- Один пример в `examples/basic_resource.rs` — InMemoryCache
- Показывает: implement Resource, register, acquire from action, cleanup
- НЕ в src/ — только в examples/

#### 1.7 End-to-end тест

```
register(InMemoryCache) → acquire("cache") → use → release → shutdown → verify cleanup
```

#### Exit criteria
- Action может вызвать `ctx.resource::<MyResource>("key").await?`
- Pool acquire/release работает корректно (включая Drop)
- Dependencies инициализируются в правильном порядке
- Shutdown корректно очищает все ресурсы
- >= 80% покрытие ключевых путей (manager, pool, lifecycle)

---

### Phase 2: Production Readiness

> **Цель:** Крейт можно использовать в production с мониторингом и credential'ами.
> **Длительность:** 6-8 недель
> **Результат:** Стабильный, наблюдаемый, безопасный ресурсный менеджер.

#### 2.1 Credential интеграция

- `AuthenticatedResource` — extension trait для ресурсов, требующих credentials
- `ResourceContext` получает `Option<Arc<dyn CredentialProvider>>`
- Credentials запрашиваются при `create()`, а не при регистрации
- Credential refresh: при `recycle()` проверяется, не истёк ли credential
- Feature gate: `credentials`

#### 2.2 Metrics интеграция

- Через крейт `metrics` (стандарт Rust экосистемы)
- Метрики пула: `pool.size`, `pool.available`, `pool.in_use`, `pool.acquire_duration_ms`
- Метрики health: `resource.health_check.duration_ms`, `resource.health_check.status`
- Метрики lifecycle: `resource.create.duration_ms`, `resource.cleanup.count`
- Feature gate: `metrics`

#### 2.3 Tracing интеграция

- Через крейт `tracing` (стандарт)
- `tracing::instrument` на всех pub async методах
- Span'ы: `resource.acquire`, `resource.release`, `resource.health_check`
- Feature gate: `tracing`

#### 2.4 Graceful shutdown

- `ResourceManager::shutdown()` — фаза drain (перестать выдавать новые, дождаться возврата), фаза cleanup (вызвать cleanup на каждом экземпляре), фаза terminate (отмена health checks)
- Timeout на каждую фазу (configurable)
- `CancellationToken` propagation из engine → manager → health checker

#### 2.5 Config validation

- `ResourceConfig::validate()` вызывается при `register()` — fail fast
- Structured validation errors (field name, constraint, actual value)

#### 2.6 Property tests + Benchmarks

- proptest: lifecycle state machine invariants (transition consistency)
- proptest: serde roundtrip для всех public types
- proptest: scope containment transitivity
- criterion: pool acquire/release throughput (target: >100k ops/sec)
- criterion: manager register/lookup latency

#### 2.7 Документация

- Заменить 19 aspirational docs на 5 реальных:
  - `Architecture.md` — общая архитектура, диаграммы
  - `ResourceTrait.md` — как реализовать Resource (tutorial)
  - `Pooling.md` — стратегии пулинга, конфигурация
  - `HealthChecks.md` — настройка health-проверок
  - `Integration.md` — интеграция с action/engine

#### Exit criteria
- Credential-aware ресурсы работают end-to-end
- Prometheus-совместимые метрики экспортируются
- Tracing spans видны в Jaeger/Zipkin
- Graceful shutdown за <5s при 100 pooled ресурсах
- Property tests: >1000 итераций без failures
- Benchmarks: acquire/release >100k ops/sec
- Документация соответствует коду 1:1

---

### Phase 3: Observability & Events

> **Цель:** Полная наблюдаемость ресурсной системы.
> **Длительность:** 4-6 недель
> **Результат:** Операторы видят состояние каждого ресурса в реальном времени.

#### 3.1 ResourceEvent system

```rust
pub enum ResourceEvent {
    Created { resource_id: String, scope: ResourceScope },
    Acquired { resource_id: String, pool_stats: PoolStats },
    Released { resource_id: String, duration: Duration },
    HealthChanged { resource_id: String, from: HealthState, to: HealthState },
    PoolExhausted { resource_id: String, waiters: usize },
    CleanedUp { resource_id: String, reason: CleanupReason },
    Error { resource_id: String, error: ResourceError },
}
```

- Реализация через `tokio::sync::broadcast` (stateless events, как рекомендуется в CLAUDE.md)
- `ResourceManager::subscribe() -> broadcast::Receiver<ResourceEvent>`
- Подписчики обрабатывают события асинхронно, не блокируя менеджер

#### 3.2 MetricsCollector

- Агрегирует метрики из пулов, health checker'ов, менеджера
- Экспортирует через `metrics` крейт (Prometheus counter/gauge/histogram)
- Dashboard-ready: pool utilization %, p50/p95/p99 acquire latency, health status timeline
- Configurable collection interval

#### 3.3 Structured logging

- Каждая операция логируется через `tracing` с structured fields
- `resource.id`, `resource.scope`, `pool.size`, `pool.available` — как span fields
- Уровни: ERROR для failures, WARN для degraded, INFO для lifecycle, DEBUG для acquire/release

#### Exit criteria
- Все lifecycle события эмитируются корректно
- Метрики видны в Prometheus/Grafana
- Structured logs содержат достаточно контекста для debugging
- Event throughput > 50k events/sec без backpressure

---

### Phase 4: Advanced Resource Management

> **Цель:** Устойчивость при частичных отказах.
> **Длительность:** 6-8 недель
> **Результат:** Ресурсы переживают сбои без каскадного эффекта.

#### 4.1 Health check pipeline

Multi-stage проверки вместо одного `health_check()`:

```rust
pub struct HealthPipeline {
    stages: Vec<Box<dyn HealthStage>>,
}

pub trait HealthStage: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, instance: &dyn Any) -> StageResult;
}

// Встроенные стадии:
// 1. Connectivity — TCP ping / простой запрос
// 2. Performance — latency < threshold
// 3. ResourceUtilization — CPU/memory/connections в пределах
// 4. DependencyHealth — зависимые ресурсы healthy
```

#### 4.2 Degraded state handling

Ресурс может быть "degraded" — работает, но медленнее или с ограничениями:

```rust
pub enum HealthState {
    Healthy,
    Degraded { reason: String, impact: f32 },  // 0.0-1.0
    Unhealthy { recoverable: bool },
    Unknown,
}
```

- При `Degraded`: предупреждение, но ресурс продолжает выдаваться
- При `Unhealthy(recoverable: true)`: перестать выдавать, пулить replacement'ы
- При `Unhealthy(recoverable: false)`: немедленный cleanup, уведомление

#### 4.3 Resource warming

- Pre-create `min_idle` экземпляров при регистрации (не ленивое создание)
- Background task поддерживает `min_idle` (если экземпляры умирают — создаёт новые)
- Configurable: `warm_on_register: bool`, `warm_concurrency: usize`

#### 4.4 Connection recycling

- `max_lifetime` — максимальное время жизни экземпляра (даже если healthy)
- `idle_timeout` — максимальное время простоя перед cleanup
- Background task проверяет expired/idle экземпляры и вызывает `recycle()` или `cleanup()`

#### 4.5 Cross-resource dependency health

- Если ресурс A зависит от B, и B стал Unhealthy → A автоматически помечается Degraded
- DependencyGraph используется для propagation health status
- Cascade depth limit (configurable, default: 3)

#### Exit criteria
- Health pipeline с 2+ stages работает для reference resource
- Degraded state корректно ограничивает выдачу ресурсов
- Warming создаёт min_idle экземпляров при register
- Expired/idle экземпляры recycled/cleaned up
- Cascade health propagation работает для графа из 3+ ресурсов

---

### Phase 5: Lifecycle Hooks

> **Цель:** Расширяемость жизненного цикла без модификации кода ресурса.
> **Длительность:** 4-6 недель
> **Результат:** Пользователи могут подключать логику на каждом этапе жизненного цикла.

#### 5.1 Hook registry

```rust
#[async_trait]
pub trait ResourceHook: Send + Sync {
    /// Вызывается перед операцией. Может отменить (Err).
    async fn before(&self, event: &HookEvent) -> Result<(), ResourceError> {
        let _ = event;
        Ok(())
    }

    /// Вызывается после операции.
    async fn after(&self, event: &HookEvent, result: &HookResult) {
        let _ = (event, result);
    }

    /// Приоритет (меньше = раньше). Default: 100.
    fn priority(&self) -> u32 { 100 }

    /// Фильтр — для каких ресурсов срабатывает.
    fn filter(&self) -> HookFilter { HookFilter::All }
}

pub enum HookEvent {
    BeforeCreate { resource_id: String, config: serde_json::Value },
    AfterCreate { resource_id: String },
    BeforeAcquire { resource_id: String },
    AfterAcquire { resource_id: String, wait_duration: Duration },
    BeforeRelease { resource_id: String, usage_duration: Duration },
    AfterRelease { resource_id: String },
    BeforeCleanup { resource_id: String, reason: CleanupReason },
    AfterCleanup { resource_id: String },
    HealthChanged { resource_id: String, from: HealthState, to: HealthState },
}
```

#### 5.2 Built-in hooks

- **AuditHook** — логирует все операции для compliance
- **MetricsHook** — собирает timing метрики
- **CredentialRefreshHook** — обновляет credential при acquire, если истекает
- **SlowAcquireHook** — предупреждение если acquire > threshold

#### Exit criteria
- Hooks вызываются в правильном порядке (priority)
- before() может отменить операцию
- Filter корректно ограничивает scope хуков
- Built-in hooks работают end-to-end

---

### Phase 6: Ecosystem — Driver Crates

> **Цель:** Готовые к использованию ресурсы для популярных сервисов.
> **Длительность:** Ongoing, 2-3 недели на каждый драйвер
> **Результат:** Пользователи подключают `nebula-resource-postgres` и получают pooled, health-checked PostgreSQL.

#### 6.1 nebula-resource-http

- Обёртка над `reqwest::Client`
- Config: base_url, timeout, headers, TLS, proxy
- Health check: GET /health или configurable endpoint
- SSRF protection: blocklist для internal IP ranges
- Retry: через caller (nebula-resilience), не встроено

#### 6.2 nebula-resource-postgres

- Обёртка над `sqlx::PgPool` (sqlx уже делает свой pooling)
- Resource trait оборачивает sqlx pool — НЕ дублирует пулинг
- Config: url (redacted в Debug), max_connections, statement_cache_size
- Health check: `SELECT 1`
- Credential: из CredentialProvider, не из URL

#### 6.3 nebula-resource-redis

- Обёртка над `redis::Client`
- Config: url, database, pool_size
- Health check: `PING`

#### 6.4 nebula-resource-kafka

- Обёртка над `rdkafka::producer::FutureProducer` / `rdkafka::consumer::StreamConsumer`
- Config: brokers, topic, group_id, security (SASL/SSL)
- Health check: metadata fetch

#### 6.5 nebula-resource-mongodb

- Обёртка над `mongodb::Client`
- Config: connection_string, database
- Health check: `ping` command

#### Принципы для каждого драйвера
- Отдельный крейт в workspace
- Зависит от `nebula-resource` (framework trait)
- НЕ дублирует pooling, если underlying library уже делает его (sqlx, mongodb)
- Custom Debug для config (redact credentials)
- Testcontainers для интеграционных тестов
- >= 80% тестовое покрытие
- README с примером использования

#### Exit criteria (per driver)
- Реализует `Resource` trait
- Реализует `HealthCheckable`
- Credentials не утекают в логи
- Интеграционные тесты с testcontainers
- README + example

---

### Phase 7: Developer Experience

> **Цель:** Создание нового ресурса — просто и приятно.
> **Длительность:** 4-6 недель
> **Результат:** Новый ресурс — это 20-30 строк кода вместо 100+.

#### 7.1 nebula-resource-derive

Отдельный крейт `crates/resource-derive/`:

```rust
use nebula_resource_derive::Resource;

#[derive(Resource)]
#[resource(id = "my-cache", health_check)]
pub struct MyCache {
    client: redis::Client,
}

#[derive(ResourceConfig)]
pub struct MyCacheConfig {
    #[config(validate = "not_empty")]
    pub url: String,

    #[config(default = 10)]
    pub max_connections: u32,

    #[config(secret)]  // Redacted в Debug
    pub password: Option<String>,
}
```

Генерирует:
- `impl Resource for MyCache` с boilerplate (id, create, cleanup)
- `impl ResourceConfig for MyCacheConfig` с validate()
- Custom `Debug` для MyCacheConfig (redact #[config(secret)] полей)

#### 7.2 Testing utilities

Расширить `testing/` модуль:

```rust
// В src/testing/mod.rs

/// Mock ресурс для тестов action'ов.
pub struct MockResource<T> { ... }

/// Test pool с configurable failure injection.
pub struct TestPool<T> { ... }

/// Builder для тестовых сценариев.
pub struct ResourceTestHarness { ... }

impl ResourceTestHarness {
    /// Создать harness с in-memory ресурсами.
    pub fn new() -> Self { ... }

    /// Зарегистрировать mock ресурс.
    pub fn with_resource<R: Resource>(&mut self, r: R) -> &mut Self { ... }

    /// Получить ActionContext с настроенным ResourceProvider.
    pub fn action_context(&self) -> ActionContext { ... }
}
```

#### 7.3 Документация и примеры

- `examples/basic_resource.rs` — минимальный ресурс
- `examples/pooled_resource.rs` — ресурс с пулингом
- `examples/health_checked_resource.rs` — ресурс с health check
- `examples/authenticated_resource.rs` — ресурс с credential'ами
- Rustdoc для всех pub items (cargo doc --no-deps)

#### Exit criteria
- `#[derive(Resource)]` генерирует корректный код
- `#[config(secret)]` redact'ит в Debug
- `ResourceTestHarness` позволяет тестировать action с mock ресурсами
- `cargo doc` — все pub items документированы
- 4 примера компилируются и запускаются

---

### Phase 8: Enterprise Features

> **Цель:** Устойчивость на уровне enterprise production.
> **Длительность:** 8-12 недель
> **Результат:** Система справляется с cascade failures, автоматически восстанавливается.

#### 8.1 Quarantine system

Изоляция "сломанных" ресурсов от здоровых:

```rust
pub struct QuarantineManager {
    quarantined: DashMap<String, QuarantineEntry>,
    recovery_strategies: Vec<Box<dyn RecoveryStrategy>>,
}

pub struct QuarantineEntry {
    resource_id: String,
    reason: QuarantineReason,
    quarantined_at: Instant,
    recovery_attempts: u32,
    max_recovery_attempts: u32,
    next_recovery_at: Instant,
}

#[async_trait]
pub trait RecoveryStrategy: Send + Sync {
    /// Попытка восстановления. Возвращает true если ресурс восстановлен.
    async fn attempt_recovery(
        &self,
        resource_id: &str,
        manager: &ResourceManager,
    ) -> Result<bool, ResourceError>;

    /// Задержка между попытками (exponential backoff).
    fn backoff(&self, attempt: u32) -> Duration;
}
```

- Автоматический карантин: N consecutive health check failures → quarantine
- Recovery attempts: exponential backoff (1s, 2s, 4s, 8s, max 60s)
- Max recovery attempts: configurable (default: 10), после чего → permanent failure
- Manual release: `quarantine_manager.release(resource_id)`

#### 8.2 Cascade failure detection

- При переходе ресурса в Unhealthy → проверить все зависящие от него ресурсы
- Если >50% dependents тоже Unhealthy → это cascade, не одиночный сбой
- Cascade detection уведомляет через EventBus
- Circuit breaker на уровне dependency group (не на отдельном ресурсе)

#### 8.3 Pool auto-scaling

Не ML, а простые правила:

```rust
pub struct AutoScalePolicy {
    /// Увеличить pool при utilization > high_watermark в течение scale_up_window.
    pub high_watermark: f32,       // 0.8
    pub scale_up_window: Duration, // 30s
    pub scale_up_step: usize,      // +2

    /// Уменьшить pool при utilization < low_watermark в течение scale_down_window.
    pub low_watermark: f32,        // 0.2
    pub scale_down_window: Duration, // 5min
    pub scale_down_step: usize,    // -1

    /// Абсолютные пределы.
    pub min_size: usize,
    pub max_size: usize,
}
```

- Background task проверяет utilization каждые N секунд
- Scale up — создать новые экземпляры (до max_size)
- Scale down — пометить idle как evictable (до min_size)

#### 8.4 Configuration hot-reload

- `ResourceManager::reload_config(resource_id, new_config)`
- Стратегия: создать новый пул с новым конфигом → drain старый → swap
- Не прерывает текущих пользователей
- Через `nebula-config` hot-reload если доступен

#### Exit criteria
- Quarantine: ресурс автоматически изолируется после N failures, восстанавливается
- Cascade: обнаруживает каскадный сбой в графе из 5+ ресурсов
- Auto-scale: pool растёт/сжимается при изменении нагрузки
- Hot-reload: смена конфига без downtime

---

## 4. Timeline (ориентировочный)

```
2026 Q1 (Feb-Mar):    Phase 0  — Foundation Reset
2026 Q1-Q2 (Mar-May): Phase 1  — Core Framework
2026 Q2-Q3 (May-Jul): Phase 2  — Production Readiness
2026 Q3 (Jul-Aug):    Phase 3  — Observability & Events
2026 Q3-Q4 (Aug-Oct): Phase 4  — Advanced Resource Management
2026 Q4 (Oct-Nov):    Phase 5  — Lifecycle Hooks
2026 Q4+ (Nov+):      Phase 6  — Driver Crates (ongoing)
2027 Q1 (Jan-Feb):    Phase 7  — Developer Experience
2027 Q1-Q2 (Feb-Apr): Phase 8  — Enterprise Features
```

## 5. Quality Gates (CI pipeline, каждый PR)

```bash
# Обязательные
cargo fmt --all -- --check
cargo clippy -p nebula-resource -- -D warnings
cargo check -p nebula-resource --all-features
cargo test -p nebula-resource
cargo doc --no-deps -p nebula-resource

# После Phase 2
cargo test -p nebula-resource -- --ignored  # Integration tests
cargo bench -p nebula-resource              # Regression check

# После Phase 6 (per driver)
cargo test -p nebula-resource-postgres      # Requires Docker
cargo test -p nebula-resource-redis
```

## 6. Метрики зрелости

| Метрика | Phase 0 | Phase 1 | Phase 2 | Phase 4 | Phase 8 |
|---------|---------|---------|---------|---------|---------|
| LOC (src/) | ~2,500 | ~3,500 | ~4,500 | ~6,000 | ~8,000 |
| Тесты | ~40 | ~80 | ~120 | ~170 | ~220 |
| Property тесты | 2 | 4 | 8 | 12 | 15 |
| Бенчмарки | 0 | 0 | 3 | 5 | 7 |
| Pub traits | 3 | 3 | 4 | 5 | 6 |
| Feature flags | 2 | 3 | 5 | 5 | 6 |
| Docs (matching code) | 0 | 2 | 5 | 5 | 5 |
| Driver crates | 0 | 0 | 0 | 0-2 | 3-5 |

## 7. Принципы (на весь путь)

1. **Каждая фаза — shippable.** Не бывает "промежуточных" фаз, где крейт сломан.
2. **Тесты первее кода.** Property tests для state machines, roundtrip для serde, integration для end-to-end.
3. **Документация = код.** Если код изменился, документация обновляется в том же PR.
4. **Драйверы — отдельно.** Framework не знает о PostgreSQL, Redis, Kafka.
5. **Security by default.** Credentials redacted в Debug, scope isolation enforced, no SSRF.
6. **Follow existing patterns.** ResourceProvider как CredentialProvider. Driver crates как sandbox-inprocess.
7. **No premature abstraction.** Если фича не используется в 2+ местах — не абстрагировать.
8. **Boring is good.** Понятный, скучный, надёжный код лучше clever abstractions.
