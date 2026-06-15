# nebula-resource — design

| Field | Value |
|-------|-------|
| **Status** | `frontier` — redesign frontier; production bind-population still missing |
| **Layer** | Domain (engine-owned resource lifecycle; depends on credential/schema/eventbus, consumed by action/engine/sdk/plugin) |
| **Redesign role** | **Touched — фронт текущего credential/resource redesign.** Уже принял ADR-0093 (teardown-контракт + topology bind-inversion) и шаг 5 ADR-0092 (перенос `credential_fanout/` из engine). Открытый хвост — production bind-population (§M12.4). |
| **Related** | ADR-0092 step 5, ADR-0093, PRODUCT_CANON L2-§11.4 (release best-effort on crash) / L2-§13.3 (attributable lifecycle) / §3.5 (что есть «Resource») |

---

## 1. Назначение и границы

`nebula-resource` владеет **engine-owned жизненным циклом внешних ресурсов** — пулы БД, HTTP-клиенты, SDK-клиенты. Это реализация паттерна Bulkhead (Release It!): изоляция истощения по топологиям, чтобы одна выбранная-до-дна группа не каскадила на несвязанные пути. Action-код получает `ResourceGuard<R>`, который дерефится в `R::Instance` и освобождается на drop; framework гарантирует здоровье инстанса до выдачи гварда (`src/resource.rs:366`, `src/guard.rs:99`).

**Владеет:** acquire-петлёй (framework-owned, не у топологии), health-check / hot-reload / scope-bounded release, fenced idle-queue (`InstanceStore`), двухфазным credential-revoke (taint→drain), structural cross-tenant барьером (`SlotIdentity`), generation-stamped слот-ячейками (`SlotCell`), per-slot ротационным fan-out (под feature `rotation`), типизированной ошибкой с retry-классификацией.

**ЯВНО НЕ делает:** не хранит ключи и не шифрует (это `nebula-storage` / `nebula-crypto`); не определяет credential-типы и не выполняет refresh/lease/rotation-state (это `nebula-credential`); не содержит long-running worker'ов и pull-подписок — `Daemon` / `EventSource` живут в `nebula_engine::daemon` (канон §3.5 резервирует «Resource» под pool/SDK-клиенты, README.md:18); внутренний epoch-blind `cell::Cell` намеренно НЕ реэкспортируется (`src/lib.rs:65`).

## 2. Публичная поверхность

| Элемент | Где |
|---------|-----|
| `Provider` — центральный трейт: assoc `Config`/`Instance`/`Topology`, `key()`, `create/check/shutdown/destroy`, per-slot `on_credential_refresh`/`on_credential_revoke` | `src/resource.rs:366` |
| `HasCredentialSlots` — epoch-fold по слотам, эмитится derive `Resource` | `src/resource.rs:618` |
| `ResourceConfig` (supertrait `HasSchema`, `validate`+`fingerprint`); `TeardownCx`/`TeardownReason` (ADR-0093); `CheckCost` | `src/resource.rs:61,250,271,294` |
| `Topology<R>` — открытый slot-centric трейт; framework владеет петлёй, топология НЕ может достать revoke-fence (storage-safety через lifetime-bound `&InstanceStore`) | `src/topology/contract.rs:379` |
| `InstanceStore<S>`, `Checkout`, `CheckedOut`, `ReturnOutcome` — fenced idle-queue | `src/topology/store.rs:92,417,444,475` |
| Встроенные топологии `Pooled<R>` / `Resident<R>` / `Bounded<R>` (capped/exclusive/unbounded) + hook-трейты `PoolProvider`/`ResidentProvider`/`BoundedProvider` | `src/runtime/pool.rs:126`, `resident.rs:56`, `bounded.rs:54`; `src/topology/pooled.rs:83`, `resident.rs:20`, `bounded.rs:68` |
| `Manager` — единая воронка `register(RegistrationSpec)`, acquire-dispatch, revoke (`TaintedSlot`/`RevokeTail`), shutdown | `src/manager/mod.rs:485,386,432` |
| `RegistrationSpec<R>` — plain struct (без builder): resource/config/scope/slot_identity/topology/recovery_gate | `src/manager/options.rs:230` |
| `SlotIdentity` (`Unbound`/`Structural`) — структурный cross-tenant барьер; `DedupKey` | `src/dedup.rs:54,113` |
| `SlotCell<S>` — публичная generation-stamped lock-free ячейка слота | `src/slot.rs:57` |
| `ResourceGuard<R>` (RAII Owned/Guarded); `ResourceRef<R>` (lazy-ссылка) | `src/guard.rs:99`; `src/resource_ref.rs:55` |
| `Registry`, sealed `ManagedHandle`, `LookupOutcome` — type-erased хранилище, scope-aware lookup | `src/registry.rs:391,61,297` |
| `ReleaseQueue` — best-effort async drain (§11.4); `RecoveryGate`+`RecoveryTicket`/`RecoveryWaiter`/`GateState` — thundering-herd | `src/release_queue.rs:106`; `src/recovery/gate.rs:327` |
| `Error`/`ErrorKind`/`ErrorScope`; `ResourceEvent`; `ResourceOpsMetrics`/`ResourceOpsSnapshot` | `src/error.rs:15,119,127`; `src/events.rs:20`; `src/metrics.rs:78,370` |
| `ResourceContext` + scope-хелперы; `AcquireOptions`; `ReloadOutcome`; `ResourcePhase`/`ResourceStatus` | `src/context.rs:103,213-246`; `src/options.rs:24`; `src/reload.rs:18`; `src/state.rs:9,52` |
| feature `rotation`: `ResourceFanoutDriver`, `ResourceFanoutIndex`, `Bind`, `RotationOutcome` | `src/credential_fanout/` |
| Derive (subcrate `macros/`): `Resource`, `ResourceConfig`, `ClassifyError` | `macros/src/lib.rs` |
| Реэкспорты чужого: `Credential`/`CredentialContext`/`CredentialId`, `HasSchema`/`Schema`/`ValidSchema`, `Subscriber`, `ResourceKey`/`ScopeLevel`/`resource_key!`; `prelude` | `src/lib.rs:84-118,180` |

## 3. Зависимости и зависимые

- **Deps:** `nebula-core`, `nebula-credential`, `nebula-eventbus`, `nebula-expression` (default-features=false, `cache`), `nebula-metrics`, `nebula-metadata`, `nebula-resource-macros` (path=`macros`), `nebula-schema`, `nebula-error`; рантайм `tokio`/`tokio-util`/`futures`/`async-trait`/`arc-swap`/`dashmap`/`smallvec`/`serde`/`serde_json`/`semver`/`thiserror`/`tracing`. Feature `rotation` без новых deps.
- **Dependents:** `nebula-action`, `nebula-engine` (прокидывает feature `rotation`), `nebula-sdk`, `nebula-plugin`.

## 4. Внутренняя архитектура

- `lib.rs` — фасад, реэкспорты, `prelude`.
- `resource.rs` — `Provider`, `ResourceConfig`, `HasCredentialSlots`, `ResourceMetadata(+Builder)`, `TeardownCx/Reason`, `CheckCost`.
- `manager/` — `mod` (`Manager`, каноническая doc двухфазного revoke-инварианта), `acquire`, `gate`, `options`, `registration`, `rotation`, `shutdown`.
- `topology/` — `contract` (открытый `Topology<R>`), `store` (`InstanceStore`), `pooled`/`resident`/`bounded` (hook-трейты + конфиги).
- `runtime/` — `pool`/`resident`/`bounded` (сами структуры топологий), `managed` (`ManagedResource` — framework acquire loop).
- `slot.rs` / `cell.rs` — публичный `SlotCell` vs внутренний epoch-blind `Cell`.
- `registry.rs` — type-erased `Registry`, `(key, scope, slot_identity)`-dedup.
- `credential_fanout/` `[feature rotation]` — driver + index ротационного fan-out.
- `guard.rs` / `hook_guard.rs` — RAII guard / внутренний hook-guard.
- `recovery/` — `RecoveryGate`; `release_queue.rs` — drain; `reload.rs` — `ReloadOutcome`.
- Прочие однотипные модули: `context.rs`, `options.rs`, `dedup.rs`, `error.rs`, `events.rs`, `ext.rs`, `metrics.rs`, `state.rs`, `resource_ref.rs`, `topology_tag.rs`.
- `macros/` — subcrate `nebula-resource-macros` (`config.rs`, `field_slots.rs`, `slots.rs`).

**Поток данных (acquire):** `Manager::register(RegistrationSpec)` → дедуп по `(key, scope, slot_identity)` в `Registry` → framework-owned acquire loop в `ManagedResource` дёргает `Topology<R>` за тонкие хуки, тянет инстанс из fenced `InstanceStore` либо создаёт через `Provider::create` (со здоровьем по `Provider::check`) → выдаётся `ResourceGuard<R>` → drop возвращает инстанс в idle-queue или ставит в `ReleaseQueue` (best-effort §11.4). **Revoke:** двухфазный `TaintedSlot`→`RevokeTail` (taint→drain), закрывающий TOCTOU между выдачей гварда и инвалидацией credential.

## 5. Инварианты и контракты

- **Framework владеет acquire-петлёй и revoke-fence.** Открытый `Topology<R>` намеренно НЕ возвращает `ResourceGuard<R>` и не даёт топологии доступ к store/fence (storage-safety через lifetime-bound `&InstanceStore`, `src/topology/contract.rs:379`) — закрывает failure-mode «façade-twice» из bind-inversion (ADR-0093).
- **Release best-effort on crash (L2-§11.4).** Drop гварда никогда не паникует и не блокирует; недослитое уходит в `ReleaseQueue` (`src/release_queue.rs:106`).
- **Attributable lifecycle (L2-§13.3).** Каждая операция несёт `ResourceContext`/scope; `ResourceEvent` + `ResourceOpsMetrics` дают трассируемость по умолчанию.
- **Cross-tenant barrier by-construction.** `SlotIdentity::Structural` входит в dedup-ключ (`src/dedup.rs:54,113`, `src/registry.rs`), поэтому инстанс одного тенанта структурно не может быть отдан другому — это не runtime-проверка, а форма ключа.
- **Revoke без TOCTOU.** Двухфазный taint→drain (`src/manager/mod.rs:485`) гарантирует, что после revoke ни один уже выданный гвард не продолжит работать на отозванном credential.
- **Generation-stamped слоты.** `SlotCell<S>` lock-free и штампует поколение; epoch-blind `cell::Cell` скрыт (`src/lib.rs:65`), чтобы автор не прочитал слот мимо epoch-инварианта.
- **Teardown-контракт (ADR-0093).** `reset`/`destroy` — fallible-async; safe-by-default reset; deadline (а не `Duration`) через `TeardownCx`/`TeardownReason` (`src/resource.rs:250,271`).

## 6. Известные напряжения / долг (честно)

1. **Устаревшее имя в миграционном рецепте.** README.md:170 говорит `#[derive(ResourceSlots)]` + «impl `Resource`»; фактически derive называется `Resource`, трейт — `Provider` (`macros/src/lib.rs:9-16`).
2. **Несуществующий публичный тип в доках.** README.md:140 и docs/README.md:266 упоминают `AnyManagedResource`; в коде его нет — есть sealed `ManagedHandle` (`src/registry.rs:61`).
3. **Примеры не компилируются.** README.md:288 (Telegram-пример) и README.md:171 (шаг 6) дают `RegistrationSpec` с полями `resilience`/`acquire`; фактическая структура (`src/manager/options.rs:230-249`) их не имеет (`AcquireResilience` удалён, Non-goals README.md:194).
4. **Недосчёт топологий в doc-комменте.** `src/lib.rs:9-10` пишет «Three built-in topologies … `Pooled`, `Resident`» — перечислены две из трёх (пропущен `Bounded`).
5. **Устаревшее «fan-out lands in a follow-up».** README.md:174 говорит, что fan-out придёт позже; на деле он уже landed и живёт В ЭТОМ крейте (`src/credential_fanout/mod.rs:1`, ADR-0092 step 5).
6. **Мёртвая тестовая команда.** AGENTS.md:10 предлагает `cargo test -p nebula-resource --features test-util`; такого feature нет (Cargo.toml: только `rotation`), а Cargo.toml:53 фиксирует, что step 8 ADR-0092 удалил `test-util` из `nebula-credential`.
7. **Дублирующий комментарий.** Cargo.toml:51-58 — два почти дословных комментария про мок `TestCredential` в `tests/rotation.rs`.
8. **README отстаёт от кода.** last-reviewed 2026-04-29 — до bind-inversion (ADR-0093) и переноса fan-out; раздел «Public API (v4)» местами расходится с кодом.

## 7. Роль в пост-0092 credential/resource модели

После ADR-0092 граница такова: `nebula-credential` — единый крейт (контракт + рантайм resolver/refresh/lease/rotation-state + фасад `CredentialService` + builtin-типы); `nebula-crypto` — Cipher/Kdf; `nebula-engine` credential-модуль — только accessor-мосты + `default_in_memory_coordinator`; `nebula-storage` — durable-stores + decorators + `KeyProvider` + `RefreshClaimRepo`. **`nebula-resource` в этой модели владеет per-slot ротационным FAN-OUT** (`credential_fanout/`, перенесён из engine шагом 5 ADR-0092) **плюс `SlotCell` + `Manager` + topology.**

Швы (seam), которыми крейт стыкуется с credential-стеком:

- **Consumer-binding seam.** Resource объявляет `#[credential(key=…)]` слот-поля; framework заполняет их типизированными `CredentialGuard<Scheme>` через `&self` до вызова `create`; derive `Resource` эмитит `<field>_slot()` accessor и `HasCredentialSlots`. Слоты (`slot_bindings`) отделены от параметров; значения персистятся как values-only, схема приходит из зарегистрированных типов через `HasSchema` → `nebula-metadata` → API-каталог.
- **Rotation seam.** `nebula-credential` решает refresh/rotation-state; `nebula-resource` подписывается на событие и через `ResourceFanoutDriver`/`ResourceFanoutIndex` доставляет его до конкретных слотов живых инстансов, вызывая `Provider::on_credential_refresh(&self, slot, runtime)`. Маршрутизация по `policy(&State)` и owner-изоляция (`OwnerScopedKey`) — обязанность credential-крейта; resource доверяет уже-маршрутизированному событию.
- **Revoke seam.** `on_credential_revoke` + двухфазный taint→drain — точка, где отзыв credential инвалидирует ресурсные инстансы без TOCTOU; lease — first-class в credential, resource держит лишь срок жизни инстанса вокруг неё.

**Что меняется:** fan-out окончательно переехал сюда из engine (engine оставляет только мосты); teardown-контракт и bind-inversion уже приняты. **Что остаётся:** `Provider`/`Topology<R>`/`Manager`/`SlotCell` — стабильный костяк; resource НЕ перетягивает к себе ни refresh-транспорт (узкий типизированный `RefreshTransport` — в credential), ни policy-маршрутизацию, ни key-storage.

## 8. Forward design / открытые вопросы

- **Production bind-population (§M12.4) — главный незакрытый хвост.** `register_and_bind` имеет quiesce-контракт, но ноль продакшн-вызовов: нет производственного credential→slot resolver, который наполнял бы `slot_bindings` реальными биндингами. Пока его нет, статус крейта остаётся `frontier`. Это следующий resource-follow-up.
- **Несинхронизированные breaking-коммиты.** На ветке `dreamy-kare-8698d4` лежат ещё 4 breaking-коммита redesign API, не влитые в этот worktree; их надо re-derive против пост-0093 состояния перед мержем (риск дрейфа `RegistrationSpec`/topology API).
- **Долг по докам — это риск онбординга, а не косметика.** Несуществующий `AnyManagedResource`, некомпилирующиеся `RegistrationSpec`-примеры и устаревший derive-рецепт (§6.1-6.5) активно дезинформируют авторов ресурсов; README нужно прогнать против кода и поднять last-reviewed.
- **Authoring-унификация (`#[property]`/единый authoring) — Phase-5, ещё НЕ построена.** Слот-биндинг и параметры пока остаются раздельными поверхностями; решение по унифицированному authoring откладывается до credential Phase-5 и не должно опережать его здесь.
- **Гигиена feature/тест-команд.** Убрать мёртвую `--features test-util` из AGENTS.md и схлопнуть дублирующий Cargo.toml-комментарий (§6.6-6.7) — мелочь, но ломает доверие к «зелёным» командам.
