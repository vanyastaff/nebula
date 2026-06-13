# nebula-log — design

| Field | Value |
|-------|-------|
| **Status** | Stable — cross-cutting init layer (не затронут redesign'ом) |
| **Layer** | Cross-cutting (leaf; единственная workspace-зависимость — `nebula-error`) |
| **Redesign role** | **Not touched** — без deps на credential/resource; даёт только `tracing`-pipeline. Касание чисто косвенное (см. §7) |
| **Related** | PRODUCT_CANON §12.5 (no secrets in logs), AGENTS.md:21, `feedback_observability_as_completion` |

---

## 1. Назначение и границы

`nebula-log` — **единая инициализация `tracing`-подписчика** для всех бинарей Nebula.
Один вызов (`auto_init` / `init` / `init_with`) настраивает формат (pretty/compact/json/logfmt),
writer-бэкенды (stderr/stdout/file/fanout), структурные поля (service/env/version/instance/region),
runtime-reload уровня и опциональные OTLP/Sentry.

**Владеет:** entry points инициализации с идемпотентным no-op guard (#379, TOCTOU-обработка),
`LoggerBuilder`/`LoggerGuard` (RAII на всю жизнь процесса), `Config` + пресеты, форматтеры
(вкл. собственный `LogfmtFormatter`), writer-фабрику с failure policy, runtime-reload (`ReloadHandle`,
file-watch через feature `async`), task-local контекст-пропагацию, timing-утилиты, и подсистему
`observability/` (events/hooks/registry/context).

**ЯВНО НЕ делает:** не редактирует секреты — контракт §12.5 «no secrets in logs» enforced на стороне
вызывающих (AGENTS.md:21), `nebula-log` лишь предоставляет pipeline. Не имеет upward-зависимостей.

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `auto_init()` (zero-config, идемпотентен) | `src/lib.rs:205` |
| `init()` / `init_with(Config)` | `src/lib.rs:240` / `:259` |
| `LoggerBuilder` (`build_startup`: explicit > env > preset) | `src/builder/mod.rs:32`, `:104` |
| `LoggerGuard` (RAII) | `src/builder/mod.rs:41` |
| `ReloadHandle::reload(&str)` (runtime-смена фильтра) | `src/builder/reload.rs:15` |
| `watch_config` / `WatcherGuard` (feature `async`) | `src/builder/watcher.rs:43`, `:22` |
| `Config` + `development()/production()/test()/from_env()` | `src/config/base.rs`, `src/config/presets.rs:8-49` |
| `Format`, `Level`, `WriterConfig`, `Rolling`, `DestinationFailurePolicy` | `src/config/writer.rs:10-95` |
| `Fields` (service/env/version/...) | `src/config/fields.rs:7` |
| `LogError`, `LogResult`, `LogResultExt` | `src/core/error.rs:11`, `src/core/result.rs:6` |
| `Timer`/`TimerGuard`/`Timed` (замер длительности) | `src/timing.rs:14`, `:78`, `:100` |
| `Context` (task-local контекст-пропагация) | `src/layer/context.rs:86` |
| `ObservabilityEvent`/`ObservabilityHook`/`LoggingHook`, `emit_event`/`register_hook`/`set_hook_policy`/`shutdown_hooks` | `src/observability/hooks.rs`, `registry.rs:84` |
| `OperationStarted/Completed/Failed`, `OperationTracker` | `src/observability/events.rs` |
| `GlobalContext`/`ExecutionContext`/`NodeContext`/`ResourceMap`, `LoggerResource`/`ResourceAwareHook` | `src/observability/context.rs:60`, `resources.rs:96` |
| `HookPolicy` (`Inline` \| `Bounded{timeout_ms, queue_capacity}`) | `src/observability/mod.rs:60` |
| `TelemetryConfig`, `OtelLayer`/`build_layer` (feature `telemetry`); `sentry::init` (feature `sentry`) | `src/telemetry/otel.rs:24`, `:91`; `sentry.rs:10` |
| re-export `tracing::{debug,error,info,instrument,span,trace,warn}` + `prelude` | `src/lib.rs:182`, `:167` |

## 3. Зависимости и зависимые

- **Workspace-deps:** только `nebula-error` (features=`["derive"]`, `Cargo.toml:34`). Прочее внешнее:
  `tracing`/`tracing-subscriber`, `tokio` (opt), `tracing-appender`+`flate2` (opt `file`),
  `opentelemetry*` (opt `telemetry`), `sentry` 0.48 rustls-only (opt), `serde`, `time`, `arc-swap`,
  `parking_lot`, `smallvec`, `pin-project`.
- **Dependents:** ТОЛЬКО `nebula-expression` (path-dep, `crates/expression/Cargo.toml:16`) — и лишь ради
  ре-экспортированных макросов `trace!`/`debug!`. `nebula-log` отсутствует в root `workspace.dependencies`
  (подключается только path-депом).
- **Features:** `default=[ansi,async]`; `file`; `log-compat`; `telemetry`; `sentry`; `full`.

## 4. Внутренняя архитектура

- `lib.rs` — entry points + идемпотентный init (#379).
- `builder/` — `LoggerBuilder`/`Guard` (слойка через макрос `build_subscriber!`), `format.rs`+`telemetry.rs`
  (макросы), `reload.rs` (`ReloadHandle`), `watcher.rs` (file-watch reload, async).
- `config/` — `base.rs` (`Config`/`Format`/`Level`/`TelemetryConfig`), `env.rs` (`ResolvedConfig`/`Source`),
  `fields.rs`, `presets.rs`, `writer.rs`.
- `core/` — `error.rs` (`LogError`, thiserror), `result.rs` (`LogResultExt`/`LogIoResultExt`).
- `format.rs` — `LogfmtFormatter` + `make_timer`. `writer.rs` — `make_writer` (stderr/stdout/file/fanout
  с failure policy). `layer/context.rs` — task-/thread-local `Context`. `macros.rs` — timing-макросы.
- `observability/` — крупнейший блок (~2.9k строк): events, hooks, registry (global hook registry +
  `HookPolicy`), context, resources, filter, semantic (`EventKind`), span.
- `telemetry/` — `otel.rs` (OTLP layer, endpoint-резолюция, opt-out `"disabled"`), `sentry.rs`.
- Поток данных: `Config` (explicit > env > preset) → `build_subscriber!` собирает layer-stack
  (format + writer + reload + опц. otel/sentry) → `LoggerGuard` держит flush до конца процесса;
  события идут в `tracing`, хуки `observability` получают их параллельно по `HookPolicy`.

## 5. Инварианты и контракты

- **Идемпотентность init** — повторный `init` = no-op guard (#379), TOCTOU обработан → один глобальный
  подписчик на процесс by-construction.
- **No-secrets (§12.5)** — НЕ by-construction здесь: enforcement на call-sites credential-кода; крейт даёт
  только pipeline (AGENTS.md:21).
- **Bounded hook policy** — `HookPolicy::Bounded{timeout_ms, queue_capacity}` ограничивает влияние медленных
  хуков на hot path; `Inline` — синхронно.
- **Reload-семантика** — `ReloadHandle::reload(&str)` меняет фильтр атомарно (arc-swap), без пересоздания
  подписчика.
- **RAII flush** — `LoggerGuard` обязан жить весь процесс; дроп раньше времени теряет буферизованный вывод
  (file/fanout).

## 6. Известные напряжения / долг

1. **Дубль OTLP-инициализации.** `crates/api/src/telemetry_init.rs` зеркалирует логику
   `nebula_log::telemetry::otel` (комменты `:51`, `:232`, `:272` — «mirrors the nebula_log edge») вместо
   feature `telemetry` → split-brain, две точки правды для endpoint-резолюции/shutdown.
2. **README vs код.** `README.md:73` описывает feature `observability`, которой нет в `Cargo.toml`
   (модуль observability безусловный).
3. **Единственный потребитель — ради макросов.** `nebula-expression` тянет весь крейт лишь ради
   `trace!`/`debug!` (мог бы использовать `tracing` напрямую); остальные крейты используют `tracing`
   напрямую и `nebula-log` не подключают. «Single logging pipeline» как библиотечный контракт фактически
   не потребляется из workspace-кода (init живёт в бинарях/тестах).
4. **Два `Timer`.** `src/format.rs:21` (enum, timestamp-формат) и `src/timing.rs:14` (struct, замер
   длительности) — коллизия имён внутри крейта; публично экспортируется только `timing::Timer`.
5. **Deprecated.** `src/observability/semantic.rs:27` (переименована в `NODE_KEY`) и
   `src/observability/registry.rs:97` (`#[deprecated]` «for semver compatibility») — semver-довод спорен,
   крейт приватный.
6. **Терминологическое наложение «resource».** `observability/resources.rs` (`LoggerResource`,
   `ResourceAwareHook`, `ResourceMap`) — собственное понятие «resource» (sentry_dsn/webhook/sampling
   per-resource), не связанное с `nebula-resource`; examples (`resource_based_observability.rs`,
   `span_like_resources.rs`) закрепляют коллизию.

## 7. Роль в пост-0092 credential/resource модели

**Не затронут.** Стабильный cross-cutting фундамент без deps на credential/resource; redesign его не
касается. Косвенные стыки: (1) §12.5 «no secrets in logs» — enforcement на call-sites credential-кода,
крейт лишь даёт pipeline; (2) `LoggerResource`/`ResourceAwareHook` — легаси-«resource»-модель, не
проходящая через `nebula-resource`, кандидат на ревизию при unified in-process registry pivot;
(3) `feedback_observability_as_completion` (trace span как DoD) опирается на `tracing` напрямую, не на хуки
этого крейта.

## 8. Forward design / открытые вопросы

Крейт стабилен; крупных изменений не планируется. Открытые вопросы — расшивка напряжений §6, не новая
функциональность:
- свести OTLP-инициализацию `api` к feature `telemetry` (закрыть split-brain §6.1);
- развести имена `Timer` и термин «resource» в `observability/` (§6.4, §6.6) — либо документировать как
  заведомо несвязанные с `nebula-resource`;
- синхронизировать README с реальными features (§6.2);
- решить судьбу `#[deprecated]`-точек в приватном крейте (§6.5).
