# nebula-execution — design

| Field | Value |
|-------|-------|
| **Status** | Stable existing state-machine surface; Partial default-public revision/bundle vocabulary |
| **Layer** | Core (зависит только вниз: `nebula-core` / `nebula-error` / `nebula-workflow`) |
| **Redesign role** | **Не затронут** post-0092 credential/resource редизайном — нет зависимостей на `nebula-credential` / `nebula-resource`; стабильный фундамент под движком |
| **Related** | PRODUCT_CANON §11.2 (retry-модель), ROADMAP §M0.3, regression #273 (out-of-band failure при rotation) |

---

## 1. Назначение и границы

`nebula-execution` — это **общая типовая модель execution-времени** для движка: машина статусов,
журнал-WAL, ключи идемпотентности, параллельное расписание из DAG и персистентное состояние прогона.

**Владеет:** 8-статусной машиной `ExecutionStatus` и легальностью переходов (execution- и node-уровень,
включая retry-рёбра); `JournalEntry` (WAL-форма журнала; производственного писателя у типа пока нет); детерминированным
`IdempotencyKey` формата `{execution_id}:{node_id}:{attempt}`; `ExecutionPlan` (параллельное расписание);
персистентным `ExecutionState` / `NodeExecutionState` + retry-механикой; `ExecutionContext` / `ExecutionBudget`,
`ReplayPlan`, `ExecutionResult`, `ExecutionOutput`, `ExecutionError`; default-public
`ExecutionRevisions` и immutable Graph-v1 `ExecutionContractBundle` с recorded-wire validation.

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
| `AttemptOutcome` / `NodeAttempt` (`.error: Option<ErrorEnvelope>`) | `src/state.rs:29` / `src/attempt.rs:12` |
| `IdempotencyKey(String)` — детерминированный ключ | `src/idempotency.rs:26` |
| `JournalEntry` — enum 9 событий, serde `tag="event"`; поле `error` — `ErrorEnvelope` | `src/journal.rs:12` |
| `ExecutionBudget` / `ExecutionContext` (+ optional `W3cTraceContext`) | `src/context.rs:44,153` |
| `ExecutionPlan` / `ReplayPlan` | `src/plan.rs:12` / `src/replay.rs:35` |
| `ExecutionResult` / `ExecutionOutput` (есть `BlobRef`) / `NodeOutput` | `src/result.rs:26` / `src/output.rs:35,90` |
| `ExecutionError` — typed `thiserror` | `src/error.rs:11` |
| `ErrorEnvelope` — durable-запись ошибки: version, typed code/category/retryable и bounded `redacted_message` | `src/error_envelope.rs:1` |
| `ExecutionRevisions` — workflow + worker-flavor revision pins | `src/revision.rs` |
| `ExecutionProfile`, `ExecutionContractBundle` | `src/bundle.rs` |
| `RecordedExecutionContractBundleV1`, `ExecutionContractBundleIntegrityError` | `src/bundle.rs` |
| `ExecutionContractBundleV2`, `ExecutionBindingManifestV2`, typed site/target contracts | `src/bundle_v2.rs` |
| re-export `W3cTraceContext` из `nebula-core` | `src/lib.rs:55` |

## 3. Зависимости и зависимые

- **Deps:** `nebula-core` (path), `nebula-error` (workspace, features `derive`, `serde`), `nebula-workflow` (path);
  `serde`, `serde_json`, `sha2`, `thiserror`, `tracing`, `chrono`. Dev: `insta`, `rstest`,
  `pretty_assertions`.
- **Dependents:** `nebula-engine` (`crates/engine/Cargo.toml:32`), `nebula-api` (`crates/api/Cargo.toml:24`).

## 4. Внутренняя архитектура

К существующей модульной модели добавлены `revision.rs` (revision pins) и `bundle.rs`
(Graph-v1 bundle, canonical fingerprint, recorded-wire validation). `lib.rs` — корни модулей +
re-exports.
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
- **Durable-ошибка типизирована и ограничена.** `ErrorEnvelope` несёт version, `ErrorCode`, category,
  retryable и bounded `redacted_message` (control-escaped, ≤512 байт на char-границе). Запись строится из
  верхнеуровневого `Display` и **не** обходит `.source()`: цепочка источников и есть канал утечки
  провайдерского текста. Строка pre-envelope-формы отвергается при чтении (fail closed), а не читается
  как opaque-ошибка.
- **`#[non_exhaustive]` на причинах терминации** + `forbid(unsafe_code)` — расширяемость и отсутствие unsafe.
- **Bundle structural integrity, не authority.** Recorded-v1 принимает только supported
  versions/profile, canonical unique credential IDs и совпадающий fingerprint. Отдельный V2
  envelope канонически фиксирует `(node|trigger, slot_key)` → typed credential/resource target,
  expected contract и credential capabilities; fingerprint включает tenant, plan/workflow/flavor
  revisions и всю mapping, но не secret/material epoch. Admission отдельно проверяет tenant
  authority, exact revisions и binding closure; `PluginSetId` остаётся
  независимым pin, а не proof полного registry/schema/runtime behavior.

## 6. Известные напряжения / долг (честно)

1. **Граница retry зафиксирована.** Engine-level retry реализован только как operator-declared
   retry (`NodeDefinition.retry_policy` / `WorkflowConfig.retry_policy`): retry-рёбра
   `Failed→WaitingRetry→Ready` задокументированы (`transition.rs`), `schedule_node_retry` ставит
   `next_attempt_at`, есть `has_exhausted_retry_budget` и `max_total_retries`. Крейт хранит
   state/idempotency-формы; решение и re-dispatch остаются в `nebula-engine`. `ActionResult::Retry`
   не является текущей публичной поверхностью.
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
7. **Durable-запись ошибки: граница redaction и потеря диагностики.** #1016 заменил free-text
   `error: String` в durable-состоянии и журнале на [`ErrorEnvelope`] (code, category, retryable,
   bounded `redacted_message`). Запись больше не обходит `.source()`-цепочку: именно этот обход
   публиковал провайдерский текст в storage, журнал, OnError-payload и спаны. Цена честная, но не та,
   что казалась раньше: когда отказ несёт типизированный `ActionError` (прямой `EngineError::Action`
   или обёрнутый в `RuntimeError::ActionError`), запись уже сохраняет код, категорию и retryability
   именно этого действия через его собственный `Classify` (`EngineError::as_action_error` —
   `durable_error_envelope` больше не берёт классификацию у обёртки-константы
   `RuntimeError::ActionError`). Пропадает только свободный текст детали («credential not
   configured» и т.п.), а не код причины. `ErrorEnvelope::source_codes` при этом всегда пуст: для
   `Action`/`Execution` `Classify::code` и так возвращает код вложенной ошибки (дублирование), а
   `StorageError` и revision/projection-мосты `Classify` не реализуют, так что для них у движка
   действительно нет кода, который можно было бы назвать — это остаётся следующим шагом, а не
   отсутствием `Classify` на `ActionError`.
8. **Остаточные каналы `Display` — это класс, а не перечень.** `durable_error_envelope`
   (`engine/mod.rs`) пишет `error.to_string()` верхнего уровня, и для части вариантов этот текст не
   авторства фреймворка: `TaskPanicked` несёт payload паники задачи (`persistence.rs:214`), а
   `PlanningFailed` интерполирует чужой текст ошибки. Таких интерполяций в движке порядка двадцати, и
   каждая новая становится каналом в тот день, когда её текст дойдёт до durable-записи, поэтому здесь
   фиксируется класс и признак проверки, а не список.

   Достижимость у них разная. Два сайта в `resume/mod.rs` (`satisfy_signal_waits`, `cancel_dangling_nodes`)
   и `timer_scan.rs` больше не интерполируют ошибку вообще. Доходящий до них `PlanningFailed` становится
   `ControlDispatchError::Deferred`, а `Deferred` возвращает строку в `Pending` через `release_claim`
   (`control_consumer.rs:865-896`): текст уходит в лог и в `ExecutionEvent::ResumeDeferred`, durable-записи
   не возникает. Durable остаётся текст `ControlDispatchError::Internal` — `control_consumer.rs:898-907`
   вызывает `ack_failed(&token, &e.to_string())`, и тот пишет `mark_failed`. Признак, по которому
   проверяется конкретный сайт: становится ли его текст `Internal`, а не `Deferred`.

   Вторая половина класса живёт ниже, в `nebula-storage-port`. `StorageError`'s `Display` достигает
   `%error`-полей логов, reason-строк `Deferred` и durable `error_message` через тот же `Internal`; `impl
   From<serde_json::Error>` теперь отдаёт value-free сводку, но прямые конструкторы
   `Serialization(…to_string())` его обходят: измерено 28 вхождений в `crates/storage/src`, в основном в
   `sqlite/**` и `postgres/**`. Причина, по которой правки одной конверсии недостаточно: `StorageError`
   существует в двух видах (`storage-port/src/error.rs:13` и `storage/src/error.rs:13`), и у каждого свой
   `From`. Это тот же класс утечки с другим владельцем. Текст ограничен и экранирован, но provenance не
   подтверждён: осознанная граница, не недосмотр.
9. **`ExplicitFail.message` вне объёма #1016.** `status.rs:139` (`message: String`) хранит авторскую
   причину терминации, а не перехваченную провайдерскую ошибку. Замена поля на структурный `ErrorCode`
   уже обещана в §6.6 (`ExecutionTerminationCode` → `ErrorCode`).
10. **Устранено: ложная doc-претензия в `lib.rs:25`.** Док утверждал, что [`JournalEntry`] «backs
    `execution_journal` append-only table». Производственного писателя у типа нет: единственные
    упоминания — тесты. `port_execution_journal` пишется через
    `nebula_storage_port::dto::JournalEntry` (`{seq, payload}`), payload непрозрачен для порта.

## 7. Роль в пост-0092 credential/resource модели

**Не затронут.** Крейт не зависит на `nebula-credential` / `nebula-resource`; упоминания только косвенные
(`transition.rs:174` — regression-тест #273 про out-of-band failure при credential rotation; `state.rs:1037`
— строка-пример в тесте `mark_setup_failed`). Это Core-слой, зависящий только вниз (core/error/workflow);
потребители — engine и api. Consumer-binding (`#[credential]` / `#[resource]` слоты, `CredentialGuard<Scheme>`)
живёт выше по стеку и не пересекается с типами execution-времени.

## 8. Forward design / открытые вопросы

Крейт стабилен. Накопившийся долг — **документационный, не структурный**: §6.2–6.4 закрываются синхронизацией
crate-доков и README. Retry-модель синхронизирована с canon §11.2: execution хранит state/idempotency-формы,
engine делает operator-declared retry, `ActionResult::Retry` не является текущей публичной поверхностью.
Единственная кодовая зависимость от чужого роадмапа — swap `ExecutionTerminationCode` → структурный
`ErrorCode` в action-v2 (§6.6); до тех пор opaque-код стабилен по контракту. Границы, оставленные #1016
(§6.7–6.9), касаются полноты диагностики внутри уже принятой формы, а не структуры: `ErrorEnvelope`
закрывает обход `.source()`-цепочки, а именование причин ждёт `Classify` на внутренних ошибках. Одно
исключение требует владельца, а не ожидания: остаточный класс `Display` из §6.8 второй половиной живёт
в `crates/storage/src/{sqlite,postgres}/**` (28 прямых конструкторов `Serialization(…to_string())`), и
закрывается только там. В самом крейте execution структурных открытых вопросов нет.
