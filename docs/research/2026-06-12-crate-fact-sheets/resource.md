# nebula-resource — fact sheet

## Назначение
Engine-owned жизненный цикл ресурсов (пулы БД, HTTP-клиенты, SDK-клиенты): acquire / health-check / hot-reload / scope-bounded release. Паттерн Bulkhead (Release It!). Action-код получает `ResourceGuard`, дерефящийся в `R::Instance`, освобождение на drop. Framework владеет acquire-петлёй и credential-revoke fence; топология даёт только тонкие хуки. Канон-инварианты L2-§11.4 (release best-effort on crash) и L2-§13.3 (attributable lifecycle). Статус: `frontier`.

## Публичная поверхность
- `Provider` — центральный трейт: 3 assoc types (`Config`/`Instance`/`Topology`), `key()`, `create/check/shutdown/destroy`, хуки `on_credential_refresh`/`on_credential_revoke` (per-slot) — src/resource.rs:366
- `HasCredentialSlots` — отдельный трейт epoch-fold по слотам, эмитится derive — src/resource.rs:618
- `ResourceConfig` (трейт, supertrait `HasSchema`, `validate`+`fingerprint`) — src/resource.rs:61; `TeardownCx`/`TeardownReason` (ADR-0093) — src/resource.rs:250,271; `CheckCost` — src/resource.rs:294
- `Topology<R>` — открытый slot-centric трейт; framework владеет петлёй, топология не может достать revoke-fence (storage safety через lifetime-bound `&InstanceStore`) — src/topology/contract.rs:379
- `InstanceStore<S>`, `Checkout`, `CheckedOut`, `ReturnOutcome` — fenced idle-queue — src/topology/store.rs:92,417,444,475
- Встроенные топологии: `Pooled<R>` — src/runtime/pool.rs:126, `Resident<R>` — src/runtime/resident.rs:56, `Bounded<R>` (capped/exclusive/unbounded) — src/runtime/bounded.rs:54; hook-трейты `PoolProvider` — src/topology/pooled.rs:83, `ResidentProvider` — src/topology/resident.rs:20, `BoundedProvider` — src/topology/bounded.rs:68
- `Manager` — единая воронка `register(RegistrationSpec)`, acquire dispatch, revoke (`TaintedSlot`/`RevokeTail` — двухфазный taint→drain TOCTOU-close), shutdown — src/manager/mod.rs:485,386,432
- `RegistrationSpec<R>` — plain struct без builder: resource/config/scope/slot_identity/topology/recovery_gate — src/manager/options.rs:230
- `SlotIdentity` (`Unbound`/`Structural`) — структурный cross-tenant барьер, `DedupKey` — src/dedup.rs:54,113
- `SlotCell<S>` — публичная generation-stamped lock-free ячейка слота — src/slot.rs:57 (внутренний epoch-blind `cell::Cell` намеренно НЕ реэкспортируется — src/lib.rs:65)
- `ResourceGuard<R>` — RAII Owned/Guarded — src/guard.rs:99; `ResourceRef<R>` lazy-ссылка — src/resource_ref.rs:55
- `Registry`, sealed `ManagedHandle`, `LookupOutcome` — type-erased хранилище, scope-aware lookup — src/registry.rs:391,61,297
- `ReleaseQueue` — best-effort async drain (§11.4) — src/release_queue.rs:106; `RecoveryGate`+`RecoveryTicket`/`RecoveryWaiter`/`GateState` — thundering-herd — src/recovery/gate.rs:327
- `Error`/`ErrorKind`/`ErrorScope` — типизированная ошибка с retry-классификацией — src/error.rs:15,119,127; `ResourceEvent` — src/events.rs:20; `ResourceOpsMetrics`/`ResourceOpsSnapshot` — src/metrics.rs:78,370
- `ResourceContext` + scope-хелперы — src/context.rs:103,213-246; `AcquireOptions` — src/options.rs:24; `ReloadOutcome` — src/reload.rs:18; `ResourcePhase`/`ResourceStatus` — src/state.rs:9,52
- feature `rotation`: `ResourceFanoutDriver`, `ResourceFanoutIndex`, `Bind`, `RotationOutcome` — src/credential_fanout/ (перенесено из engine, ADR-0092 step 5)
- Derive-макросы (subcrate macros/): `Resource` (slot plumbing: DeclaresDependencies + `<field>_slot()` accessors + HasCredentialSlots), `ResourceConfig` (структурный fingerprint), `ClassifyError` — macros/src/lib.rs
- Реэкспорты чужого: `Credential`/`CredentialContext`/`CredentialId` (nebula-credential), `HasSchema`/`Schema`/`ValidSchema` (nebula-schema), `Subscriber` (eventbus), `ResourceKey`/`ScopeLevel`/`resource_key!` (core) — src/lib.rs:84-118
- `prelude` — src/lib.rs:180

## Workspace-зависимости
Deps (Cargo.toml): nebula-core, nebula-credential, nebula-eventbus, nebula-expression (default-features=false, "cache"), nebula-metrics, nebula-metadata, nebula-resource-macros (path=macros), nebula-schema, nebula-error + tokio/tokio-util/futures/async-trait/arc-swap/dashmap/smallvec/serde/serde_json/semver/thiserror/tracing. Feature `rotation` (без новых deps).
Зависят от него: nebula-action, nebula-engine (+ прокидывает `rotation`), nebula-sdk, nebula-plugin.

## Структура модулей
- `lib.rs` — фасад, реэкспорты, prelude
- `resource.rs` — Provider, ResourceConfig, HasCredentialSlots, ResourceMetadata(+Builder), TeardownCx/Reason, CheckCost
- `manager/` — mod (Manager, двухфазный revoke-инвариант — канонич. doc), acquire, gate, options, registration, rotation, shutdown
- `topology/` — contract (открытый Topology<R>), store (InstanceStore), pooled/resident/bounded (hook-трейты+конфиги)
- `runtime/` — pool/resident/bounded (структуры топологий), managed (ManagedResource — framework acquire loop)
- `slot.rs` / `cell.rs` — публичный SlotCell vs внутренний epoch-blind Cell
- `registry.rs` — type-erased Registry, (key,scope,slot_identity)-dedup
- `credential_fanout/` [feature rotation] — driver + index ротационного fan-out
- `guard.rs` / `hook_guard.rs` — RAII guard / внутренний hook-guard
- `recovery/` — RecoveryGate; `release_queue.rs` — drain; `reload.rs` — ReloadOutcome
- `context.rs`, `options.rs`, `dedup.rs`, `error.rs`, `events.rs`, `ext.rs`, `metrics.rs`, `state.rs`, `resource_ref.rs`, `topology_tag.rs` — по одному типу/группе
- `macros/` — subcrate nebula-resource-macros (config.rs, field_slots.rs, slots.rs)

## Напряжения
- README.md:170 — миграционный рецепт говорит `#[derive(ResourceSlots)]` + «impl Resource», фактически derive называется `Resource`, трейт — `Provider` (macros/src/lib.rs:9-16). Устаревшее имя.
- README.md:140 и docs/README.md:266 — упоминают публичный `AnyManagedResource`; в коде его нет, есть sealed `ManagedHandle` (src/registry.rs:61).
- README.md:288 (Telegram-пример) и README.md:171 (шаг 6) — `RegistrationSpec` с полями `resilience`/`acquire`; фактическая структура (src/manager/options.rs:230-249) этих полей не имеет (AcquireResilience удалён, см. Non-goals README.md:194). Примеры не компилируются.
- src/lib.rs:9-10 — «Three built-in topologies … `Pooled`, `Resident`.» — перечислены две из трёх (Bounded пропущен).
- README.md:174 — «engine-side fan-out machinery … lands in a follow-up» — устарело: fan-out уже landed и живёт В ЭТОМ крейте (src/credential_fanout/mod.rs:1, ADR-0092 step 5).
- AGENTS.md:10 — команда `cargo test -p nebula-resource --features test-util`: у крейта нет feature `test-util` (Cargo.toml features: только `rotation`); сам Cargo.toml:53 говорит, что ADR-0092 step 8 удалил test-util из nebula-credential. Мёртвая команда.
- Cargo.toml:51-58 — два почти дословно дублирующих комментария про мок `TestCredential` в tests/rotation.rs.
- README last-reviewed 2026-04-29 — до bind-inversion (ADR-0093) и переноса fan-out; раздел «Public API (v4)» местами отстаёт от кода.

## Роль в credential/resource redesign
Крейт — фронт текущего redesign. Уже в нём: ADR-0093 teardown contract + topology bind-inversion (framework-owned acquire loop, merged `Topology<R>`, façade-twice защита), ADR-0092 перенос credential_fanout из engine, SlotIdentity cross-tenant барьер, двухфазный revoke. Открытый хвост: production bind-population (credential→slot resolver, §M12.4) — `register_and_bind` имеет quiesce-контракт, но ноль продакшн-вызовов; статус остаётся `frontier`. На ветке `dreamy-kare-8698d4` лежат ещё 4 breaking-коммита redesign API (не в этом worktree).
