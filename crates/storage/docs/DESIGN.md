# nebula-storage — design

| Field | Value |
|-------|-------|
| **Status** | Partial — single port-adapter крейт; Postgres compile-verified, `DATABASE_URL`-gated runtime |
| **Layer** | Adapter (реализует `nebula-storage-port`; над ним `nebula-tenancy`-декораторы, `engine` / `api`) |
| **Redesign role** | **Затронут напрямую** — здесь живёт вся credential-персистенция (durable stores, `EncryptionLayer`/`AuditLayer`/`CacheLayer`, `KeyProvider`, refresh-claim CAS-repo); пост-0092 граница storage↔credential пересматривается |
| **Related** | ADR-0072 (port/adapter/tenancy), ADR-0041 (durable refresh-claim store), ADR-0088/ADR-0092 (credential rewrite/consolidation), PRODUCT_CANON §11.1/§11.3/§11.5/§12.2/§12.3 |

---

## 1. Назначение и границы

`nebula-storage` — **единственная крейт-реализация (адаптеры) spec-16 контракта**
`nebula-storage-port`. Это persistence-шов, который engine и API гоняют без привязки
к конкретной БД.

**Владеет:**
- execution CAS-state (`ExecutionStore::commit` + lease `FencingToken`), append-only
  журнал, control-queue outbox — атомарно в одном `TransitionBatch`;
- idempotency-ключи, checkpoint-стора, node-result-стора, workflow/version-стора;
- identity-зоопарк (user / org / workspace / membership / resource / trigger / quota /
  audit / blob);
- credential-персистенцию: durable stores, шифрование / аудит / кэш-слои, `KeyProvider`,
  ADR-0041 refresh-claim CAS-repo;
- SQLite (feature `sqlite`) и Postgres (feature `postgres`) как deployment-бэкенды;
  InMemory — только internal test/reference/conformance adapter.

**ЯВНО НЕ делает** (из non-goals README + границы fact-sheet):
- не execution state-machine (типы состояний / легальность переходов — `nebula-execution`);
- не engine-оркестратор (драйвит порт `ExecutionStore` — `nebula-engine`);
- не action-dispatcher (`nebula-runtime`);
- не KV-кэш (Redis) как production execution-backend — Redis-фича только KV;
- не key-storage логика _шифрования_: AES-256-GCM / Argon2id + `Cipher`/`Kdf`-порты живут в
  `nebula-crypto` (ADR-0088); storage держит _ключи_ (`KeyProvider`) и _обёртку_
  (`EncryptionLayer`), но не сами примитивы.

## 2. Публичная поверхность

Контракт — это порт в `nebula-storage-port` (`ExecutionStore` + атомарный
`TransitionBatch`, `ExecutionJournalReader`, `NodeResultStore`, `CheckpointStore`,
`IdempotencyGuard`/`IdempotencyStore`, `WorkflowStore`/`WorkflowVersionStore`,
`ControlQueue`, `WebhookActivationStore`, `RefreshClaimStore`, identity-стора,
owner-bound `CredentialPersistence`; `Scope`).
Этот крейт даёт адаптеры:

| Item | Where |
|------|-------|
| `StorageError` (крейт-локальный enum) | `src/error.rs:13`, реэкспорт `src/lib.rs:102` |
| `InMemoryExecutionStore` / `InMemoryIdempotencyGuard` | `src/inmem/execution.rs:69,362` |
| `InMemoryControlQueue` | `src/inmem/control_queue.rs:26` |
| workflow/journal/checkpoint/node-result/identity in-mem | реэкспорт `src/inmem/mod.rs:19-29`, `src/lib.rs:104-109` |
| `sqlite::init_schema` | `src/sqlite/mod.rs:39` |
| `SqliteExecutionStore` / `SqliteControlQueue` | `src/sqlite/execution.rs:20`, `src/sqlite/control_queue.rs:21` |
| `postgres::init_schema` | `src/postgres/mod.rs:38` |
| `PgExecutionStore` / `PgControlQueue` | `src/postgres/execution.rs:21`, `src/postgres/control_queue.rs:22` |
| `auth::*` — Plane-A аккаунты (вне порта): трейты и строки | `UserRepo`/`SessionRepo`/`PatRepo`/`OAuthStateRepo`/`ExternalIdentityRepo`/`VerificationTokenRepo` `src/auth/repos.rs`; `MfaEnrollmentRepo`; `UserRow`, `SessionRow`, …; `identity_secret`, `session_token` |
| `auth::postgres::*` (feature postgres) — реализации | `PgUserRepo`, `PgSessionRepo`, `PgPatRepo`, `PgOAuthLoginFinalizer`, `PgIdentitySecretMigrator`, … |
| `http_idempotency::*` — кэш идемпотентности API | `IdempotencyStoreRepo`, `InMemoryIdempotencyStoreRepo`, `PgHttpIdempotencyStore` |
| `webhook_activation::*` | `WebhookActivationSpec` |
| credential persistence | `SqliteCredentialPersistence` and `PgCredentialPersistence`, both implementing the port-local object-safe contract |
| `KeyProvider` / `EnvKeyProvider` / `FileKeyProvider` | `src/credential/key_provider.rs` |
| credential decorator-слои `EncryptionLayer`/`CacheLayer`/`AuditLayer` | `src/credential/layer/` |
| `ProviderCacheLayer` / `RotationBackup` (feature rotation) / `InMemoryPendingStore` | `src/credential/provider_cache.rs`, `src/credential/backup.rs`, `src/credential/pending.rs` |
| refresh-claim (ADR-0041) | `InMemoryRefreshClaimRepo` `src/credential/refresh_claim/in_memory.rs:48`, `SqliteRefreshClaimRepo` `…/sqlite.rs:33`, `PgRefreshClaimRepo` `…/postgres.rs:22`; трейт+DTO — алиасы на порт `src/credential/refresh_claim/mod.rs:37-41` |

## 3. Зависимости и зависимые

- **Workspace-deps:** `nebula-core`, `nebula-env`, `nebula-credential`, `nebula-crypto`,
  `nebula-storage-port`.
- **Внешние:** `sqlx` (opt; фичи postgres/sqlite), `moka`, `uuid`, `parking_lot`, `zeroize`,
  `base64`, `sha2`.
- **Фичи:** `sqlite`, `postgres` (TLS rustls-native-roots по умолчанию), `rotation`
  (→ `nebula-credential/rotation`), `credential-in-memory`.
- **Зависимые:** `nebula-engine` (`crates/engine/Cargo.toml:39,72`), `nebula-api`
  (`crates/api/Cargo.toml:17,134`), `apps/server` (`apps/server/Cargo.toml:22`),
  `examples` (`examples/Cargo.toml:25`). Соседний `nebula-storage-loom-probe` сознательно
  БЕЗ dep (cfg loom).

## 4. Внутренняя архитектура

- `inmem/` — in-memory порт-адаптеры (один `parking_lot::Mutex` на стор; tests /
  single-process / loom).
- `sqlite/` (feature) — порт-адаптеры над `port_*`-схемой, single-writer;
  `init_schema` выполняет catalog-only admission и запускает единый
  упорядоченный SQLx migration catalog для файловых, `:memory:` и тестовых
  пулов; credential-семантику проверяет только credential Ready constructor
  под тем же guard/session; отдельного schema snapshot нет.
- `postgres/` (feature) — production порт-адаптеры (real tx + `FOR UPDATE SKIP LOCKED`).
- `auth/` — Plane-A аккаунты вне порта: трейты (`repos.rs`), строки (`rows.rs`),
  OAuth-finalizer, MFA enrollment, `identity_secret`, `session_token`; PostgreSQL-реализации
  в `auth/postgres/` (feature postgres).
- `http_idempotency/` — кэш идемпотентного replay API (`IdempotencyStoreRepo`,
  in-memory и `PgHttpIdempotencyStore`); не путать с портовым `IdempotencyStore`.
- `webhook_activation.rs` — `WebhookActivationSpec` в `triggers.config`.
- `credential/` — credential-стора, `KeyProvider`, decorator-слои (`layer/`: encryption,
  audit, cache), `provider_cache`, `pending`, `backup`, `refresh_claim/`.
- `sql_error.rs` — единственная классификация `sqlx::Error` → порт-`StorageError`
  (value-free) и `decode_u64`. Крейт-локального `StorageError` нет.
- `test_support/` (cfg test) — фикстуры Plane-A строк; `tests/` —
  конформанс-матрица {InMemory, SQLite, Pg} + tenancy-декораторы.

Поток данных (execution-путь): engine собирает `TransitionBatch` (state-переход + journal-
append + control-queue enqueue) → один из `*ExecutionStore::commit` применяет CAS на
`version`, проверяет lease `FencingToken`, пишет всё в одной логической операции (tx в
SQLite/Postgres, под Mutex в InMemory).

## 5. Инварианты и контракты

- **[L2-§11.1] CAS + lease fencing.** `ExecutionStore::commit` — единственный источник
  истины execution-state; CAS на `version` + gate каждого перехода lease-токеном.
  `acquire_lease` возвращает монотонный `FencingToken`, и superseded-holder отвергается
  даже при совпадающем CAS-`version` (zombie-runner дыра закрыта; verify
  `crates/engine/tests/lease_takeover.rs`, loom-probe `lease_handoff.rs`, конформанс).
- **[L2-§11.3] Local idempotency.** Форма ключа
  `{execution_id}:{node_id}:{attempt}`; адаптер складывает scope в storage — каллеры не
  могут шарить ключи между тенантами (first-writer-wins). Это только локальный
  replay/dedup oracle: он не атомарен с remote provider, не доказывает исход внешнего
  эффекта и не даёт single-effect/exactly-once гарантию. Будущий effect ledger —
  отдельный контракт runtime control: storage-minted `EffectSlotId` задаёт намеренную
  multiplicity и связывает fingerprint со stable runtime-minted `OperationId`.
  Same-slot mismatch — `OperationMismatch` без durable delta; разные slots остаются
  разными. Только pinned stable-key destination может bounded повторить effecting call
  для той же `Prepared` operation и того же `OperationId`, пока гарантия valid;
  reconciliation всегда read-only. Exhaustion или expiry переводит operation в
  `OutcomeUnknown`, после чего effecting call нельзя повторять. `AcknowledgementUnknown`
  относится к prepare и outcome DB commits: prepare uncertainty запрещает provider
  invocation, пока DB reconciliation не подтвердит exact durable prepared record и ID;
  outcome uncertainty разрешает только ledger reads и exact frozen-evidence recommit.
- **[L2-§11.5] Durable journal, fenced iteration checkpoint.** `TransitionBatch::journal`
  пишется в том же commit, что и переход (append-only, replayable). `CheckpointStore` —
  fenced iteration checkpoints journaled stateful actions (migration 0062): запись под
  execution fence, монотонная, привязанная к action key + version. Недоступный store
  только пропускает сохранение (не абортит исполнение); потеря строки = replay с
  iteration 0. Authority над effects остаётся у operation ledger.
- **[L2-§12.2] Atomic outbox.** `execution_control_queue` пишется в **той же логической
  операции**, что и сопровождаемый переход; cancel-сигнал enqueue-ится атомарно с
  `cancelling`-переходом (нельзя «переход без enqueue» или «enqueue без перехода»).
- **[L2-§12.3] One local path.** Дефолтный локальный путь — SQLite (file или `:memory:`);
  in-process тесты открывают `sqlite::memory:` через тот же `init_schema`; `inmem` —
  эталон/конформанс-модель, не deployment-backend.
- **[ADR-0041] Refresh-claim atomicity.** `try_claim` атомарен под контеншеном — ровно один
  из N acquirers по N репликам выигрывает (CAS `INSERT … ON CONFLICT DO UPDATE WHERE
  expires_at < now() AND sentinel = Normal` в SQL; per-key Mutex-swap в in-memory).
  `heartbeat`, `mark_sentinel` и `release` валидируют UUID `claim_id` вместе с generation.
  Expired `Normal` безопасно удаляется; expired `RefreshInFlight` никогда не reclaim-ится в
  provider replay. `reclaim_stuck` атомарно записывает ровно одно evidence-событие на claim UUID,
  сохраняет poison-row и возвращает только newly-accounted incidents для threshold observation.
- **Multi-tenant by construction.** `auth::*Row` несут обязательные `workspace_id`/`org_id`;
  identity-стора tenant-scoped на уровне row-DTO.

## 6. Известные напряжения / долг

1. **Два `StorageError` — закрыт (2026-10-06).** Крейт-локальный enum удалён; все
   адаптеры возвращают порт-`StorageError`, ошибки `sqlx` классифицирует `sql_error`.
2. **Коллизия имён idempotency — закрыта (2026-10-06).** Кэш API живёт в
   `http_idempotency` (`PgHttpIdempotencyStore`), портовый per-attempt стор — в
   `postgres::PgIdempotencyStore`. Legacy `ControlQueueRepo` удалён: портовый
   `ControlQueue` — единственный outbox.
3. **`pg/` vs `postgres/` — закрыт (2026-10-06).** `pg/`+`repos/`+`rows/` заменены на
   `auth/` (+ `auth/postgres/`), `http_idempotency/`, `webhook_activation`.
5. **Legacy-алиасы refresh_claim.** `RefreshClaimStore as RefreshClaimRepo`,
   `RefreshClaimError as RepoError` (`src/credential/refresh_claim/mod.rs:37-41`) —
   rename-on-import ради исторических путей потребителей.
6. **Стейл README §ADR-0009 — закрыт.** Ссылки на
   `ExecutionRepo::set_workflow_input` / `ExecutionRepoError::UnknownSchemaVersion`
   заменены на `NodeResultStore::set_workflow_input` /
   `StorageError::UnknownSchemaVersion`; долговое замечание оставлено как
   история.
7. **AGENTS.md:37** «Cross-crate calls go through nebula-eventbus» — у крейта нет dep на
   eventbus; правило-копипаста из корневого AGENTS.md.
8. **Postgres runtime un-verified.** Pg-адаптер + identity-стора compile-verified и
   структурно идентичны runtime-verified SQLite-дереву, но runtime-покрытие
   `DATABASE_URL`-gated и skip-clean (ADR-0072 «Verification status»).

(`TODO`/`FIXME`/`deprecated` в `src/` отсутствуют — grep чисто.)

## 7. Роль в пост-0092 credential/resource модели

Этот крейт — **persistence-сторона** консолидированного credential-стека. После ADR-0092
`nebula-credential` стал одним крейтом (contract + runtime + `CredentialService`-facade +
builtin types), а `nebula-crypto` владеет `Cipher`/`Kdf`-портами. `nebula-storage`
остаётся durable-слоем: предоставляет durable stores + `Encryption`/`Cache`/`Audit`-
декораторы + `KeyProvider` + `RefreshClaimRepo`-адаптер.

**Что остаётся (швы, которые держат):**
- **`EncryptionLayer` как decorator-шов.** Через ADR-0092 `Cipher`-порт `EncryptionLayer`
  становится generic над cipher — storage даёт _обёртку_ и _ключи_ (`KeyProvider`),
  `nebula-crypto` даёт _примитив_. Это и есть inversion-seam: storage не тянет
  `aes-gcm`/`argon2` напрямую, а инжектит `Cipher`/`Kdf`.
- **`RefreshClaimRepo` как L2 durable claim.** Engine-side `RefreshCoordinator` (L1
  in-process coalescer + L2 durable) опирается на CAS-`try_claim` отсюда; narrow typed
  `RefreshTransport`-шов (conference correction) живёт _выше_ storage — крейт даёт только
  атомарный claim-стейт, не IdP-транспорт.
- **Values-only persistence.** Credential-стора персистят _значения_; схема приходит из
  зарегистрированных типов (`HasSchema` → `nebula-metadata` → API catalog), не из storage.
  Крейт не хранит схему — только зашифрованные данные + метаданные ротации.
- **Owner-scoped credential isolation.** Credential-операции напрямую принимают обязательный
  `CredentialOwner`/`CredentialSelector`; каждый SQL-предикат включает owner, а metadata stamp
  никогда не является authority. В отличие от общих Scope-taking портов, credential persistence
  намеренно не оборачивается `nebula-tenancy`-декоратором.

**Что пересматривается:**
- **Граница storage↔credential.** План full-rewrite `nebula-credential`
  (`project_credential_rewrite_plan`) фиксирует дубль **refresh-CAS×2** и «dead SQL row-
  model» — обе точки указывают _сюда_ (refresh_claim-дерево + credential row-mapping).
  Merge runtime→credential + single-public-sdk (`project_single_public_crate_sdk`) делают
  эту границу пересматриваемой: т.к. публичен только `nebula-sdk`, внутреннее
  распределение credential-персистенции между storage и credential — без внешнего semver.
- **`rotation`-фича** (→ `nebula-credential/rotation`, `RotationBackup`
  `src/credential/backup.rs`) — durable-сторона rotation-state; пост-0092 fan-out ротации
  владеет `nebula-resource` (per-slot), а storage остаётся бэкапом состояния, не драйвером.

**Resource redesign (ADR-0093, bind-population M12.4)** крейт затрагивает _косвенно_:
портовые resource-runtime адаптеры (`*/resource_runtime.rs`) — место durable bind-state.

## 8. Forward design / открытые вопросы

- **K2 owner schema migration закрыта новой `0039`.** Историческая
  `0030_credentials_store.sql` остаётся SQLx-checksummed и byte-immutable, включая legacy-комментарий.
  Paired SQLite/PostgreSQL `0039_credentials_owner_and_record_state.sql` проверяет legacy rows,
  делает owner/state структурными DB-инвариантами и добавляет nullable `claim_id` только ради
  совместимости со старым evidence; все новые sentinel incidents несут UUID и защищены глобальным
  partial unique index.
- **Свернуть refresh-CAS×2.** Дубль refresh-claim между storage и credential-rewrite-планом
  — закрыть _до_ старта rewrite credential (иначе мигрируем дубль). Решить, чья сторона
  владеет CAS-предикатом.
- **Дедуп idempotency.** Портовый `*IdempotencyStore` vs `http_idempotency::IdempotencyStoreRepo`
  (кэш API): имена разведены; перевод middleware на порт остаётся открытым вопросом.
- **Postgres runtime-verify.** Снять `DATABASE_URL`-gate в CI (M7 ROADMAP) — единственный
  residual после spec-16 merge; до этого «pg-verified» нельзя заявлять.
- **Durable bind-state (M12.4).** Когда resource bind-population дойдёт до production
  producer, спроектировать шов в resource-runtime адаптерах ДО, чтобы не вклеивать ad-hoc.
