# Core Action Types

## StatelessAction

~80% всех nodes. Чистая функция: Input → Output. Параллелизуется без координации.

```rust
pub trait StatelessAction: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;

    fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}
```

**Допустимые ActionResult:** Success, Skip, Branch, Route, MultiOutput, Retry.

---

## StatefulAction

Итеративное выполнение с persistent state. Engine вызывает execute() повторно,
сохраняя State между итерациями. State инициализируется через `init_state()`.

```rust
pub trait StatefulAction: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;
    type State: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Инициализация state при первом запуске.
    ///
    /// **Contract:** init_state MUST be a pure function or fully idempotent.
    /// It MUST NOT make external API calls, create sessions, or cause any
    /// side effects that cannot be safely repeated. All external mutations
    /// belong in execute(), which is protected by retry policy and state
    /// persistence guarantees.
    ///
    /// Rationale: if crash occurs after init_state but before state persist,
    /// runtime will call init_state again on retry.
    fn init_state(
        &self,
        input: &Self::Input,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<Self::State, ActionError>> + Send;

    /// Выполнение одной итерации.
    fn execute(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;

    /// Версия формата State. При изменении — engine вызывает migrate_state.
    fn state_version(&self) -> u32 { 1 }

    /// Миграция state из предыдущей версии.
    /// Has access to ActionContext — migration may need new parameters or
    /// resources introduced in the newer version.
    fn migrate_state(
        &self,
        persisted: PersistedState,
        ctx: &ActionContext,
    ) -> Result<Self::State, StateMigrationError> {
        Err(StateMigrationError::NotImplemented {
            from_version: persisted.state_version,
            to_version: self.state_version(),
        })
    }
}
```

**Дополнительные ActionResult:** Continue, Break, Wait.

### PersistedState envelope

Engine хранит state в envelope с version metadata:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    /// Action interface version at time of persistence.
    pub action_version: u32,
    /// State format version (from StatefulAction::state_version()).
    pub state_version: u32,
    /// Serialized state payload.
    pub payload: serde_json::Value,
    /// Timestamp of last update.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

### NextPersistedState (handler → engine)

```rust
#[derive(Debug, Clone)]
pub struct NextPersistedState {
    pub state_version: u32,
    pub payload: serde_json::Value,
}
```

### StateMigrationError

```rust
#[derive(Debug, thiserror::Error)]
pub enum StateMigrationError {
    #[error("migration not implemented: v{from_version} → v{to_version}")]
    NotImplemented { from_version: u32, to_version: u32 },
    #[error("incompatible state: {reason}")]
    Incompatible { reason: String },
    #[error("deserialization failed: {0}")]
    Deserialization(#[from] serde_json::Error),
}
```

### Execution semantics: Continue vs Wait

**Continue = cooperative yield.** Action сделал часть работы, хочет вернуть partial
output и продолжить. Engine СНАЧАЛА durably persists state, ЗАТЕМ emits partial
output, ЗАТЕМ requeues node. OOM-safe пагинация.

**Wait = park execution.** Action ждёт внешнего события. Engine persists state,
подписывается на event, освобождает worker.

### Durable commit contract (6-step, normative)

```
1. Engine loads PersistedState from store (if exists)
2. If state_version < handler.state_version() → handler.migrate_state(persisted, ctx)
   - If migration fails → whole step fails, no side effects
3. Pre-execute snapshot: save current state as recovery point
4. handler.execute(input, state, ctx) → StatefulHandlerResult
   - If panic → runtime catches, state = pre-execute snapshot, classify Fatal
   - If error → state = pre-execute snapshot
5. Engine durably writes PersistedState { action_version, state_version,
   payload: result.next_state.payload, updated_at: now() }
   - If write fails → whole step fails, no routing side effects
6. ONLY after durable write confirmed:
   - Emit output to downstream / park / requeue
   - Never emit partial output before durable commit
```

---

## ResourceAction

Graph-level DI. Engine выполняет acquire() ПЕРЕД downstream nodes.
Resulting lease доступен downstream через ScopedResourceMap.
Engine вызывает release() при завершении downstream scope.

**ResourceAction ≠ Resource (nebula-resource).** Разные абстракции:
- `Resource` — долгоживущий managed resource с topology, lifecycle, health
- `ResourceAction` — graph-level scoped lease в workflow execution

**Ownership note:** `resource_typed<R>()` returns a typed **managed handle/lease**,
not the raw resource object. Handle lifecycle is managed by resource layer;
action code uses it via Deref to the underlying lease type.

```rust
pub trait ResourceAction: Action {
    type Lease: Send + Sync + 'static;

    /// Acquire a scoped resource for the downstream branch.
    fn acquire(
        &self,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<Self::Lease, ActionError>> + Send;

    /// Release the leased resource when downstream scope ends.
    ///
    /// **Contract:** Runtime wraps release in a timeout (default 30s).
    /// On timeout or error, runtime logs degraded cleanup and marks scope
    /// as partially cleaned. The lease is considered leaked for monitoring.
    fn release(
        &self,
        lease: Self::Lease,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<ReleaseOutcome, ActionError>> + Send;
}

#[derive(Debug, Clone)]
pub enum ReleaseOutcome {
    Released,
    Leaked { reason: String },
}
```

### Scoping mechanism

```
ResourceAction[PostgresPool].acquire(ctx) → PoolLease
  │
  ├── ScopedResourceMap: { (TypeId(PoolLease), "pg_pool") → lease }
  │
  ├── QueryUsers:  ctx.scoped_resource::<PoolLease>("pg_pool") → &PoolLease
  ├── QueryOrders: ctx.scoped_resource::<PoolLease>("pg_pool") → same lease
  │
  └── scope end → ResourceAction[PostgresPool].release(lease, ctx)
```

**ScopedResourceMap key:** `(TypeId, resource_key: String)` — composite key prevents
collision when two ResourceActions of the same type exist in one workflow.

**Lookup order:**
1. ScopedResourceMap (from parent ResourceAction)
2. Global Manager (nebula-resource::Manager)
