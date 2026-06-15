# nebula-resilience — design

| Field | Value |
|-------|-------|
| **Status** | Stable — leaf in-process resilience layer; перестройка не планируется |
| **Layer** | Cross-cutting (leaf; единственный workspace-dep — `nebula-error`) |
| **Redesign role** | **Не затронут** пост-0092 credential/resource консолидацией. Поставщик примитивов (`retry_with`, `CircuitBreaker`), не участник contract-перестройки; API менять не требуется. |
| **Related** | PRODUCT_CANON §4.2 / §4.3 / §11.2, `nebula-error::Classify`, README + `docs/` (api-reference, composition, observability, gate) |

---

## 1. Назначение и границы

`nebula-resilience` — это **in-process слой устойчивости для исходящих вызовов** внутри actions:
композируемые паттерны (retry, circuit breaker, bulkhead, rate limiter, timeout, hedge, load shed,
fallback), собираемые в `ResiliencePipeline<E>`. Фильтрация retry управляется
`nebula-error::Classify::retry_hint()` (transient vs permanent — явная классификация, не folklore
в теле action).

**Владеет:** композицией паттернов и их конфигами, обёрткой ошибок `CallError<E>` (без type
erasure), классификационным швом `ErrorClassifier<E>` над `Classify`, контрактом
cancel/deadline/scope (`PolicyContext`, `Deadline`, `Gate`) и observability-событиями (`MetricsSink`).

**ЯВНО НЕ делает:** не engine-level retry-scheduler (по канону §11.2 engine не пере-исполняет ноды —
это ЕДИНСТВЕННАЯ retry-поверхность стека); не durable control plane (только in-process; durable
cancel/dispatch — это `execution_control_queue`, §12.2/§4.5); не metrics-export (события *кормят*
`nebula-metrics` через хуки, не наоборот); не обёртки сторонних limiter-крейтов (встроенные
алгоритмы живут в `rate_limiter.rs`).

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `ResiliencePipeline<E>` / `PipelineBuilder<E>` (`build_checked` / `build_recommended_order` / `call_with_policy_context[_and_fallback]`) | `src/pipeline.rs:492` / `:109` |
| `CallError<E>` (`#[non_exhaustive]`, без erasure), `CallErrorKind`, `ConfigError`, `CallResult` | `src/error.rs:27` / `:422` / `:389` / `:464` |
| `retry::{RetryConfig, BackoffConfig, JitterConfig, retry, retry_with}` | `src/retry.rs:264` / `:71` / `:218` |
| `CircuitBreaker` / `CircuitBreakerConfig` / `Outcome` (+ `OutcomeWindow` `doc(hidden)`) | `src/circuit_breaker.rs:274` / `:48` / `:159` / `:378` |
| `Bulkhead` / `BulkheadConfig` / `BulkheadPermit` | `src/bulkhead.rs:104` / `:40` / `:327` |
| `rate_limiter::{RateLimiter (trait), ErasedRateLimiter, TokenBucket, LeakyBucket, SlidingWindow, AdaptiveRateLimiter}` | `src/rate_limiter.rs:158` / `:251` / `:331` / `:521` / `:691` / `:862` |
| `timeout::{timeout, timeout_with_policy_context, TimeoutExecutor}` | `src/timeout.rs:175` |
| `hedge::{HedgeConfig, HedgeSafety, HedgeExecutor, AdaptiveHedgeExecutor}` (НЕ входит в `PipelineBuilder` by design) | `src/hedge.rs:69` / `:58` / `:156` / `:290` |
| `load_shed::{load_shed, load_shed_with_policy_context[_and_sink]}` | `src/load_shed.rs` |
| `fallback::{FallbackStrategy, ValueFallback, FunctionFallback, CacheFallback, ChainFallback, PriorityFallback, FallbackOperation}` | `src/fallback.rs:31` / `:107` / `:158` / `:268` / `:385` / `:471` / `:573` |
| `ErrorClassifier<E>` + `ErrorClass` + `NebulaClassifier` / `AlwaysTransient` / `AlwaysPermanent` / `FnClassifier` | `src/classifier.rs:140` / `:67` / `:270` |
| `PolicyContext` (cancel + deadline + scope), `Deadline` | `src/context.rs:15`, `src/deadline.rs:16` |
| `CancellationContext` / `CancellableFuture` / `CancellationExt` | `src/cancellation.rs:42` / `:211` / `:289` |
| `Gate` / `GateGuard` / `GateCloseTimeout` (кооперативный shutdown-drain) | `src/gate.rs:146` / `:93` / `:66` |
| `sink::{MetricsSink, ResilienceEvent, ResilienceEventKind, PolicyScope, PipelineOutcome, RecordingSink, NoopSink}` | `src/sink.rs:317` / `:204` / `:265` / `:89` / `:165` / `:348` / `:324` |
| `policy::{PolicySource, LoadSignal, LoadSnapshot, ConstantLoad}` | `src/policy.rs:29` / `:46` / `:80` / `:160` |
| `clock::{Clock, SystemClock, MockClock}` (не реэкспортирован в корне) | `src/clock.rs:41` / `:54` / `:75` |

## 3. Зависимости и зависимые

- **Deps:** `nebula-error` (единственный workspace-dep); `tokio` (rt/sync/time), `tokio-util`,
  `thiserror`, `tracing`, `parking_lot`, `smallvec`, `fastrand`; optional `serde` (default on), `loom`.
- **Features:** `serde` (default), `full` (= serde), `loom` (model-checking при `RUSTFLAGS="--cfg loom"`).
- **Зависимые:** `nebula-engine` (`TokenBucket` в `engine.rs:3360+`), `nebula-credential`
  (`retry_with` в `service/facade.rs:34` + `rotation/events.rs`; `CircuitBreaker` в
  `runtime/refresh/l1.rs:103`), `nebula-api` (`SlidingWindow` в `transport/webhook/ratelimit.rs:28`).

## 4. Внутренняя архитектура

Крейт — набор независимых паттерн-модулей плюс композитор. `pipeline.rs` (2187 LoC) собирает
паттерны в `ResiliencePipeline<E>` и владеет порядком исполнения (`build_recommended_order`).
Каждый паттерн самодостаточен и используется standalone: `circuit_breaker.rs` — closed/open/half-open
state machine поверх `OutcomeWindow` (sliding window); `rate_limiter.rs` — 4 алгоритма за одним
trait + erased-обёртка; `retry.rs` — backoff (fixed/exponential) + jitter + `Classify`-aware
фильтрация; `fallback.rs` — 5 стратегий graceful degradation. Поток данных: вызов оборачивается
паттернами снаружи внутрь, ошибки сворачиваются в `CallError<E>`, события эмитятся в `MetricsSink`.
`lib.rs` — `deny(unsafe_code)`, `warn(missing_docs)`. Покрытие: cancel-safety / stress (5K–10K
задач) / proptest backoff / fault-injection тесты + 13 criterion-бенчей.

## 5. Инварианты и контракты

- **§11.2 — единственная retry-поверхность.** Engine не пере-исполняет ноды; retry/circuit
  breaking/timeout для исходящих вызовов живут только здесь, скомпонованные внутри action.
- **§4.2 — классификация, не folklore.** Retry-фильтрация driven by `Classify::retry_hint()`;
  transient vs permanent — явное решение, по-construction единое (`ErrorClassifier<E>` — шов).
- **Без type erasure.** `CallError<E>` сохраняет исходный тип ошибки — нет forced mapping,
  consumer не теряет свой `E`.
- **In-process boundary.** Только process-state; durable cancel/dispatch — НЕ здесь (§12.2).
- **Безопасность под конкуренцией** проверена by-construction тестами: permit-leak detection,
  cooperative shutdown под нагрузкой (`Gate` с типизированным `GateCloseTimeout`).

## 6. Известные напряжения / долг (честно)

1. **«Сколько паттернов».** `lib.rs:8-9` перечисляет 7 паттернов БЕЗ fallback (с load shed),
   `README.md:20-21` — 7 БЕЗ load shed (с fallback); фактически их **8**. Косметика, но
   канон-описание расходится — выровнять при следующем касании docs.
2. **Расхождение в числе бенчей.** `AGENTS.md:15,21` заявляет «14 criterion benches» — фактически
   **13** bench-таргетов (`Cargo.toml:62-112`, 13 файлов в `benches/`).
3. **`doc(hidden)` утечки для бенчей.** `OutcomeWindow` (`lib.rs:166`), `LatencyTracker`
   (`lib.rs:178`), `retry_with_inner` (`lib.rs:190`) намеренно в pub-поверхности ради бенчей —
   internals видны снаружи.
4. **Скопированная нерелевантная строка.** `AGENTS.md:38` «Cross-crate calls go through
   nebula-eventbus» — шаблон; у крейта нет eventbus-зависимости и нет cross-crate вызовов.
5. **`planned` engine-level retry.** `README.md:14` / `lib.rs:13-15` помечают engine-level retry с
   persisted attempt accounting как `planned` — потенциальная будущая ревизия инварианта
   «единственная retry-поверхность».
- TODO/FIXME/deprecated в `src/` отсутствуют; shims нет — чистый крейт.

## 7. Роль в пост-0092 credential/resource модели

**Не затронут — стабильный фундамент.** Перестройка credential (контракт + runtime + facade +
builtin в одном крейте) и resource (per-slot rotation fan-out, SlotCell) использует крейт *как есть*:
`retry_with` в `CredentialService` facade и rotation-events, per-credential `CircuitBreaker` в
L1RefreshCoalescer (`runtime/refresh`). `nebula-resource` от крейта **не зависит** —
acquire/teardown-петля ADR-0093 его не использует. Единственный гипотетический риск redesign-а — если
credential-merge поменяет retry-конфиги; API самого крейта при этом менять не требуется.

## 8. Forward design / открытые вопросы

Крейт стабилен; целенаправленного forward-design нет. Открытые мелочи — косметические: выровнять
канон-счёт паттернов (7 vs 8) и бенчей (13 vs 14) в `lib.rs`/`README.md`/`AGENTS.md`. Единственный
архитектурно значимый сигнал — `planned` engine-level retry с persisted attempt accounting: если он
будет реализован, инвариант §11.2 («единственная retry-поверхность») придётся пересмотреть; до тех
пор он остаётся истинным.
