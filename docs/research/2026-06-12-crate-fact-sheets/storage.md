# nebula-storage — fact sheet

## Назначение
Единственная крейт-реализация (адаптеры) spec-16 контракта `nebula-storage-port`: execution CAS-state
(`commit` + lease `FencingToken`), append-only журнал, control-queue outbox (атомарно в `TransitionBatch`),
idempotency, identity-зоопарк (user/org/workspace/...), плюс credential-персистенция (durable stores,
шифрование/аудит/кэш-слои, ADR-0041 refresh-claim repo). Бэкенды: InMemory + SQLite (feature) + Postgres (feature).

## Публичная поверхность
- `StorageError` — крейт-локальный enum, `src/error.rs:13`; реэкспорт в `src/lib.rs:102`
- `StorageFormat` (JSON/MessagePack) — `src/format.rs:14`
- `inmem::*` порт-адаптеры: `InMemoryExecutionStore`/`InMemoryIdempotencyGuard` `src/inmem/execution.rs:69,362`; `InMemoryControlQueue` `src/inmem/control_queue.rs:26`; workflow/journal/checkpoint/node-result/identity — реэкспорт `src/inmem/mod.rs:19-29` и `src/lib.rs:104-109`
- `sqlite::init_schema` `src/sqlite/mod.rs:39`; `SqliteExecutionStore` `src/sqlite/execution.rs:20`; `SqliteControlQueue` `src/sqlite/control_queue.rs:21`; identity-стора `src/sqlite/identity.rs` (User/Org/Workspace/Membership/Resource/Trigger/Quota/Audit/Blob)
- `postgres::init_schema` `src/postgres/mod.rs:38`; `PgExecutionStore` `src/postgres/execution.rs:21`; `PgControlQueue` `src/postgres/control_queue.rs:22`; identity-стора `src/postgres/identity.rs`
- `repos::*` — НЕ-портовые трейты с живыми потребителями: `ControlQueueRepo`+`InMemoryControlQueueRepo` `src/repos/control_queue.rs`; `IdempotencyStoreRepo` `src/repos/idempotency.rs`; `WebhookActivationRepo` `src/repos/webhook_activation.rs:67`; identity-row трейты (`UserRepo`/`SessionRepo`/`PatRepo`/`OAuthStateRepo`/... `src/repos/user.rs`, org/workspace/quota/trigger/audit/blob/resource)
- `pg::*` (feature postgres) — Postgres-глю для `repos`-трейтов: `PgControlQueueRepo`, `PgUserRepo` `src/pg/user.rs:41`, `PgSessionRepo`, `PgWebhookActivationRepo` `src/pg/webhook_activation.rs:44` и т.д.
- `rows::*` — row-DTO (`UserRow` `src/rows/user.rs:11`, `WorkflowRow`, `WebhookActivationSpec` `src/rows/webhook_activation.rs:72`, ...)
- credential: `SqliteCredentialStore` `src/credential/sqlite.rs:40`; `PgCredentialStore` `src/credential/postgres.rs`; `KeyProvider`/`EnvKeyProvider`/`FileKeyProvider` `src/credential/key_provider.rs`; слои `EncryptionLayer`/`CacheLayer`/`AuditLayer` `src/credential/layer/`; `ProviderCacheLayer` `src/credential/provider_cache.rs`; `RotationBackup` (feature rotation) `src/credential/backup.rs`; `InMemoryPendingStore` (test/credential-in-memory) `src/credential/pending.rs`
- refresh-claim (ADR-0041): `InMemoryRefreshClaimRepo` `src/credential/refresh_claim/in_memory.rs:48`, `SqliteRefreshClaimRepo` `…/sqlite.rs:33`, `PgRefreshClaimRepo` `…/postgres.rs:22`; трейт+DTO — алиасы на порт `src/credential/refresh_claim/mod.rs:37-41`
- `pool::{Backend, PoolConfig}` `src/pool.rs:14,48`; `mapping::{ids,json,timestamps}` — утилиты row↔domain

## Workspace-зависимости
Deps: nebula-core, nebula-env, nebula-credential, nebula-crypto, nebula-storage-port; внешние: sqlx (opt, фичи postgres/sqlite), redis (opt), aws-config+aws-sdk-s3 (opt), moka, uuid, parking_lot, zeroize, base64, sha2, rmp-serde (opt).
Фичи: `sqlite`, `postgres` (TLS rustls-native-roots по умолчанию), `redis`, `s3`, `rotation` (→ nebula-credential/rotation), `credential-in-memory`, `msgpack-storage`.
Зависят от nebula-storage: nebula-engine (`crates/engine/Cargo.toml:39,72`), nebula-api (`crates/api/Cargo.toml:17,134`), apps/server (`apps/server/Cargo.toml:22`), examples (`examples/Cargo.toml:25`). Соседний `nebula-storage-loom-probe` сознательно БЕЗ dep (cfg loom).

## Структура модулей
- `inmem/` — in-memory порт-адаптеры (один parking_lot::Mutex на стор; tests/single-process/loom)
- `sqlite/` (feature) — порт-адаптеры над port_*-схемой, single-writer; embedded `schema.sql` + `init_schema`
- `postgres/` (feature) — продакшен порт-адаптеры (real tx + `FOR UPDATE SKIP LOCKED`)
- `pg/` (feature postgres) — Postgres-глю для residual `repos`-трейтов (identity rows, control-queue, oauth_state, pat, session...)
- `repos/` — residual не-портовые трейты (outbox, idempotency-cache, webhook-activation, identity rows)
- `rows/` — row-DTO структуры (multi-tenant by construction: workspace_id/org_id обязательны)
- `credential/` — credential-стора, KeyProvider, слои (layer/: encryption, audit, cache), provider_cache, pending, backup, refresh_claim/
- `mapping/` — конверсии ids/json/timestamps; `format.rs` — JSON/MessagePack; `pool.rs` — конфиг пула; `error.rs` — StorageError
- `test_support/` (cfg test) — fixtures + sqlite_memory_* harness; tests/ — конформанс-матрица {InMemory,SQLite,Pg} + tenancy-декораторы

## Напряжения
- **Два StorageError**: README.md:52 утверждает «`StorageError` (re-exported from the port)», но `src/lib.rs:102` реэкспортирует крейт-ЛОКАЛЬНЫЙ enum `src/error.rs:13`; порт-адаптеры при этом возвращают `nebula_storage_port::StorageError` (`src/sqlite/mod.rs:39`, `src/postgres/mod.rs:38`). Двойственность типов ошибок порт vs residual-repos.
- **redis/s3 объявлены, кода нет**: фичи и deps в Cargo.toml:62-75,102-103, но в src/ нет ни одного redis/s3 модуля; cargo-shear ignored (Cargo.toml:127-132 «implementation is landing incrementally»). README:232-233 называет их «experimental» — фактически пустые фичи.
- **Дубль control-queue/idempotency/webhook-activation**: портовая семья (`inmem/sqlite/postgres::*ControlQueue`, `*IdempotencyStore`, `*WebhookActivationStore`) И residual `repos::ControlQueueRepo`/`IdempotencyStoreRepo`/`WebhookActivationRepo` + `pg::*Repo`. Задокументировано как намеренный остаток (lib.rs:25-36), но это две параллельные реализации одних концернов.
- **pg/ vs postgres/** — два Postgres-дерева с разными ролями, имена почти неразличимы (`src/pg/control_queue.rs` vs `src/postgres/control_queue.rs`).
- **Legacy-алиасы refresh_claim**: `RefreshClaimStore as RefreshClaimRepo`, `RefreshClaimError as RepoError` (`src/credential/refresh_claim/mod.rs:37-41`) — rename-on-import ради исторических путей потребителей.
- **Стейл README §ADR-0009** (README.md:99-105): ссылается на `ExecutionRepo::set_workflow_input` / `ExecutionRepoError::UnknownSchemaVersion`, хотя сам README (строки 56-57, 142-143) объявляет `ExecutionRepo` удалённым по ADR-0072.
- AGENTS.md:37 «Cross-crate calls go through nebula-eventbus» — у крейта нет dep на eventbus; правило-копипаста из корневого AGENTS.md.
- TODO/FIXME/deprecated в src/ — отсутствуют (grep чисто).

## Роль в credential/resource redesign
Затронут напрямую: здесь живёт вся credential-персистенция (SqliteCredentialStore/PgCredentialStore,
EncryptionLayer/AuditLayer/CacheLayer, KeyProvider, ProviderCacheLayer, refresh_claim CAS-repo, pending, RotationBackup).
План полного rewrite nebula-credential (memory: project_credential_rewrite_plan) фиксирует дубль refresh-CAS×2 и
«dead SQL row-model» — обе точки указывают сюда; merge runtime→credential и single-public-sdk
(project_single_public_crate_sdk) делают границу storage↔credential пересматриваемой. Resource redesign (ADR-0093,
bind-population M12.4) крейт затрагивает косвенно: `repos::ResourceRepo`/`rows` + identity-стора — потенциальное
место durable bind-state, но сейчас resource-редизайн кода здесь не менял.
