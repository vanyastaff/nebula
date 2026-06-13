# nebula-storage-loom-probe — fact sheet

## Назначение
Standalone-крейт с loom-модельными пробами CAS-критических секций `nebula-storage`:
(1) refresh-claim для credential (ADR-0041, `InMemoryRefreshClaimRepo::try_claim`) и
(2) execution-lease handoff (spec-16 `InMemoryExecutionStore::{acquire,renew,release}_lease`).
Формы продакшен-кода переписаны вручную на `loom::sync::Mutex`; крейт намеренно НЕ зависит от
`nebula-storage`, чтобы `RUSTFLAGS="--cfg loom"` не ломал транзитивные deps
(`concurrent-queue` via `moka`→`async-lock`→`event-listener`). `publish = false`, весь код под `#![cfg(loom)]`.

## Публичная поверхность
- `ClaimRow { holder, generation, expired }` — src/lib.rs:41 (минимальная CAS-строка claim)
- `Repo { rows: Mutex<HashMap<u32, ClaimRow>> }` — src/lib.rs:52
- `Outcome::{Acquired, Contended}` — src/lib.rs:58 (зеркало `ClaimAttempt`)
- `Repo::try_claim(&self, cid, holder) -> Outcome` — src/lib.rs:69 (acquire только если строка отсутствует/expired)
- `pub mod lease_handoff` — src/lib.rs:31
- `lease_handoff::LeaseRow { holder, generation, expired, held }` — src/lease_handoff.rs:47
- `lease_handoff::LeaseRepo` — src/lease_handoff.rs:68
- `AcquireOutcome::{Acquired(u64), Contended}` — src/lease_handoff.rs:76 (↔ `Ok(Some(FencingToken))`/`Ok(None)`)
- `HolderOutcome::{Applied, Rejected}` — src/lease_handoff.rs:87 (↔ `Ok(true)/Ok(false)`)
- `LeaseRepo::acquire_lease(exec_id, holder) -> AcquireOutcome` — src/lease_handoff.rs:101 (каждый успех бампает fencing generation; монотонна per-row, release НЕ удаляет строку)
- `LeaseRepo::renew_lease(exec_id, token) -> HolderOutcome` — src/lease_handoff.rs:131 (fenced по generation, не по holder-строке)
- `LeaseRepo::release_lease(exec_id, token) -> HolderOutcome` — src/lease_handoff.rs:147 (clears `held`, строка и generation сохраняются)
- `LeaseRepo::flag_expired(exec_id) -> bool` — src/lease_handoff.rs:160 (test-only: моделирование TTL — loom не видит время)
- `LeaseRepo::snapshot(exec_id) -> Option<(u32, u64)>` — src/lease_handoff.rs:172 (test-only инспектор)

## Workspace-зависимости
- Deps: только `loom = "0.7"` (optional, за фичей `loom-test = ["dep:loom"]`); workspace-зависимостей НЕТ — это принцип крейта (Cargo.toml:15-25).
- Обратные зависимости: ни один крейт workspace не зависит от него; единственное упоминание — комментарий в `crates/storage/Cargo.toml:121`, объясняющий, почему loom-пробы вынесены сюда.
- Запуск: `RUSTFLAGS="--cfg loom" cargo nextest run -p nebula-storage-loom-probe --features loom-test --profile ci --no-tests=pass`; обычный `cargo check` компилируется «в ничто» (всё под `#![cfg(loom)]`).

## Структура модулей
- `src/lib.rs` — refresh-claim проба: `Repo::try_claim` поверх `Mutex<HashMap>` + crate-level docs «почему отдельный крейт».
- `src/lease_handoff.rs` — execution-lease проба: acquire/renew/release с fencing-generation + test-хелперы.
- `tests/refresh_claim_loom.rs` — loom-модель: 2 конкурирующих `try_claim` → ровно один `Acquired`.
- `tests/lease_handoff_loom.rs` — loom-модели: single-winner acquire, отклонение superseded-токена после takeover, stale release после expiry-takeover = no-op (companion runtime-тесты: `crates/engine/tests/lease_takeover.rs`, `crates/storage/tests/execution_lease_pg_integration.rs`).

## Напряжения
- src/lib.rs:2 — мусорная doc-строка `//!.` (опечатка, осталась от редактирования заголовка).
- Несимметричный старт generation: `try_claim` начинает с 0 (src/lib.rs:76 `map_or(0, ...)`), `acquire_lease` — с 1 (src/lease_handoff.rs:112 `map_or(1, ...)`). Вероятно отражает реальный `fencing_generation`, но в claim-пробе ничем не мотивировано — кандидат на расхождение зеркал.
- README.md:4 frontmatter `status: partial` без объяснения, что именно «partial» — обе заявленные пробы существуют и описание сходится с кодом.
- Дисциплина «зеркало = ручная копия» хрупка by design: AGENTS.md:21 требует обновлять пробу при изменении CAS реального адаптера — автоматической проверки эквивалентности нет (mitigated docs-only).
- Противоречий README vs код не найдено; doc-комментарий миграции legacy `InMemoryExecutionRepo` → spec-16 store (src/lease_handoff.rs:14-26) актуален и честен.

## Роль в credential/resource redesign
Косвенная: refresh-claim проба доказывает single-claim инвариант, на который опирается rotation/refresh
кредитной подсистемы (Moat#1 active credential lifecycle), но крейт не входит в перетряску crate-топологии
(merge runtime→credential, sole-public-sdk) — он останется sibling-пробой. Если full rewrite credential
изменит CAS-форму refresh-claim в storage, зеркало в src/lib.rs нужно будет переписать вслед
(требование invariant-equivalence из AGENTS.md:21). Resource-redesign (ADR-0093 teardown, topology
bind-inversion) его не затрагивает.
