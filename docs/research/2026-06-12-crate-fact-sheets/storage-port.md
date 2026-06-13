# nebula-storage-port — fact sheet

## Назначение
Чистый контрактный крейт (Core-tier) хранилища: object-safe `#[async_trait]` repository-трейты,
port-локальные DTO-строки, plain-data `Scope`, `StorageError` и атомарный unit-of-work
`TransitionBatch`. Никакого backend-кода и **никакого sqlx** — адаптеры живут в `nebula-storage`,
scope-политика в `nebula-tenancy`. Контракт зафиксирован в ADR-0072 (+ ADR-0041 для RefreshClaimStore).

## Публичная поверхность
- `Scope { workspace_id, org_id }` — src/scope.rs:11; `Scope::credential_owner_id()` (length-prefixed, collision-safe, ADR-0088 D7) — src/scope.rs:50
- `StorageError` (#[non_exhaustive]: NotFound/Conflict/Duplicate/LeaseUnavailable/FencedOut/Timeout/UnknownSchemaVersion/ScopeViolation/Serialization…) — src/error.rs:11
- `FencingToken(u64)` — монотонный lease-токен против zombie-runner — src/ids.rs:17
- `ids::*` — re-export типизированных ULID из nebula-core (ExecutionId, OrgId, WorkflowId…) — src/ids.rs:7
- `TransitionBatch` (приватные поля, builder-only; scope+CAS+fencing обязательны структурно) — src/batch.rs:19; `TransitionBatchBuilder` — src/batch.rs:86; `TransitionOutcome` — src/batch.rs:177
- `ExecutionStore` — атомарный агрегат §12.2: create/get/`commit(TransitionBatch)`/acquire_lease/renew_lease/release_lease — src/store/execution.rs:17
- `ControlQueue` + `ReclaimOutcome` — src/store/control_queue.rs:25,9
- `WorkflowStore` / `WorkflowVersionStore` — src/store/workflow.rs:8,86
- `NodeResultStore` — src/store/node_result.rs:12; `ExecutionJournalReader` — src/store/journal.rs:10; `CheckpointStore` — src/store/checkpoint.rs:12
- `IdempotencyGuard` / `IdempotencyStore` — src/store/idempotency.rs:15,41
- Identity-семейство (9 трейтов): `UserStore`/`OrgStore`/`WorkspaceStore`/`MembershipStore`/`ResourceStore`/`TriggerStore`/`QuotaStore`/`AuditStore`/`BlobStore` — src/store/identity.rs:17–157
- `WebhookActivationStore` — src/store/webhook.rs:10
- `RefreshClaimStore` + `ReplicaId`/`ClaimToken`/`RefreshClaim`/`ClaimAttempt`/`HeartbeatError`/`RefreshClaimError`/`SentinelState`/`ReclaimedClaim` — src/store/refresh_claim.rs:144,21–130 (re-homed shape-unchanged из адаптера, loom-verified, ADR-0041)
- DTO: `ExecutionRecord`, `WorkflowRecord`/`WorkflowVersionRecord`, `ControlMsg`/`ControlCommand`, `JournalEntry`, `NodeResultRecord` (+`MAX_SUPPORTED_RESULT_SCHEMA_VERSION`), `CachedRecord`, `WebhookActivationRecord`, identity-rows (`UserRow`…`BlobRow`, `ScopeKind`, `PrincipalKind`) — src/dto/

## Workspace-зависимости
- Deps: `nebula-core` (path) + async-trait, thiserror, serde, serde_json, chrono, uuid. Dev: tokio. Без sqlx (намеренно, комментарий в Cargo.toml:22).
- Кто зависит: `nebula-api`, `nebula-credential`, `nebula-engine`, `nebula-tenancy` (декораторы), `nebula-storage` (адаптер-имплементор). apps/server явно НЕ зависит (комментарий apps/server/Cargo.toml:18).

## Структура модулей
- `lib.rs` — корень, re-export Scope/StorageError/FencingToken/TransitionBatch{,Builder,Outcome}
- `batch.rs` — TransitionBatch: одна транзакция state+outbox+journal под CAS+fencing
- `error.rs` — единый StorageError, fail-closed
- `ids.rs` — id-шов (re-export core-ULID) + FencingToken
- `scope.rs` — plain-data Scope без политики + credential_owner_id
- `store/` — 10 файлов ISP-сегрегированных ролевых трейтов (см. выше)
- `dto/` — 8 файлов port-локальных строк, только serde_json::Value (никаких higher-tier типов)
- `tests/` — batch, conformance_contract, dto, error, object_safe, scope

## Напряжения
- `RefreshClaimStore` — credential-домен внутри storage-порта (src/store/refresh_claim.rs:1-10): «re-homed shape-unchanged», backend-ошибка деградирована до `String` вместо typed. Это одна из двух копий refresh-CAS, отмеченных в аудите credential-rewrite — кандидат на переезд при коллапсе credential-крейтов.
- Несоответствие id-шва: `ids.rs` re-export'ит типизированные ULID, но сигнатуры трейтов берут `&str`/`String` (`ExecutionStore::create(scope, id: &str, workflow_id: &str, …)` src/store/execution.rs:19-25; `TransitionBatch.execution_id: String` src/batch.rs:21) — типизация теряется на границе порта.
- Упоминания снятого legacy-кодирования id в доках (src/store/control_queue.rs:21, src/dto/control.rs:37) — чисто исторические заметки, кода нет.
- TODO/FIXME/deprecated — отсутствуют. README ↔ код противоречий не найдено (README и AGENTS.md точны).
- `store/checkpoint.rs` и `store/webhook.rs` не упомянуты в AGENTS.md «Key files» (мелкая неполнота карты).

## Роль в credential/resource redesign
Затронут косвенно, но существенно: (1) `Scope::credential_owner_id` — единственная каноническая деривация owner_id для всех credential-производителей (ADR-0088 D7, закрытие split-brain); (2) `RefreshClaimStore` — persistence-шов refresh-CAS, который credential full-rewrite (carry-forward 2026-06-01) числит как дубль ×2 — при merge runtime→credential его место жительства надо решить заново; (3) `nebula-credential` напрямую зависит от порта. Resource-redesign (ADR-0093/topology) крейт не трогает — `ResourceStore` здесь это identity-CRUD строка `ResourceRow`, не nebula-resource.
