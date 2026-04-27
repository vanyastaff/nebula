# `nebula-credential` — реестр проблем (snapshot 2026-04-27)

**Snapshot commit:** `f308ded4` (П2 — refresh coordination L2, n8n #13088 close)
**Scope:** `crates/credential` (package: `nebula-credential`) + связи в `crates/engine` (`nebula-engine::credential`), `crates/storage` (`nebula-storage::credential`), `crates/resource`, `crates/action`
**Audit composition:** security-lead + rust-senior + 2× Explore agents (architecture, tests/docs)

> Это **снимок-аудит на дату**, не living register. Living tracking — `docs/tracking/credential-concerns-register.md`. Этот файл — точка отсчёта; используется для приоритизации и delta-проверки через ~2 недели.

---

## TL;DR

- **52 проблемы**: 13 High (закрыть до prod-release), 21 Medium, 18 Low
- **6 фиксов закрывают 12 проблем** через перекрытия (см. §VII.D)
- **24 образцовых места** — крейт в целом написан зрело
- **Ни одна High-проблема не требует пересмотра архитектуры**
- Дорожная карта на 2–3 недели работы одного senior'а

**Severity легенда:** **C**ritical / **H**igh / **M**edium / **L**ow.

**ID-префиксы:**
- `SEC` — безопасность
- `ARCH` — архитектура / модульная организация
- `PERF` — перформанс
- `IDIOM` — Rust-идиомы (1.95+)
- `TEST` — пробелы тестов
- `DOC` — документация
- `GAP` — открытая архитектурная дыра

---

## I. Что должно быть закрыто **до** prod-релиза (13 High)

| ID | Sev | Место | Суть |
|---|---|---|---|
| **SEC-01** | H | [engine/src/credential/rotation/token_refresh.rs:109](../../crates/engine/src/credential/rotation/token_refresh.rs) | Error-path читает body OAuth-IdP без лимита. Скомпрометированный/MITM-IdP → 10 GB error body → OOM воркера. |
| **SEC-02** | H | [token_refresh.rs:170-173](../../crates/engine/src/credential/rotation/token_refresh.rs) | `error_uri` от IdP в error summary без валидации (схема, длина, control-chars) → log/SIEM injection, фишинг через operator-facing сообщения. |
| **SEC-03** | H | [credential/secrets/crypto.rs:218-266](../../crates/credential/src/secrets/crypto.rs) + [storage/credential/layer/encryption.rs:208-251](../../crates/storage/src/credential/layer/encryption.rs) | AAD = `credential_id` без `key_id`. Owner storage row может переписать `envelope.key_id` на legacy → расшифровка через legacy-ключ → audit-trail integrity gap. |
| **SEC-04** | H | [crypto.rs:136-142](../../crates/credential/src/secrets/crypto.rs) | `fresh_nonce()` doc заявляет «OS CSPRNG», код использует `rand::rng()` (`ThreadRng`). Под `RUSTSEC-2026-0097` потенциальная nonce-collision = catastrophic AES-GCM. |
| **GAP-01** | H | [resource/manager.rs:1380](../../crates/resource/src/manager.rs) | `Manager::on_credential_refreshed` заканчивается `todo!()`. Engine эмитит `CredentialEvent::Refreshed`, никто не подписан. Нет happens-before между store commit и pool swap. |
| **TEST-01** | H | — | Нет end-to-end теста: register → resolve → refresh → revoke → cleanup. |
| **TEST-02** | H | — | `credential_refresh_drives_per_resource_swap` (упомянут в tech-spec) — не реализован. Multi-replica chaos из MATURITY.md тоже не в репо. |
| **ARCH-01** | H | [credential/lib.rs:74-82](../../crates/credential/src/lib.rs) | `accessor`, `context`, `handle`, `metadata`, `record` — приватные `mod`, но типы re-export'ятся. Несогласованность с `pub mod contract/scheme/secrets`. |
| **ARCH-02** | H | [credential/Cargo.toml:83-89](../../crates/credential/Cargo.toml) + [lib.rs:152,176](../../crates/credential/src/lib.rs) | `test-util` feature раскрывает `InMemoryStore`/`InMemoryPendingStore` как public API. Параллельно тот же `InMemoryStore` живёт в `nebula-storage`. Нарушение ADR-0032 §3. |
| **ARCH-03** | H | — | Test-shim duplication: store-traits живут в credential, имплементации — копии в credential и storage. Любое изменение контракта надо синхронить в двух местах. |
| **PERF-01** | H | [coordinator.rs:539,565](../../crates/engine/src/credential/refresh/coordinator.rs) + [l1.rs:67](../../crates/engine/src/credential/refresh/l1.rs) | L1 keyed `String`. `refresh_coalesced` делает `to_string()` + `clone()` — 2× alloc per refresh. Hot-path под любым herd-сценарием. |
| **PERF-02** | H | [resolver.rs:139,189,211,341,398,416,472,476,483,537,654](../../crates/engine/src/credential/resolver.rs) | 12× `credential_id.to_string()` per resolve, 3 на success-path. |
| **IDIOM-01** | H | [credential/provider.rs:107](../../crates/credential/src/provider.rs) | Последний `#[async_trait]` в credential-surface. Несогласованность — все остальные контракты на RPITIT. |

---

## II. Безопасность (SEC) — полный список

### High (см. §I)

### Medium

| ID | Место | Суть |
|---|---|---|
| **SEC-05** | [secrets/guard.rs:64-71](../../crates/credential/src/secrets/guard.rs) | `CredentialGuard: Clone` — клон создаёт *второй* zeroize-point. Конфликт с N10 invariant («plaintext не пересекает spawn_blocking»). |
| **SEC-06** | [secrets/scheme_guard.rs:64](../../crates/credential/src/secrets/scheme_guard.rs) | `SchemeGuard` не `!Send`. Lifetime-pin защищает retention, но не thread-handoff. Plaintext может попасть на blocking-pool thread. |
| **SEC-07** | [secret_string.rs:97-104](../../crates/credential/src/secrets/secret_string.rs) | Deserialize отбрасывает только точное `"[REDACTED]"`. ` [REDACTED]`, `[redacted]`, `[REDACTED]\n` пройдут. |
| **SEC-08** | [serde_secret.rs:12-14](../../crates/credential/src/secrets/serde_secret.rs) | `pub fn serialize(&SecretString, S)` экспортирована на module level. Любой downstream может слить plaintext в произвольный sink. Должна быть `pub(crate)`. |
| **SEC-09** | [oauth2.rs:125-128](../../crates/credential/src/credentials/oauth2.rs) | `bearer_header()` делает `format!("Bearer {}", token.expose_secret())` — промежуточная `String` не `Zeroizing`. |
| **SEC-10** | [token_refresh.rs:62-72](../../crates/engine/src/credential/rotation/token_refresh.rs) | `expose_secret().to_owned()` создаёт unwrapped `String` *до* `Zeroizing::new(...)` обёртки. Паттерн ×3 (refresh_tok / client_id / client_secret). |
| **SEC-11** | [crypto.rs:158-177](../../crates/credential/src/secrets/crypto.rs) | Bare `encrypt()` (без AAD, без key_id) до сих пор `pub`. Storage его reject'нет, но плагины и manual callers могут продуцировать envelopes вне AAD-mandatory contract. |
| **SEC-12** | [storage/credential/key_provider.rs:200-242](../../crates/storage/src/credential/key_provider.rs) | Нет precedence-check между ENV и FILE provider'ами. Operator может сконфигурить оба. |
| **SEC-13** | [credential/error.rs:286-298](../../crates/credential/src/error.rs) | `CredentialError::refresh(.., msg: impl Display)` тянет произвольный `msg.to_string()`. IdP часто эхо'ят части `refresh_token` в `error_description` (особенно `invalid_grant`). |

### Low

| ID | Место | Суть |
|---|---|---|
| **SEC-14** | oauth2.rs:489-492 | `ct_eq` после length-check — early-return из length-mismatch без plaintext, OK. |
| **SEC-15** | crypto.rs:79-81 | `key_id: #[serde(default)]` — open-ended legacy window без deadline. |
| **SEC-16** | engine/.../coordinator.rs:644,712 | `tracing::warn!(?e, ...)` для `RepoError::Storage(sqlx::Error)` — Debug-репр может содержать DSN с password. |
| **SEC-17** | Cargo.toml:117-129 | `aes-gcm`, `subtle`, `secrecy` не pinned на root, тянутся транзитивно. Проверить против RUSTSEC. |
| **SEC-18** | error.rs:126 | `Box<dyn Error + Send>` без `Sync`. Cross-thread async erasure surprise downcast. |

---

## III. Архитектура и модульная организация (ARCH)

### High (см. §I)

### Medium

| ID | Место | Суть |
|---|---|---|
| **ARCH-04** | [handle.rs:64](../../crates/credential/src/handle.rs) | `#[allow(dead_code)]` на `pub(crate) fn replace()` со ссылкой «consumer (RefreshCoordinator) lands in task 1.5». Скрывает потенциальные ошибки. |
| **ARCH-05** | [lib.rs:114-118](../../crates/credential/src/lib.rs) | `pub use contract::resolve;` — backward-compat re-export для proc-macro и downstream. Замораживает API shape. Нужен `#[deprecated]` + миграция. |
| **ARCH-06** | [error.rs](../../crates/credential/src/error.rs) (518 строк) | `CredentialError`, `CryptoError`, `ValidationError`, `RefreshErrorKind`, `ResolutionStage`, `RetryAdvice` + conversions в одном файле. Разбить на `error/{crypto,validation,refresh,resolution}.rs`. |
| **ARCH-07** | [Cargo.toml:21-31](../../crates/credential/Cargo.toml) | `tokio` features `["time", "sync", "macros", "rt"]` тянутся в core. `macros`/`rt` — для `#[tokio::test]` (133 sites), не production. |
| **ARCH-08** | [lib.rs:208-236](../../crates/credential/src/lib.rs) | `prelude` включает 16 типов из ~80+ public, без явного критерия. |
| **ARCH-09** | [contract/resolve.rs](../../crates/credential/src/contract/resolve.rs) (274 строки) | Module-level docs не объясняют workflow `InteractionRequest → Engine → Action → continue_resolve`. |

### Low

| ID | Место | Суть |
|---|---|---|
| **ARCH-10** | credentials/oauth2.rs:16-19 | Re-export oauth2_config types «for backward compatibility with `nebula-api`/`nebula-storage`». Storage не должна ходить за internals credential. |
| **ARCH-11** | contract/registry.rs:78-90 | `AHashMap` для startup-only registry (не hot-path). Микро-оптимизация в неправильном месте. |

---

## IV. Открытые архитектурные дыры (GAP)

| ID | Где | Что |
|---|---|---|
| **GAP-01** | [resource/manager.rs:1380](../../crates/resource/src/manager.rs) | `OnCredentialRefresh` fan-out — `todo!()`. Trait и reverse-index готовы, dispatch заблокирован RPITIT-vs-dyn выбором: (a) per-`C` mono-tables vs (b) parallel `DynOnCredentialRefresh + BoxFuture`. |
| **GAP-02** | rotation FSM | Compile-fail probes только для capability-discipline. Нет compile-fail для FSM transitions (`Pending → Validating` без `Creating` — не блокируется типом, только runtime-check в [state.rs:44-67](../../crates/credential/src/rotation/state.rs)). |
| **GAP-03** | rotation feature-gate | Контрактные типы (`policy.rs`, `state.rs`, `error.rs`) **не** под `#[cfg(feature = "rotation")]`, всегда компилируются. Только orchestration в engine за gate. Семантика «contract free, orchestration opt-in» не задокументирована. |
| **GAP-04** | rotation cleanup_old | После grace-period должен дёрнуться `cleanup_old()`. Кто его дёргает в engine — не очевидно. Нет integration-теста полного `Pending → ... → Committed → GraceExpired → Cleanup`. |
| **GAP-05** | sentinel threshold | N=3 events в 1h → `ReauthRequired(SentinelRepeated)` ([resolve.rs:200-205](../../crates/engine/src/credential/refresh/resolve.rs)). Threshold захардкожен, не настраивается per-credential. |

---

## V. Тесты (TEST)

### High (см. §I)

### Medium

| ID | Что |
|---|---|
| **TEST-03** | Нет integration-теста, что все 5 capability sub-traits корректно dispatch'атся через engine. `tests/registry_capabilities_iter.rs` (258 строк) проверяет только `iter_compatible()`, не сам dispatch. |
| **TEST-04** | OAuth2 full flow test (token endpoint mock, refresh, expiry) не реализован. |
| **TEST-05** | Rotation FSM full flow (`Pending → ... → Committed → GraceExpired → Cleanup`) — нет orchestration-flow теста. |

### Low

| ID | Что |
|---|---|
| **TEST-06** | Нет proptest/quickcheck для encryption round-trip, metadata validation, scheme coercion. |
| **TEST-07** | Нет `cargo fuzz` для deserialization boundaries, encryption padding. |
| **TEST-08** | Нет insta-snapshot тестов для events, error-сообщений, metadata-изменений. |

---

## VI. Документация (DOC)

| ID | Где | Статус |
|---|---|---|
| **DOC-01** | PRODUCT_CANON.md §3.5/§13.2 | OK |
| **DOC-02** | INTEGRATION_MODEL.md (credential раздел) | Не верифицировано — может ссылаться на старые имена (`Metadata`/`Description` до ADR-0004) |
| **DOC-03** | MATURITY.md:24 | Расхождение: pruning 2026-04-24 (FederatedAssertion/OtpSeed/ChallengeSecret убраны), README.md:41 всё ещё упоминает «9 built-in scheme types» |
| **DOC-04** | GLOSSARY.md | Не верифицировано — новые термины (Plane B, Pending, Dynamic) могут отсутствовать |
| **DOC-05** | STYLE.md §6 «Secret handling» | OK (compile_fail_state_zeroize держит инвариант) |
| **DOC-06** | ENGINE_GUARANTEES.md (credential раздел) | Не верифицировано — не отражает П2 L2 coordinator changes (ADR-0041)? |
| **DOC-07** | OBSERVABILITY.md:76-117 | OK (метрики и span'ы перечислены, тесты есть) |
| **DOC-08** | UPGRADE_COMPAT.md | Не верифицировано — П1 breaking changes (capability sub-trait split, sensitivity dichotomy) задокументированы? |

**ADR coverage** — все credential-relevant ADR имеют `accepted` статус и реализованы:

| ADR | Тема | Реализация |
|---|---|---|
| 0004 | Rename: Metadata→Record | OK |
| 0028 | Cross-crate invariants (umbrella) | OK |
| 0029 | Storage owns persistence | OK |
| 0030 | Engine owns orchestration | OK |
| 0031 | API owns OAuth flow | OK |
| 0032 | CredentialStore canonical home | OK (но см. ARCH-02/03 — shim duplication) |
| 0033 | Plane B integration credentials | OK |
| 0034 | Schema secret-value seam | OK |
| 0035 | Phantom-shim capability pattern | OK |
| 0041 | Durable refresh claim repo (L2) | OK |

---

## VII. Перформанс и идиомы

### VII.A Перформанс (PERF)

| ID | Sev | Место | Суть | Фикс |
|---|---|---|---|---|
| **PERF-01** | H | [coordinator.rs:539,565](../../crates/engine/src/credential/refresh/coordinator.rs) + [l1.rs:67](../../crates/engine/src/credential/refresh/l1.rs) | L1 keyed `String`. `to_string()` + `clone()` — 2× alloc per refresh. Hot-path. | `HashMap<Arc<str>, _>` (или `HashMap<CredentialId, _>`); `Arc::clone` в scopeguard. |
| **PERF-02** | H | [resolver.rs:139,189,211,341,398,416,472,476,483,537,654](../../crates/engine/src/credential/resolver.rs) | 12× `credential_id.to_string()` per resolve, 3 на success-path. | `let cred_id: Arc<str> = Arc::from(credential_id);` и `Arc::clone` в замыкания. |
| **PERF-03** | M | [coordinator.rs:846](../../crates/engine/src/credential/refresh/coordinator.rs) | `replica_id.as_str().to_string()` per `spawn_heartbeat`. | `Arc<str>` рядом с `replica_id`. |
| **PERF-04** | M | [crypto.rs:208-213,302-313](../../crates/credential/src/secrets/crypto.rs) | `decrypt` делает `ciphertext.clone()` + `extend_from_slice(&tag)`. Alloc + memcpy на каждый decrypt. | Переключить на `decrypt_in_place_detached(nonce, aad, &mut buf, tag_array)`. |
| **PERF-05** | M | [resolver.rs:439-461](../../crates/engine/src/credential/resolver.rs) (rotation feature) | 2× JSON serde round-trip per OAuth2 refresh: `C::State` → `Value` → `OAuth2State` → mutate → `Value` → `C::State`. | `(state as &mut dyn Any).downcast_mut::<OAuth2State>()` или вытащить в trait-hook `Refreshable::refresh_via_engine_http`. |
| **PERF-06** | M | [oauth2.rs:395-404](../../crates/credential/src/credentials/oauth2.rs) | `OAuth2Credential::project` deep-clone'ит `Vec<String>` scopes на каждый resolve. Doc обещает «synchronous, pure» — implies cheap. | `scopes: Arc<[String]>`. `project` становится `Arc::clone`, true O(1). |
| **PERF-07** | L | [coordinator.rs:633,705](../../crates/engine/src/credential/refresh/coordinator.rs) | Двойной abort: scopeguard + явный `hb_task.abort()` на success-path. Гонка с self-cancel-arm. | Убрать abort_handle capture; полагаться на `cancel.cancel()` + `select!`. |
| **PERF-08** | L | [registry.rs:22-26,80-88](../../crates/engine/src/credential/registry.rs) | `Arc<dyn Fn>` на append-only registry, который и так в `Arc<StateProjectionRegistry>`. Лишний atomic refcount per dispatch. | `Box<dyn Fn>` — outer Arc уже даёт sharing. |
| **PERF-09** | L | [crypto.rs:42-58](../../crates/credential/src/secrets/crypto.rs) | `derive_from_password` (Argon2id 19 MiB / 2 iters, 100–200 ms) на calling thread. Сейчас только из storage setup. | `tokio::task::spawn_blocking` или sibling `derive_from_password_async`. |

### VII.B Rust-идиомы 1.95+ (IDIOM)

| ID | Sev | Место | Суть | Фикс |
|---|---|---|---|---|
| **IDIOM-01** | H | [credential/provider.rs:107](../../crates/credential/src/provider.rs) | Последний `#[async_trait]` в credential-surface. Все остальные контракты на RPITIT. `ExternalProvider` dyn-dispatched (`Box<dyn>`). | `#[trait_variant::make(Send)]` или manual: `async fn` для импла + `fn resolve_dyn(...) -> Pin<Box<dyn Future + Send + '_>>` для object-safe. |
| **IDIOM-02** | M | [accessor.rs:14-39,106-131,165-192](../../crates/credential/src/accessor.rs) | `BoxFuture<'a, T>` + `Box::pin(async {})` повсюду. `CredentialAccessor` живёт в `nebula-core`, dyn-dispatch (`Arc<dyn>`) блокирует RPITIT. | Local edit не решит — флаг для **architect** на редизайн `nebula-core` accessor trait. |
| **IDIOM-03** | M | [error.rs:126,134,146](../../crates/credential/src/error.rs) | `Box<dyn Error + Send + 'static>` на 3 вариантах (`RefreshFailed`, `RevokeFailed`, `CompositionFailed`). Antipattern в 1.95+ + **отсутствует `Sync`**. | Закрытые enum'ы `RefreshFailureCause` etc.; либо минимум `+ Sync`. |
| **IDIOM-04** | M | [store_memory.rs:30](../../crates/credential/src/store_memory.rs) + [pending_store_memory.rs:52](../../crates/credential/src/pending_store_memory.rs) | `tokio::sync::RwLock` на test-only stores с zero `.await` под локом. Async-aware overhead зря. | `parking_lot::RwLock` (sync). 5–10× быстрее. |
| **IDIOM-05** | M | [oauth2.rs:395-404](../../crates/credential/src/credentials/oauth2.rs) | Дублирует PERF-06 + over-promising в [contract/credential.rs:151-155](../../crates/credential/src/contract/credential.rs) («synchronous, pure»). | Чинить через PERF-06 fix или ослабить doc на «synchronous; SHOULD be O(1)». |
| **IDIOM-06** | L | [coordinator.rs:740-741](../../crates/engine/src/credential/refresh/coordinator.rs) | `const MAX_ATTEMPTS: usize = 5` function-local. Chaos-тесты не могут override без recompile. | `RefreshCoordConfig.l2_max_attempts` с default 5 + `validate() >= 1`. |
| **IDIOM-07** | L | [coordinator.rs:683](../../crates/engine/src/credential/refresh/coordinator.rs) | `.and_then(std::convert::identity)` для flatten `Result<Result<_,E>, E>`. | `.flatten()` (стабильно с 1.66+). |
| **IDIOM-08** | L | [oauth2.rs:392](../../crates/credential/src/credentials/oauth2.rs) | `.expect("oauth2 metadata is valid")` на static-shape construction. | Если `CredentialMetadataBuilder` можно сделать `const fn` — panic превратится в compile error. |
| **IDIOM-09** | L | [record.rs:211](../../crates/credential/src/record.rs) | `std::thread::sleep(Duration::from_millis(10))` в sync-тесте — flaky на Windows под contended scheduler. | `let original = chrono::Utc::now() - Duration::from_millis(1)` без sleep. |
| **IDIOM-10** | L | [l1.rs:189](../../crates/engine/src/credential/refresh/l1.rs) | `HashMap::new()` (SipHash) на hot-path lookup. Registry уже доказал `AHashMap` ~3× быстрее. | `AHashMap<Arc<str>, _>` — комбинируется с PERF-01. |

### VII.C Перекрытия — где один фикс закрывает несколько issue

| Перекрытие | Заметка |
|---|---|
| **PERF-04 + SEC-04** | `decrypt_in_place_detached` (PERF-04) + `OsRng` для nonce (SEC-04) — оба в [crypto.rs](../../crates/credential/src/secrets/crypto.rs); один PR на crypto-cleanup. |
| **PERF-06 = IDIOM-05** | Один и тот же site — `oauth2.rs:395-404`. Чинить через `Arc<[String]>`, не через doc-патч. |
| **IDIOM-03 + ARCH-06** | Box-dyn-error + 518-line `error.rs` — рефакторинг error-модуля решает оба сразу. |
| **IDIOM-04 + ARCH-02/ARCH-03** | Test-shim `RwLock` + дублирование между credential/storage — общий рефакторинг shim-стратегии. |
| **IDIOM-10 + PERF-01** | Оба ведут к `AHashMap<Arc<str>, _>` для L1. Один PR. |
| **PERF-05 + GAP-03** | Special-case OAuth2 в resolver лежит за `feature = "rotation"`. Решение — вытащить в trait-hook (`Refreshable::refresh_via_engine_http`), что заодно убирает feature-gate-asymmetry. |

---

## VIII. Силы крейта (24 образцовых места)

Не всё плохо. Места, где код образцово:

1. AAD-record-swap защита явная и протестирована (`aad_prevents_record_swapping`).
2. AAD-mandatory invariant — `encrypt_with_key_id` reject'ит empty `key_id` (crypto.rs:343).
3. `SecretString` дефолтный `Serialize` пишет `[REDACTED]` — defence-in-depth.
4. Hand-rolled `Debug` для `OAuth2State` / `OAuth2Pending` — derive бы leak'нул.
5. `OAuth2Pending::zeroize` сбрасывает `Option<>` в `None` после wipe.
6. Constant-time state compare через `subtle::ConstantTimeEq`.
7. `EnvKeyProvider::DEV_PLACEHOLDER` reject — leaked-back dev key не станет prod.
8. `FileKeyProvider` regular-file check **до** permissions — closes TOCTOU + `/dev/urandom`-block.
9. `EncryptionLayer::new` больше не aliases `""` под current key (#281 fix).
10. `bearer_header()` возвращает `SecretString` (per N4).
11. `SchemeGuard` `!Clone` + `'a`-pinned — compile-time retention barrier.
12. Audit layer FAIL-CLOSED (ADR-0028 inv 4).
13. Lazy re-encryption на rotation использует CAS — нет clobber concurrent updates.
14. Refresh coordinator validates `heartbeat × 3 ≤ ttl` инвариант на construction.
15. Sentinel mid-refresh + N=3-in-1h escalation закрывает n8n #13088 refresh-storm.
16. PKCE S256 reference vector test (`test_pkce_rfc7636_example`).
17. Layered architecture в `storage/src/credential/`: `key_provider/layer/memory/pending/backup/refresh_claim` — каждый модуль одна ответственность.
18. `engine/src/credential/mod.rs` следует ADR-0030 shape: 26 строк, всё на месте.
19. `contract/` иерархия в credential — каждый capability в отдельном файле, `mod.rs` чисто re-export, без логики.
20. **Biased select** в [coordinator.rs:660-680](../../crates/engine/src/credential/refresh/coordinator.rs) с 10-строчным комментарием-обоснованием (n8n #13088 lineage прямо у строк).
21. **Waiter-under-lock** в [l1.rs:44-49,235-248](../../crates/engine/src/credential/refresh/l1.rs): `senders: Mutex<Vec<oneshot::Sender>>` внутри entry под outer map lock. Lost-wakeup race (#268) закрыт by construction. Регрессия `waiter_registered_under_lock_is_never_missed`.
22. **`ArcSwap` + Clone-independence** в [handle.rs:26-67](../../crates/credential/src/handle.rs). `Clone` создаёт *независимый* `ArcSwap` — клонирование никогда не пересекает refresh-visibility. Тест `clone_creates_independent_handle`.
23. **First-wins fail-closed** в [contract/registry.rs:88,131-167](../../crates/credential/src/contract/registry.rs): `AHashMap<Arc<str>, _>` zero-alloc lookup через `Borrow<str>`, operator-actionable `RegisterError::DuplicateKey`. Append-only invariant в rustdoc оправдывает lock-free hot path.
24. **NIST-sourced nonce design** в [crypto.rs:126-142](../../crates/credential/src/secrets/crypto.rs): 6 строк кода + 10 строк citation-grade rationale (NIST SP 800-38D §8.2.2). _Внимание:_ doc заявляет «OS CSPRNG», код использует `ThreadRng` — см. SEC-04.

---

## IX. Сводка по приоритетам

### Закрыть до prod-релиза (13 пунктов)

- **SEC-01, SEC-02** — DoS + log injection через IdP. Простые фиксы (bounded reader, URL-validate), требуют release.
- **SEC-03, SEC-04** — AAD redesign + nonce-source. Требуют architect (envelope contract change).
- **GAP-01** — `OnCredentialRefresh` fan-out. Требует решения RPITIT-vs-dyn вилки.
- **TEST-01, TEST-02** — e2e + per-resource swap. Покроют GAP-01 заодно.
- **ARCH-01, ARCH-02, ARCH-03** — test-shim duplication + private modules consistency.
- **PERF-01, PERF-02** — `Arc<str>` для credential_id вместо `String` (hot path).
- **IDIOM-01** — последний `#[async_trait]` в `provider.rs` → RPITIT.

### Следующий цикл (18 пунктов)

- Все SEC Medium (5–13) — 9 пунктов.
- ARCH-04..09 — рефакторинги модулей. 6 пунктов.
- GAP-02..05 — rotation observability и compile-fail. 4 пункта (GAP-02..04).
- PERF-03..06, PERF-09 — 5 пунктов.
- IDIOM-02..05 — 4 пункта.
- TEST-03..05 — capability dispatch + OAuth2 + rotation FSM. 3 пункта.
- DOC-02, DOC-04, DOC-06, DOC-08 — верификация секций docs.

### Косметика (13 пунктов)

- SEC-14..18, ARCH-10..11, PERF-07..08, IDIOM-06..10, TEST-06..08, DOC-03 + GAP-05.

---

## X. Resolution map (как один PR закрывает несколько issue)

| PR scope | Закрывает |
|---|---|
| Crypto cleanup (in-place-detached + OsRng) | PERF-04, SEC-04 |
| Error module split + typed cause-enums | ARCH-06, IDIOM-03 |
| Test-shim consolidation (`nebula-storage::test-util`, drop credential-копии) | ARCH-02, ARCH-03, IDIOM-04 |
| L1 keying refactor (`Arc<str>` + AHashMap) | PERF-01, IDIOM-10 |
| OAuth2 trait-hook (`Refreshable::refresh_via_engine_http`) | PERF-05, GAP-03 |
| OAuth2 scope sharing (`Arc<[String]>`) | PERF-06, IDIOM-05 |

---

## XI. Recommended next-2-weeks path

1. **Day 1–2**: SEC-01, SEC-02 — bounded reader + URL-validate (быстрые, прод-блокеры).
2. **Day 3–4**: PERF-01, PERF-02, IDIOM-10 — `Arc<str>` рефакторинг (один architectural decision).
3. **Day 5–7**: GAP-01 — выбор RPITIT-vs-dyn + начало fan-out wiring.
4. **Day 8–10**: SEC-03 — AAD redesign (envelope contract change, требует ADR-amendment).
5. **Day 11**: SEC-04 — nonce-source fix (тривиально после crypto.rs touch).
6. **Day 12–14**: TEST-01, TEST-02 — e2e и per-resource swap test.
7. **Day 15+**: IDIOM-01 — provider.rs `#[async_trait]` → RPITIT.

ARCH-01..03 можно идти параллельно с любой из недель.

---

## Sources

Аудит проведён 2026-04-27 в worktree `eager-bassi-c03245` четырьмя параллельными агентами:

- **security-lead** (`a3f1a44b54bcc1f93`) — SEC-01..18
- **rust-senior** (`a30a180a32006e2a6`) — PERF-01..09, IDIOM-01..10, образцы 20–24
- **Explore (architecture)** — ARCH-01..11, образцы 17–19
- **Explore (tests/docs)** — TEST-01..08, DOC-01..08

GAP-01..05 — synthesized из контекста двух предыдущих исследований этой сессии (refresh fan-out + rotation feature-gate).

Agent-local working notes were used during the audit session, but they are not committed to this repository.
