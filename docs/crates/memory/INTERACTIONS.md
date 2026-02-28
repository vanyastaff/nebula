# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `core`: базовые типы и соглашения платформы.
- `system`: системная информация о памяти и pressure-сигналы.
- `log`: structured logging (feature `logging`).
- `runtime`/`engine`/`worker`: оркестрация и применение memory policy в исполнении.
- `action`: потребитель pool/cache/arena стратегий в hot-path.
- `resilience`: retry/circuit/backpressure политика поверх memory ошибок.
- `config`: источник лимитов/размеров/политик для memory модулей.
- `metrics`/`telemetry`: экспорт и визуализация runtime метрик.

## Planned crates

- `memory-adapters`:
  - why it will exist: шаблоны интеграции memory-политик для конкретных runtime профилей.
  - expected owner/boundary: адаптеры без изменения core memory contracts.

## Downstream Consumers

- `runtime/engine`:
  - expectations from this crate: предсказуемые allocation/reuse примитивы и pressure-сигналы.
- `action`:
  - expectations from this crate: low-latency pools/caches для повторяющихся операций.

## Upstream Dependencies

- `system`:
  - why needed: чтение памяти хоста и уровня pressure.
  - hard contract relied on: корректные `MemoryInfo`/`MemoryPressure` данные.
  - fallback behavior if unavailable: деградация мониторинга при сохранении базовых allocator/pool функций.
- `log` (optional):
  - why needed: диагностика pressure, ошибок и lifecycle событий.
  - hard contract relied on: стандартные logging macros.
  - fallback behavior if unavailable: no-op logging.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| memory <-> system | in | memory info + pressure states | sync | degrade monitoring path | monitoring module |
| memory <-> runtime/engine | out | alloc/pool/cache/budget APIs | sync/async | runtime chooses retry/fallback | critical path |
| memory <-> action | out | reuse primitives for hot execution | sync/async | caller handles exhaustion | workload-specific configs |
| memory <-> resilience | in/out | retryability semantics via `MemoryError` | sync | retries only for retryable classes | policy ownership outside memory |
| memory <-> config | in | tuned limits and capacity values | sync | invalid config fails startup path | bootstrap boundary |
| memory <-> log/telemetry | out | structured diagnostics and metrics | sync/async | observability failures non-fatal | additive |

## Runtime Sequence

1. Runtime loads memory config and initializes memory subsystem.
2. Workload path uses selected primitives (pool/arena/cache/budget).
3. Monitoring and stats observe pressure and usage.
4. Caller applies policy decisions (throttle/retry/fallback) on signals/errors.
5. Shutdown drains/cleans memory-owned structures.

## Cross-Crate Ownership

- who owns domain model: `memory` owns allocation/reuse contracts.
- who owns orchestration: `runtime/engine` own policy orchestration.
- who owns persistence: not `memory`.
- who owns retries/backpressure: `resilience` + caller.
- who owns security checks: memory safety boundaries in `memory`; request authn/authz in upper layers.

## Failure Propagation

- how failures bubble up:
  - typed `MemoryError` variants returned to consumers.
- where retries are applied:
  - caller/resilience layer for retryable classes.
- where retries are forbidden:
  - corruption, invalid layout/alignment, invalid state.

## Versioning and Compatibility

- compatibility promise with each dependent crate:
  - stable error semantics and key API contracts within major version.
- breaking-change protocol:
  - proposal -> decision -> migration note -> major release.
- deprecation window:
  - minimum one minor version for non-critical removals.

## Contract Tests Needed

- system pressure mapping contract tests.
- runtime integration tests for budget + pool behavior under load.
- retryability mapping tests for resilience wrappers.
- feature-matrix compile tests (`default`, `full`, selective combos).
