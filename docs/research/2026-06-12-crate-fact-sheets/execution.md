# nebula-execution — fact sheet

## Назначение
Общая модель execution-времени для движка: 8-статусная машина `ExecutionStatus` с валидацией переходов,
`JournalEntry` (WAL за таблицей `execution_journal`), `IdempotencyKey` (`{execution_id}:{node_id}:{attempt}`),
`ExecutionPlan` (параллельное расписание из DAG) и персистентное состояние `ExecutionState`/`NodeExecutionState`.
Типы + легальность переходов; персистентность/CAS — в storage, оркестрация — в engine. `#![forbid(unsafe_code)]`.

## Публичная поверхность
- `ExecutionStatus` — enum 8 состояний: `Created, Running, Paused, Cancelling, Completed, Failed, Cancelled, TimedOut` — `src/status.rs:11`
- `ExecutionTerminationReason` — почему терминальное состояние (`NaturalCompletion/ExplicitStop/ExplicitFail/Cancelled/SystemError`), `#[non_exhaustive]` — `src/status.rs:106`
- `ExecutionTerminationCode(Arc<str>)` — opaque код, обещан swap на `ErrorCode` в Phase 10 action-v2 — `src/status.rs:163`
- `can_transition_execution` / `validate_execution_transition` — `src/transition.rs:14,41`
- `can_transition_node` / `validate_node_transition` — вкл. retry-рёбра `Failed→WaitingRetry→Ready` — `src/transition.rs:64,87`
- `ExecutionState` — крупнейший тип: `transition_status`, `transition_node`, `schedule_node_retry` (state.rs:568), `increment_total_retries` (298), `has_exhausted_retry_budget` (311), `set_terminated_by` (370), `idempotency_key_for_node` (489), `mark_setup_failed` (714) — `src/state.rs:177`
- `NodeExecutionState` — per-node state + `next_attempt_at: Option<DateTime<Utc>>` для retry-парковки — `src/state.rs:51`
- `AttemptOutcome` — enum исхода попытки — `src/state.rs:29`
- `NodeAttempt` — attempt-keyed запись (номер, таймстемпы, статус) — `src/attempt.rs:12`
- `IdempotencyKey(String)` — детерминированный ключ; enforcement в storage — `src/idempotency.rs:26`
- `JournalEntry` — enum 9 событий (`ExecutionStarted`…`CancellationRequested`), serde tag="event" — `src/journal.rs:12`
- `ExecutionBudget` — `max_concurrent_nodes`/`max_duration`/`max_output_bytes`/`max_total_retries` — `src/context.rs:44`
- `ExecutionContext` — execution_id + budget + optional `W3cTraceContext` — `src/context.rs:153`
- `ExecutionPlan` — пред-вычисленное параллельное расписание — `src/plan.rs:12`
- `ReplayPlan` — резюме с чекпойнта — `src/replay.rs:35`
- `ExecutionResult` — итог: статус, тайминги, счётчики, termination_reason — `src/result.rs:26`
- `ExecutionOutput` (enum, есть `BlobRef`) / `NodeOutput` — `src/output.rs:35,90`
- `ExecutionError` — typed thiserror — `src/error.rs:11`
- re-export `W3cTraceContext` из nebula-core — `src/lib.rs:55`

## Workspace-зависимости
Deps: `nebula-core` (path), `nebula-error` (workspace, feature derive), `nebula-workflow` (path); serde, serde_json, thiserror, tracing, chrono.
Dev-deps: insta, rstest, pretty_assertions. Lints workspace.
Зависят от него: `nebula-engine` (crates/engine/Cargo.toml:32), `nebula-api` (crates/api/Cargo.toml:24).

## Структура модулей (13 файлов, ~4.1k строк)
- `lib.rs` (60) — корни модулей + re-exports; pub(crate) alias `serde_duration_opt` → nebula-core helper
- `status.rs` (317) — `ExecutionStatus`, `ExecutionTerminationReason`, `ExecutionTerminationCode`
- `transition.rs` (321) — легальность переходов execution- и node-уровня (matches!-таблицы)
- `state.rs` (1520) — `ExecutionState`/`NodeExecutionState` + retry-механика; крупнейший модуль, ~половина — тесты
- `journal.rs` (262) — `JournalEntry` enum (WAL)
- `idempotency.rs` (148) — `IdempotencyKey`
- `context.rs` (261) — `ExecutionBudget`, `ExecutionContext`
- `plan.rs` (187) — `ExecutionPlan`
- `replay.rs` (339) — `ReplayPlan`
- `result.rs` (241) — `ExecutionResult`
- `output.rs` (225) — `ExecutionOutput`/`NodeOutput`
- `attempt.rs` (144) — `NodeAttempt`
- `error.rs` (89) — `ExecutionError`

## Напряжения
1. **Док vs код: engine-retry.** lib.rs:13-14, README.md:68-72 (§11.2 «engine does not retry nodes; attempt counter never advances past 1») и AGENTS.md:23 («Do NOT add engine-level node retry») прямо противоречат коду: `transition.rs:54-62` документирует retry-рёбра `Failed→WaitingRetry→Ready`, `state.rs:568 schedule_node_retry` ставит `next_attempt_at`, `state.rs:311 has_exhausted_retry_budget`, `context.rs:63-76 max_total_retries` («engine consults both on every failure», `Some(0)` disables engine-level retry). Retry-модель реализована, доки крейта не обновлены.
2. **README stale: 5 panic! как invariant guards.** README.md:108-109 заявляет «5 panic! sites in transition and status modules». Фактически в lib-коде panic! нет вообще; все 7 вхождений — в `#[cfg(test)]` (status.rs:320,338; state.rs:1370,1433; result.rs:228,247; output.rs:184). Долг уже погашен, README не обновлён.
3. **README stale: имя варианта.** README.md:38 перечисляет `Pending`, но enum начинается с `Created` (status.rs:13). lib.rs:18 называет машину корректно 8-state, но без перечня.
4. **Дебрис вычищенных plan-ID в доках:** оборванные ссылки «retry path from :» (transition.rs:56), «ROADMAP §M0.3, )» (status.rs:85), «(.1 / T4 acceptance)» (context.rs:64-65) — текст ломаный после скраба идентификаторов плана.
5. **Мини-shim:** lib.rs:56-57 `pub(crate) use ... as serde_duration_opt` — legacy-alias, чтобы старые внутренние пути резолвились; кандидат на прямое использование helper'а.
6. **Forward-promise в API:** `ExecutionTerminationCode` (status.rs:152-160) и поле `code` (status.rs:131-137) обещают замену на структурный `ErrorCode` в «Phase 10 action-v2» — отложенная зависимость от чужого роадмапа, закодированная в doc-комментариях.

## Роль в credential/resource redesign
Крейт НЕ затронут редизайном: нет зависимостей на nebula-credential/nebula-resource, упоминания только косвенные
(`transition.rs:174` — regression-тест #273 про out-of-band failure при credential rotation; `state.rs:1037` — строка-пример
в тесте mark_setup_failed). Слой Core, зависит только вниз (core/error/workflow); потребители — engine и api.
