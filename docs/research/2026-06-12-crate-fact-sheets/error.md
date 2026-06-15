# nebula-error — fact sheet

## Назначение
Фундаментный крейт ошибок workspace: единая таксономия (категория/код/severity/retryability)
через трейт `Classify`, generic-обёртка `NebulaError<E>` с TypeId-типизированными деталями и
context chain, и `RetryHint` как данные для `nebula-resilience`. Делает transient-vs-permanent
явным решением (паттерн ErrorClassifier, канон §4.2, инвариант L2-§12.4).

## Публичная поверхность
- `Classify` (trait) — category()/code() обязательны; severity()/is_retryable()/retry_hint() с дефолтами — src/traits.rs:45
- `ErrorClassifier` — предикат по `ErrorCategory`; builders `retryable()/client_errors()/server_errors()` — src/traits.rs:98
- `NebulaError<E: Classify>` — обёртка: `new/with_message/with_source/with_detail/context/map_inner` — src/error.rs:61
- `NebulaError::context_chain()` / `details()` / `source()` — src/error.rs:268,256,273; Display обязан печатать всю context chain (regression-fix #405)
- `ErrorCategory` — 14 вариантов, `#[non_exhaustive]` (NotFound…DataTooLarge) — src/category.rs:24
- `ErrorCategory::is_default_retryable()` — Timeout|Exhausted|External|RateLimit|Unavailable — src/category.rs:73
- `ErrorCategory::http_status_code()` / `from_http_status()` — двусторонний HTTP-маппинг — src/convert.rs:39,76
- `ErrorCode` — newtype `Cow<'static,str>`, `const fn new` + `custom()` — src/code.rs:23
- `codes::*` — 14 предопределённых констант (NOT_FOUND…DATA_TOO_LARGE) — src/code.rs:129-155
- `ErrorSeverity` — Error/Warning/Info — src/severity.rs:20
- `RetryHint` — `after`/`max_attempts`, данные не исполнение — src/retry.rs:23
- `ErrorDetail` (marker trait: Any+Send+Sync+Debug) + `ErrorDetails` (TypeId-keyed map: insert/get/remove) — src/details.rs:31,52
- 13 готовых detail-структур: `BadRequest, FieldViolation, ResourceInfo, RequestInfo, QuotaInfo, PreconditionFailure, ExecutionContext, ErrorRoute, TypeMismatch, HelpLink, DebugInfo, DependencyInfo, PreconditionViolation` — src/detail_types.rs
- `ErrorCollection<E>` + `BatchResult<T,E>` — агрегация batch/validation ошибок; `any_retryable/max_severity/uniform_category` — src/collection.rs:47,269
- `type Result<T, E> = Result<T, NebulaError<E>>` — src/lib.rs:67
- `#[derive(Classify)]` — proc-macro из sibling `nebula-error-macros` (feature `derive`) — macros/src/lib.rs:57

## Workspace-зависимости
- Deps: НЕТ nebula-зависимостей (foundation). Опционально: `serde` (feature `serde`), `nebula-error-macros` (path = macros, feature `derive`). Обе фичи off by default.
- Dev-deps: serde_json, thiserror, insta, pretty_assertions, rstest, nebula-error-macros.
- Зависят от него (Cargo.toml grep, 16 крейтов): action, api, core, credential, crypto, engine, execution, expression, log, metadata, metrics, plugin, resilience, resource, validator, workflow. Фактически весь workspace.

## Структура модулей
- `lib.rs` — re-exports + `Result` alias, модульный гейт (71 строка)
- `traits.rs` — `Classify` + `ErrorClassifier` (центральный seam L2-§12.4)
- `error.rs` — `NebulaError<E>`: message/source/details/context chain (523 строки, крупнейший)
- `category.rs` — `ErrorCategory` enum + предикаты retryable/client/server
- `code.rs` — `ErrorCode` newtype + модуль `codes`
- `convert.rs` — HTTP status маппинг (двусторонний)
- `severity.rs` — `ErrorSeverity`
- `retry.rs` — `RetryHint` (только данные)
- `details.rs` — TypeId-keyed `ErrorDetails` контейнер
- `detail_types.rs` — 13 prebuilt detail-структур (Google error model / AWS SDK style)
- `collection.rs` — `ErrorCollection` / `BatchResult`
- `macros/` — sibling proc-macro crate `nebula-error-macros` (362 строки)
- `tests/` — derive.rs, serde.rs (integration)

## Напряжения
- Доки derive-макроса перечисляют только 12 категорий («…`unsupported`») — macros/src/lib.rs:40-41, но парсер принимает ещё `unavailable` и `data_too_large` — macros/src/lib.rs:258-259. Drift док vs код.
- convert.rs:4 обещает «protocol bridges (gRPC, etc.) behind feature flags» — фич нет, заявление аспирационное.
- TODO/FIXME/deprecated/shims — НЕТ (grep чистый). Крейт компактный и согласованный, README/AGENTS соответствуют коду.
- Никакого `ValidationError` в крейте нет — канонический `nebula-error::ValidationError` существует только на unmerged ветке `refactor/error-unify-validation`, не в этом worktree.

## Роль в credential/resource redesign
Напрямую не затронут — это стабильный фундамент, от которого credential/crypto/resource зависят.
Косвенно: (1) error-unify ветка (unmerged) добавит сюда канонический ValidationError;
(2) `RetryHint`/`ErrorCategory` — словарь, которым credential rotation и resource teardown
(ADR-0093) классифицируют transient-vs-permanent; сам API стабилен, breaking changes не планируются (README: status stable).
