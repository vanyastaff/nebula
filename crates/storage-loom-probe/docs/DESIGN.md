# nebula-storage-loom-probe — design

| Field | Value |
|-------|-------|
| **Status** | Stable — standalone loom-model probe crate (`publish = false`) |
| **Layer** | Exec (sibling probe; not consumed by any production crate) |
| **Redesign role** | **Not touched** by the post-0092 credential/resource crate-topology rework — remains a sibling probe outside the merge. Only contract: if a credential full-rewrite changes the storage CAS *shape*, the mirror here must be rewritten in lockstep (AGENTS.md:21). |
| **Related** | ADR-0041 (historical refresh-claim, `docs/adr/HISTORICAL.md`), ADR-0072 (live `storage-port`/`storage`/`tenancy`), AGENTS.md:21 |

---

## 1. Назначение и границы

Standalone-крейт с loom-модельными пробами для CAS-критических секций `nebula-storage`: проверка через model-checker, что под произвольным чередованием потоков сохраняется single-winner инвариант.

**Владеет** двумя пробами: (1) refresh-claim для credential (`Repo::try_claim`, форма `InMemoryRefreshClaimRepo::try_claim`, ADR-0041) и (2) execution-lease handoff (`lease_handoff`, форма spec-16 `InMemoryExecutionStore::{acquire,renew,release}_lease`). Формы продакшен-кода переписаны вручную поверх `loom::sync::Mutex`.

**ЯВНО НЕ делает:** не зависит от `nebula-storage` (намеренно — чтобы `RUSTFLAGS="--cfg loom"` не протёк в транзитивные deps `concurrent-queue` via `moka`→`async-lock`→`event-listener`); не содержит продакшен-кода стораджа; не покрывает инварианты вне этих двух; не выполняется при обычном `cargo check` (весь код под `#![cfg(loom)]`, компилируется «в ничто»).

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `ClaimRow { holder, generation, expired }` | src/lib.rs:41 |
| `Repo { rows: Mutex<HashMap<u32, ClaimRow>> }` | src/lib.rs:52 |
| `Outcome::{Acquired, Contended}` (зеркало `ClaimAttempt`) | src/lib.rs:58 |
| `Repo::try_claim(&self, cid, holder) -> Outcome` (acquire только если строки нет/expired) | src/lib.rs:69 |
| `pub mod lease_handoff` | src/lib.rs:31 |
| `lease_handoff::LeaseRow { holder, generation, expired, held }` | src/lease_handoff.rs:47 |
| `lease_handoff::LeaseRepo` | src/lease_handoff.rs:68 |
| `AcquireOutcome::{Acquired(u64), Contended}` (↔ `Ok(Some(FencingToken))`/`Ok(None)`) | src/lease_handoff.rs:76 |
| `HolderOutcome::{Applied, Rejected}` (↔ `Ok(true)`/`Ok(false)`) | src/lease_handoff.rs:87 |
| `LeaseRepo::acquire_lease(exec_id, holder) -> AcquireOutcome` (каждый успех бампает fencing generation; release строку НЕ удаляет) | src/lease_handoff.rs:101 |
| `LeaseRepo::renew_lease(exec_id, token) -> HolderOutcome` (fenced по generation, не по holder) | src/lease_handoff.rs:131 |
| `LeaseRepo::release_lease(exec_id, token) -> HolderOutcome` (clears `held`; строка и generation сохраняются) | src/lease_handoff.rs:147 |
| `LeaseRepo::flag_expired(exec_id) -> bool` (test-only: моделирование TTL — loom не видит время) | src/lease_handoff.rs:160 |
| `LeaseRepo::snapshot(exec_id) -> Option<(u32, u64)>` (test-only инспектор) | src/lease_handoff.rs:172 |

## 3. Зависимости и зависимые

- **Deps:** только `loom = "0.7"` (optional, за фичей `loom-test = ["dep:loom"]`); workspace-зависимостей НЕТ — это принцип крейта (Cargo.toml:15-25).
- **Dependents:** ни один крейт workspace не зависит от него; единственное упоминание — комментарий в `crates/storage/Cargo.toml:121`, объясняющий, почему loom-пробы вынесены сюда.
- **Запуск:** `RUSTFLAGS="--cfg loom" cargo nextest run -p nebula-storage-loom-probe --features loom-test --profile ci --no-tests=pass`.

## 4. Внутренняя архитектура

- `src/lib.rs` — refresh-claim проба: `Repo::try_claim` поверх `Mutex<HashMap<u32, ClaimRow>>` + crate-level docs «почему отдельный крейт».
- `src/lease_handoff.rs` — execution-lease проба: acquire/renew/release с fencing-generation + test-хелперы (`flag_expired`, `snapshot`).
- `tests/refresh_claim_loom.rs` — loom-модель: 2 конкурирующих `try_claim` → ровно один `Acquired`.
- `tests/lease_handoff_loom.rs` — loom-модели: single-winner acquire, отклонение superseded-токена после takeover, stale release после expiry-takeover = no-op. Companion runtime-тесты: `crates/engine/tests/lease_takeover.rs`, `crates/storage/tests/execution_lease_pg_integration.rs`.

Поток данных: каждый тест строит `Repo`/`LeaseRepo`, под `loom::model` запускает конкурирующие потоки на shared `Arc`, и проверяет, что финальное состояние удовлетворяет single-winner инварианту во всех чередованиях.

## 5. Инварианты и контракты

- **Single-claim (refresh).** При конкурирующих `try_claim` по одному `credential_id` ровно один результат `Acquired`; основа rotation/refresh credential-подсистемы (Moat#1 active credential lifecycle, ADR-0041).
- **Single-owner + fencing (lease).** `acquire_lease` монотонно бампает generation per-row; `renew`/`release` fenced по generation, а не по holder-строке — superseded-токен после takeover отклоняется (`Rejected`), stale release после expiry-takeover = no-op. `release` строку и generation сохраняет (монотонность fencing).
- **Изоляция cfg.** Весь крейт под `#![cfg(loom)]` и зависит только от `loom` — `--cfg loom` активирует код только там, где `loom` в scope, by-construction не ломая транзитивные deps.
- **Invariant-equivalence (by-discipline, НЕ by-construction).** AGENTS.md:21: при изменении CAS реального адаптера пробу обновляют вручную; автоматической проверки эквивалентности зеркала нет.

## 6. Известные напряжения / долг

1. Мусорная doc-строка `//!.` (опечатка от редактирования заголовка) — src/lib.rs:2.
2. Несимметричный старт generation: `try_claim` начинает с 0 (src/lib.rs:76 `map_or(0, ...)`), `acquire_lease` — с 1 (src/lease_handoff.rs:112 `map_or(1, ...)`). Вероятно отражает реальный `fencing_generation`, но в claim-пробе ничем не мотивирован — кандидат на расхождение зеркал.
3. `status: partial` во frontmatter README.md:4 без объяснения, что именно «partial» — обе заявленные пробы существуют и описание сходится с кодом.
4. Дисциплина «зеркало = ручная копия» хрупка by design (см. §5): нет автоматической проверки эквивалентности с реальным адаптером — mitigated docs-only (AGENTS.md:21).
5. Противоречий README vs код не найдено; doc-комментарий миграции legacy `InMemoryExecutionRepo` → spec-16 store (src/lease_handoff.rs:14-26) актуален и честен.

## 7. Роль в пост-0092 credential/resource модели

Крейт **не затронут** перетряской crate-топологии (merge runtime/builtin→credential, sole-public-sdk, nebula-crypto-extraction) — он остаётся sibling-пробой, стабильным фундаментом проверки CAS-инвариантов. Связь косвенная: refresh-claim проба доказывает single-claim инвариант, на который опирается rotation/refresh credential. Единственный forward-контракт: если full rewrite credential изменит CAS-форму refresh-claim в storage, зеркало в src/lib.rs нужно переписать вслед (AGENTS.md:21). Resource-redesign (ADR-0093 teardown, topology bind-inversion) его не затрагивает.

## 8. Forward design / открытые вопросы

Крейт стабилен — никаких новых API не планируется. Каждая новая проба должна приземляться сюда под той же дисциплиной (sibling-crate / `cfg(loom)` / `loom-test` feature). Открытые мелочи косметические: вычистить `//!.` (п.6.1), выровнять старт generation или задокументировать асимметрию (п.6.2), уточнить смысл `status: partial` (п.6.3). Единственный структурный долг — отсутствие автоматической invariant-equivalence-проверки зеркала против реального адаптера; пока mitigated docs-only.
