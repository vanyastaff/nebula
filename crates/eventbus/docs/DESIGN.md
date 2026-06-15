# nebula-eventbus — design

| Field | Value |
|-------|-------|
| **Status** | Stable — transport-only leaf primitive |
| **Layer** | Cross-cutting (leaf; **ноль** `nebula-*` зависимостей) |
| **Redesign role** | **Не затронут** post-0092 — стабильный фундамент. Служит каналом fan-out (`CredentialEvent`), но сам не содержит ни одного доменного типа. |
| **Related** | ADR-0067 (rotation/revoke fan-out wired через эту шину), `project_eventbus_status`, issue #262 (атрибуция Lagged) |

---

## 1. Назначение и границы

`nebula-eventbus` — **транспортный** generic pub/sub: один `EventBus<E>` поверх `tokio::sync::broadcast`, bounded, in-process, ephemeral, best-effort, с back-pressure-политикой и Lagged-recovery.

**Владеет:** generic-механикой доставки — emit/subscribe, выбором back-pressure-политики, статистикой доставки, фильтрацией и скоупингом подписок, мульти-шинным реестром по ключу, Stream-адаптерами.

**ЯВНО НЕ делает:** не владеет **ни одним** доменным event-типом (доменные крейты держат `EventBus<ExecutionEvent>` и т.п. у себя); не durable — persistence вне области; не транспорт для команд (cancel/dispatch идут через `execution_control_queue`, не сюда); не тянет ни одной `nebula-*` зависимости (deny.toml-обёртки это запрещают — самая чистая граница слоя).

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `EventBus<E>` — `new(buffer_size)` (panic при 0) / `with_policy`; `Default`=1024 | `src/bus.rs:42`, `:267` |
| `EventBus::emit()` — неблокирующий hot-path (1 broadcast::send + relaxed-атомики, без аллокаций) | `src/bus.rs:95` |
| `EventBus::emit_awaited()` — уважает `Block{timeout}` (poll + экспон. backoff 50us..1ms) | `src/bus.rs:155`, `:169` |
| `subscribe()` / `subscribe_filtered()` / `subscribe_scoped()` | `src/bus.rs:215/223/231` |
| `stats()` / `has_subscribers()` / `pending_len()` / `buffer_size()` / `policy()` | `src/bus.rs:240..264` |
| `Subscriber<E>` — `recv()` (None при закрытии) / `try_recv()` / `lagged_count()` / `is_closed()` / `into_stream()` | `src/subscriber.rs:55` |
| `BackPressurePolicy` (`#[non_exhaustive]`) — `DropOldest` (default) / `DropNewest` / `Block{timeout}` | `src/policy.rs:11` |
| `PublishOutcome` — `Sent` / `DroppedNoSubscribers` / `DroppedByPolicy` / `DroppedTimeout`; `is_sent()` | `src/outcome.rs:6` |
| `EventBusStats` — sent/dropped/subscriber_count + `total_attempts()` / `drop_ratio()` | `src/stats.rs:25` |
| `EventFilter<E>` — `all()` / `custom(pred)` / `by_scope(scope)` (Arc<dyn Fn>) | `src/filter.rs:9` |
| `FilteredSubscriber<E>` — recv/try_recv с предикатом | `src/filtered_subscriber.rs:21` |
| `SubscriptionScope` (Global/Workflow/Execution/Resource) + trait `ScopedEvent` | `src/scope.rs:6/41` |
| `EventBusRegistry<K,E>` + `EventBusRegistryStats` — `get_or_create` (double-checked RwLock), `prune_without_subscribers` | `src/registry.rs:36/11` |
| `SubscriberStream<E>` / `FilteredStream<E>` (`futures_core::Stream`) | `src/stream.rs:37/87` |
| `prelude` — реэкспорт всего публичного | `src/prelude.rs` |

## 3. Зависимости и зависимые

- **Deps** (`Cargo.toml:14-18`): `tokio` (sync), `parking_lot`, `futures-core`, `tokio-stream` (sync) — **ноль** `nebula-*`.
- **Dev-deps:** `criterion` (бенчи emit/throughput), `insta`, `rstest`, `pretty_assertions`, `tracing-subscriber`.
- **Зависимые:** `nebula-credential` (`:48`), `nebula-engine` (`:35`), `nebula-api` (`:42`), `nebula-resource` (`:25`), `nebula-metrics` (`:16`).

## 4. Внутренняя архитектура

- `src/bus.rs` — `EventBus`: emit/emit_awaited, выбор политики, статистика, фабрики подписок.
- `src/subscriber.rs` — recv-loop, поглощающий `RecvError::Lagged` и атрибутирующий лаг в shared `dropped_count` (issue #262).
- `src/policy.rs` / `src/outcome.rs` — enum back-pressure-политики и результата публикации.
- `src/stats.rs` — двухсигнальная семантика `dropped_count` (emit-time + recv-time lag).
- `src/registry.rs` — `HashMap<K, Arc<EventBus>>` под `parking_lot::RwLock`.
- `src/scope.rs` / `src/filter.rs` / `src/filtered_subscriber.rs` — скоуп, предикат-фильтр, фильтрующая обёртка над `Subscriber`.
- `src/stream.rs` — Stream-адаптеры поверх `BroadcastStream` (лаг тоже учитывается).
- `src/lib.rs` — crate-docs (контракт, lag-семантика) + канонические pub use (`:146-155`).

Поток данных: `emit` → `broadcast::send` → веер `Subscriber`'ов; лаг (медленный потребитель переполнил буфер) поглощается на recv-стороне и отражается в статистике, а не роняет подписчика.

## 5. Инварианты и контракты

- **Transport-only / zero domain.** В крейте нет доменных типов и нет `nebula-*` зависимостей by-construction (граница защищена deny.toml).
- **Bounded by-construction.** `new(0)` паникует; буфер фиксирован → нет неограниченного роста памяти.
- **Best-effort, не durable.** Доставка ephemeral; back-pressure всегда разрешается выбранной политикой, а не блокировкой shutdown.
- **Lag survivable.** `RecvError::Lagged` никогда не закрывает подписку — поглощается и учитывается в `dropped_count` (issue #262).
- **Per-tenant изоляция.** `EventBusRegistry<K,E>` даёт независимую шину на ключ — готовый примитив изоляции арендаторов; `get_or_create` потокобезопасен (double-checked locking).

## 6. Известные напряжения / долг

1. **Alias-имена в доках ≠ реальные экспорты.** `lib.rs:25-32` и `README.md:35-40` описывают API как `Outcome` / `Filter<E>` / `Registry` / `Scope` / `Stats`; реальные — `PublishOutcome` / `EventFilter` / `EventBusRegistry` / `SubscriptionScope` / `EventBusStats` (`lib.rs:146-155`). `AGENTS.md:13` объявляет lib.rs авторитетным, но `lib.rs:25-32` сам использует те же алиасы.
2. **README выдумывает вариант `Lagged`.** `README.md:37` / `lib.rs:29` упоминают `Lagged` в `Outcome`; в `PublishOutcome` (`outcome.rs:6-15`) такого варианта нет, а `NoSubscribers` на деле `DroppedNoSubscribers`.
3. **Plan-phase язык в коде.** `lib.rs:100-108` — «in-memory only in Phase 2… Persistence planned for Phase 3»; нарушает правило «no plan IDs in committed code» и противоречит README Non-goals (`README.md:50`).
4. **Стейл-метрики в README.** `README.md:58` — «3 unit tests and 2 integration tests»; фактически в одном `bus.rs` ~25 тестов; last-reviewed `2026-04-17` (`README.md:5`).
5. **Discipline-trap.** Фильтр, не матчащий ничего, крутит `recv()` бесконечно — задокументировано как anti-pattern (`filtered_subscriber.rs:16-19`), но структурно не закрыто.
6. **`Block` в sync `emit()` молча ведёт себя как `DropOldest`** (`bus.rs:97`, `policy.rs:26-27`) — задокументировано, но вызывающий легко промахнётся мимо `emit_awaited`.
7. **`dropped_count` = сумма per-subscriber лагов** (N подписчиков × 1 событие = N drops) — намеренно (`stats.rs:8-23`), но при чтении метрик легко неверно интерпретировать.

## 7. Роль в пост-0092 credential/resource модели

Крейт redesign-ом **не затронут**: transport-only, zero deps, stable. Он лишь **канал** fan-out — `nebula-credential` и `nebula-engine` гоняют через него `CredentialEvent` (rotation/revoke fan-out по ADR-0067, wired), а `nebula-resource`/`nebula-api` зависят напрямую. `EventBusRegistry` остаётся готовым примитивом per-tenant изоляции для consumer-binding-слоя. Доменные типы и маршрутизация живут на стороне потребителей — eventbus только переносит байты.

## 8. Forward design / открытые вопросы

Крейт стабилен; целевых изменений в нём не планируется. Долг — преимущественно документационный (пп. 1–4 §6: выровнять alias-имена и убрать plan-phase язык из `lib.rs`). Единственный реальный архитектурный долг **вне** этого крейта: `ExecutionEvent` в `nebula-engine` всё ещё на raw `mpsc` — миграция на eventbus нужна для multi-subscriber (`project_eventbus_status`); это работа в engine, не здесь.
