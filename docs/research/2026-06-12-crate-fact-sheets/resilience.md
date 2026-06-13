# nebula-resilience — fact sheet

## Назначение
In-process слой устойчивости для исходящих вызовов внутри actions: композируемые паттерны
(retry, circuit breaker, bulkhead, rate limiter, timeout, hedge, load shed, fallback),
собираемые в `ResiliencePipeline<E>`. Фильтрация retry управляется `nebula-error::Classify::retry_hint()`.
По канону §11.2 это ЕДИНСТВЕННАЯ retry-поверхность стека (engine не пере-исполняет ноды). Статус: stable.

## Публичная поверхность
- `ResiliencePipeline<E>` / `PipelineBuilder<E>` — src/pipeline.rs:492 / :109; builder-шаги timeout/retry/circuit_breaker/bulkhead/rate_limiter/load_shed; `build_checked()` / `build_recommended_order()`; `call_with_policy_context[_and_fallback]()`
- `CallError<E>` (#[non_exhaustive], без type erasure) — src/error.rs:27; `CallErrorKind` :422, `ConfigError` :389, `CallResult` :464
- `retry::{RetryConfig, BackoffConfig, JitterConfig, retry, retry_with}` — src/retry.rs:264/:71/:218
- `CircuitBreaker` / `CircuitBreakerConfig` / `Outcome` — src/circuit_breaker.rs:274/:48/:159 (+`OutcomeWindow` :378, doc(hidden) для бенчей)
- `Bulkhead` / `BulkheadConfig` / `BulkheadPermit` — src/bulkhead.rs:104/:40/:327
- `rate_limiter::{RateLimiter (trait) :158, ErasedRateLimiter :251, TokenBucket :331, LeakyBucket :521, SlidingWindow :691, AdaptiveRateLimiter :862}` — src/rate_limiter.rs
- `timeout::{timeout, timeout_with_policy_context, TimeoutExecutor :175}` — src/timeout.rs
- `hedge::{HedgeConfig :69, HedgeSafety :58, HedgeExecutor :156, AdaptiveHedgeExecutor :290}` — src/hedge.rs; hedge НЕ входит в PipelineBuilder (by design)
- `load_shed::{load_shed, load_shed_with_policy_context[_and_sink]}` — src/load_shed.rs
- `fallback::{FallbackStrategy :31, ValueFallback :107, FunctionFallback :158, CacheFallback :268, ChainFallback :385, PriorityFallback :471, FallbackOperation :573}` — src/fallback.rs
- `ErrorClassifier<E>` + `ErrorClass` + `NebulaClassifier` / `AlwaysTransient` / `AlwaysPermanent` / `FnClassifier` — src/classifier.rs:140/:67/:270
- `PolicyContext` (cancel + deadline + scope для вызовов из workflow-runtime) — src/context.rs:15; `Deadline` — src/deadline.rs:16
- `CancellationContext` / `CancellableFuture` / `CancellationExt` — src/cancellation.rs:42/:211/:289
- `Gate` / `GateGuard` / `GateCloseTimeout` (кооперативный shutdown-drain) — src/gate.rs:146/:93/:66
- `sink::{MetricsSink :317, ResilienceEvent :204, ResilienceEventKind :265, PolicyScope :89, PipelineOutcome :165, RecordingSink :348, NoopSink :324}` — src/sink.rs
- `policy::{PolicySource :29, LoadSignal :46, LoadSnapshot :80, ConstantLoad :160}` — src/policy.rs
- `clock::{Clock :41, SystemClock :54, MockClock :75}` — src/clock.rs (не реэкспортирован в корне)

## Workspace-зависимости
- Deps (Cargo.toml): `nebula-error` (единственный nebula-dep); tokio (rt/sync/time), tokio-util, thiserror, tracing, parking_lot, smallvec, fastrand; optional: serde (default on), loom.
- Features: `serde` (default), `full` (= serde), `loom` (model-checking при RUSTFLAGS="--cfg loom").
- Кто зависит: `nebula-engine` (crates/engine/Cargo.toml:37 — TokenBucket в engine.rs:3360+), `nebula-credential` (crates/credential/Cargo.toml:45 — retry_with в service/facade.rs:34, rotation/events.rs; CircuitBreaker в runtime/refresh/l1.rs:103), `nebula-api` (crates/api/Cargo.toml:31 — SlidingWindow в transport/webhook/ratelimit.rs:28).

## Структура модулей (LoC)
- `lib.rs` (191) — crate-docs + карта реэкспортов; deny(unsafe_code), warn(missing_docs)
- `pipeline.rs` (2187) — ResiliencePipeline/PipelineBuilder, композиция паттернов
- `circuit_breaker.rs` (1622) — closed/open/half-open state machine + OutcomeWindow (sliding window)
- `rate_limiter.rs` (1291) — 4 алгоритма + trait + erased-обёртка
- `retry.rs` (1129) — backoff (fixed/exponential), jitter, Classify-aware фильтрация
- `fallback.rs` (947) — 5 стратегий graceful degradation + FallbackOperation
- `hedge.rs` (647) — спекулятивное дублирование для idempotent-операций + LatencyTracker
- `error.rs` (555) — CallError<E> per-pattern варианты
- `bulkhead.rs` (490) — semaphore-ограничение конкурентности
- `gate.rs` (456) — shutdown-барьер с типизированным таймаутом дрейна
- `sink.rs` (440) — MetricsSink observability-события
- `classifier.rs` (403) — ErrorClassifier-шов над nebula-error::Classify
- `timeout.rs` (375), `cancellation.rs` (336), `policy.rs` (328), `load_shed.rs` (245), `clock.rs` (169), `context.rs` (165), `deadline.rs` (125)
- tests/: cancel_safety, stress (5K-10K задач), proptest_backoff, fault-injection; benches/: 13 criterion-бенчей

## Напряжения
- Противоречие «семь паттернов»: lib.rs:8-9 перечисляет 7 БЕЗ fallback (с load shed), README.md:20-21 — 7 БЕЗ load shed (с fallback). Фактически паттернов 8. Косметика, но канон-описание расходится.
- AGENTS.md:15,21 заявляет «14 criterion benches» — фактически 13 bench-таргетов (Cargo.toml:62-112, 13 файлов в benches/).
- doc(hidden) утечки для бенчей: `OutcomeWindow` (lib.rs:166), `LatencyTracker` (lib.rs:178), `retry_with_inner` (lib.rs:190) — намеренно, но это internals в pub-поверхности.
- AGENTS.md:38 «Cross-crate calls go through nebula-eventbus» — скопированный шаблон, у крейта нет eventbus-зависимости и нет cross-crate вызовов; нерелевантная строка.
- README.md:14 (lib.rs:13-15): engine-level retry с persisted attempt accounting помечен `planned` — потенциальная будущая ревизия инварианта «единственная retry-поверхность».
- TODO/FIXME/deprecated в src/ — отсутствуют; shims нет; чистый крейт.

## Роль в credential/resource redesign
Сам крейт redesign-ом НЕ затронут (stable, перестройка не планируется). Он — поставщик примитивов
для credential-rewrite: `retry_with` в CredentialService facade и rotation events, per-credential
`CircuitBreaker` в L1RefreshCoalescer (runtime/refresh). nebula-resource от него НЕ зависит —
acquire/teardown-петля ADR-0093 его не использует. Риск redesign-а: только если credential merge
изменит retry-конфиги, API крейта менять не требуется.
