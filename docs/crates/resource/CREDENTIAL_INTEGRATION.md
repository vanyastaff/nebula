# Credential Integration Plan: nebula-resource

**Статус**: Отложено — credential-зависимость удалена 2026-03-15.
**Причина**: `nebula-credential` ещё не устоялся по API. После стабилизации вернуться с другой стороны.

---

## Что было удалено

При очистке от `nebula-credential` из крейта были удалены следующие части:

### Из `pool.rs`
- Трейт `CredentialHandler<I>` — type-erased интерфейс авторизации инстанса.
- Поля `PoolInner`:
  - `credential_state: Arc<RwLock<Option<serde_json::Value>>>` — текущее состояние кредентиала для новых инстансов.
  - `credential_handler: Option<Arc<dyn CredentialHandler<R::Instance>>>` — обработчик авторизации.
- `Pool::with_hooks_and_credential(...)` — конструктор с поддержкой кредентиалов.
- Авторизация при создании инстанса: вызов `handler.authorize(instance, state)` после `Resource::create()`.
- `Pool::handle_rotation(new_state, strategy, credential_key)` — обработка ротации: HotSwap или drain.
- `drain_idle()` — выселение idle-инстансов при DrainAndRecreate/Reconnect.
- Тесты: `handle_rotation_hot_swap_calls_authorize_on_idle`, `handle_rotation_drain_and_recreate_evicts_idle`, `handle_rotation_without_handler_returns_error`.

### Из `manager.rs`
- Поле `credential_pool_map: Arc<DashMap<CredentialId, Vec<RotationEntry>>>` — маппинг credential → пулы.
- Структура `RotationEntry { credential_key, strategy, pool: Weak<dyn RotatablePool> }`.
- `Manager::register_with_handler(resource, config, pool_config, handler)` — регистрация с обработчиком.
- `Manager::spawn_rotation_listener(sub)` — фоновый слушатель событий ротации.

### Из `manager_pool.rs`
- Трейт `RotatablePool` — type-erased интерфейс ротации пула.
- Имплементация `RotatablePool for TypedPool<R>`.

### Из `handler.rs`
- Весь файл: `TypedCredentialHandler<I>` — обобщённый handler, десериализует JSON в `<I::Credential as CredentialType>::State` и вызывает `instance.authorize(&typed_state)`.

### Из `dependency.rs`
- Метод `ResourceDependencies::credential() -> Option<Box<dyn AnyCredential>>` — декларация зависимости от кредентиала.

### Из `error.rs`
- `Error::CredentialNotConfigured { resource_key }` — handler не зарегистрирован в пуле.
- `Error::MissingCredential { credential_id, resource_key }` — кредентиал не найден.

### Из `events.rs`
- `ResourceEvent::CredentialRotated { resource_key, credential_key, strategy }`.
- `CleanupReason::CredentialRotated` — причина выселения при ротации.

### Из `metrics.rs`
- Обработка `ResourceEvent::CredentialRotated` → счётчик `NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL`.

### Удалённые тесты
- `tests/credential_integration.rs` — полный интеграционный тест цикла ротации (HotSwap, DrainAndRecreate).

---

## Функциональные требования к будущей интеграции

### F-01: Привязка кредентиала к ресурсу при регистрации
Ресурс должен мочь декларировать, какой кредентиал ему нужен — на уровне типа, без прямой зависимости от `nebula-credential` в `nebula-resource`.

**Что нужно помнить:**
- Декларация должна быть статической (в trait-методе, а не в рантайме).
- Тип кредентиала должен задавать стратегию ротации (HotSwap, DrainAndRecreate, Reconnect).
- Ключ кредентиала (`CredentialKey` из `nebula-core`) — единственная часть из `nebula-core`, которая может остаться в `ResourceDependencies`.

### F-02: Авторизация инстанса при создании
После `Resource::create()` пул должен применить текущее состояние кредентиала к новому инстансу перед тем как отдать его вызывающему.

**Что нужно помнить:**
- Состояние кредентиала хранится в пуле как `Option<serde_json::Value>`.
- Применяется через callback (см. F-05) — не через прямой вызов метода на инстансе.
- Ошибка авторизации при создании должна провалить весь create-запрос.

### F-03: Три стратегии ротации
При ротации кредентиала пул выбирает одну из трёх стратегий:

| Стратегия | Поведение |
|-----------|-----------|
| `HotSwap` | Вызвать `authorize()` на всех idle-инстансах немедленно. In-flight инстансы будут авторизованы при следующем возврате в пул (или при следующем acquire). |
| `DrainAndRecreate` | Выселить все idle-инстансы. Новые инстансы получат авторизацию при создании. In-flight инстансы завершают работу со старыми кредентиалами. |
| `Reconnect` | То же что DrainAndRecreate. Используется для протоколов, где сессия неотделима от кредентиала (TCP-сессии, OAuth-токены встроенные в соединение). |

**Что нужно помнить:**
- Стратегия задаётся типом кредентиала или инстансом, не вызывающим кодом.
- `HotSwap` требует что инстанс thread-safe для мутации через shared ref или через `&mut` внутри пула.
- При `DrainAndRecreate` idle-инстансы должны получить `CleanupReason::CredentialRotated`.

### F-04: Менеджер маппит credential_id → пулы для диспатча
При ротации менеджер должен найти все пулы связанные с данным кредентиалом и вызвать ротацию на каждом.

**Что нужно помнить:**
- Маппинг использует `Weak<dyn RotatablePool>` чтобы не держать пул живым.
- UUID кредентиала (`CredentialId`) — реальный ключ маппинга; при регистрации он может быть неизвестен (только ключ строкой), поэтому нужен механизм разрешения ключ→UUID.
- Пул может быть удалён до ротации — поэтому `Weak::upgrade()` с silent skip.

### F-05: Decoupled authorization callback
Чтобы избежать зависимости `nebula-resource` → `nebula-credential`, авторизация должна передаваться как opaque callback при регистрации.

**Предлагаемый интерфейс (для обсуждения):**
```rust
// В nebula-resource — никакой зависимости от nebula-credential
pub type AuthorizeCallback<I> =
    Arc<dyn Fn(&mut I, &serde_json::Value) -> Result<()> + Send + Sync>;

// Регистрация ресурса с авторизацией:
manager.register_with_auth(
    resource,
    config,
    pool_config,
    RotationStrategy::HotSwap,
    Arc::new(|instance: &mut MyClient, state: &Value| {
        let typed: MyCredState = serde_json::from_value(state.clone())?;
        instance.set_token(typed.token);
        Ok(())
    }),
)?;
```

Типизация и десериализация живут в `nebula-credential` или в коде плагина. `nebula-resource` видит только `Arc<dyn Fn>`.

### F-06: RotationStrategy — куда переехать
Текущий `RotationStrategy` живёт в `nebula-credential`. Для decoupling его нужно перенести.

**Варианты:**
- `nebula-core` — минимальный тип, нет зависимостей. Минус: core растёт.
- `nebula-resource` — семантически правильно, ресурс решает как реагировать. **Предпочтительный вариант.**
- Оставить в `nebula-credential`, экспортировать в `nebula-resource` через re-export (но это оставляет зависимость).

### F-07: EventBus-driven rotation listener
Менеджер подписывается на событие ротации через `nebula-eventbus` — **без прямого импорта `nebula-credential`**.

**Что нужно помнить:**
- Событие должно содержать минимум: `credential_id: CredentialId`, `new_state: serde_json::Value`.
- Эти типы живут в `nebula-core` и `serde_json` — оба уже в зависимостях `nebula-resource`.
- Конкретный тип события (`CredentialRotationEvent`) определяется в `nebula-credential` и передаётся в `spawn_rotation_listener` как generic параметр через трейт или конкретный тип.
- Альтернатива: менеджер принимает `impl Stream<Item = (CredentialId, serde_json::Value)>` — полностью decoupled.

### F-08: ResourceDependencies без AnyCredential
Текущий `ResourceDependencies::credential()` возвращал `Box<dyn AnyCredential>` из `nebula-credential`. Нужно заменить на что-то независимое.

**Предлагаемый интерфейс:**
```rust
pub trait ResourceDependencies {
    /// Ключ кредентиала, который требует этот ресурс.
    /// Используется менеджером для построения credential→pool маппинга.
    fn credential_key() -> Option<CredentialKey>  // CredentialKey из nebula-core
    where
        Self: Sized,
    {
        None
    }

    fn resources() -> Vec<Box<dyn AnyResource>>
    where
        Self: Sized,
    {
        vec![]
    }
}
```

### F-09: Новые варианты ошибок
При восстановлении нужно добавить обратно:
- `Error::CredentialNotConfigured { resource_key }` — пул не имеет авторизационного callback.
- `Error::MissingCredential { credential_id, resource_key }` — ожидаемый кредентиал не найден в маппинге.

### F-10: События и метрики
При восстановлении нужно добавить обратно:
- `ResourceEvent::CredentialRotated { resource_key, credential_key, strategy: String }` — после успешной ротации.
- `CleanupReason::CredentialRotated` — для метрик и хуков при выселении инстансов.
- Счётчик `NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL` в `MetricsCollector`.

---

## Порядок реализации (когда придёт время)

```
1. Стабилизировать nebula-credential API (типы протоколов, CredentialRotationEvent, etc.)

2. Перенести RotationStrategy в nebula-resource (или nebula-core)

3. Добавить AuthorizeCallback в pool.rs:
   - Восстановить credential_state + callback в PoolInner
   - Восстановить with_hooks_and_credential / register_with_auth
   - Восстановить авторизацию при create
   - Восстановить handle_rotation + drain_idle

4. Обновить ResourceDependencies:
   - Добавить credential_key() -> Option<CredentialKey>
   - Убрать Box<dyn AnyCredential>

5. Восстановить Manager:
   - credential_pool_map + RotationEntry
   - spawn_rotation_listener принимает Stream или EventSubscriber<T: HasCredentialId + HasNewState>

6. Восстановить ошибки, события, метрики

7. Написать интеграционный тест credential_integration.rs:
   - HotSwap: вращение кредентиала, проверка что idle-инстансы обновились
   - DrainAndRecreate: вращение, проверка что idle выселены и новые создаются с новым состоянием
   - Reconnect: аналогично DrainAndRecreate
   - Ошибка при ротации без handler
   - Проверка метрики NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL

8. Интеграция с nebula-plugin:
   - Плагин регистрирует ресурс + callback через SDK
   - SDK предоставляет helper register_resource_with_credential(resource, cred_type, pool_config)
```

---

## Ключевые инварианты которые нельзя нарушить

1. **`nebula-resource` не зависит от `nebula-credential`** — зависимость только через `nebula-core` типы (`CredentialId`, `CredentialKey`) и `EventBus`.
2. **Credential state хранится как `serde_json::Value`** — никаких typed state в пуле (типизацию делает callback в коде плагина).
3. **Ротация атомарна с точки зрения пула** — либо все idle обновлены (HotSwap), либо все выселены (Drain). Нет частичного состояния.
4. **In-flight инстансы не прерываются** — они завершают работу со старыми кредентиалами при любой стратегии.
5. **Слабые ссылки на пулы** — `credential_pool_map` держит `Weak<dyn RotatablePool>`, не `Arc`, чтобы не мешать shutdown.
6. **Ошибка авторизации = ошибка создания** — если callback вернул `Err`, инстанс не попадает в пул и не возвращается вызывающему.
