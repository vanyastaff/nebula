# Nebula Deep Review — 2026-04-14

Глубокий code review всего workspace в стиле CodeRabbit / Greptile / Cursor BugBot / Qodo / Cubic.
Параллельная работа 5 агентов по слоям + отдельный аудит context drift.

- **Baseline:** `cargo check --workspace` и `cargo clippy --workspace --all-targets -- -D warnings` чистые. Тесты проходят.
- **Философия:** только реальные баги и дизайн-дыры. Никаких style-нитов. Confidence ≥ MEDIUM. Пропущено всё, что уже ловит clippy.
- **Предыдущий аудит:** `.project/audit/audit-2026-04-13.md` — поверхностный (0 bugs), заменён этим.

## Executive summary

| Слой | Crates | Findings | CRITICAL | HIGH | MEDIUM | LOW | INFO |
|------|--------|----------|----------|------|--------|-----|------|
| Cross-cutting | 8 | 15 | 0 | 1 | 9 | 5 | 0 |
| Core          | 6 | 18 | 1 | 5 | 8 | 4 | 0 |
| Business      | 4 | 26 | 3 | 6 | 9 | 5 | 3 |
| Exec / API    | 7 | 34 | 0 | 6 | 16 | 11 | 1 |
| Context drift | —  | ~30| — | — | — | — | — |
| **Всего**     |   | **93**+ctx | **4** | **18** | **42** | **25** | **4** |

## TOP-10 must-fix (по приоритету)

1. **[BL-C4-01] CRITICAL** — OAuth2 PKCE сломан: `code_challenge` не добавляется в auth URL, `code_verifier` не шлётся в token exchange. `credential/src/credentials/oauth2_flow.rs:39+319`.
2. **[BL-C4-02] CRITICAL** — OAuth2 без `state` (CSRF) и без `redirect_uri`. Account takeover + несовместимо со strict-providers. Та же область.
3. **[BL-C4-03] CRITICAL** — `PendingStoreMemory::consume` удаляет запись *до* валидации owner/kind/session. Любой ложный запрос уничтожает pending state. `credential/src/pending_store_memory.rs:107`.
4. **[CR-C1-01] CRITICAL** — Histogram sum в `nebula-telemetry` отравляется одним NaN навсегда. `telemetry/src/metrics.rs:216`.
5. **[CO-C1-01] CRITICAL** — DoS-бюджет evaluation в `nebula-expression` обходится лямбдами: `eval_lambda` и `reduce` сбрасывают счётчик шагов. `expression/src/eval.rs:831, 1137`.
6. **[EX-C1-01] HIGH** — `POST /api/v1/workflows/:id/execute` и `start_execution` зовут `repo.transition(id, 0, …)` вместо `create()`. Возвращают 500 всегда. `api/src/handlers/{execution,workflow}.rs`.
7. **[EX-C4-01] HIGH** — `EngineCredentialAccessor` всегда инициализируется пустым allowlist, а пустой set = allow all. Любой action читает любой credential. `engine/src/engine.rs:1149`.
8. **[CO-C1-05] HIGH** — `SecretString::Deserialize` принимает литерал `"[REDACTED]"`. Round-trip через serde молча превращает секрет в строку `[REDACTED]`. `core/src/secret_string.rs:127`.
9. **[CO-C1-03] HIGH** — `ReplayPlan::pinned_outputs` помечен `#[serde(skip)]`. Persisted plan при replay перевыполняет всё, дублируя side-effects. `execution/src/replay.rs:25`.
10. **[BL-C4-09] HIGH** — AES-GCM nonce 64-бит counter + 32-бит random. После restart только 32 бита против collision → birthday bound ~65k. Плюс nonce reuse = полная потеря confidentiality + forgery. `credential/src/crypto.rs:144`.

---

## Часть 1 — Cross-cutting layer (log, system, eventbus, telemetry, metrics, config, resilience, error)

### [CR-C1-01] CRITICAL — `crates/telemetry/src/metrics.rs:216`
Confidence: HIGH. Histogram `observe(NaN)` отравляет `sum_bits` навсегда через CAS-loop (`old + NaN = NaN`), все последующие `sum()`/percentile возвращают NaN. Один stray NaN = сломанный `/metrics` до перезапуска.
**Fix:** `if !value.is_finite() { return; }` в начале `observe`.

### [CR-C1-02] MEDIUM — `crates/resilience/src/retry.rs:390`
Счётчик попыток возвращается как `config.max_attempts` при выходе по total_budget, хотя реально попыток было меньше. Метрики врут.

### [CR-C1-03] MEDIUM — `crates/config/src/loaders/file.rs:230`
`load_directory`: файлы с non-UTF-8 именами теряются молча; два файла с одним stem (`db.yaml` + `db.json`) переписывают друг друга в порядке `read_dir`.

### [CR-C1-04] MEDIUM — `crates/config/src/watchers/polling.rs:104`
Watcher сравнивает только mtime+size. На FAT/ext3 (1-2s resolution) быстрые правки невидимы. Поле `hash: Option<String>` существует, но никогда не заполняется (см. CR-C8-01).

### [CR-C1-05] MEDIUM — `crates/telemetry/src/metrics.rs:282`
`bucket_counts` для boundary меньше первого bucket'а тихо возвращает cumulative первого bucket'а — over-reports без сигнала.

### [CR-C2-01] MEDIUM — `crates/config/src/watchers/polling.rs:300`
Non-atomic guard: load `watching` → spawn task → store true. Два concurrent `start_watching` могут оба пройти проверку и заспаунить два таска. Fix: `compare_exchange(false, true)`.

### [CR-C2-02] MEDIUM — `crates/config/src/watchers/polling.rs:354`
`stop_watching` на timeout дропает `JoinHandle` без `.abort()` — таск продолжает работать.

### [CR-C3-01] MEDIUM — `crates/eventbus/src/bus.rs:96`
`publish_drop_oldest`: `broadcast::send` возвращает `Ok`, даже когда lagging subscribers видят `Lagged`. `EventBusStats::dropped_count` всегда 0 под default policy. Главный observability signal шины — ложь.

### [CR-C3-02] LOW — `crates/config/src/interpolation.rs:151`
Default value из паттерна `${DB_PASSWORD:-dev_password}` логируется WARN'ом целиком. Слабый но реальный credential leak.

### [CR-C4-01] MEDIUM — `crates/config/src/loaders/file.rs:126`
`FileLoader::load` читает конфиг через `read_to_string` без size cap. Symlink на `/dev/urandom` или 50 GB YAML = OOM до того как логирование поднимется.

### [CR-C6-01] MEDIUM — `crates/error/src/error.rs:274`
`Display` для `NebulaError<E>`: когда задан `with_message`, печатается **только** override. `context_chain` теряется. Весь `{err}` debugging trail пропадает.

### [CR-C6-02] LOW — `crates/resilience/src/bulkhead.rs:150`
Queue timeout возвращает `CallError::Timeout` вместо `BulkheadFull`. Не отличимо от upstream timeout — брикий workaround через сравнение Duration.

### [CR-C7-01] LOW — `crates/eventbus/src/bus.rs:160`
`emit_blocking` busy-polls с exponential backoff 50µs→1ms. Под sustained back-pressure = 1000 wake-ups/s на emitter. Fix: `tokio::sync::Notify` от receiver side.

### [CR-C8-01] LOW — `crates/config/src/watchers/polling.rs:42`
Поле `FileMetadata::hash` никогда не заполняется. Dead field. Либо wire up hashing (решает CR-C1-04), либо удалить.

### [CR-C9-01] LOW — `crates/eventbus/src/bus.rs:80`
Doc говорит про `Block` policy через `emit_awaited`, но `publish_drop_oldest` мапает Block в тот же путь — простой `emit()` под Block ведёт себя как DropOldest без разницы.

**Clean crates:** nebula-system, nebula-metrics, nebula-log.

---

## Часть 2 — Core layer (core, validator, parameter, expression, workflow, execution)

### [CO-C1-01] CRITICAL — `crates/expression/src/eval.rs:831` + `:1137`
`eval_lambda` и `reduce` делегируют в `self.eval()`, который безусловно делает `self.steps.store(0, ...)` на строке 73. Каждый `filter`/`map`/`reduce`/`find`/`group_by`/`flat_map` сбрасывает DoS-бюджет. Вредоносный `map(range, x => expensive)` = unbounded CPU.
**Fix:** `eval_lambda` должна звать `eval_with_depth` с threading depth, без reset счётчика. Reset только в outermost public entry.

### [CO-C1-02] HIGH — `crates/expression/src/eval.rs:122` (Negate)
`number_as_i64` кастует floats (`f as i64`). `-3.7` через i64-ветку даёт `-3`. Плюс `-(i64::MIN)` = overflow panic/wrap.
**Fix:** `n.is_i64() && !n.is_f64()` + `checked_neg`.

### [CO-C1-03] HIGH — `crates/execution/src/replay.rs:25`
`pinned_outputs: HashMap<NodeId, serde_json::Value>` помечен `#[serde(skip)]`. Serialized plan теряет pinned data. После reload replay перевыполняет весь workflow → дубликаты side-effects (emails, charges).

### [CO-C1-04] HIGH — `crates/execution/src/replay.rs:58`
`partition_nodes` вычисляет `rerun = all_nodes \ pinned`. Параллельные ветки без зависимости с `replay_from` попадают в rerun. Sibling branches перевыполняются.
**Fix:** три набора — ancestors (pinned), target+descendants (rerun), unrelated (reuse). Нужен successors map.

### [CO-C1-05] HIGH — `crates/core/src/secret_string.rs:127`
`Serialize` пишет `"[REDACTED]"`, `Deserialize` принимает любую строку. Round-trip любой структуры с bare `SecretString` превращает секрет в литерал `[REDACTED]`. Молчаливая порча credentials.
**Fix:** `Deserialize` должен либо отвергать `"[REDACTED]"`, либо требовать `#[serde(with = "serde_secret")]` явно (удалить manual Serialize).

### [CO-C1-06] HIGH — `crates/execution/src/state.rs:159`
`set_node_state` пропускает transition validation и version bump. Documented invariant "version bumped on each change" нарушается. Optimistic concurrency принимает stale writes.

### [CO-C1-07] MEDIUM — `crates/workflow/src/graph.rs:28`
Дубликат `NodeId` в definition: `add_node` создаёт два `NodeIndex`, но `index_map.insert` перезаписывает → первый нод orphan. `ExecutionPlan::from_workflow` не зовёт `validate_workflow`, так что проходит тихо.

### [CO-C1-08] MEDIUM — `crates/expression/src/parser.rs:112`
Depth-check живёт только в `parse_expression_with_depth`. `parse_binary_op_with_depth` рекурсирует на `a**b**c**…` без проверки. `MAX_PARSER_DEPTH = 256` не enforced на hot-path.
**Fix:** проверка в начале каждой `parse_*_with_depth` функции.

### [CO-C1-09] MEDIUM — `crates/expression/src/eval.rs:301` (arithmetic)
```rust
li.checked_add(ri).map(...).or_else(|| Some(json!(li as f64 + ri as f64)))
  .ok_or_else(|| ExpressionError::expression_eval_error("Arithmetic overflow"))
```
`.or_else(|| Some(...))` всегда Some → `.ok_or_else` — dead code. Overflow молча промотится в f64 с потерей точности (`i64::MAX + 1` даёт `9.223372036854776e18`).

### [CO-C1-10] MEDIUM — `crates/expression/src/eval.rs:1094` (reduce)
Каждая итерация `reduce` клонирует весь `EvaluationContext`. 10k-element reduce = 10k × ctx-size аллокаций. Плюс reset step budget (CO-C1-01).

### [CO-C1-11] LOW — `crates/expression/src/eval.rs:402` (divide)
Проверка `rf == 0.0` ловит 0, но не NaN. `x / NaN = NaN`, затем `serde_json::json!(NaN) → Value::Null` молча. Юзер видит `1/NaN → null` вместо ошибки.

### [CO-C2-01] MEDIUM — `crates/expression/src/eval.rs:1128`
`Evaluator::steps: AtomicUsize` thread-unsafe reset: `Arc<Evaluator>` расшаренный между задачами → concurrent `eval()` сбрасывают счётчик друг другу. Step budget best-effort only.

### [CO-C3-01] MEDIUM — `crates/execution/src/idempotency.rs:33`
`IdempotencyManager { seen: HashSet<String> }` — ни eviction, ни TTL, ни bound. Ключ `"{exec_id}:{node_id}:{attempt}"` включает `execution_id` → нет естественного upper bound. Slow memory leak.

### [CO-C5-01] LOW — `crates/execution/src/context.rs:53`
`with_max_concurrent_nodes(0)` silently принимается; semaphore(0) = deadlock. Field `usize` (не `Option`), inconsistent с другими budget полями.

### [CO-C6-01] MEDIUM — `crates/execution/src/transition.rs:9`
Из `Paused` можно только в `Running`/`Cancelling`. Нельзя в `TimedOut`/`Failed`/`Cancelled`. Wall-clock deadline на paused execution требует unpause → race.

### [CO-C6-02] MEDIUM — `crates/validator/src/rule.rs:696` (`validate_value`)
Каждое rule тихо passes на type mismatch: `MinLength(5).validate_value(json!(42)) = Ok(())`, `OneOf(["a","b"]).validate_value(json!(42)) = Ok(())`. Schema `{"type":"string","minLength":5}` принимает число. False-accept паттерн.
**Design discussion:** ввести `validate_value_strict` который erroрит на type mismatch, оставить текущий как `_coerce`.

### [CO-C7-01] LOW — `crates/workflow/src/graph.rs:65` (`compute_levels`)
`remaining.retain` + `filter` = O(n²). На тысячи нодов — слишком. Use Kahn's с `VecDeque`.

### [CO-C8-01] LOW — `crates/execution/src/plan.rs:4`
Stale TODO: `// TODO: ExecutionBudget is currently unavailable` — крейт использует `crate::context::ExecutionBudget`. Удалить комментарий.

---

## Часть 3 — Business layer (credential, resource, action, plugin)

### [BL-C4-01] CRITICAL — `crates/credential/src/credentials/oauth2_flow.rs:39`
`build_auth_url` анонсирует `code_challenge_method=S256`, но не добавляет `code_challenge`. Verifier из `crypto::generate_pkce_verifier()` не thread-ится в запрос, не persisted в `OAuth2Pending`, не шлётся как `code_verifier` в `exchange_authorization_code`. PKCE симулируется.
**Impact:** Authorization-code interception attacks не предотвращены.

### [BL-C4-02] CRITICAL — `crates/credential/src/credentials/oauth2_flow.rs:39`
Нет CSRF `state` в auth URL. Нет `redirect_uri`. `crypto::generate_random_state()` существует, но не зовётся. Google/Microsoft/GitHub strict отклонят; lax silently примут.
**Impact:** OAuth2 CSRF → account takeover.

### [BL-C4-03] CRITICAL — `crates/credential/src/pending_store_memory.rs:107`
`consume` удаляет entry **до** валидации `credential_kind`/`owner_id`/`session_id`. Любой wrong-owner запрос уничтожает pending state. Один shot DoS.
**Fix:** `get` → validate → `remove` только на success.

### [BL-C4-04] HIGH — `crates/credential/src/pending_store_memory.rs:125`
`String::eq` на validation chain — early return ladder + short-circuit = timing oracle на owner_id/session_id.
**Fix:** `subtle::ConstantTimeEq`, один generic error.

### [BL-C4-05] HIGH — `crates/credential/src/credentials/oauth2.rs:125`
Derived `Debug` на `OAuth2Pending` печатает `device_code: Some("...")` verbatim. Любой `tracing::debug!(?pending)` = leak device-flow bearer.

### [BL-C4-06] HIGH — `crates/credential/src/pending.rs:72`
`PendingToken(String)`: `Display` редактит, но derived `Debug` печатает underlying base64 token. OAuth2 callback hijack через log leak.
**Fix:** manual `Debug` зеркалирующий Display.

### [BL-C4-07] HIGH — `crates/credential/src/credentials/oauth2_flow.rs:373`
`parse_token_response` интерполирует весь response body в error message. Некоторые провайдеры эхоят client_id/client_secret или refresh tokens в non-2xx envelopes. Утечка в `CredentialError::Provider` → audit sink → logs. Audit layer contract ("never log credential data") нарушен.

### [BL-C2-08] HIGH — `crates/credential/src/resolver.rs:158`
Когда circuit breaker open, resolver возвращает expired state без refresh и без ошибки. `CredentialHandle` выглядит валидным, но гарантированно падает на remote API. Скрывает истинную причину.

### [BL-C4-09] HIGH — `crates/credential/src/crypto.rs:144`
AES-GCM nonce: 64-bit atomic counter (reset на restart) + 32-bit random. После restart collision protection = 32 бита → birthday ~65k. NIST SP 800-38D §8.2 violation. Nonce reuse в GCM = full plaintext recovery + forgery.
**Fix:** полностью random 96-bit nonce per encryption.

### [BL-C4-10] MEDIUM — `crates/credential/src/layer/encryption.rs:67`
Single-key constructor регистрирует empty-string `key_id` alias → legacy records с `key_id: ""` молча декриптятся текущим ключом. `encrypt()` тоже продуцирует envelopes с `key_id: ""`. Migration story "no legacy fallback" сломана.

### [BL-C1-11] MEDIUM — `crates/plugin/src/plugin_type.rs:31`
`PluginType::Versions(PluginVersions::new())` — пустой — compileable извне. `key()` → `.expect("non-empty...")` = panic в library code на reachable input.
**Fix:** либо private field, либо `key() -> Option<&PluginKey>`.

### [BL-C4-12] MEDIUM — `crates/credential/src/credentials/oauth2.rs:145`
`Zeroize for OAuth2Pending`: `client_secret = SecretString::new("")` (replace, не in-place zeroize), `client_id` / `interval` / `config` вообще игнорированы. Heap dump после drop содержит auth_url, token_url, client_id.

### [BL-C7-13] MEDIUM — `crates/credential/src/credentials/oauth2_flow.rs:319`
`refresh_token` materializes secrets через `expose_secret(|s| s.to_owned())` в raw `String`, которые живут всю HTTP-транзакцию без zeroize. Defeats `SecretString` contract.

### [BL-C9-14] MEDIUM — `crates/action/src/result.rs:160`
`ActionResult::Terminate` documented как "not yet wired — behaves as Skip", но `ControlOutcome::Terminate` и `ControlAction` adapter поощряют автора вернуть его. Silent footgun.
**Fix:** либо wire up engine cancellation, либо gate за feature flag, либо warn в `ControlActionAdapter::execute`.

### [BL-C2-15] MEDIUM — `crates/credential/src/refresh.rs:152`
`circuit_breakers: HashMap<String, Arc<CircuitBreaker>>` растёт unbounded для каждого distinct `credential_id`. Attacker-shapable IDs (webhook payload → credential_id) = slow-burn memory leak.

### [BL-C5-16] MEDIUM — `crates/credential/src/crypto.rs:125`
`Drop for EncryptedData` зироет `ciphertext`/`nonce`/`tag` — но это public ciphertext bytes, уже на диске. Security theater + performance overhead на hot path. Только plaintext (уже в `Zeroizing<Vec<u8>>`) нужен protection.

### [BL-C6-17] MEDIUM — `crates/credential/src/layer/encryption.rs:131`
Lazy re-encryption path делает CAS write, но возвращает `StoredCredential` со **старым** `version`. Caller со стареньким version → phantom CAS conflicts.

### [BL-C4-18] LOW — `crates/credential/src/credentials/oauth2_config.rs:61`
`OAuth2Config` derives `Debug`. Сегодня не секрет, но любое новое поле (client_assertion и т.п.) автоматом leak.

### [BL-C8-19] LOW — `crates/credential/src/credentials/oauth2.rs:407`
`refresh` reconstructs config через `OAuth2Config::client_credentials()` — вводит в заблуждение (refresh это отдельный grant type). Ломается в момент когда `refresh_token` зависит от `pkce`/`redirect_uri`.

### [BL-C1-20] LOW — `crates/credential/src/credentials/oauth2.rs:418`
`secs as i64` касты для `expires_in` (u64 из JSON) — overflow для гигантских значений. `i64::try_from(secs).unwrap_or(i64::MAX)`.

### [BL-C2-21] LOW — `crates/credential/src/refresh.rs:211`
`acquire_permit`: `.expect("refresh semaphore closed")` — unconditional panic на reachable path. Replace with error propagation.

### [BL-C10-22] LOW — `crates/credential/src/credentials/oauth2.rs:728`
Тест `zeroize()` проверяет только `client_secret` и `device_code`. Не покрывает `config`, `client_id`, `interval` — tautological test, дыра в BL-C4-12.

### [BL-C6-23] INFO — pattern
Каждый scheme (`secret_token`, `identity_password`, `connection_uri`, `certificate`) руками пишет redacted `Debug`. Convention fragile — новый scheme author легко забудет.
**Suggest:** `#[derive(RedactedDebug)]` proc-macro с `#[secret]` attribute, или lint запрещающий `derive(Debug)` на типах с `SecretString`.

### [BL-C8-24] INFO — `crates/credential/src/lib.rs:31`
`#![deny(unsafe_code)] + #![forbid(unsafe_code)]`. Forbid implies deny — deny redundant.

---

## Часть 4 — Exec / API layer (engine, runtime, storage, api, sdk, sandbox, plugin-sdk)

### [EX-C1-01] HIGH — `crates/api/src/handlers/execution.rs:238` + `workflow.rs:478`
`start_execution` и `execute_workflow` зовут `repo.transition(execution_id, 0, state)` чтобы создать новую execution. `transition` — это CAS UPDATE по `id AND version = $3`. Для нового ID row не существует → `rows_affected = 0` → `Ok(false)` → `Internal("Failed to create execution record")`. **HTTP execution-start endpoints возвращают 500 всегда.**
**Fix:** `repo.create(execution_id, workflow_id, state).await?` — метод уже есть, engine его зовёт корректно.

### [EX-C2-01] HIGH — `crates/runtime/src/stream_backpressure.rs:56`
`tokio::sync::Notify` lost-wakeup race: `notify_one` вызывается из `pop` **до** того как `push` дойдёт до `notified().await`. `Notify::notify_one` сохраняет permit только если есть waiter → waiter не зарегистрирован до `.await`. Тот же shape в `pop` для `not_empty`.
**Fix:** enable pattern —
```rust
let notified = self.inner.not_full.notified();
tokio::pin!(notified);
notified.as_mut().enable();
drop(queue);
notified.await;
```

### [EX-C2-02] MEDIUM — `crates/runtime/src/queue.rs:123`
`dequeue` держит receiver `Mutex` через `tokio::time::timeout(timeout, rx.recv())`. N workers = effectively single-consumer. Throughput collapse до 1/timeout.

### [EX-C4-01] HIGH — `crates/engine/src/credential_accessor.rs:119` + `engine.rs:1149`
`EngineCredentialAccessor::is_allowed`: `allowed_keys.is_empty() || contains(id)`. Engine всегда передаёт `HashSet::new()`. **Empty = allow all.** Любой action читает любой credential по ID, обходя per-action declarations.
**Fix:** invert empty semantics → deny, сделать allowlist mandatory.

### [EX-C2-03] HIGH — `crates/sandbox/src/process.rs:223` (`try_dispatch`)
`dispatch_envelope` не observe'ит `CancellationToken`. После отправки envelope блокируется на `self.timeout` даже если workflow cancelled. `SandboxedContext` не select!'ится.
**Fix:** race round-trip против `context.cancellation.cancelled()`, surface `ActionError::Cancelled`, invalidate handle.

### [EX-C2-04] MEDIUM — `crates/sandbox/src/process.rs:223`
Sandbox держит `self.handle` (`tokio::sync::Mutex`) через весь round-trip. Slow plugin = H-O-L blocking. Slice 1d должен решить.

### [EX-C2-05] HIGH — `crates/api/src/webhook/ratelimit.rs:103`
Race: два request'а на новый path оба проходят fast `windows.get(path)` miss, оба CAS'ают `path_count`, но только один insert'ит в `windows` (другой в no-op ветке `or_insert_with`). `path_count` overcounts → в итоге пересекает `max_paths` и per-path limiting молча отключается.
**Fix:** bump `path_count` только в `Vacant` arm.

### [EX-C4-02] MEDIUM — `crates/api/src/webhook/transport.rs:266`
Webhook handler проверяет `body_limit_bytes` **после** того как axum уже буферизировал `Bytes`. Default limit 2 MiB скрывает любое большее значение; меньшие всё равно буферизируют до 2 MiB до reject.
**Fix:** `axum::extract::DefaultBodyLimit::max(body_limit_bytes)` на webhook router.

### [EX-C5-01] MEDIUM — `crates/engine/src/engine.rs:53`
`type EventSender = mpsc::UnboundedSender<ExecutionEvent>`. `emit_event` = `let _ = sender.send(event);`. Slow consumer = unbounded RAM growth. 10k-node workflow = ~50k messages.
**Fix:** bounded channel с drop-on-full counter.

### [EX-C5-02] MEDIUM — `crates/sandbox/src/process.rs:319`
`tokio::spawn(drain_plugin_stderr)` — `JoinHandle` dropped. `kill_on_drop` ловит child, но dangling drain task выживает до закрытия pipe FD kernel'ом. Под plugin churn = leak task handles.
**Fix:** store handle в `PluginHandle`, abort on drop.

### [EX-C2-06] MEDIUM — `crates/sandbox/src/process.rs:213`
`dispatch_envelope` retry'ит **любую** ошибку, включая non-idempotent invocations. Plugin выполнил side effect, host failed на read → respawn + re-run = at-least-twice semantics без idempotency token в envelope.

### [EX-C9-01] LOW — `crates/runtime/src/runtime.rs:235`
Doc всё ещё ссылается на gRPC для sandbox protocol. gRPC был dropped (`b607ac39`), теперь UDS+JSON duplex v2.

### [EX-C9-02] LOW — `crates/runtime/src/runtime.rs:281`
"Phase 1 broker" terminology из dropped gRPC plan. Update на slice 1d/1e plan.

### [EX-C2-07] MEDIUM — `crates/sandbox/src/discovery.rs:89`
Sync `std::fs::read_dir` + `entry.path().metadata()` из `async fn`. Блокирует executor thread во время plugin discovery.
**Fix:** `tokio::fs::read_dir` или `spawn_blocking`.

### [EX-C4-03] MEDIUM — `crates/sandbox/src/capabilities.rs:194`
Capability path check: `canonicalize` + fallback на lexical `normalize_lex` когда any side fails. Lexical не резолвит symlinks → attacker с write access к подкаталогу может положить symlink на `/etc`, и если target не существует ещё, lexical проверка пропустит.
**Fix:** canonicalize deepest existing ancestor, reject если parent не canonicalize'ится.

### [EX-C3-01] MEDIUM — `crates/api/src/middleware/auth.rs:64`
API key prefix gate (`starts_with(API_KEY_PREFIX)`) — early return до constant-time compare → timing oracle на существование prefix'а. Leaks 7 байт (`nbl_sk_`).
**Fix:** убрать prefix gate, либо run constant-time compare независимо.

### [EX-C8-01] LOW — `crates/api/src/middleware/rate_limit.rs:1`
Модуль существует, экспортируется, но **пустой**. Нет per-IP rate limiting на `/api/v1/*`.
**Fix:** удалить placeholder либо wire `tower_governor`.

### [EX-C5-03] MEDIUM — `crates/api/src/handlers/workflow.rs:184`
`SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64` в 6 handlers. Panic если system clock < 1970.
**Fix:** `chrono::Utc::now().timestamp()`.

### [EX-C5-04] LOW — `crates/api/src/handlers/workflow.rs:53`
`list_workflows`: `total = workflow_responses.len()` — размер страницы, не total collection. Pagination UIs over-report progress.
**Fix:** добавить `WorkflowRepo::count`, как в execution repo.

### [EX-C1-02] MEDIUM — `crates/engine/src/engine.rs:649` (`resume_execution`)
Прямое field assignment (`ns.state = NodeState::Pending`) bypasses `transition_to` invariants. Не поднимает `attempt_count` для previously-Running нод. Combined с EX-C1-03 → resume re-использует stale output.

### [EX-C1-03] MEDIUM — `crates/engine/src/engine.rs:1238`
`let idem_key = format!("{execution_id}:{node_id}:1");` — `:1` захардкожен. Должен быть `attempt_count`. Все retries коллапсируют в один ключ.

### [EX-C7-01] MEDIUM — `crates/engine/src/resolver.rs:46`
`for entry in outputs.iter()`: deep-clone всех prior outputs в per-node `EvaluationContext`. O(n²) на длинных workflow с большими payloads.
**Fix:** `Arc<serde_json::Value>`, cloning = refcount bump.

### [EX-C3-02] LOW — `crates/sandbox/src/process.rs:159`
`const ONE_SHOT_ID: u64 = 1` — каждое envelope использует correlation id `1`. Stale response от timed-out call может матчиться против следующего request'а. Cross-talk между workflows.

### [EX-C5-05] LOW — `crates/api/src/extractors/json_extractor.rs:24`
`ValidatedJson` полагается на axum default `DefaultBodyLimit` (2 MiB) globally. Нет domain cap. Inconsistent с webhook.

### [EX-C8-02] LOW — `crates/api/src/state.rs`
Webhook router inherits global CORS allowing `Authorization`+`X-API-Key` from `Any` origin. Webhook не должен принимать эти headers.
**Fix:** CORS только на v1 nest.

### [EX-C9-03] INFO — `crates/storage/src/backend/postgres.rs:106`
`format!("SELECT value FROM {} WHERE key = $1", table)` — table name из config. Operator-controlled сегодня, но pattern fragile.
**Fix:** regex validation `^[A-Za-z_][A-Za-z0-9_]*$` на construction.

### [EX-C8-03] LOW — `crates/api/src/handlers/execution.rs:74`
`list_executions(workflow_id)` игнорирует `workflow_id` — зовёт `list_running()` без фильтра, возвращает cross-tenant execution IDs. Information disclosure.

### [EX-C7-02] LOW — `crates/sandbox/src/process.rs:266`
`std::env::var` из `async fn` — `getenv`/`setenv` не thread-safe в glibc. Race с любым `set_var` = UB.
**Fix:** snapshot env в `Arc<HashMap>` at startup.

### [EX-C1-04] LOW — `crates/api/src/handlers/workflow.rs:266`
`update_workflow` не может clear description — `None` = "skip". REST `PUT` semantics broken.

### [EX-C1-05] MEDIUM — `crates/sandbox/src/process.rs:213` + `plugin-sdk/transport.rs:151`
`dial` принимает handshake line от child process blindly. Compromised plugin binary может print `NEBULA-PROTO-2|unix|/run/other-plugin/sock` → cross-plugin pivot.
**Fix:** host-generated per-plugin dir, env-passed, reject paths outside.

### [EX-C2-09] LOW — `crates/plugin-sdk/src/transport.rs:62`
TOCTOU: `remove_dir_all` → `create_dir` → `set_permissions`. Attacker на том же UID может race. PID predictable.
**Fix:** `tempfile::TempDir::new_in` (atomic mkdtemp).

### [EX-C5-06] LOW — `crates/api/src/webhook/transport.rs:222`
`generate_nonce` — `format!` per byte loop. Aesthetic. Also `Uuid::new_v4` CSPRNG depends on feature flags.

---

## Часть 5 — Context drift (`.project/context/**` vs реальность)

### Крупные противоречия
1. **macros.md** документирует несуществующий `nebula-macros` crate. Фактически 9 per-parent macro sub-crates. Удалить или разнести.
2. **PostgresStorage status** — `pitfalls.md L12` ("not implemented"), `active-work.md L11` ("memory only"), `decisions.md L31` ("PostgreSQL for production"), `storage.md` (есть Pg backend). Четыре версии. Код: `crates/storage/src/backend/postgres.rs` + `pg_execution.rs` существуют (sqlx + migration), но не wired в runtime/engine. Согласовать.
3. **`webhook` как crate** — `CLAUDE.md L18` и `ROOT.md L13` говорят `webhook` в API layer. Такого крейта нет: module `crates/api/src/webhook/`. Удалить из диаграмм.
4. **`memory` crate в core layer** — `CLAUDE.md L24` всё ещё упоминает `memory`. `nebula-memory` удалён (pitfalls.md L15). Удалить.
5. **"enforced by cargo deny"** — `CLAUDE.md L15` утверждает, `pitfalls.md L16` опровергает. `[bans.deny]=[]` — convention only.
6. **runtime.md L20** — упоминает gRPC. Дропнут за UDS+JSON (`b607ac39`).
7. **system.md L31** — "Used by nebula-memory". Memory удалён.
8. **resource.md L38** — "Depended on by nebula-webhook". Не существует.
9. **CLAUDE.md L9** — "25-crate workspace". Реально 30+ workspace members (25 main crates + 9+ macro sub-crates). Уточнить формулировку.

### Token budget overflows (7 файлов)
| File | Current | Budget | Over |
|------|---------|--------|------|
| crates/action.md | 5313 | 1500 | +3813 |
| decisions.md | 2080 | 900 | +1180 |
| crates/execution.md | 824 | 500 | +324 |
| crates/api.md | 813 | 500 | +313 |
| active-work.md | 556 | 200 | +356 |
| crates/engine.md | 563 | 500 | +63 |
| crates/sandbox.md | 508 | 500 | +8 |

### Мелкая drift
- `storage.md L22` — ссылка на migration `00000000000007`, реально только `00000000000001`.
- `credential.md L7-L8` — завышенный список re-exports из `nebula-action` (только `CredentialGuard` реально re-exported).
- `decisions.md L36` vs `api.md L3` — WebSocket: planned vs implemented.
- `apps/desktop` — не workspace member, Cargo project живёт в `apps/desktop/src-tauri/`. Уточнить в ROOT.md/CLAUDE.md.
- Missing: `sdk/macros-support` — нет context file, нет mention.
- 6 missing macro-crate context files (либо формально exempt в convention).

---

## Applied auto-fixes

Ниже в файле `### Fix log` — будет обновлено после применения. Фиксы группами, один git-friendly блок за раз. Фиксы которые требуют обсуждения помечены **[SKIP-DISCUSS]** и оставлены нетронутыми.

### Fix log (applied in this session)

**Verification:** `cargo clippy --workspace --all-targets -- -D warnings` clean ·
`cargo nextest run --workspace` → 3271 tests, 0 failures, 13 skipped.

#### Cross-cutting layer
- **CR-C1-01** ✅ `telemetry/src/metrics.rs:216` — early-return on `!value.is_finite()` to stop NaN from poisoning `sum_bits` via the CAS loop.
- **CR-C1-02** ✅ `resilience/src/retry.rs:354` — track `attempts_executed` locally; `CallError::RetriesExhausted::attempts` now reflects the actual loop count on the total-budget break path.
- **CR-C2-01** ✅ `config/src/watchers/polling.rs:300` — replaced load/store with `compare_exchange(false, true)` to close the double-spawn race.
- **CR-C2-02** ✅ `config/src/watchers/polling.rs:354` — capture `abort_handle()` before the timeout race; explicitly `abort()` the task if it does not exit within the grace window.
- **CR-C3-02** ✅ `config/src/interpolation.rs:155` — dropped default value from the WARN log (defaults routinely hold fallback secrets).
- **CR-C8-01** ✅ `config/src/watchers/polling.rs:42` — removed unused `hash: Option<String>` field.

#### Core layer
- **CO-C1-05** ✅ `core/src/secret_string.rs:127` — `Deserialize` now rejects the exact literal `"[REDACTED]"` sentinel so a default-serde round-trip no longer silently rewrites secrets to the redaction placeholder.
- **CO-C1-07** ✅ `workflow/src/graph.rs:28` — `DependencyGraph::from_definition` returns `WorkflowError::DuplicateNodeId` instead of silently orphaning the first of two duplicate nodes inside `petgraph`.
- **CO-C1-08** ✅ `expression/src/parser.rs:112, 173` — enforce `MAX_PARSER_DEPTH` at the top of `parse_binary_op_with_depth` and `parse_unary_with_depth`, closing the stack-overflow window for right-associative chains and long unary stacks.
- **CO-C1-09** ✅ `expression/src/eval.rs:301, 346, 376` — removed dead `ok_or_else("Arithmetic overflow")` on `add`/`subtract`/`multiply` (`.or_else(|| Some(...))` is always `Some`); behaviour-preserving `map_or_else` keeps the f64 fallback explicit.
- **CO-C1-11** ✅ `expression/src/eval.rs:402` — reject non-finite divisor and non-finite division result so `1 / NaN` surfaces as an error instead of `Value::Null`.
- **CO-C5-01** ✅ `execution/src/context.rs:53` — `with_max_concurrent_nodes(0)` now panics with a clear message (a zero semaphore would deadlock the scheduler silently).
- **CO-C8-01** ✅ `execution/src/plan.rs:4` — deleted the stale `// TODO: ExecutionBudget is currently unavailable` comment.

#### Business layer
- **BL-C4-03** ✅ `credential/src/pending_store_memory.rs:107` (**CRITICAL**) — `consume` now holds the write lock through the entire validation, removes the entry only after credential_kind + owner_id + session_id all match, and leaves the entry in place on validation failure. Failure path returns a single generic `ValidationFailed` reason to avoid leaking which dimension mismatched. Three previously-tautological tests were rewritten to also assert the legitimate caller can still consume the entry after a bad probe.
- **BL-C4-05** ✅ `credential/src/credentials/oauth2.rs:125` — `OAuth2Pending` now has a manual `Debug` impl that redacts `client_id`, `client_secret`, and `device_code`.
- **BL-C4-06** ✅ `credential/src/pending.rs:72` — `PendingToken` now has a manual `Debug` mirroring the existing redacted `Display`.
- **BL-C4-09** ✅ `credential/src/crypto.rs:144` (**HIGH**) — replaced the 64-bit counter + 32-bit random nonce scheme with a fully-random 96-bit nonce per encryption (`rand::rng().random::<[u8; 12]>()`). Closes the ~65k-restart birthday-collision window. Call sites in `encrypt`/`encrypt_with_aad`/`encrypt_with_key_id` updated; `NonceGenerator` struct + its tests removed in favour of a standalone `fresh_nonce()` free function.
- **BL-C4-12** ✅ `credential/src/credentials/oauth2.rs:145` — `Zeroize for OAuth2Pending` now scrubs `client_secret` in place before replacing it, scrubs `client_id`, and drops `device_code`/`interval` entirely. Test updated to cover the full wipe.
- **BL-C5-16** ✅ `credential/src/crypto.rs:125` — removed the `Drop for EncryptedData` that zeroized ciphertext/nonce/tag (public bytes by design; security theatre on the decrypt hot path).
- **BL-C8-24** ✅ `credential/src/lib.rs:31` — dropped redundant `#![deny(unsafe_code)]` (already implied by `#![forbid(unsafe_code)]`).

#### Exec / API layer
- **EX-C1-01** ✅ `api/src/handlers/execution.rs:238` + `workflow.rs:478` (**HIGH**) — `start_execution` and `execute_workflow` now call `ExecutionRepo::create(id, workflow_id, state)` instead of `transition(id, 0, state)`. `transition` is a CAS UPDATE keyed on `id AND version = $3` and could never match a brand-new row, so every HTTP call to these two endpoints previously returned a 500.
- **EX-C2-01** ✅ `runtime/src/stream_backpressure.rs:56, 93` (**HIGH**) — both `push` (`Overflow::Block`) and `pop` now register the `Notified` future via the `tokio::pin!` + `as_mut().enable()` pattern *before* releasing the queue lock, closing the lost-wakeup race against `notify_one`.
- **EX-C2-05** ✅ `api/src/webhook/ratelimit.rs:103` — `path_count` is now incremented only inside the `Vacant` arm of `DashMap::entry`, via a dedicated `try_reserve_slot` helper. Two concurrent first-time requests for the same path can no longer double-count and silently disable limiting once `max_paths` is crossed.
- **EX-C5-01** ✅ `engine/src/engine.rs:53, 283` — replaced `UnboundedSender<ExecutionEvent>` with bounded `mpsc::Sender` (default capacity `DEFAULT_EVENT_CHANNEL_CAPACITY = 1024`, now a re-exported constant). `emit_event` uses `try_send` and logs a warning on `Full`; slow consumers can no longer stall the engine or drive memory growth. `apps/cli/src/commands/run.rs` updated to pair with `mpsc::channel(DEFAULT_EVENT_CHANNEL_CAPACITY)`.
- **EX-C5-03** ✅ `api/src/handlers/workflow.rs` (×4) + `execution.rs` (×2) — replaced `SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64` with `chrono::Utc::now().timestamp()`, removing the panic path for clocks set before 1970.
- **EX-C2-07** ✅ `sandbox/src/discovery.rs:89` — switched `std::fs::read_dir` + sync iterator to `tokio::fs::read_dir` + async `next_entry().await`. Plugin discovery no longer blocks executor threads on networked filesystems.
- **EX-C9-01 / EX-C9-02** ✅ `runtime/src/runtime.rs:235, 281` — doc strings now reference UDS + JSON duplex v2 and sandbox slice 1d, dropping the stale gRPC / "Phase 1 broker (gRPC)" terminology that pre-dated commit `b607ac39`.

#### Context drift
- **CLAUDE.md** — project overview now reflects "25 main crates plus per-crate macro sub-crates"; layer diagram dropped the phantom `webhook` and `memory` entries; "enforced by cargo deny" downgraded to "convention only — not enforced by tooling" with a pointer to `pitfalls.md`.
- **runtime.md context** → `crates/runtime/src/runtime.rs` doc comments updated (see EX-C9-01 above).
- **system.md** — dropped the `nebula-memory` consumer (crate was removed); flagged the crate as "available for use" instead of pretending at a fictitious consumer.
- **resource.md** — replaced `nebula-webhook` consumer with `nebula-api` (webhook module), noting there is no `nebula-webhook` crate.

### Not auto-fixed (require human discussion or larger refactor)

High-impact findings that were left in the report for explicit review because the fix is not mechanical:

- **BL-C4-01 / BL-C4-02** (CRITICAL) — OAuth2 PKCE is missing `code_challenge` + `code_verifier`, and the flow has no `state` or `redirect_uri`. Protocol-level work across `build_auth_url`, `OAuth2Pending`, `exchange_authorization_code`, and the `OAuth2Config` parameter schema.
- ~~**CO-C1-01** (CRITICAL) — `eval_lambda` / `reduce` reset the DoS step counter by calling `self.eval` instead of `self.eval_with_depth`.~~ **FIXED** — closed via GitHub issue #252. Replaced `Evaluator::steps: AtomicUsize` with a stack-local `EvalFrame { depth, steps, max_steps }` threaded by `&mut` through every recursive path. `Evaluator::eval` is the sole place that constructs a frame; internal recursion uses `eval_with_frame`, so the budget is enforced across ALL nested lambda/reduce work. `eval_lambda` dropped to `pub(crate)`. 12 regression tests added incl. concurrent `Arc<Evaluator>` thread-safety. Follow-up tracked in memory `pitfall_expression_builtin_frame` — builtins still receive `&Evaluator` without the caller's frame.
- **CO-C1-03 / CO-C1-04** (HIGH) — `ReplayPlan::pinned_outputs` is `#[serde(skip)]` (silent replay data loss) and `partition_nodes` treats unrelated sibling branches as "rerun". Both are design-level decisions about replay semantics.
- **CO-C1-06** (HIGH) — `ExecutionState::set_node_state` bypasses `transition_to`/version bump; needs a typed API method on `ExecutionState`.
- **EX-C4-01** (HIGH) — `EngineCredentialAccessor` treats an empty allowlist as "allow all", and the engine always constructs one. Fail-closed fix requires plumbing the declared-credential set from `ActionMetadata` into every `spawn_node` call.
- **EX-C2-03 / EX-C2-06** (HIGH / MEDIUM) — sandbox `dispatch_envelope` ignores `CancellationToken` and retries all failures (at-least-twice for non-idempotent actions). Needs a `select!` cancel path and an idempotency token in the envelope schema.
- **EX-C1-05** (MEDIUM) — plugin `dial` trusts the child's handshake line, enabling cross-plugin socket hijack. Fix requires host-generated per-plugin directories passed in via env.
- **CO-C3-01** (MEDIUM) — `IdempotencyManager::seen` is unbounded. Requires an eviction policy (LRU / TTL) decision.
- **CR-C3-01** (MEDIUM) — `EventBus::publish_drop_oldest` silently under-reports drops. Fix touches the stats contract of the bus.
- **CR-C6-01** (MEDIUM) — `NebulaError::Display` drops the context chain on `with_message` override. User-visible change to every `{err}` consumer.
- **CO-C6-02** (MEDIUM) — `validator::Rule::validate_value` silently passes on type mismatch. Would introduce a new typed-rule path.
- **BL-C7-13** (MEDIUM) — refresh path materialises secrets via `expose_secret(|s| s.to_owned())`. Fix is a refactor that keeps `&str` borrows inside the closure scope.

All remaining CRITICAL and HIGH items above should be handled in dedicated PRs — each has architectural or security-contract implications that the auto-fix pass intentionally avoided.