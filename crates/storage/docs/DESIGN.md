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
- три бэкенда: InMemory (всегда), SQLite (feature `sqlite`), Postgres (feature `postgres`).

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
`ControlQueue`, `WebhookActivationStore`, `RefreshClaimStore`, identity-стора; `Scope`).
Этот крейт даёт адаптеры:

| Item | Where |
|------|-------|
| `StorageError` (крейт-локальный enum) | `src/error.rs:13`, реэкспорт `src/lib.rs:102` |
| `StorageFormat` (JSON / MessagePack) | `src/format.rs:14` |
| `InMemoryExecutionStore` / `InMemoryIdempotencyGuard` | `src/inmem/execution.rs:69,362` |
| `InMemoryControlQueue` | `src/inmem/control_queue.rs:26` |
| workflow/journal/checkpoint/node-result/identity in-mem | реэкспорт `src/inmem/mod.rs:19-29`, `src/lib.rs:104-109` |
| `sqlite::init_schema` | `src/sqlite/mod.rs:39` |
| `SqliteExecutionStore` / `SqliteControlQueue` | `src/sqlite/execution.rs:20`, `src/sqlite/control_queue.rs:21` |
| `postgres::init_schema` | `src/postgres/mod.rs:38` |
| `PgExecutionStore` / `PgControlQueue` | `src/postgres/execution.rs:21`, `src/postgres/control_queue.rs:22` |
| `repos::*` (не-портовые трейты с живыми потребителями) | `ControlQueueRepo`+`InMemoryControlQueueRepo` `src/repos/control_queue.rs`; `IdempotencyStoreRepo` `src/repos/idempotency.rs`; `WebhookActivationRepo` `src/repos/webhook_activation.rs:67`; identity-row трейты `src/repos/user.rs` (+ org/workspace/quota/trigger/audit/blob/resource) |
| `pg::*` (feature postgres) — Postgres-глю для `repos`-трейтов | `PgUserRepo` `src/pg/user.rs:41`, `PgWebhookActivationRepo` `src/pg/webhook_activation.rs:44`, `PgControlQueueRepo`, `PgSessionRepo`, … |
| `rows::*` — row-DTO (multi-tenant by construction) | `UserRow` `src/rows/user.rs:11`, `WorkflowRow`, `WebhookActivationSpec` `src/rows/webhook_activation.rs:72`, … |
| credential stores | `SqliteCredentialStore` `src/credential/sqlite.rs:40`; `PgCredentialStore` `src/credential/postgres.rs` |
| `KeyProvider` / `EnvKeyProvider` / `FileKeyProvider` | `src/credential/key_provider.rs` |
| credential decorator-слои `EncryptionLayer`/`CacheLayer`/`AuditLayer` | `src/credential/layer/` |
| `ProviderCacheLayer` / `RotationBackup` (feature rotation) / `InMemoryPendingStore` | `src/credential/provider_cache.rs`, `src/credential/backup.rs`, `src/credential/pending.rs` |
| refresh-claim (ADR-0041) | `InMemoryRefreshClaimRepo` `src/credential/refresh_claim/in_memory.rs:48`, `SqliteRefreshClaimRepo` `…/sqlite.rs:33`, `PgRefreshClaimRepo` `…/postgres.rs:22`; трейт+DTO — алиасы на порт `src/credential/refresh_claim/mod.rs:37-41` |
| `pool::{Backend, PoolConfig}` | `src/pool.rs:14,48` |
| `mapping::{ids,json,timestamps}` — утилиты row↔domain | `src/mapping/` |

## 3. Зависимости и зависимые

- **Workspace-deps:** `nebula-core`, `nebula-env`, `nebula-credential`, `nebula-crypto`,
  `nebula-storage-port`.
- **Внешние:** `sqlx` (opt; фичи postgres/sqlite), `redis` (opt), `aws-config`+`aws-sdk-s3`
  (opt), `moka`, `uuid`, `parking_lot`, `zeroize`, `base64`, `sha2`, `rmp-serde` (opt).
- **Фичи:** `sqlite`, `postgres` (TLS rustls-native-roots по умолчанию), `redis`, `s3`,
  `rotation` (→ `nebula-credential/rotation`), `credential-in-memory`, `msgpack-storage`.
- **Зависимые:** `nebula-engine` (`crates/engine/Cargo.toml:39,72`), `nebula-api`
  (`crates/api/Cargo.toml:17,134`), `apps/server` (`apps/server/Cargo.toml:22`),
  `examples` (`examples/Cargo.toml:25`). Соседний `nebula-storage-loom-probe` сознательно
  БЕЗ dep (cfg loom).

## 4. Внутренняя архитектура

- `inmem/` — in-memory порт-адаптеры (один `parking_lot::Mutex` на стор; tests /
  single-process / loom).
- `sqlite/` (feature) — порт-адаптеры над `port_*`-схемой, single-writer; embedded
  `schema.sql` + `init_schema` для `:memory:` / тест-пулов.
- `postgres/` (feature) — production порт-адаптеры (real tx + `FOR UPDATE SKIP LOCKED`).
- `pg/` (feature postgres) — Postgres-глю для **residual** `repos`-трейтов (identity rows,
  control-queue, oauth_state, pat, session…).
- `repos/` — residual не-портовые трейты (outbox, idempotency-cache, webhook-activation,
  identity rows) с живыми потребителями (API idempotency-middleware, `pg::*`-глю).
- `rows/` — row-DTO структуры (multi-tenant by construction: `workspace_id`/`org_id`
  обязательны).
- `credential/` — credential-стора, `KeyProvider`, decorator-слои (`layer/`: encryption,
  audit, cache), `provider_cache`, `pending`, `backup`, `refresh_claim/`.
- `mapping/` — конверсии ids/json/timestamps; `format.rs` — JSON/MessagePack; `pool.rs` —
  конфиг пула; `error.rs` — `StorageError`.
- `test_support/` (cfg test) — fixtures + `sqlite_memory_*` harness; `tests/` —
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
- **[L2-§11.3] Idempotency.** Форма ключа `{execution_id}:{node_id}:{attempt}`; адаптер
  складывает scope в storage — каллеры не могут шарить ключи между тенантами (first-writer-
  wins).
- **[L2-§11.5] Durable journal, best-effort checkpoint.** `TransitionBatch::journal`
  пишется в том же commit, что и переход (append-only, replayable). `CheckpointStore` —
  best-effort: сбой логируется, не абортит исполнение.
- **[L2-§12.2] Atomic outbox.** `execution_control_queue` пишется в **той же логической
  операции**, что и сопровождаемый переход; cancel-сигнал enqueue-ится атомарно с
  `cancelling`-переходом (нельзя «переход без enqueue» или «enqueue без перехода»).
- **[L2-§12.3] One local path.** Дефолтный локальный путь — SQLite (file или `:memory:`);
  in-process тесты идут через `test_support` (`sqlite_memory_*`), не через отдельный
  HashMap-backend.
- **[ADR-0041] Refresh-claim atomicity.** `try_claim` атомарен под контеншеном — ровно один
  из N acquirers по N репликам выигрывает (CAS `INSERT … ON CONFLICT DO UPDATE WHERE
  expires_at < now()` в SQL; per-key Mutex-swap в in-memory). `heartbeat` валидирует
  `ClaimToken.generation` (stale-holder не продлит reclaimed-claim); `reclaim_stuck`
  возвращает reclaimed-credentials атомарно (иначе sentinel-state un-observed, N=3-in-1h
  escalation недосчитан).
- **Multi-tenant by construction.** `rows::*` несут обязательные `workspace_id`/`org_id`;
  identity-стора tenant-scoped на уровне row-DTO.

## 6. Известные напряжения / долг

1. **Два `StorageError`.** README.md:52 говорит «`StorageError` (re-exported from the
   port)», но `src/lib.rs:102` реэкспортирует **крейт-локальный** enum `src/error.rs:13`;
   при этом порт-адаптеры возвращают `nebula_storage_port::StorageError`
   (`src/sqlite/mod.rs:39`, `src/postgres/mod.rs:38`). Двойственность типов ошибок
   порт vs residual-repos.
2. **redis/s3 объявлены, кода нет.** Фичи и deps в `Cargo.toml:62-75,102-103`, но в `src/`
   нет ни одного redis/s3 модуля; cargo-shear ignored (`Cargo.toml:127-132`,
   «implementation is landing incrementally»). README:232-233 называет их «experimental» —
   фактически пустые фичи.
3. **Дубль control-queue / idempotency / webhook-activation.** Портовая семья
   (`inmem/sqlite/postgres::*ControlQueue`, `*IdempotencyStore`, `*WebhookActivationStore`)
   И residual `repos::ControlQueueRepo`/`IdempotencyStoreRepo`/`WebhookActivationRepo` +
   `pg::*Repo`. Задокументировано как намеренный остаток (`lib.rs:25-36`), но это две
   параллельные реализации одних концернов.
4. **`pg/` vs `postgres/`.** Два Postgres-дерева с разными ролями, имена почти
   неразличимы (`src/pg/control_queue.rs` vs `src/postgres/control_queue.rs`).
5. **Legacy-алиасы refresh_claim.** `RefreshClaimStore as RefreshClaimRepo`,
   `RefreshClaimError as RepoError` (`src/credential/refresh_claim/mod.rs:37-41`) —
   rename-on-import ради исторических путей потребителей.
6. **Стейл README §ADR-0009** (README.md:99-105): ссылается на
   `ExecutionRepo::set_workflow_input` / `ExecutionRepoError::UnknownSchemaVersion`, хотя
   сам README (56-57, 142-143) объявляет `ExecutionRepo` удалённым по ADR-0072.
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
- **Owner-scoped isolation.** Тенант-изоляция делается row-DTO `workspace_id`/`org_id` +
  `nebula-tenancy`-декораторами; это согласуется с `OwnerScopedKey` owner-isolation
  (conference correction) — durable-сторона уже multi-tenant by construction.

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
`repos::ResourceRepo` + `rows` + identity-стора — потенциальное место durable bind-state,
но сейчас resource-редизайн кода здесь не менял.

## 8. Forward design / открытые вопросы

- **Унифицировать `StorageError`.** Решить порт-локальный vs крейт-локальный enum
  (напряжение №1) — выбрать один канон до того, как residual-repos семья вырастет; сейчас
  README врёт про re-export.
- **Свернуть refresh-CAS×2.** Дубль refresh-claim между storage и credential-rewrite-планом
  — закрыть _до_ старта rewrite credential (иначе мигрируем дубль). Решить, чья сторона
  владеет CAS-предикатом.
- **Дедуп control-queue / idempotency / webhook-activation.** Портовая семья vs
  `repos::*Repo` — два пути к одним концернам; либо мигрировать потребителей residual на
  порт, либо явно зафиксировать why-two в ADR (сейчас только `lib.rs`-комментарий).
- **redis/s3: реализовать или удалить.** Пустые фичи + ignored-shear — либо landing
  завершить, либо снять deps/фичи (честность Cargo.toml).
- **Postgres runtime-verify.** Снять `DATABASE_URL`-gate в CI (M7 ROADMAP) — единственный
  residual после spec-16 merge; до этого «pg-verified» нельзя заявлять.
- **`pg/` vs `postgres/` именование.** Переименовать одно из деревьев — текущая
  неразличимость имён множит ошибки навигации.
- **Durable bind-state (M12.4).** Когда resource bind-population дойдёт до production
  producer, решить, садится ли durable bind-state в `repos::ResourceRepo`/`rows` здесь или
  в отдельный шов — спроектировать ДО, чтобы не вклеивать ad-hoc.
