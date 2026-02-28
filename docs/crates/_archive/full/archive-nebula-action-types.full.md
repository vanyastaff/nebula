# Archived From "docs/archive/nebula-action-types.md"

# Nebula — Action Type System

## Философия

Система типов actions в Nebula построена на двух уровнях:

- **Core types** — типы которые движок реально различает и обрабатывает по-разному.
- **DX types** — удобные обёртки поверх core для частых паттернов. Опытный разработчик может реализовать любой DX-тип вручную через соответствующий core-тип.

Аналогия из Rust: `BufReader` — это DX поверх `Read`. Движок работает с `Read`, `BufReader` просто убирает boilerplate.

---

## Core Types

Движок различает именно эти четыре типа. Всё остальное — надстройки.

```
Action (базовый трейт — метаданные)
├── StatelessAction
├── StatefulAction
│   └── TriggerAction
└── ResourceAction
```

### Action

Базовый трейт. Содержит только метаданные — id, name, description, capabilities. Не определяет поведение выполнения.

```rust
pub trait Action: Send + Sync {
    fn metadata(&self) -> &ActionMetadata;
}
```

---

### StatelessAction

**Бывший ProcessAction.** Чистая функция: `Input → Output`. Не сохраняет состояние между вызовами. Может выполняться параллельно без координации.

```rust
pub trait StatelessAction: Action {
    type Input: DeserializeOwned;
    type Output: Serialize;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>>;
}
```

**Возможные `ActionResult`:** `Success`, `Skip`, `Branch`, `Route`, `MultiOutput`, `Retry`.

**Когда использовать:**
- Трансформации данных
- API-вызовы без side effects
- Валидация
- Генерация контента

---

### StatefulAction

Шаг workflow с персистентным состоянием между вызовами. Движок сохраняет `State` после каждой итерации и восстанавливает при следующем вызове.

```rust
pub trait StatefulAction: Action {
    type State: Serialize + DeserializeOwned + Default;
    type Input: DeserializeOwned;
    type Output: Serialize;

    async fn execute(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>>;

    async fn initialize_state(
        &self,
        input: &Self::Input,
        ctx: &ActionContext,
    ) -> Result<Self::State> {
        Ok(Self::State::default())
    }

    fn state_version(&self) -> u32 { 1 }

    async fn migrate_state(
        &self,
        old_state: serde_json::Value,
        old_version: u32,
        new_version: u32,
    ) -> Result<Self::State>;
}
```

**Дополнительные `ActionResult`:** `Continue { delay }`, `Break { reason }`, `Wait { condition }`.

`Wait` позволяет приостановить выполнение до внешнего события — например, человеческого апрувала или HTTP-callback — без создания отдельного типа action:

```rust
Ok(ActionResult::Wait {
    condition: WaitCondition::Approval {
        approver: "manager@company.com".into(),
        message: "Approve deployment to production?".into(),
    },
    timeout: Some(Duration::from_secs(86400)),
    partial_output: None,
})
```

**Когда использовать:**
- Пагинация через большие датасеты
- Батч-обработка с прогрессом и resume
- Rate-limited операции
- Accumulation / агрегация
- Human-in-the-loop через `Wait`

---

### TriggerAction

Extends `StatefulAction`. Стартер workflow — живёт вне графа выполнения и *порождает* execution-ы. Управляется отдельным `TriggerManager` внутри движка, параллельно с `WorkflowEngine`.

State обязателен по природе: триггер должен помнить что уже обработал, чтобы не порождать дубли.

```rust
pub trait TriggerAction: StatefulAction {
    type Event: Serialize;

    /// Старт мониторинга — вызывается один раз при деплое workflow.
    async fn start(&self, ctx: &TriggerContext) -> Result<()>;

    /// Остановка — при undeployе или shutdown.
    async fn stop(&self, ctx: &TriggerContext) -> Result<()>;
}
```

`TriggerContext` отличается от `ActionContext` — он не привязан к конкретному execution, так как триггер существует на уровне workflow.

**Когда использовать:**
- Любой внешний источник событий
- Инициация workflow по внешнему условию

---

### ResourceAction

**Бывший SupplyAction.** Узел в графе который предоставляет capability (ресурс, tool, сервис) своим downstream узлам. Аналог `supplyData` в n8n — именно так AI Agent node получает tools от подключённых узлов.

Движок управляет lifecycle особым образом:
1. Выполняет `ResourceAction::configure()` **до** downstream узлов
2. Создаёт `Resource::Instance` через `nebula-resource` (с pooling, health checks)
3. Делает ресурс доступным downstream через `ctx.resource()`
4. После завершения всех downstream вызывает `cleanup()` с owned instance — гарантия что никто больше не держит ресурс

```rust
pub trait ResourceAction: Action {
    type Resource: Resource; // из nebula-resource

    async fn configure(
        &self,
        ctx: &ActionContext,
    ) -> Result<<Self::Resource as Resource>::Config>;

    // Owned Instance — движок гарантирует что это последний вызов
    async fn cleanup(
        &self,
        resource: <Self::Resource as Resource>::Instance,
        ctx: &ActionContext,
    ) -> Result<()> {
        drop(resource);
        Ok(())
    }
}
```

**Три причины почему это core type, а не DX:**

- **Порядок выполнения** — движок обязан выполнить `ResourceAction` до downstream. Это топологическая зависимость которую движок понимает явно.
- **Scoped lifecycle** — ресурс живёт только пока выполняются downstream узлы данной ветки, потом `cleanup`. Не глобально.
- **Изоляция** — ресурс доступен только downstream в этой ветке графа, не всему workflow.

Это принципиально отличается от `ctx.resource()` который достаёт ресурс из глобального registry `nebula-resource`. `ResourceAction` — это **dependency injection через граф**, `ctx.resource()` — это **глобальный доступ**.

```
┌─────────────────────┐
│ PostgresPool        │  ← ResourceAction (configure + lifecycle)
│ (ResourceAction)    │
└──────────┬──────────┘
           │ scoped resource — только для этой ветки
           ▼
┌─────────────────────┐
│ QueryUsers          │  ← ctx.resource::<DatabasePool>()
│ (StatelessAction)   │
└─────────────────────┘
```

**Когда использовать:**
- Предоставить DB connection pool дочерним узлам
- Подключить AI tools к Agent узлу
- Сконфигурировать HTTP client с кредами для конкретной ветки графа

---

## DX Types

Обёртки для частых паттернов. Движок работает с базовым core-типом — DX-тип просто убирает boilerplate. Профессиональный разработчик может реализовать любой из них напрямую через `StatefulAction` или `TriggerAction`.

```
StatefulAction
├── InteractiveAction   — Wait { Approval / Webhook } + удобный UI API
└── TransactionalAction — Saga pattern + compensation boilerplate

TriggerAction
├── WebhookAction       — endpoint регистрация + signature verification
└── PollAction          — cursor management + interval scheduling
```

---

### InteractiveAction *(DX over StatefulAction)*

Паттерн для участия человека в workflow: кнопки, ссылки, формы, ручное "продолжить". По сути это `StatefulAction` который возвращает `Wait { condition }` с удобным API для описания UI.

Без этого DX-типа то же самое делается через:
```rust
Ok(ActionResult::Wait {
    condition: WaitCondition::Approval { .. },
    ..
})
```

С DX-типом разработчик описывает взаимодействие декларативно, не думая о механике `Wait`.

**Два паттерна участия человека — влияют на конфигурацию `WaitCondition`:**

**Human in the Loop** — прямое участие в принятии решения. Используется когда цена ошибки высока: медицина, финансовые транзакции, юридические документы. Без апрувала workflow не продолжается. Таймаут заканчивается эскалацией, не автоапрувалом:

```rust
WaitCondition::Approval {
    approver: "legal@company.com".into(),
    message: "Approve contract before sending to client".into(),
    // таймаут → эскалация к вышестоящему, не автопродолжение
    on_timeout: OnTimeout::Escalate { to: "cto@company.com".into() },
}
```

**Human on the Loop** — надзор, вмешательство только при аномалиях. Система работает автономно, человек получает уведомление и может вмешаться в течение окна. Если не вмешался — workflow продолжается автоматически:

```rust
WaitCondition::Approval {
    approver: "ops@company.com".into(),
    message: "Deployment ready. Override within 10 min to cancel.".into(),
    // таймаут → автоапрув, продолжаем без вмешательства
    on_timeout: OnTimeout::AutoApprove,
}
```

---

### TransactionalAction *(DX over StatefulAction)*

Saga pattern с автоматическим управлением compensation. По сути `StatefulAction` который хранит `CompensationData` в state и при ошибке вызывает rollback.

```rust
pub trait TransactionalAction: StatefulAction {
    type CompensationData: Serialize + DeserializeOwned;

    async fn execute_tx(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<(Self::Output, Self::CompensationData)>;

    async fn compensate(
        &self,
        data: Self::CompensationData,
        ctx: &ActionContext,
    ) -> Result<()>;

    fn max_compensation_retries(&self) -> u32 { 3 }

    fn step_kind(&self) -> SagaStepKind { SagaStepKind::Compensable }
}
```

**Типология шагов Saga — движок обрабатывает каждый вид по-разному:**

```rust
pub enum SagaStepKind {
    /// Может быть отменён — есть компенсационная транзакция.
    /// При ошибке любого последующего шага — движок вызывает compensate().
    Compensable,

    /// Точка невозврата. После успеха компенсация невозможна.
    /// Движок не будет пытаться откатить этот шаг и всё до него.
    /// Пример: отправка email, публикация события.
    Pivot,

    /// Идёт после Pivot — только вперёд, компенсации нет.
    /// Должен быть идемпотентен, движок ретраит до успеха.
    /// Пример: обновление статуса заказа после списания денег.
    Retryable,
}
```

Практический пример — оформление заказа:

```
[Reserve Inventory]  — Compensable  → compensate: release inventory
[Charge Payment]     — Pivot        → точка невозврата, деньги списаны
[Update Order Status]— Retryable    → только вперёд, ретраи до успеха
[Send Confirmation]  — Retryable    → только вперёд, идемпотентно
```

Если `Charge Payment` упал — движок компенсирует `Reserve Inventory`. Если `Update Order Status` упал после успешного `Charge Payment` — движок только ретраит, компенсировать нечего.

---

### WebhookAction *(DX over TriggerAction)*

Extends `TriggerAction`. Добавляет готовый API для регистрации endpoint, верификации подписи и обработки входящего запроса.

```rust
pub trait WebhookAction: TriggerAction {
    async fn register(
        &self,
        ctx: &TriggerContext,
    ) -> Result<WebhookRegistration>;

    async fn handle_request(
        &self,
        request: IncomingRequest,
        state: &Self::State,
        ctx: &TriggerContext,
    ) -> Result<Option<Self::Event>>;

    async fn verify_signature(
        &self,
        request: &IncomingRequest,
        secret: &str,
    ) -> Result<bool>;
}
```

State хранит registration_id, endpoint URL, secret. Движок автоматически вызывает `register` при старте и `handle_request` при входящем HTTP-запросе.

---

### PollAction *(DX over TriggerAction)*

Extends `TriggerAction`. Добавляет cursor management и interval scheduling. State хранит cursor — движок гарантирует сохранение cursor только после успешной обработки событий.

```rust
pub trait PollAction: TriggerAction {
    type Cursor: Serialize + DeserializeOwned + Default;

    fn poll_interval(&self) -> Duration;

    async fn poll(
        &self,
        cursor: &Self::Cursor,
        ctx: &TriggerContext,
    ) -> Result<PollResult<Self::Event, Self::Cursor>>;
}

pub struct PollResult<E, C> {
    pub events: Vec<E>,
    pub next_cursor: C,
    /// true = движок вызовет poll снова немедленно
    pub has_more: bool,
}
```

---

## Потребление ресурсов

Все типы actions могут потреблять ресурсы через `ActionContext` — это ортогональная возможность, не связанная с иерархией типов:

```rust
// Доступно в любом типе action
let db = ctx.resource::<DatabaseResource>().await?;
let cache = ctx.resource::<CacheResource>().await?;
```

`ResourceAction` — это про *предоставление* ресурса в граф, а `ctx.resource()` — про *потребление*.

---

## Сводная таблица

| Тип | Core / DX | State | Extends | Назначение |
|---|---|---|---|---|
| `StatelessAction` | Core | ❌ | `Action` | Input → Output, чистая функция |
| `StatefulAction` | Core | ✅ | `Action` | Итеративная обработка с state |
| `TriggerAction` | Core | ✅ | `StatefulAction` | Стартер workflow |
| `ResourceAction` | Core | ❌ | `Action` | Предоставляет ресурс дочерним узлам |
| `InteractiveAction` | DX | ✅ | `StatefulAction` | Human-in-the-loop, кнопки, апрувалы |
| `TransactionalAction` | DX | ✅ | `StatefulAction` | Saga / compensation |
| `WebhookAction` | DX | ✅ | `TriggerAction` | Входящий HTTP webhook |
| `PollAction` | DX | ✅ | `TriggerAction` | Периодический опрос источника |

---

## Выбор типа

```
Нужен action?
│
├── Инициирует workflow извне?
│   └── да → TriggerAction
│             ├── Входящий HTTP? → WebhookAction (DX)
│             └── Опрос источника? → PollAction (DX)
│
├── Предоставляет ресурс дочерним узлам?
│   └── да → ResourceAction
│
├── Нужен state между вызовами?
│   └── да → StatefulAction
│             ├── Нужен Saga/rollback? → TransactionalAction (DX)
│             └── Нужен human input? → InteractiveAction (DX)
│                                       (или просто ActionResult::Wait)
│
└── нет → StatelessAction
```

