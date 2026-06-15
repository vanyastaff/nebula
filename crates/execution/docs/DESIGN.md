# nebula-execution — design

| Field | Value |
|-------|-------|
| **Status** | Stable — Core-слой, типовой фундамент execution-времени |
| **Layer** | Core (зависит только вниз: `nebula-core` / `nebula-error` / `nebula-workflow`) |
| **Redesign role** | **Не затронут** post-0092 credential/resource редизайном — нет зависимостей на `nebula-credential` / `nebula-resource`; стабильный фундамент под движком |
| **Related** | PRODUCT_CANON §11.2 (retry-модель), ROADMAP §M0.3, regression #273 (out-of-band failure при rotation) |

---

## 1. Назначение и границы

`nebula-execution` — это **общая типовая модель execution-времени** для движка: машина статусов,
журнал-WAL, ключи идемпотентности, параллельное расписание из DAG и персистентное состояние прогона.

**Владеет:** 8-статусной машиной `ExecutionStatus` и легальностью переходов (execution- и node-уровень,
включая retry-рёбра); `JournalEntry` (WAL за таблицей `execution_journal`); детерминированным
`IdempotencyKey` формата `{execution_id}:{node_id}:{attempt}`; `ExecutionPlan` (параллельное расписание);
персистентным `ExecutionState` / `NodeExecutionState` + retry-механикой; `ExecutionContext` / `ExecutionBudget`,
`ReplayPlan`, `ExecutionResult`, `ExecutionOutput`, `ExecutionError`.

**Явно НЕ делает:** не выполняет узлы и не оркеструет (это `nebula-engine`); не персистит и не делает CAS —
типы описывают *что* такое легальный переход, а enforcement идемпотентности и атомарная запись — в
`nebula-storage`. Крейт даёт типы и предикаты легальности, не побочные эффекты. `#![forbid(unsafe_code)]`.

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `ExecutionStatus` — enum 8 состояний (`Created…TimedOut`) | `src/status.rs:11` |
| `ExecutionTerminationReason` (`#[non_exhaustive]`) | `src/status.rs:106` |
| `ExecutionTerminationCode(Arc<str>)` — opaque код | `src/status.rs:163` |
| `can_transition_execution` / `validate_execution_transition` | `src/transition.rs:14,41` |
| `can_transition_node` / `validate_node_transition` (retry-рёбра `Failed→WaitingRetry→Ready`) | `src/transition.rs:64,87` |
| `ExecutionState` (`transition_status`, `transition_node`, `schedule_node_retry`, `has_exhausted_retry_budget`, `idempotency_key_for_node`, `mark_setup_failed`) | `src/state.rs:177` |
| `NodeExecutionState` (+ `next_attempt_at: Option<DateTime<Utc>>`) | `src/state.rs:51` |
| `AttemptOutcome` / `NodeAttempt` | `src/state.rs:29` / `src/attempt.rs:12` |
| `IdempotencyKey(String)` — детерминированный ключ | `src/idempotency.rs:26` |
| `JournalEntry` — enum 9 событий, serde `tag="event"` | `src/journal.rs:12` |
| `ExecutionBudget` / `ExecutionContext` (+ optional `W3cTraceContext`) | `src/context.rs:44,153` |
| `ExecutionPlan` / `ReplayPlan` | `src/plan.rs:12` / `src/replay.rs:35` |
| `ExecutionResult` / `ExecutionOutput` (есть `BlobRef`) / `NodeOutput` | `src/result.rs:26` / `src/output.rs:35,90` |
| `ExecutionError` — typed `thiserror` | `src/error.rs:11` |
| re-export `W3cTraceContext` из `nebula-core` | `src/lib.rs:55` |

## 3. Зависимости и зависимые

- **Deps:** `nebula-core` (path), `nebula-error` (workspace, feature `derive`), `nebula-workflow` (path);
  `serde`, `serde_json`, `thiserror`, `tracing`, `chrono`. Dev: `insta`, `rstest`, `pretty_assertions`.
- **Dependents:** `nebula-engine` (`crates/engine/Cargo.toml:32`), `nebula-api` (`crates/api/Cargo.toml:24`).

## 4. Внутренняя архитектура

13 модулей, ~4.1k строк (≈половина `state.rs` — тесты). `lib.rs` (60) — корни модулей + re-exports.
`status.rs` (317) — статусы + причины/коды терминации. `transition.rs` (321) — `matches!`-таблицы легальности
переходов на двух уровнях. `state.rs` (1520) — крупнейший: `ExecutionState` / `NodeExecutionState` +
retry-механика. `journal.rs` (262) — WAL-события. `idempotency.rs` (148), `context.rs` (261),
`plan.rs` (187), `replay.rs` (339), `result.rs` (241), `output.rs` (225), `attempt.rs` (144), `error.rs` (89).

Поток: движок читает легальность через `validate_*_transition`, применяет переход на `ExecutionState`,
эмитит `JournalEntry`; storage персистит и форсит идемпотентность по `IdempotencyKey`.

## 5. Инварианты и контракты

- **Легальность переходов by-construction.** Машина переходов — единственный источник истины о допустимых
  рёбрах; `transition_node` возвращает ошибку на нелегальном переходе (нельзя «протолкнуть» state мимо таблицы).
- **Детерминированный ключ идемпотентности.** `IdempotencyKey` = `{execution_id}:{node_id}:{attempt}` —
  одинаковый ввод даёт один ключ; enforcement (уникальность/CAS) делегирован в storage.
- **Retry-бюджет.** `has_exhausted_retry_budget` (`state.rs:311`) + `ExecutionBudget::max_total_retries`
  (`context.rs:63`) — движок сверяется с обоими на каждом отказе; `Some(0)` отключает engine-level retry.
- **`#[non_exhaustive]` на причинах терминации** + `forbid(unsafe_code)` — расширяемость и отсутствие unsafe.

## 6. Известные напряжения / долг (честно)

1. **Док vs код: engine-retry.** `lib.rs:13-14`, `README.md:68-72` (§11.2 «engine does not retry nodes;
   attempt counter never advances past 1») и `AGENTS.md:23` прямо противоречат коду: retry-рёбра
   `Failed→WaitingRetry→Ready` задокументированы (`transition.rs:54-62`), `schedule_node_retry` ставит
   `next_attempt_at` (`state.rs:568`), есть `has_exhausted_retry_budget` (`state.rs:311`) и
   `max_total_retries` (`context.rs:63-76`). Retry-модель реализована — доки крейта не обновлены.
2. **README stale: «5 panic! как invariant guards»** (`README.md:108-109`). В lib-коде panic! нет вообще;
   все 7 вхождений — `#[cfg(test)]` (`status.rs:320,338`; `state.rs:1370,1433`; `result.rs:228,247`;
   `output.rs:184`). Долг погашен, README не обновлён.
3. **README stale: имя варианта.** `README.md:38` перечисляет `Pending`, но enum начинается с `Created`
   (`status.rs:13`). `lib.rs:18` называет машину корректно 8-state, но без перечня.
4. **Дебрис вычищенных plan-ID в доках:** оборванные ссылки «retry path from :» (`transition.rs:56`),
   «ROADMAP §M0.3, )» (`status.rs:85`), «(.1 / T4 acceptance)» (`context.rs:64-65`).
5. **Мини-shim:** `lib.rs:56-57` `pub(crate) use ... as serde_duration_opt` — legacy-alias под старые
   внутренние пути; кандидат на прямое использование helper'а из `nebula-core`.
6. **Forward-promise в API:** `ExecutionTerminationCode` (`status.rs:152-160`) и поле `code`
   (`status.rs:131-137`) обещают замену на структурный `ErrorCode` в «Phase 10 action-v2» — отложенная
   зависимость от чужого роадмапа, закодированная в doc-комментариях.

## 7. Роль в пост-0092 credential/resource модели

**Не затронут.** Крейт не зависит на `nebula-credential` / `nebula-resource`; упоминания только косвенные
(`transition.rs:174` — regression-тест #273 про out-of-band failure при credential rotation; `state.rs:1037`
— строка-пример в тесте `mark_setup_failed`). Это Core-слой, зависящий только вниз (core/error/workflow);
потребители — engine и api. Consumer-binding (`#[credential]` / `#[resource]` слоты, `CredentialGuard<Scheme>`)
живёт выше по стеку и не пересекается с типами execution-времени.

## 8. Forward design / открытые вопросы

Крейт стабилен. Накопившийся долг — **документационный, не структурный**: §6.1–6.4 закрываются синхронизацией
crate-доков и README с реальной retry-моделью (включая выбор canon-формулировки §11.2). Единственная кодовая
зависимость от чужого роадмапа — swap `ExecutionTerminationCode` → структурный `ErrorCode` в action-v2 (§6.6);
до тех пор opaque-код стабилен по контракту. Структурных открытых вопросов нет.
