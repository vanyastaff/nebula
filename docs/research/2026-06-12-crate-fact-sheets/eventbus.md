# nebula-eventbus — fact sheet

## Назначение
Транспортный (transport-only) generic pub/sub: один `EventBus<E>` поверх `tokio::sync::broadcast` —
bounded, in-process, ephemeral, best-effort, с back-pressure политикой и Lagged-recovery.
Доменные крейты сами владеют типами событий (`EventBus<ExecutionEvent>` и т.п.); здесь НЕТ ни одного
доменного event-типа. Не durable: cancel/dispatch идут через `execution_control_queue`, не сюда.

## Публичная поверхность
- `EventBus<E>` — src/bus.rs:42; `new(buffer_size)` / `with_policy` (panic при 0), `Default`=1024 (bus.rs:267)
- `EventBus::emit()` — bus.rs:95; неблокирующий hot-path: 1 broadcast::send + relaxed-атомики, без аллокаций
- `EventBus::emit_awaited()` — bus.rs:155; уважает `Block{timeout}` (poll+экспон. backoff 50us..1ms, bus.rs:169)
- `EventBus::subscribe()` / `subscribe_filtered()` / `subscribe_scoped()` — bus.rs:215/223/231
- `EventBus::stats()` / `has_subscribers()` / `pending_len()` / `buffer_size()` / `policy()` — bus.rs:240–264
- `Subscriber<E>` — src/subscriber.rs:55; `recv()` (None при закрытии), `try_recv()`, `lagged_count()`, `is_closed()`, `into_stream()`
- `BackPressurePolicy` — src/policy.rs:11; `DropOldest` (default) / `DropNewest` / `Block{timeout}`; non_exhaustive
- `PublishOutcome` — src/outcome.rs:6; `Sent` / `DroppedNoSubscribers` / `DroppedByPolicy` / `DroppedTimeout`; `is_sent()`
- `EventBusStats` — src/stats.rs:25; sent/dropped/subscriber_count + `total_attempts()`, `drop_ratio()`
- `EventFilter<E>` — src/filter.rs:9; `all()` / `custom(pred)` / `by_scope(scope)`, Arc<dyn Fn>
- `FilteredSubscriber<E>` — src/filtered_subscriber.rs:21; recv/try_recv с предикатом
- `SubscriptionScope` (Global/Workflow/Execution/Resource) + trait `ScopedEvent` — src/scope.rs:6/41
- `EventBusRegistry<K,E>` + `EventBusRegistryStats` — src/registry.rs:36/11; мульти-бас по ключу (per-tenant), `get_or_create` (double-checked RwLock), `prune_without_subscribers` (best-effort)
- `SubscriberStream<E>` / `FilteredStream<E>` (futures_core::Stream) — src/stream.rs:37/87
- `prelude` — src/prelude.rs; реэкспорт всего публичного

## Workspace-зависимости
Deps (Cargo.toml:14-18): tokio(sync), parking_lot, futures-core, tokio-stream(sync) — **ноль nebula-* deps**
(самая чистая граница слоя; deny.toml-обёртки запрещают добавлять).
Dev-deps: criterion (2 бенча: emit, throughput), insta, rstest, pretty_assertions, tracing-subscriber.
Кто зависит (Grep по crates/*/Cargo.toml): **nebula-credential** (credential/Cargo.toml:48),
**nebula-engine** (engine/Cargo.toml:35), **nebula-api** (api/Cargo.toml:42),
**nebula-resource** (resource/Cargo.toml:25), **nebula-metrics** (metrics/Cargo.toml:16).

## Структура модулей
- `src/lib.rs` — crate-docs (контракт, lag-семантика) + wiring + канонические pub use (lib.rs:146-155)
- `src/bus.rs` — EventBus: emit/emit_awaited, политики, статистика, фабрики подписок (+~25 unit-тестов)
- `src/subscriber.rs` — Subscriber: recv-loop с поглощением `RecvError::Lagged`, атрибуция лага в shared `dropped_count` (issue #262)
- `src/policy.rs` — enum BackPressurePolicy (3 варианта)
- `src/outcome.rs` — enum PublishOutcome
- `src/stats.rs` — EventBusStats; документирована двухсигнальная семантика dropped_count (emit-time + recv-time lag)
- `src/registry.rs` — EventBusRegistry: HashMap<K, Arc<EventBus>> под parking_lot::RwLock
- `src/scope.rs` — SubscriptionScope + trait ScopedEvent (matches_scope)
- `src/filter.rs` — EventFilter: Arc<dyn Fn(&E)->bool>
- `src/filtered_subscriber.rs` — FilteredSubscriber поверх Subscriber
- `src/stream.rs` — Stream-адаптеры поверх BroadcastStream, лаг тоже учитывается
- `src/prelude.rs` — реэкспорты; tests/integration.rs + tests/helpers.rs; benches/{emit,throughput}.rs; examples/subscriber_patterns.rs

## Напряжения
- **Alias-имена в доках ≠ реальные экспорты**: lib.rs:25-32 и README.md:35-40 описывают API как
  `Outcome`, `Filter<E>`, `Registry`, `Scope`, `Stats` — реальные имена `PublishOutcome`, `EventFilter`,
  `EventBusRegistry`, `SubscriptionScope`, `EventBusStats` (lib.rs:146-155). AGENTS.md:13 фиксирует
  «lib.rs authoritative over README's shorter aliases», но lib.rs:25-32 сам использует те же алиасы.
- **README выдумывает вариант `Lagged`**: README.md:37 и lib.rs:29 — «`Outcome` (`Sent`, `NoSubscribers`,
  `Lagged`, …)»; в `PublishOutcome` (outcome.rs:6-15) варианта `Lagged` НЕТ, а `NoSubscribers`
  на деле `DroppedNoSubscribers`.
- **Plan-phase язык в коде**: lib.rs:100-108 «in-memory only in Phase 2… Persistence planned for Phase 3» —
  противоречит правилу «no plan IDs in committed code» и README Non-goals (README.md:50 «persistence out of scope»).
- **Стейл-метрики в README**: README.md:58 «3 unit tests and 2 integration tests» — фактически в одном
  bus.rs ~25 тестов; last-reviewed 2026-04-17 (README.md:5).
- **Discipline-trap**: фильтр, не матчящий ничего, крутит `recv()` бесконечно — задокументировано как
  anti-pattern warning (filtered_subscriber.rs:16-19), структурно не закрыто.
- **`Block` в sync `emit()` молча ведёт себя как DropOldest** (bus.rs:97, policy.rs:26-27) — задокументировано,
  но вызывающий легко промахнётся мимо `emit_awaited`.
- dropped_count = сумма per-subscriber лагов (N подписчиков × 1 событие = N drops) — намеренно,
  задокументировано (stats.rs:8-23), но при чтении метрик легко неверно интерпретировать.

## Роль в credential/resource redesign
Крейт сам по себе redesign-ом не затронут (transport-only, zero deps, stable). Но он — канал fan-out:
nebula-credential/engine используют его для `CredentialEvent` (rotation/revoke fan-out ADR-0067 wired),
nebula-resource и nebula-api зависят напрямую; `EventBusRegistry` — готовый примитив per-tenant изоляции.
Известный долг по памяти проекта: `ExecutionEvent` в engine всё ещё на raw mpsc — миграция на eventbus
нужна для multi-subscriber (project_eventbus_status).
