# nebula-storage-port — design

| Field | Value |
|-------|-------|
| **Status** | Stable — pure contract crate (Core tier), no backend code |
| **Layer** | Core (contract); зависит только от `nebula-core` + serde/async-trait. Адаптеры в `nebula-storage`, scope-политика в `nebula-tenancy` |
| **Redesign role** | **Косвенно затронут, но существенно.** Не переписывается; владеет двумя швами, на которые опирается post-0092 модель: `Scope::credential_owner_id` (каноническая деривация owner_id, ADR-0088 D7) и `RefreshClaimStore` (persistence-шов refresh-CAS, ADR-0041). DTO/трейты не трогаются. |
| **Related** | [ADR-0072](../../../docs/adr/0072-nebula-storage-spec16-port-adapter-tenancy.md), [ADR-0088](../../../docs/adr/0088-credential-subsystem-rewrite.md) D7, [ADR-0041](../../../docs/adr/HISTORICAL.md) (durable refresh-claim store), [ADR-0092](../../../docs/adr/0092-credential-subsystem-consolidation.md), PRODUCT_CANON §12.2 |

---

## 1. Назначение и границы

`nebula-storage-port` — это **контракт хранилища**: он декларирует, *что* должно
уметь персистентное хранилище, и не реализует ни одного backend. Он существует,
чтобы engine/api/credential потребляли storage как `Arc<dyn …>`, не таща за собой
sqlx, миграции и пул соединений.

**Владеет:**
- object-safe `#[async_trait]` repository-трейтами (ISP-сегрегированными по ролям);
- port-локальными DTO-строками, зависящими только от `serde_json::Value`;
- plain-data `Scope { workspace_id, org_id }` (значение без политики);
- единым `StorageError` (`#[non_exhaustive]`, fail-closed);
- атомарным unit-of-work `TransitionBatch` (state + outbox + journal под CAS+fencing);
- id-швом (`FencingToken` + re-export типизированных ULID из `nebula-core`).

**ЯВНО НЕ делает:**
- **никакого sqlx** — нет драйвера БД, миграций, пула (намеренно, комментарий в
  `Cargo.toml:22`); адаптеры (InMemory/SQLite/Postgres) живут в `nebula-storage`;
- **не резолвит и не энфорсит scope** — деривация `Scope` из principal и запрет
  кросс-тенантного доступа — работа `nebula-tenancy` (декораторы-обёртки);
- **не делает шифрование/кэш/аудит** — это decorator-стек в `nebula-storage`
  (`EncryptionLayer`/Cache/Audit + `KeyProvider`);
- **не зависит от higher-tier типов** — DTO никогда не ссылаются на `ActionResult`
  и т.п. (защита от инверсии зависимостей в Core).

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `Scope { workspace_id, org_id }` (plain-data, без политики) | `src/scope.rs:11` |
| `Scope::credential_owner_id()` (length-prefixed, collision-safe, ADR-0088 D7) | `src/scope.rs:50` |
| `StorageError` (`#[non_exhaustive]`: NotFound/Conflict/Duplicate/LeaseUnavailable/FencedOut/Timeout/UnknownSchemaVersion/ScopeViolation/Serialization…) | `src/error.rs:11` |
| `FencingToken(u64)` — монотонный lease-токен против zombie-runner | `src/ids.rs:17` |
| `ids::*` — re-export типизированных ULID (`ExecutionId`, `OrgId`, `WorkflowId`…) | `src/ids.rs:7` |
| `TransitionBatch` (приватные поля, builder-only; scope+CAS+fencing обязательны структурно) | `src/batch.rs:19` |
| `TransitionBatchBuilder` / `TransitionOutcome` | `src/batch.rs:86 / 177` |
| `ExecutionStore` — §12.2 агрегат: create/get/`commit(TransitionBatch)`/acquire_lease/renew_lease/release_lease | `src/store/execution.rs:17` |
| `ControlQueue` + `ReclaimOutcome` | `src/store/control_queue.rs:25,9` |
| `WorkflowStore` / `WorkflowVersionStore` | `src/store/workflow.rs:8,86` |
| `NodeResultStore` / `ExecutionJournalReader` / `CheckpointStore` | `src/store/node_result.rs:12`, `src/store/journal.rs:10`, `src/store/checkpoint.rs:12` |
| `IdempotencyGuard` / `IdempotencyStore` | `src/store/idempotency.rs:15,41` |
| Identity-семейство (9 трейтов): `UserStore`/`OrgStore`/`WorkspaceStore`/`MembershipStore`/`ResourceStore`/`TriggerStore`/`QuotaStore`/`AuditStore`/`BlobStore` | `src/store/identity.rs:17–157` |
| `WebhookActivationStore` | `src/store/webhook.rs:10` |
| `RefreshClaimStore` + `ReplicaId`/`ClaimToken`/`RefreshClaim`/`ClaimAttempt`/`HeartbeatError`/`RefreshClaimError`/`SentinelState`/`ReclaimedClaim` (re-homed shape-unchanged, loom-verified, ADR-0041) | `src/store/refresh_claim.rs:144,21–130` |
| DTO: `ExecutionRecord`, `WorkflowRecord`/`WorkflowVersionRecord`, `ControlMsg`/`ControlCommand`, `JournalEntry`, `NodeResultRecord` (+`MAX_SUPPORTED_RESULT_SCHEMA_VERSION`), `CachedRecord`, `WebhookActivationRecord`, identity-rows (`UserRow`…`BlobRow`, `ScopeKind`, `PrincipalKind`) | `src/dto/` |

## 3. Зависимости и зависимые

- **Deps:** `nebula-core` (path) + `async-trait`, `thiserror`, `serde`,
  `serde_json`, `chrono`, `uuid`. Dev: `tokio`. **Без sqlx** (намеренно,
  `Cargo.toml:22`).
- **Зависимые:** `nebula-api`, `nebula-credential`, `nebula-engine`,
  `nebula-tenancy` (декораторы), `nebula-storage` (адаптер-имплементор).
  `apps/server` **явно НЕ** зависит напрямую (комментарий `apps/server/Cargo.toml:18`)
  — concrete-adapter и tenancy-декоратор сшиваются только в композиционных корнях
  (api `AppState`, knife-тест).

## 4. Внутренняя архитектура

- `lib.rs` — корень, re-export `Scope`/`StorageError`/`FencingToken`/
  `TransitionBatch{,Builder,Outcome}`.
- `batch.rs` — `TransitionBatch`: одна транзакция (state + outbox + journal) под
  CAS+fencing; поля приватны, конструкция только через builder.
- `error.rs` — единый `StorageError`, fail-closed (`#[non_exhaustive]`).
- `ids.rs` — id-шов: re-export core-ULID + `FencingToken`.
- `scope.rs` — plain-data `Scope` без политики + `credential_owner_id`.
- `store/` — 10 файлов ISP-сегрегированных ролевых трейтов (execution,
  control_queue, workflow, node_result, journal, checkpoint, idempotency,
  identity, webhook, refresh_claim).
- `dto/` — 8 файлов port-локальных строк, только `serde_json::Value` (никаких
  higher-tier типов).
- `tests/` — batch, conformance_contract, dto, error, object_safe, scope.

Поток данных: потребитель держит `Arc<dyn …Store>` → собирает `TransitionBatch`
билдером (scope/CAS/fencing обязательны) → `ExecutionStore::commit` атомарно
применяет state+outbox+journal; адаптер из `nebula-storage` материализует это в
конкретный backend, а decorator-стек (encryption/cache/audit/tenancy) оборачивает
тот же object-safe трейт.

## 5. Инварианты и контракты

- **§12.2 атомарность.** `TransitionBatch` — единственный путь мутации
  execution-агрегата: state + outbox + journal коммитятся одной транзакцией;
  scope, CAS-предусловие и `FencingToken` **обязательны структурно** (приватные
  поля + builder-only), их нельзя забыть — `commit` нельзя вызвать с неполным
  batch (`src/batch.rs:19`).
- **Fencing против zombie-runner.** `FencingToken(u64)` монотонен; устаревший
  владелец lease получает `StorageError::FencedOut` (`src/ids.rs:17`,
  `src/error.rs:11`).
- **Owner-isolation by-construction.** `Scope::credential_owner_id()` —
  length-prefixed, collision-safe деривация (ADR-0088 D7): два разных
  `(workspace_id, org_id)` не могут схлопнуться в один owner_id. Это единственная
  каноническая деривация для всех credential-производителей (`src/scope.rs:50`).
- **Scope = значение, не политика.** Порт хранит `Scope` как данные; энфорсмент
  кросс-тенантного запрета вынесен в `nebula-tenancy` — порт не может «забыть»
  проверку, потому что проверка вообще не его ответственность.
- **Fail-closed ошибки.** `StorageError` `#[non_exhaustive]`; неизвестная
  schema-версия → `UnknownSchemaVersion` (forward-compat), нарушение scope →
  `ScopeViolation` (`src/error.rs:11`).
- **Object-safety.** Все store-трейты `dyn`-совместимы (тест `object_safe`);
  per-call boxed-future — шум на фоне сетевого/дискового I/O.
- **refresh-CAS корректность.** `RefreshClaimStore` re-homed shape-unchanged и
  loom-verified (ADR-0041): claim/heartbeat/reclaim — атомарный CAS против
  двойного refresh между репликами.

## 6. Известные напряжения / долг

1. **`RefreshClaimStore` — credential-домен внутри storage-порта.**
   `src/store/refresh_claim.rs:1-10` помечен «re-homed shape-unchanged»; backend-
   ошибка деградирована до `String` вместо typed-варианта. Это **одна из двух
   копий refresh-CAS**, отмеченных в аудите credential-rewrite — кандидат на
   переезд при коллапсе credential-крейтов (см. §7).
2. **Несоответствие id-шва.** `ids.rs` re-export'ит типизированные ULID, но
   сигнатуры трейтов берут `&str`/`String`: `ExecutionStore::create(scope, id:
   &str, workflow_id: &str, …)` (`src/store/execution.rs:19-25`),
   `TransitionBatch.execution_id: String` (`src/batch.rs:21`). Типизация теряется
   на границе порта — id-newtype не доходят до сигнатур.
3. **Исторические заметки о снятом legacy-кодировании id** в доках
   (`src/store/control_queue.rs:21`, `src/dto/control.rs:37`) — кода нет, чисто
   комментарии; кандидаты на вычистку.
4. **Мелкая неполнота карты.** `store/checkpoint.rs` и `store/webhook.rs` не
   упомянуты в AGENTS.md «Key files».
5. TODO/FIXME/deprecated — отсутствуют; README ↔ код противоречий не найдено.

## 7. Роль в пост-0092 credential/resource модели

Крейт **не переписывается**, но держит два шва, без которых post-0092 модель не
сходится:

- **`Scope::credential_owner_id` — основа owner-isolation.** В post-0092
  `nebula-credential` (единый крейт: контракт + runtime + `CredentialService`
  facade + builtin-типы) facade-уровневая tenant-изоляция строится на owner_id; а
  единственная каноническая, collision-safe деривация owner_id живёт **здесь**
  (`src/scope.rs:50`, ADR-0088 D7 — закрытие split-brain). Conference-correction
  «OwnerScopedKey owner isolation» опирается ровно на этот примитив. Шов
  стабилен; меняться не должен.

- **`RefreshClaimStore` — persistence-шов refresh-CAS, чьё место жительства под
  вопросом.** Credential full-rewrite (carry-forward 2026-06-01) числит refresh-
  CAS как **дубль ×2**. После merge `nebula-credential-runtime → nebula-credential`
  и появления узкого typed `RefreshTransport`-шва (conference-correction)
  встаёт вопрос: остаётся ли claim-store контрактом в storage-порте, или
  переезжает к credential как доменный шов. Текущее состояние — re-homed
  shape-unchanged + деградация ошибки до `String` (§6.1) — это явный долг,
  ожидающий решения; **не закрыт** этим документом.

- **Зависимость credential → порт прямая.** `nebula-credential` зависит от
  `nebula-storage-port` напрямую; storage-адаптер реализует `RefreshClaimRepo`
  и держит `KeyProvider`/`Encryption` decorator-стек — то есть durable-сторона
  credential lifecycle бьётся об этот контракт. lease как first-class
  (conference-correction) на стороне execution отражён в lease-методах
  `ExecutionStore`; для credential lease-семантика остаётся за facade, не за
  портом.

- **Resource-redesign (ADR-0093/topology) крейт НЕ трогает.** `ResourceStore`
  здесь — это identity-CRUD строка `ResourceRow`
  (`src/store/identity.rs`), а **не** `nebula-resource`. Per-slot rotation fan-out,
  SlotCell, Manager, topology живут в `nebula-resource` и порт не задевают. Это
  важно не перепутать: имя `ResourceStore` коллизирует по смыслу, но домены
  разные.

## 8. Forward design / открытые вопросы

1. **Решить место жительства refresh-CAS.** До или одновременно с финализацией
   коллапса credential-крейтов: оставить `RefreshClaimStore` контрактом в порте
   ИЛИ перенести в `nebula-credential` как доменный шов. При любом исходе —
   поднять backend-ошибку из `String` обратно в typed-вариант `StorageError`
   (закрыть §6.1). Не дублировать вторую копию CAS.
2. **Закрыть id-шов (§6.2).** Протащить типизированные ULID в сигнатуры
   store-трейтов и поля DTO (`execution_id: ExecutionId` вместо `String`) —
   восстановить типизацию на границе порта. Breaking для всех имплементоров
   (`nebula-storage`) и потребителей; делать одной волной expand→contract.
3. **Вычистить исторические заметки** о снятом legacy-кодировании id
   (`control_queue.rs:21`, `dto/control.rs:37`) и дополнить AGENTS.md «Key files»
   пропущенными `checkpoint.rs`/`webhook.rs` (§6.3–6.4).
4. **Не путать `ResourceStore` (identity-row) с `nebula-resource`** при будущих
   правках — рассмотреть переименование строки/трейта, если коллизия имён начнёт
   вводить в заблуждение при resource-работах.
5. **Риск:** порт — Core-tier контракт с пятью прямыми потребителями; любое
   изменение сигнатур (особенно id-шов) — синхронная волна по api/credential/
   engine/tenancy/storage. Планировать как breaking-refactor (expand-contract,
   green-per-commit), не как точечную правку.
