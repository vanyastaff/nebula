# `nebula-credential` — реестр проблем (snapshot 2026-04-27)

**Snapshot commit:** `f308ded4` (П2 — refresh coordination L2, n8n #13088 close)
**Scope:** `crates/credential` (package: `nebula-credential`) + связи в `crates/engine` (`nebula-engine::credential`), `crates/storage` (`nebula-storage::credential`), `crates/resource`, `crates/action`
**Audit composition:** security-lead + rust-senior + 2× Explore agents (architecture, tests/docs)
**Errata:** post-review 4-agent consensus (security-lead + rust-senior + tech-lead + spec-auditor) corrected counts, reclassified severity, and recorded conflicts. **See §XII** before acting on §I/§IX/§XI.

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

---

## §XII Errata (post-review 2026-04-27 — 4-agent consensus)

> **Process note.** After this audit landed at commit `f308ded4`, four specialist agents reviewed it independently. This Errata records corrections to document defects (counts, file paths, line numbers, glossary), severity reclassifications based on direct verification, and architect-handoff signals. Original §I–§XI text is preserved unchanged; **this section supersedes any conflict** with §I/§IX/§XI.

### §XII.A Process

| Agent role | Scope | agentId |
|---|---|---|
| security-lead (re-verify) | SEC-01..18 against threat models + advisories | `a371e5d799a0505d5` |
| rust-senior (re-verify) | PERF-01..09 + IDIOM-01..10 against 1.95 idioms | `a04feb80c85fcb5ca` (continued: `a0068814d902dc18a`) |
| tech-lead (priority call) | §I/§XI vs Strategy §6.5 frozen queue | `adf1f21e869808ac3` (continued: `a8d8c1bf0fcdb7188`) |
| spec-auditor (document audit) | counts, cross-refs, bookkeeping, glossary | `a486010c5ca2f7ba8` |

Review date: 2026-04-27. Performed in same worktree as audit landing.

### §XII.B Document defects (spec-auditor evidence)

| ID | Defect | Correction |
|---|---|---|
| **err-1** | TL;DR claims «52 проблемы» | Actual full inventory: **69 пунктов** (SEC 18 + ARCH 11 + GAP 5 + TEST 8 + DOC 8 + PERF 9 + IDIOM 10) |
| **err-2** | §IX «Сводка по приоритетам» sums: 13 + 18 + 13 = 44 | Header counts inconsistent with body. «Следующий цикл — 18 пунктов» actual body: **34 items**. «Косметика — 13 пунктов» actual body: **19 items** |
| **err-3** | TL;DR + §VII.C: «6 фиксов закрывают 12 проблем» | §X table sum: 2+2+3+2+2+2 = **13** issues (Test-shim consolidation closes 3, not 2) |
| **err-4** | GAP-05 cites `crates/engine/src/credential/refresh/resolve.rs` | **File does not exist.** `SentinelRepeated` lives in `crates/engine/src/credential/refresh/sentinel.rs` + `reclaim.rs` |
| **err-5** | GAP-05 claim «threshold захардкожен, не настраивается» | **False.** `SentinelThresholdConfig { threshold, window }` is runtime-configurable in `crates/engine/src/credential/refresh/sentinel.rs:26-43` |
| **err-6** | PERF-01 cites `coordinator.rs:539,565` | Actual `to_string()` sites: `coordinator.rs:557,583`. Lines 539/565 are inside comment block |
| **err-7** | §IV table missing severity column | GAP-02..05 have no severity assignment; readers cannot filter |
| **err-8** | 5 glossary terms used but undefined in `docs/GLOSSARY.md` | Missing: **Plane B**, **Pending** (rotation FSM state — only `PendingDrain` exists for resources, different concept), **Dynamic** (provider class), **N=3-in-1h** / **sentinel**, **herd-сценарий** |

**Document quality score (spec-auditor):** 5.5/10. Main artefacts of concern: triple-count drift, GAP-05 unverifiable claim cluster (matches the «agent-local working notes not committed» disclaimer — documented risk realised), §VII.C arithmetic, PERF-01 line drift, §IV severity column omission.

### §XII.C Severity reclassifications

| Issue | Auditor severity | Final severity | Source | Rationale |
|---|---|---|---|---|
| SEC-04 (`fresh_nonce` ThreadRng) | H | **L (cosmetic doc edit)** | security-lead | `rand 0.10.1::rng()` returns `ThreadRng` seeded from OS CSPRNG; RUSTSEC-2026-0097 — panic-handler thread-local soundness, **not** nonce predictability; advisory already in `deny.toml:16` with explicit ignore rationale; doc says «OS CSPRNG» — slightly imprecise but security property holds |
| SEC-03 (AAD without `key_id`) | H | **M (audit-trail integrity, not credential theft)** | security-lead | row-swap structurally possible, but attacker with raw storage write can replace ciphertext entirely; AAD = `credential_id` already catches cross-record swap (`aad_prevents_record_swapping` test, §VIII positive #1); reframe as defense-in-depth |
| SEC-01 (unbounded IdP body) | H | **M** | security-lead | `oauth_token_http_client()` enforces `timeout(30s)` covering body read; 10 GB body unrealistic in window; still operational hardening needed |
| SEC-13 (refresh err msg leak) | H | **conditionally H** | security-lead | depends on whether ADR-0030 §4 redaction CI gate is firing on refresh error path; verify before downgrading |
| GAP-01 (`manager.rs:1380` `todo!()`) | H | **П3+ deferred cascade (Medium)** | tech-lead + spec-auditor | Tech Spec [§15.7 lines 3522-3523](../superpowers/specs/2026-04-24-credential-tech-spec.md) explicitly defers `manager.rs:1378` `todo!()` as canonical П1 state via `OnCredentialRefresh<C>` parallel trait; comment at `crates/resource/src/manager.rs:1374-1378` cites concerns-register row `stage6-followup-resource-integration`; intentional, not missed wire-up |
| ARCH-02 / ARCH-03 (test-shim duplication) | H | **non-finding (intentional ADR-0032 §3 design)** | tech-lead | `test-util` exposing `InMemoryStore` parallel to storage copy is phantom-shim pattern executed deliberately per [ADR-0032](../adr/0032-credential-store-canonical-home.md) §3 + [cleanup-p6-p11.md](../superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md) §ADR-0032 row |
| TEST-01 / TEST-02 | H | **П3 planning item** | tech-lead | `user-test-integration` / `user-test-concurrency` locked-post-spike in register; multi-replica chaos (`refresh_coordinator_chaos.rs`) already in active П2 plan |
| IDIOM-01 (`provider.rs:107` async_trait) | H | **П3 planning item** | tech-lead + rust-senior | clean AFIT win (zero `dyn` consumers in workspace); slot under П3 capability sub-trait split, not Errata-blocker |
| PERF-04 (`decrypt_in_place_detached`) | M | **L (micro-perf)** | rust-senior | encrypted payloads 32-512 bytes, ~50ns saved per decrypt, on cache-miss not every request; treat as IDIOM-only |
| PERF-05 (JSON round-trip OAuth2 refresh) | M | **IDIOM (correctness erosion, reframe from PERF)** | rust-senior | not perf — `Any::downcast_mut` is escape hatch silently breaking when new state types added; reframe to «correctness» axis |

### §XII.D Architect handoff signals (rust-senior)

#### D-1. PERF-01 / PERF-02 → `CredentialId: Copy` newtype migration, NOT `Arc<str>`

Audit recommends `HashMap<Arc<str>, _>` for L1 cache key. rust-senior verified `crates/core/src/id/types.rs:21` — `CredentialId = define_ulid!` already a `Copy` newtype. Threading `CredentialId` directly is faster (register copy vs 2 atomic ops on `Arc::clone`) and ergonomically cleaner (no `Clone` infection across signatures). **Bundle PERF-01 + PERF-02 + IDIOM-10 as one architect-level PR.** Strategy §6.5 parallel track.

#### D-2. PERF-05 → `Refreshable::refresh_via_engine_http` AFIT trait-hook

Audit's `Any::downcast_mut::<OAuth2State>()` alternative is **rejected** by rust-senior: «escape hatch that silently goes wrong when someone adds OidcState or Saml2State and forgets the match arm». Trait-hook with `async fn refresh_via_engine_http(&mut self, ...) -> impl Future<...>` is the correct path. Goes into П3 capability sub-trait scope (Tech Spec §15.4), not standalone Errata fix.

#### D-3. IDIOM-03 → `Box<dyn Error + Send>` must become `+ Sync`

`RefreshFailed`, `RevokeFailed`, `CompositionFailed` cross `tokio::spawn` boundaries inside the coordinator. Compiles today only because no `&CredentialError` is held in `tokio::select!` arms or shared diagnostics state — the moment that lands, the bound breaks. **Must-fix, not stylistic.** Either close the enum or add `+ Sync`.

### §XII.E True prod-blocker list (post-review)

After security-lead reclassifications + tech-lead resolution + SEC-04 conflict resolved per §XII.G:

1. **SEC-02** (URL-validate `error_uri`) — security-lead: «only true prod-blocker among SEC-01..04 as written». Two-line fix: `Url::parse` + scheme allowlist `["https"]` + length cap.
2. **SEC-05 + SEC-06 cluster (N10 violation)** — `CredentialGuard: Clone` removal + `SchemeGuard: !Send` enforcement. PRODUCT_CANON §4.2 invariant.
3. **SEC-13** — **resolved in security hardening Stage 0.5** (PR/commit pending). Verdict per §XII.A process: gate did NOT fire; fix landed. `redact_sensitive_fields` helper added to `crates/engine/src/credential/rotation/token_refresh.rs::oauth_token_error_summary` — case-insensitive regex on `(refresh|access)_?token`, `client_secret`, `bearer`, `api_key`, `password`, `secret` field-name patterns. ADR-0030 §4 «one redaction test per token_refresh code path» CI gate now exists at `crates/engine/tests/credential_refresh_redaction.rs` with 5 initial rows (SEC-13 + 4 defensive coverage paths). Future token_refresh code paths add new rows.
4. **SEC-01** (bounded reader on OAuth IdP body) — Medium under timeout mitigation, but cheap to fix; bundle with SEC-02.
5. **SEC-03 (split from SEC-04)** — AAD + `key_id` as **separate PR with ADR amendment**, audit-trail integrity reframe. **NOT bundled with crypto cleanup** — different threat model, different review depth, different rollback shape.
6. **`CredentialId: Copy` migration bundle** — PERF-01 + PERF-02 + IDIOM-10 as one architect-level PR; parallel track to §6.5 queue, not blocker.

**Removed from prod-blocker list:** SEC-04 (false alarm — doc edit), GAP-01 (П3+ deferred), TEST-01/02 (П3 planning), ARCH-02/03 (intentional design), IDIOM-01 (П3 planning).

### §XII.F §XI roadmap conflict (tech-lead)

Audit's §XI day-by-day plan does **not** reference Strategy [§6.5 frozen sub-spec queue](../superpowers/specs/2026-04-24-credential-redesign-strategy.md) (ProviderRegistry seeding, multi-step accumulator, schema migration v1→v2, trigger↔credential, WS events). tech-lead resolution:

> «That is fine as a *parallel* track but cannot be advertised as the next-2-weeks path-of-record without disclosing that the §6.5 queue is paused.»

**Resolution.** §XI is **reframed** as «Security-hardening parallel track within `nebula-credential` and `engine/credential`». It is NOT a roadmap supersede. Strategy §6.5 queue remains authoritative for cross-cutting next-up work (ProviderRegistry → multi-step → schema migration → ...).

«13 High до prod-релиза» framing is also rejected: `nebula-credential-builtin` concrete sub-trait impls land in П3 ([docs/MATURITY.md:25](../MATURITY.md)), no prod release scheduled in 2-3 weeks. Severity column should re-bucket: **prod-blockers / П3-prerequisites / active-dev hardening / cleanup**.

### §XII.G Conflict resolution log

| Conflict | Resolution | Evidence |
|---|---|---|
| **SEC-04** (security-lead vs tech-lead) | security-lead wins (false alarm; doc edit only) | Direct verification: `crates/credential/Cargo.toml` (rand 0.10.1 resolved); `deny.toml:16` (RUSTSEC-2026-0097 already ignored with rationale); advisory text confirms panic-handler thread-local soundness, not CSPRNG flaw. tech-lead listed SEC-04 as «one-line OsRng swap» in §XII.H.4 «5 items I would actually pick up next» without re-verifying advisory text — accepted as priority-call framing, **declined as technical claim**. |

### §XII.H Full agent verdicts (verbatim)

#### §XII.H.1 security-lead verdict

> **SEC-04 (HIGH) — false alarm as framed; downgrade to LOW (cosmetic).** Code at `crates/credential/src/secrets/crypto.rs:136-142` does `let mut rng = rand::rng(); let nonce_bytes: [u8; 12] = rng.random();`. Cargo.lock resolves `rand 0.10.1`. In `rand 0.10` `rand::rng()` returns `ThreadRng` which IS CSPRNG-quality — seeded from `OsRng` (`getrandom`) and periodically reseeded. `RUSTSEC-2026-0097` IS real (`deny.toml:16`, with explicit `ignore` rationale: «rand 0.10.0 unsound only under custom panic-logger + thread-local interaction, not our usage pattern»). The advisory has nothing to do with nonce predictability or quality — it's a panic-handler thread-local soundness bug, NOT a CSPRNG flaw. The doc says «OS CSPRNG» — slightly imprecise but the security property (cryptographically-strong 96-bit random) holds. NIST SP 800-38D §8.2.2 (correctly cited in the doc comment) gives ~2³² encryptions/key under birthday bound, which is fine. Auditor's «catastrophic AES-GCM» framing is alarmist. Fix is a one-line doc edit («CSPRNG seeded from OS»), not envelope/contract surgery. **This kills SEC-04's place on the §I prod-blocker list and removes the «Crypto cleanup» §X PR's security justification — PERF-04 (`decrypt_in_place_detached`) still stands but loses its «+ SEC fix» multiplier.**
>
> **SEC-03 (HIGH) — real but misclassified; downgrade to MEDIUM.** Verified: `encrypt_with_key_id` (crypto.rs:337) uses caller-supplied AAD; `EncryptionLayer::encrypt_data/decrypt_possibly_rotating` (`crates/storage/src/credential/layer/encryption.rs:208,228`) passes `id.as_bytes()` only — `key_id` is genuinely NOT in AAD. The auditor's row-swap scenario IS structurally possible: an attacker with raw storage write can swap `key_id` to a registered legacy entry and force decryption with the older key. BUT: (a) the attacker must already have arbitrary write to the encrypted blob, at which point they can replace ciphertext entirely; (b) AAD = `credential_id` already prevents *cross-record* swap (the documented attack — `aad_prevents_record_swapping` test, audit §VIII positive #1); (c) downgrade only succeeds if legacy key is loaded AND decrypts to attacker-controlled plaintext. This is a **defense-in-depth audit-trail integrity gap**, not credential theft. Aligns with my round-2 N5 («AAD must include tenant_id + provider_spec_version») which is the right framing. Real Medium under PRODUCT_CANON §4.2/§12.5; do NOT call this a release blocker.
>
> **SEC-01 (HIGH) — confirmed but mitigated; downgrade to MEDIUM.** `crates/engine/src/credential/rotation/token_refresh.rs:109` does call `resp.text().await` unbounded on the error path. BUT `oauth_token_http_client()` (`crates/engine/src/credential/rotation/token_http.rs:49-60`) sets `timeout(30s)` + `connect_timeout(10s)` covering the entire request including body read; reqwest enforces it. A 10 GB body hits 30s timeout long before OOM unless the IdP can sustain ~333 MB/s sustained, at which point timeout is the real defense. A fast malicious IdP could still push hundreds of MB in 30s. Real prod-hardening; fix is trivial (replace with `read_token_response_limited` using the same 256 KiB cap from `OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES`). Not a release blocker — Medium.
>
> **SEC-02 (HIGH) — confirmed, real, keep HIGH.** Verified `token_refresh.rs:170-173`: `out.push_str(uri); out.push(')');` — `error_uri` from IdP is concatenated raw into operator-facing summary, then propagated through `TokenRefreshError::TokenEndpoint{summary}` into tracing/logs. No URL parse, no scheme allowlist, no length cap, no control-char filter. Compromised/MITM IdP injects `\n[CRITICAL] click https://attacker/phish-creds\n[2J` into operator alerts, SIEM rows, `Display`-rendered errors. Same class as my round-2 N1. Two-line fix (`Url::parse` + scheme `["https"]` allowlist + length cap). **This is the only one of SEC-01..04 I'd actually block release on as written.**
>
> **SEC-05..18 grouped:** SEC-05 (`CredentialGuard: Clone` at `crates/credential/src/secrets/guard.rs:64-71`) — verified, real Medium feeding round-2 N10 (plaintext crossing spawn boundaries). SEC-06 (`SchemeGuard` not `!Send`) — same N10 cluster, real Medium. SEC-07 (`[REDACTED]` deserialize accepts only exact string) — real low/medium, exploitable only if attacker controls serialized blob. SEC-08 (`pub fn serialize(&SecretString)` module-public) — real Medium, should be `pub(crate)`. SEC-09 (`format!("Bearer {}")` non-Zeroizing intermediate) — real Medium, classic leak. SEC-10 (`.expose_secret().to_owned()` before `Zeroizing::new` at `token_refresh.rs:62-72`) — verified, real Medium ×3 sites. SEC-11 (`pub fn encrypt` without AAD/key_id still public) — real Medium, should be `pub(crate)` or removed. SEC-12 (no precedence between ENV/FILE KeyProvider) — operational Medium. SEC-13 (refresh error msg may echo IdP `error_description` containing refresh_token) — real Medium; ADR-0030 §4 redaction CI gate should be checked but this specific path may bypass it. SEC-14 OK as marked. SEC-15 (`#[serde(default)]` on `key_id` open-ended) — real Low, deadline-tracking concern. SEC-16 (`tracing::warn!(?e)` on `sqlx::Error`) — real Medium, sqlx Debug can include connection-string with password. SEC-17 (RUSTSEC sweep on aes-gcm/subtle/secrecy) — needs `cargo deny check advisories` run. SEC-18 (`Box<dyn Error + Send>` without `+ Sync`) — real Low/Medium, ergonomic across-thread issue, not a leak.
>
> **Final tally:** Of 18 SEC findings, **3 I'd mark High in my terminology** — SEC-02 (confirmed); SEC-05+SEC-06 collectively as the N10 plaintext-handoff cluster; SEC-13 conditionally High if the ADR-0030 §4 redaction CI gate isn't actually firing on the refresh error path. **2 false/over-framed** — SEC-04 (doc-vs-code only); SEC-01 (Medium under timeout mitigation). **3 need upstream/CI verification** — SEC-13 redaction gate evidence; SEC-15 deadline policy; SEC-17 RUSTSEC sweep.

#### §XII.H.2 spec-auditor verdict

> **Counts triple-mismatch: BLOCKER.** TL;DR claims **52 problems = 13 H + 21 M + 18 L**. §IX bucket headers say **13 + 18 + 13 = 44**. Full inventory across §II–§VII: SEC 18 + ARCH 11 + GAP 5 + TEST 8 + DOC 8 + PERF 9 + IDIOM 10 = **69 items**. Three different totals. The §IX «Следующий цикл» header says «18 пунктов» but its body lists 9+6+3+5+4+3+4 = **34**. «Косметика» header says «13 пунктов» but body sums to **19**. Only the §I High count of 13 is internally consistent.
>
> **Cross-reference path verification: 1 broken.** Most line cites verified within tolerance. **PERF-01 line drift**: `coordinator.rs:539,565` cited, but `to_string()` actually at line 557; 539/565 are inside comment block. **GAP-05 BLOCKER**: `crates/engine/src/credential/refresh/resolve.rs` **does not exist**; `SentinelRepeated` lives in `sentinel.rs` and `reclaim.rs`. GAP-05 also asserts threshold «захардкожен» — false; `SentinelThresholdConfig { threshold, window }` (sentinel.rs:26-43) is configurable.
>
> **§VII.C / §X overlap claim: arithmetic off.** TL;DR: «6 фиксов закрывают 12 проблем». §X table sum: 2+2+3+2+2+2 = **13**, not 12.
>
> **Severity decay / tone: OK.** §VIII (24 strengths) and TL;DR align with the High distribution. Tone is internally consistent.
>
> **GAP-01..05 severity assignment: drift.** §IV table omits the `Sev` column entirely, unlike §II/§III/§VII. GAP severity is implicit-by-bucket, not declared. Medium spec drift.
>
> **Uncommitted-notes provenance: explicit-but-unmitigated.** GAP-01..05 are explicitly «synthesized» from «two previous research sessions». GAP-05's wrong file path likely traces to those uncommitted notes — **the audit's only unverifiable claims clustered in the section flagged as synthesized from uncommitted sources**. This is the documented risk realised.
>
> **Glossary: 5 missing.** `docs/GLOSSARY.md` does NOT contain: **Plane B**, **Pending** (rotation FSM state), **Dynamic** (provider class), **N=3-in-1h** / **sentinel**, **herd-сценарий**.
>
> **Document quality score: 5.5/10.**

#### §XII.H.3 rust-senior verdict

> **PERF-01 / PERF-02 — `Arc<str>` vs `CredentialId: Copy` newtype.** Reality check: `CredentialId` is the cache key handed to `RefreshCoordinator::get_or_insert`, which means every refresh request, every `cache.contains_key`, every `pending.entry(id.clone())` clones it. The «hot path» framing isn't speculative — it's the literal lookup path executed under the per-credential mutex, often N times per second when refresh storms hit (TTL stampede, which we already had B6 for). `Arc<str>` clone is two atomic ops; a `Copy` newtype around `[u8; 16]` (UUID) or `u64` is a register copy. Even at 10k refresh/s the absolute cost is tiny, but the ergonomic win is real: `&id` everywhere disappears, signatures stop infecting downstream traits with `Clone` bounds, and `HashMap<CredentialId, _>` gets a faster hash. **Verdict: real PERF + idiom upgrade.**
>
> **PERF-04 — `decrypt_in_place_detached`.** Encrypted payloads are 32–512 bytes (refresh tokens, client_secret). Cutting one alloc per decrypt saves ~50ns; we do this on every cache miss, not every request. Detached AEAD changes the on-wire layout though, which means migration cost. **Verdict: micro-perf, not load-bearing — IDIOM-only.**
>
> **PERF-05 — `Refreshable::refresh_via_engine_http` trait hook vs `Any::downcast_mut`.** This isn't perf, it's correctness erosion. `downcast_mut::<OAuth2State>()` in a generic refresh path is a `dyn Any` escape hatch that silently goes wrong when someone adds `OidcState` or `Saml2State` and forgets the match arm. A `Refreshable` trait with `fn refresh_via_engine_http(&mut self, ...) -> impl Future<...>` (AFIT) makes the contract type-checked and lets every state type carry its own protocol logic. **Verdict: real IDIOM upgrade — the audit was right to flag this.**
>
> **IDIOM-01 — `provider.rs:107` `#[async_trait]` migration.** Confirmed zero `dyn AuthProvider` consumers in the workspace (grepped `crates/credential` and downstream — only static dispatch via generic `P: AuthProvider`). That removes the only real reason to keep `#[async_trait]`. AFIT migration is mechanical here: `async fn` in trait, no `Box<Pin<Future>>`, no `+ Send` infection beyond what the impl already needs. **Verdict: clean IDIOM win, no friction.**
>
> **IDIOM-03 — `Box<dyn Error + Send>` missing `+ Sync`.** `RefreshFailed`, `RevokeFailed`, `CompositionFailed` are surfaced through `tracing::error!(error = ?e, ...)` and propagated across `tokio::spawn` boundaries inside the coordinator. `Send` alone compiles today only because we never share the error across threads while it's borrowed — but the moment someone holds `&CredentialError` in a `tokio::select!` arm or stores it in shared diagnostics state, the bound breaks. Standard library and `eyre` both default to `Send + Sync`. **Verdict: must-fix; not stylistic.**
>
> **Overall:** of nine PERF items, four are load-bearing (PERF-01, PERF-02 if treated as one, plus the cache-key reshape and the decrypt path if combined). Of ten IDIOM items, five are real upgrades (PERF-05 trait hook, IDIOM-01 AFIT, IDIOM-03 Sync bound, plus two from the original list worth keeping). The rest are cosmetic — flag them as nice-to-have, not Errata-blocking.

#### §XII.H.4 tech-lead verdict

> **GAP-01 classification.** Misclassified. Tech Spec §15.7 lines 3522–3523 explicitly state that the П1 landing shape leaves `OnCredentialRefresh<C>` as the canonical refresh-hook trait inside `nebula-credential`, with `Resource` trait integration deferred to a follow-up cascade because threading `type Credential` would ripple through 28+ impls. The code comment at `crates/resource/src/manager.rs:1374-1378` cites the concerns-register row `stage6-followup-resource-integration` and matches the spec verbatim. The `todo!()` is intentional П1 state, not a missed wire-up. Re-label Medium and tag against П3 RPITIT-vs-dyn fork. The audit mistake is structural: it synthesized GAP-01 from local code reading without cross-checking the register the comment points to.
>
> **§XI vs §6.5 conflict.** Drift, not pause. Strategy §6.5 lines 598–614 lists a dependency-ordered queue: refresh coordination (П2, landed) → ProviderRegistry/versioning → multi-step accumulator → v1→v2 schema migration → trigger↔credential → WS events. The §XI day-by-day plan covers none of these — it is a hardening interleave inside `nebula-credential` and `engine/credential`, scoped to one senior. That is fine as a *parallel* track but cannot be advertised as the next-2-weeks path-of-record without disclosing that the §6.5 queue is paused.
>
> **«13 High до prod-релиза» framing.** Wrong axis. `nebula-credential-builtin` concrete sub-trait impls land in П3 (`docs/MATURITY.md:25`), no prod release is scheduled in 2–3 weeks, and `feedback_active_dev_mode.md` warns specifically against this kind of release-gating compression. Re-bucket: prod-blockers / П3-prerequisites / active-dev hardening / cleanup. Severity column then degrades cleanly.
>
> **What I would actually pick up next.** Five items: SEC-01, SEC-02, *SEC-04 (one-line `OsRng` swap fixing `RUSTSEC-2026-0097`)*, the bundled Arc<str> PR closing PERF-01+PERF-02+IDIOM-10 with a single architectural call, and SEC-03 standalone (AAD+`key_id`, envelope contract change, ADR-amendment, storage reverse-deps audit). Defer GAP-01, TEST-01/02, IDIOM-01 into П3 planning. Reject ARCH-02/03 as bugs — `test-util` exposing `InMemoryStore` parallel to the storage copy is the ADR-0032 §3 phantom-shim pattern executed deliberately, not a regression.
>
> **Aggregation §VII.C.** Mostly sound, one mandatory split: SEC-03 (AAD redesign, envelope contract, ADR amendment, storage reverse-deps verification) must NOT be bundled with SEC-04 (one-line `ThreadRng`→`OsRng`). Same `crypto.rs`, different threat models, different review depth, different rollback shape. Bundling them buries the AAD review under nonce-fix urgency.
>
> **True prod-blockers before П3 kickoff: 5 items** — SEC-01, SEC-02, SEC-04, the Arc<str> bundle (PERF-01+PERF-02+IDIOM-10 as one PR), and SEC-03 as a separate PR. Everything else in the audit's «13 High» list is either П3-prerequisite (GAP-01, TEST-01/02, IDIOM-01) or intentional-by-design (ARCH-02/03).

> **Errata editor's note (§XII.G).** tech-lead's «5 items» retains SEC-04 as a prod-blocker. Per §XII.G conflict resolution, **security-lead's direct verification overrides** — SEC-04 is a doc edit, not an `OsRng` swap. Tech-lead's framing in §XII.H.4 reflects the audit's claim, not post-verification reality. Use **§XII.E** for the post-resolution prod-blocker list.

---

**Errata complete.** Original §I–§XI text unchanged; this section supersedes any classification or priority conflict. Next-up artefact: security-hardening sub-spec (`docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md`) drafting per the §XII.E prod-blocker list.
