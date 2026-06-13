# nebula-log — fact sheet

## Назначение
Единая инициализация `tracing`-подписчика для всех бинарей Nebula: один вызов `auto_init`/`init`/`init_with`
настраивает формат (pretty/compact/json/logfmt), writer-бэкенды (stderr/stdout/file/fanout), структурные поля
(service/env/version/instance/region), runtime-reload уровня и опциональные OTLP/Sentry. Cross-cutting слой,
без upward-зависимостей. Сам секреты НЕ редактирует — контракт §12.5 на стороне вызывающих (AGENTS.md:21).

## Публичная поверхность
- `auto_init()` — zero-config, идемпотентен (no-op guard при повторном init, #379) — src/lib.rs:205
- `init()` / `init_with(Config)` — src/lib.rs:240 / src/lib.rs:259
- `LoggerBuilder` (`build_startup`: explicit > env > preset) — src/builder/mod.rs:32, :104
- `LoggerGuard` — RAII, держать всю жизнь процесса — src/builder/mod.rs:41
- `ReloadHandle::reload(&str)` — runtime-смена фильтра — src/builder/reload.rs:15
- `watch_config` / `WatcherGuard` (feature `async`) — src/builder/watcher.rs:43, :22
- `Config` + пресеты `development()/production()/test()/from_env()` — src/config/base.rs, src/config/presets.rs:8-49
- `Format`, `Level`, `WriterConfig` (Stderr/Stdout/File/Fanout), `Rolling`, `DestinationFailurePolicy` — src/config/writer.rs:10-95
- `Fields` (service/env/version/...) — src/config/fields.rs:7
- `LogError`, `LogResult`, `LogResultExt` — src/core/error.rs:11, src/core/result.rs:6
- `Timer`/`TimerGuard`/`Timed` (замер длительности) — src/timing.rs:14, :78, :100
- `Context`, `Fields` (task-local контекст-пропагация) — src/layer/context.rs:86
- observability: `ObservabilityEvent`/`ObservabilityHook`/`LoggingHook`, `emit_event`/`register_hook`/`set_hook_policy`/`shutdown_hooks` — src/observability/hooks.rs, registry.rs:84
- observability: `OperationStarted/Completed/Failed`, `OperationTracker` — src/observability/events.rs
- observability: `GlobalContext`/`ExecutionContext`/`NodeContext`/`ResourceMap`, `LoggerResource`/`ResourceAwareHook` — src/observability/context.rs:60, resources.rs:96
- `HookPolicy` (Inline | Bounded{timeout_ms, queue_capacity}) — src/observability/mod.rs:60
- telemetry: `TelemetryConfig` (feature `telemetry`), `OtelLayer`/`build_layer` — src/telemetry/otel.rs:24, :91; sentry::init — src/telemetry/sentry.rs:10
- re-export `tracing::{debug,error,info,instrument,span,trace,warn}` + `prelude` — src/lib.rs:182, :167

## Workspace-зависимости
- Зависит из workspace: только `nebula-error` (features=["derive"]) — Cargo.toml:34. Остальное внешнее: tracing/tracing-subscriber, tokio (opt), tracing-appender+flate2 (opt `file`), opentelemetry* (opt `telemetry`), sentry 0.48 rustls-only (opt), serde, time, arc-swap, parking_lot, smallvec, pin-project.
- От него зависит ТОЛЬКО `nebula-expression` (path-dep, crates/expression/Cargo.toml:16) — и использует исключительно ре-экспортированные макросы `trace!`/`debug!` (expression/src/{engine,parser,template}.rs).
- nebula-log ОТСУТСТВУЕТ в root workspace.dependencies — подключается только path-депом.
- Features: default=[ansi,async]; file; log-compat; telemetry; sentry; full.

## Структура модулей
- `lib.rs` — entry points + идемпотентный init (#379 TOCTOU-обработка)
- `builder/` — mod.rs (LoggerBuilder/Guard/слойка через макрос build_subscriber!), format.rs+telemetry.rs (макросы), reload.rs (ReloadHandle), watcher.rs (file-watch reload, async)
- `config/` — base.rs (Config/Format/Level/TelemetryConfig), env.rs (ResolvedConfig/Source), fields.rs, presets.rs, writer.rs
- `core/` — error.rs (LogError thiserror), result.rs (LogResultExt/LogIoResultExt)
- `format.rs` — LogfmtFormatter + make_timer (timestamp-формат)
- `layer/context.rs` — task-local/thread-local Context (request_id/user_id/поля)
- `macros.rs` — timing-макросы
- `observability/` — самый крупный поджирающий блок (~2,9k строк): events, hooks, registry (global hook registry + HookPolicy), context (Global/Execution/NodeContext), resources (LoggerResource), filter, semantic (EventKind), span
- `telemetry/` — otel.rs (OTLP layer, endpoint-резолюция, opt-out "disabled"), sentry.rs
- `writer.rs` — make_writer: stderr/stdout/file/fanout c failure policy
- tests/ — api_contract, config_*, hook_policy, init_hardening, writer_fanout; benches/log_hot_path.rs; 15 examples

## Напряжения
- **Дубль OTLP-инициализации**: `crates/api/src/telemetry_init.rs` зеркалирует логику `nebula_log::telemetry::otel` (комменты :51, :232, :272 явно ссылаются «mirrors the nebula_log edge») вместо использования feature `telemetry` — split-brain, две точки правды для endpoint-резолюции/shutdown.
- **README vs код**: README.md:73 описывает feature `observability`, которой нет в Cargo.toml (модуль observability безусловный).
- **Единственный потребитель — ради макросов**: nebula-expression тянет весь крейт только ради `trace!/debug!` (мог бы использовать `tracing` напрямую); остальные крейты используют `tracing` напрямую и `nebula-log` не подключают, т.е. «single logging pipeline» как библиотечный контракт фактически не потребляется из workspace-кода (init — в бинарях/тестах).
- **Два `Timer`**: `src/format.rs:21` (enum, timestamp-формат) и `src/timing.rs:14` (struct, замер длительности) — коллизия имён внутри крейта; публично экспортируется только timing::Timer.
- Deprecated: `src/observability/semantic.rs:27` (константа переименована в NODE_KEY), `src/observability/registry.rs:97` (`#[deprecated]` функция, оставлена «for semver compatibility» — при этом крейт приватный, semver-довод спорный).
- `observability/resources.rs` (`LoggerResource`, `ResourceAwareHook`, `ResourceMap`) — собственное понятие «resource» (sentry_dsn/webhook/sampling per-resource), не связанное с nebula-resource; терминологическое наложение + examples (`resource_based_observability.rs`, `span_like_resources.rs`) закрепляют его.

## Роль в credential/resource redesign
Крейт напрямую НЕ затронут redesign'ом (cross-cutting, без deps на credential/resource). Косвенные стыки:
(1) контракт §12.5 «no secrets in logs» — enforcement на call-sites credential-кода, nebula-log лишь даёт pipeline;
(2) `LoggerResource`/`ResourceAwareHook` в observability/ — легаси-«resource»-модель, не проходящая через
nebula-resource; при unified in-process registry pivot это кандидат на ревизию/сведение;
(3) feedback_observability_as_completion (trace span как DoD) опирается на `tracing` напрямую, не на хуки этого крейта.
