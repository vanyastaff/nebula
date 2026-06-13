# Conference batch 2 (critics + n8n + temporal)


===== critic-arch =====
Достаточно. У меня механистическая картина: resolve/refresh/continue размазаны по `resolver.rs` (830), `ops.rs` (899/DispatchOps), `executor.rs`, `facade.rs` (1414); refresh-CAS в resolver + coordinator (1015); owner_id/scope в 13 файлах; oauth2.rs все еще монолит 1481. Атакую.

---

АТАКИ (механизм поломки):

A1 — «Один крейт, три контекста» не дает изоляции, которую обещает. Bounded context в Rust держит границу только через `mod` + `pub(crate)`. DESIGN §9 рисует Management→Runtime→TypeSystem стрелки, но `facade.rs` уже импортирует `runtime::{CredentialResolver, LeaseLifecycle}` И `store::StoredCredential` напрямую (строки 42-43), а `ops.rs` тянет `runtime::{execute_resolve, execute_continue, dispatch_revoke}`. Механизм поломки: ничто на уровне компилятора не запрещает Management звать в обход Runtime прямо в store/resolver. Через 3 PR «один helper в фасаде, который читает store напрямую» — и стрелка Management→Runtime становится Management↔Runtime↔Store клубком. Компилятор не поймает, потому что все `pub(crate)`. Это ровно та boundary-erosion, ради лечения которой затевался ADR-0092.

A2 — Потеря compile firewall (ADR-0092 сам это признает «Negative/accepted») недооценена в DESIGN. 15-20k LoC в одном крейте: правка `contract/credential.rs` (трейт) рекомпилирует resolver+facade+coordinator+oauth2+rotation = ~6k+ LoC горячего кода. При активной разработке Phase 1-4 это touch-trait-recompile-everything на каждой итерации. DESIGN §17 планирует 5 фаз ИТЕРАТИВНОГО переписывания контракта — то есть максимально бьет именно по той цене, которую крейт-слияние сделало максимальной. Crew будет ждать сборку на каждом шаге того самого рефактора, который трогает контракт чаще всего.

A3 — «CredentialRuntime один пайплайн заменяет 4 entry point» (D2) — на бумаге, в коде 4 точки живут в РАЗНЫХ структурах с разными дженерик-параметрами. `DispatchOps<B,PS>` (ops.rs) дженерик по pending-store; `CredentialResolver<S>` (resolver.rs) дженерик по store; `facade` non-generic erased; `executor` отдельно. Механизм: чтобы слить их в один `CredentialRuntime`, надо либо стереть `B`/`PS`/`S` в `Arc<dyn>` (что DispatchOps по комментарию делать НЕ может — «cannot fold into the non-generic Core registry because they are generic over the store/pending types»), либо протащить дженерики через единый пайплайн → опять «generic soup», та самая Pain 2 из ADR-0088. DESIGN не показывает, как `CredentialRuntime` глотает дженерик `PS` без возврата к `CredentialService<B,PS>`. Это нерешенное противоречие, а не дизайн.

A4 — Риск пересрастания Runtime+Management в «фасад-2.0». Facade.rs уже 1414 LoC и держит И CRUD (create/get/list/update/delete с owner_id) И runtime-делегацию (resolver, LeaseLifecycle, test, refresh). DESIGN §17 Phase 2 DoD = «facade < 400 LOC». Но D5 OAuth Plane B логика (state/pending/continue/refresh) тоже садится в credential, D6 reactive refresh coalescer+RefreshClaimRepo тоже. Механизм: «Management отделена от Runtime» внутри одного крейта без crate-firewall = социальная договоренность. Через rotation+lease+refresh+oauth-continue все сойдется в одну `CredentialService`-божество, потому что у нее уже есть `Arc<dyn>` на все порты и она единственная точка композиции. Получаем God-object вместо 4 entry points — хуже, не лучше: один объект на 2k LoC vs 4 явных по 800.

A5 — owner_id/tenant-isolation размазан по 13 файлам (scope.rs 29, facade.rs 41, binding.rs 8, executor.rs 4, context.rs 21…), а enforcement — «operation-level», НЕ типовой. facade.rs док прямо говорит: tenancy enforced «at the operation level (not via ScopeLayer)». Механизм утечки: каждый НОВЫЙ метод на Runtime/Management, который читает store, обязан вручную повторить `owner_id != scope → NotFound`. Это ровно «discipline-based, not structural» антипаттерн (feedback_type_enforce_not_discipline). Один забытый чек в новом `refresh_by_id`/`test_by_id` = cross-tenant чтение. ADR-0052 confused-deputy закрыт ТОЛЬКО для bind-path (`ValidatedCredentialBinding`), а runtime resolve/refresh/test — нет. 15-20k LoC одного крейта увеличивает число мест, где этот чек надо не забыть.

A6 — OAuth2 «не 1500-строчный тип» — но oauth2.rs ВСЁ ЕЩЕ 1481 строк, и DESIGN ставит цель Phase 3 «<500 LOC». Механизм провала: монолит держит 3 grant-типа (authz code / client creds / device code) + state + refresh + project + policy в одном `OAuth2State`/`OAuth2Credential`. «Protocol+config» расщепление требует, чтобы grant-логика стала data-driven, но device-code polling и authz-code PKCE-redirect — структурно РАЗНЫЕ control-flow, не конфиг. Запихивание их в один `OAuth2Protocol` + `enum GrantType` data = тот же монолит с `match` внутри, LoC не падает, он переезжает в `dispatchers`. Цель «<500» недостижима без выделения per-grant модулей — то есть code-per-grant, что противоречит D4 «один OAuth2Protocol».

A7 — DESIGN зависит от ADR-0084 reactive-only как от несущей стены, но сам ADR-0092 пишет «Open risk: если proactive refresh (1.1) потребует координации с engine scheduler — премиса „credential-domain, not orchestration“ слабеет и машинерия полезет обратно в Exec». Механизм: D6 фиксирует reactive-only на 1.0, но lease scheduler (`runtime/lease/scheduler.rs`) УЖЕ в крейте. Lease renew at N% TTL (RefreshStrategy::Lease из ADR-0088 D2) — это ПРОАКТИВНО по определению (таймер, не reactive-on-401). То есть проактивный путь уже частично в крейте под видом lease. Когда 1.1 включит proactive OAuth, либо credential получит свой планировщик (дубль engine-scheduler), либо back-edge credential→engine → ADR-0092 граф ломается. Ставка ADR-0092 уже подмочена собственным lease-кодом.

---

АЛЬТЕРНАТИВА (радикально другая нарезка):

ALT — «Protocol-crates as plugins + thin core + runtime-in-engine». Три оси разнести по КРЕЙТАМ, не по mod:
- `nebula-credential-core` (~3k LoC): только `Credential`/`AuthScheme`/`CredentialPolicy`/`CredentialState`/`CredentialRegistry` контракт + guards + secret types. Zero I/O. Это compile firewall, который ADR-0092 СОЗНАТЕЛЬНО выбросил.
- `nebula-credential-oauth2`, `-static`, `-signed`, `-lease` (по протокол-семействам, ~1-2k каждый): каждый протокол = свой крейт-плагин, регистрируется в registry. OAuth2 1481 LoC живет изолированно; правка GitHub-config НЕ рекомпилирует static-secret путь. Это лечит A2 (firewall) и A6 (oauth изоляция) структурно.
- `nebula-credential-runtime` остается отдельным Exec-крейтом (как D4-amended уже признал INFEASIBLE merge!). resolve/refresh/lease/coordinator там.
- Management (CRUD/owner_id) → `nebula-api/domain/credential` (это уже там частично живет). Tenant-scope через `ScopeLayer` (Business), как enforcement-point по типу, не по дисциплине → лечит A5.

Trade-offs ALT: (−) больше крейтов = больше Cargo.toml/deny.toml церемонии, медленнее cold build, выше барьер «добавить протокол» (новый крейт vs новый файл); (−) registry должен быть dyn-объектным через крейт-границу (boxed Protocol), теряется часть мономорфизации; (−) противоречит owner-решению «один публичный sdk» — но это ВНУТРЕННЯЯ нарезка, sdk все равно фасадит. (+) compile firewall возвращается; (+) blast radius правки контракта = только core-консьюмеры, не 20k; (+) tenant enforcement типовой; (+) per-protocol изоляция. Ключевой контр-аргумент к ADR-0092: «нет distinct external consumer ⇒ один крейт» (tower/axum rule) применен к contract/runtime, но НЕ к protocol-плагинам — а плагины как раз и есть кандидаты на крейт-границу (plugin author = distinct consumer). ADR-0092 не рассмотрел эту нарезку, отверг только «вернуть 4 крейта».

Минимальный гибрид (если owner не хочет N крейтов): хотя бы core/runtime split (2 крейта) — вернуть один firewall, по которому бьют чаще всего (контракт). Это прямо то, что D4-amended признал необходимым, а ADR-0092 откатил «ради authoring surface», хотя authoring surface дает sdk-реэкспорт, а не физический крейт.

---

СЦЕНАРИИ фейла worked-scenarios:

S1 (Plane B OAuth, D5) — DESIGN §10 правило «one PKCE/state kernel». Шаги поломки: (1) агент добавляет Microsoft OAuth; (2) видит `api/transport/oauth` PKCE И `credentials/oauth2.rs` state-signing; (3) DESIGN §19 Open Q1 «module path для unified PKCE kernel НЕ решен»; (4) агент дублирует PKCE в credential, т.к. так короче; (5) два state-kernel снова → дубль, который Plane Law хотел убить. Гейт не ловит: оба места легальны по слоям.

S2 (refresh без refresh_token, D3) — DESIGN §9 «OAuth2 без refresh_token → ReAcquire, не blind refresh». Шаги: (1) OAuth2State где `refresh_token: None`; (2) policy(state) должен вернуть `ReAcquire`; (3) но `Refreshable` impl ПРИСУТСТВУЕТ на типе (compile-gated D1) → registry bitflag = refreshable; (4) `ops.rs` читает bitflag, НЕ policy; (5) runtime зовет `refresh()` → 400 invalid_grant вместо re-auth. Это §16 «Sub-trait dispatch ignores policy» — открытый баг, и D3 (bitflag) структурно конфликтует с D2 (policy-first). Два решения противоречат друг другу: bitflag говорит «refreshable», policy говорит «ReAcquire».

S3 (tenant, A5) — Шаги: (1) добавляют `CredentialRuntime::test_by_id(id)` для health-check UI; (2) метод грузит store по id, зовет provider test; (3) автор копирует resolve, забывает `owner_id != scope → NotFound` чек (его нет в типе, только в дисциплине); (4) тенант A вызывает test на id тенанта B; (5) утечка «существует/валиден» через тайминг/ошибку. facade.rs сам пишет что enforcement operation-level — каждый новый op это новая дыра.

S4 (blast radius, A2) — Шаги: (1) Phase 1 меняет сигнатуру `Credential::acquire` (Acquisition тип); (2) рекомпиляция: contract→resolver→ops→executor→facade→coordinator→oauth2→rotation→lease; (3) crew builder на каждой из ~8 итераций Phase 1 ждет полную сборку 20k LoC; (4) reference_breaking_refactor_playbook требует whole-workspace-green per commit → каждый commit = полная сборка крейта-гиганта; (5) темп рефактора падает, растет соблазн срезать.

S5 (lease vs reactive, A7) — Шаги: (1) 1.0 ships reactive + lease scheduler в крейте; (2) Vault lease renew at 80% TTL уже проактивный таймер в `runtime/lease/scheduler.rs`; (3) 1.1 добавляет proactive OAuth pre-expiry; (4) обнаруживается, что scheduler нужен engine-координации (claim contention across replicas); (5) либо credential дублирует engine RefreshCoordinator, либо credential→engine back-edge → граф ADR-0092 (строка 146-151) ломается, «cycle dissolved by relocation» откатывается.

---

ВЕРДИКТЫ:

D1 — DISAGREE. Один крейт + 3 bounded contexts держит границы только дисциплиной (`pub(crate)`), не компилятором; facade.rs/ops.rs уже импортируют сквозь слои. Минимум нужен core/runtime crate split (D4-amended сам это признал, ADR-0092 откатил «ради sdk surface», но surface дает реэкспорт, не крейт). reqwest/sqlx-изоляция через dyn-инъекцию — единственная часть, что реально структурна, ее сохранить.

D2 — CAVEAT. Цель «один пайплайн» верна, но не показано, как `CredentialRuntime` глотает дженерик `PS`/`B` из `DispatchOps`/`resolver` без возврата к `CredentialService<B,PS>` generic-soup (Pain 2). Без явного плана стирания это переименование 4 структур, не унификация.

D3 — DISAGREE (внутренний конфликт). policy-as-data (D2-spec) и capability-bitflag-from-sub-trait-membership (ADR-0088 D3, в коде ops.rs) дают противоречивые ответы для OAuth2-без-refresh_token: bitflag=refreshable, policy=ReAcquire. Runtime обязан читать ТОЛЬКО policy(state); тогда bitflag избыточен и должен быть удален, а не сосуществовать. Сейчас сосуществуют — это §16 открытый баг, возведенный в дизайн.

D4 — CAVEAT. «Один OAuth2Protocol, провайдеры=data» верно для config (URL/scopes), но 3 grant-типа (authz/client-creds/device) — разный control-flow, не data; цель oauth2 <500 LoC недостижима без code-per-grant модулей. Признать: code-per-protocol-per-grant, config-per-provider.

D5 — CAVEAT. Plane Law правильна как закон, но Open Q1 (где живет PKCE kernel) НЕ решен — пока не решен, S1-дубль неизбежен. Заблокировать реализацию D5 до фиксации module-path единого kernel.

D6 — DISAGREE с «reactive-only снимает риск». Lease scheduler в крейте УЖЕ проактивен (renew at N% TTL). Ставка ADR-0092 «1.0 free of scheduler coupling» уже подмочена собственным lease-кодом; proactive 1.1 с высокой вероятностью вернет credential→engine back-edge. Решить координацию lease ДО 1.0, не откладывать.

D7 — AGREE (с одной оговоркой). values-only persistence + schema-from-types — единственное решение здесь, что реально done-by-construction и не зависит от дисциплины. Оговорка: §5 «Gap honest» — activation-time валидация parameters против live registry НЕ сделана; пока не сделана, stale-workflow ловится только на dispatch (поздно).

D8 — AGREE. slots отдельно от parameters, bind-to-output-scheme (D5 ADR-0088), `#[property]` unified = Phase 5 сахар — корректно и ортогонально TypedDAG. Единственное из 8, где декомпозиция и так уже типобезопасна (`ValidatedCredentialBinding` + tenant-fingerprint). Не трогать ради рефактора нижних слоев.

Файлы-улики: `crates/credential/src/service/facade.rs` (1414, CRUD+runtime смешаны, tenancy operation-level), `crates/credential/src/service/ops.rs` (899, `DispatchOps<B,PS>` дженерик — не сворачивается), `crates/credential/src/runtime/resolver.rs` (830), `crates/credential/src/runtime/refresh/coordinator.rs` (1015, refresh-CAS), `crates/credential/src/credentials/oauth2.rs` (1481, монолит жив), `crates/credential/src/runtime/lease/scheduler.rs` (проактивный таймер vs reactive-only ставка), `crates/credential/src/service/binding.rs` (confused-deputy закрыт ТОЛЬКО для bind, не resolve/refresh/test).


===== critic-types =====
I now have the complete picture of the macro-derived policy. I have all the evidence needed for a grounded adversarial critique. Key findings confirmed:

1. The macro synthesizes `policy()` with `expires_at: None, lease: None` always — it's a **static snapshot**, not state-derived, contradicting the design's "policy is computed from live state."
2. The resolver **never calls `policy()`** — it routes by `state.expires_at()` + hardcoded `C::KEY != OAuth2Credential::KEY` string check. D2/D3 policy-first routing is vaporware.
3. `RefreshStrategy::RefreshToken` vs `ReAcquire` distinction (the whole D2 selling point) is collapsed: macro always emits `RefreshToken` if a `refresh` method exists, regardless of whether refresh_token is present in state.

Let me write the critique.

---

АТАКИ

**A1. policy(state) — это не функция от state; макрос печатает константу.** Заявление DESIGN §9 «policy computed from live state, OAuth2 reports RefreshToken only while it holds a refresh token, ReAcquire otherwise» прямо опровергается `credential_attr.rs:435-447`: синтезированный `fn policy(_state: &Self::State)` игнорирует аргумент (`_state`), всегда печатает `expires_at: None, lease: None` и `refresh = if is_refreshable {RefreshToken} else if is_dynamic {Lease} else {Static}`. Механизм поломки: OAuth2-инстанс без refresh_token всё равно репортит `RefreshStrategy::RefreshToken` → движок (если бы читал policy) погнал бы слепой refresh, который IdP отбивает `invalid_grant` → именно тот ReAcquire-кейс, ради которого D2 затевался, не выражается макросом вообще. Чтобы получить честную state-derived policy, автор обязан написать `fn policy` руками — то есть DX-выигрыш D1 для единственного нетривиального случая (RefreshPair) обнуляется.

**A2. Policy-first routing — фикция; resolver роутит по строковому KEY.** DESIGN §9/§16 («runtime reads policy(state) then method», «sub-trait dispatch ignores policy → policy-first routing») не реализован: `resolver.rs:198-270` решает needs_refresh по `state.expires_at()`, а сам refresh внутри `perform_refresh` ветвится на `if C::KEY != OAuth2Credential::KEY { return Ok(None) }` (`resolver.rs:536`). Это хардкод одного протокола по строке KEY — антитеза «code-per-protocol». Grep подтверждает: `RefreshStrategy`/`CredentialLifecycle::policy` имеют ноль продакшен-консьюмеров (только lifecycle.rs + macros + один тест). Поломка: добавляешь второй refreshable-протокол (Vault renew, STS) — он молча проваливается в `<C as Refreshable>::refresh`, минуя весь OAuth-путь, а `Lease`/`ReAcquire`-стратегии не диспетчеризуются ничем. D2 — мёртвый тип-уровень.

**A3. Двойная истина о capability: bitflag (D3) vs policy.refresh (D2) не сверяются.** Capability теперь живёт в ДВУХ местах: `Capabilities::REFRESHABLE` (из `IsRefreshable::VALUE`, capability_report.rs) И `RefreshStrategy` в policy. Макрос держит их в лок-степе, но D3 явно разрешает hand-roll (`compute_capabilities` bound + escape-hatch T3). Hand-roll автор может написать `impl Refreshable` + `IsRefreshable::VALUE=true`, но `fn policy` вернуть `RefreshStrategy::Static` — никакой компилятор это не ловит (policy — обычный метод, не выводится из членства). Получаем ровно тот «capability declared twice, reconciled by assertion» анти-паттерн, который ADR-0088 Pain 1 клялся убить, только теперь рассинхрон тихий (рантайм), а не parity-assert.

**A4. Условная/state-зависимая capability невыразима.** Реальный кейс: OAuth2 client-credentials grant НЕ refreshable (нет refresh_token by design), authorization-code — refreshable. Это один тип `OAuth2Credential` (oauth2.rs поддерживает три grant в одном типе), но capability у него теперь бинарна на уровне ТИПА (`is_refreshable = items.refresh.is_some()`), не инстанса. Поскольку `OAuth2Credential` имеет метод refresh, ВСЕ его инстансы помечены REFRESHABLE, включая client-credentials, которые обязаны делать ReAcquire. Тип-уровневая capability D3 структурно не умеет «capability зависит от grant в state». Единственный выход — снова `policy(state)`, который (см. A1/A2) не работает.

**A5. dyn registry + generics: `DispatchOps<B,PS>` пережил «collapse four into one».** ADR-0088 D3 «collapse four registries into one», но сам же признаёт в impl-note: `DispatchOps<B,PS>` retained, потому что op-closures generic по store/pending. DESIGN §16 всё ещё числит «4 resolve entry points». Итог: `CredentialRegistry` (non-generic, capability+metadata) + `DispatchOps<B,PS>` (generic, ops) — это ДВЕ таблицы с разными ключами жизненного цикла, и registry-sync invariant probe (registry.rs:300 `iter_keys`) комментарием ссылается на `StateProjectionRegistry`/`CredentialDispatch`, которые якобы удалены — то есть инвариант-проба сверяет уже несуществующие таблицы. Поломка: плагин регистрируется в registry, но op-closure в `DispatchOps` не прописан (или наоборот) → рассинхрон ловится только если проба обновлена, а её doc-коммент протух.

**A6. Trait evolution / semver: добавление 6-й capability ломает каждый hand-roll и любой `match Capabilities`.** `compute_capabilities` требует все пять `IsX` бандов; `#[non_exhaustive]` на `Capabilities` есть, НО bound на `register<C>` (registry.rs:128-133) перечисляет пять трейтов явно. Добавишь `IsLeasable` шестым — каждый T3 hand-roll (bearer_token, probes, тесты) перестанет компилиться (не добавили шестой impl), и сигнатура `register` — публичная — это hard breaking change на КАЖДЫЙ новый capability. «Capabilities are data, not traits» (ADR-0088 research verdict #1) на деле наполовину traits: пять жёстко зашитых трейт-бандов в публичной сигнатуре регистратора. Эволюция модели = breaking-волна, ровно то, чего policy-as-data должен был избежать.

**A7. CredentialGuard<Scheme>: два протокола, одна схема, разная TTL-семантика — невидимый класс багов.** D5 продаёт «Vault-leased secret и static PAT выглядят одинаково `CredentialGuard<BearerToken>`; differs only refresh cadence». Но `SchemeFactory`/`resolve_with_refresh` (resolver.rs:125-151, 209) считает needs_refresh ТОЛЬКО по `state.expires_at()`. Leased-секрет (RefreshStrategy::Lease) с `expires_at: None` (сервер трекает TTL — см. lifecycle.rs:168 `is_expired_at` возвращает false для lease без inline-expiry) НИКОГДА не попадёт в окно refresh: `resolve_with_refresh` увидит `expires_at().is_none()` → `needs_refresh=false` → отдаст протухший guard. Два «BearerToken» с разной TTL-семантикой неразличимы на consumer-стороне, и фреймворк молча обслуживает мёртвый lease. Это не «differs only in cadence» — это тихая потеря renewal для целой категории (Leased/k8s), при том что `is_auto_renewable()` для них возвращает true, но НИКТО его не зовёт.

---

АЛЬТЕРНАТИВА

**Sealed enum-of-protocols вместо macro-derived sub-traits + policy-as-data.** Один `enum ProtocolKind { StaticSecret(StaticP), RefreshPair(OAuth2P), Leased(LeaseP), Federated(StsP), ... }` (~10 вариантов = ровно CredentialCategory), каждый вариант — конкретный тип с inherent методами. `fn lifecycle(&self, state) -> Lifecycle` — match по варианту, и КАЖДЫЙ вариант обязан вернуть стратегию (компилятор форсит exhaustive match при добавлении варианта — это и есть честный compile-gate, заменяющий E0046). Refresh-routing — `match self.protocol { RefreshPair(p) => p.refresh_via(state, transport), Leased(p) => p.renew(state), Static(_) => unreachable }` — единственная точка, exhaustive, без строкового `C::KEY != OAuth2Credential::KEY`.
- Trade-off (минусы): теряется open-world плагинов — внешний крейт не добавит новый ProtocolKind без правки enum (sealed). Но DESIGN §11 и так фиксирует «~10 protocol families, 1.0 focus» закрытым списком, а `inprocess_registry_pivot` отказался от out-of-process плагинов — open-world уже не цель. Custom AuthScheme плагина (DESIGN §11 «may») остаётся открытым (Output-тип), закрывается только set протоколов.
- Trade-off (плюсы): A2/A4/A7 закрываются by-construction (exhaustive match → нельзя забыть Lease-путь; condition по grant — это разные данные внутри RefreshPair-варианта, а не тип-уровневый флаг); A6 — добавление варианта ломает ТОЛЬКО match-arms внутри крейта, не публичную `register` сигнатуру и не hand-rolls (их больше нет); A3 — одна истина (вариант enum), policy и capability невозможно рассинхронить, т.к. оба производны от match. Capability-discovery (`iter_compatible`) выводится из варианта, bitflag становится derived-кэшем, не вторым источником.
- Гибрид (если open-world критичен): sealed enum для 10 built-in + один `Custom(Box<dyn DynProtocol>)` вариант, где DynProtocol — object-safe seam с `Pin<Box<dyn Future>>` (уже принятая идиома, ADR-0088 research #4). Тогда exhaustive-выгода для 99% built-in, escape-hatch для редкого плагина — но `Custom` обязан вернуть Lifecycle (метод трейта), так что A4 condition-by-state остаётся выразимым.

Typestate (oauth2 5.x style) ADR-0088 уже корректно отверг (не композится с string-keyed dyn registry) — не предлагаю.

---

СЦЕНАРИИ (фейл worked-scenarios)

**S1 (DESIGN §9 «OAuth2 without refresh_token → ReAcquire»).** Шаги: (1) provider выдал токен без refresh_token; (2) `acquire` пишет `OAuth2State{refresh_token: None, expires_at: Some(t)}`; (3) токен входит в early-window; (4) `resolve_with_refresh` видит `expires_at`, `needs_refresh=true`; (5) `perform_refresh` → `try_oauth2_refresh` (KEY совпал) → `refresh_oauth2_state` POST без refresh_token → IdP `400 invalid_grant`. Ожидалось: policy=ReAcquire, тихий re-acquire. Факт: ошибка, `reauth_required=true`, юзер выкинут в интерактив. Policy-as-data не сработал, т.к. его никто не читает.

**S2 (DESIGN §9 worked «Vault-leased = static PAT, differs only cadence»).** Шаги: (1) Leased-cred, `policy.refresh=Lease`, `lease.renewable=true`, `state.expires_at=None`; (2) consumer держит `CredentialGuard<BearerToken>`; (3) lease-scheduler по дизайну должен renew at N% TTL. Факт: ни `resolve_with_refresh` (гейт по `expires_at()`), ни какой-либо lease-scheduler в resolver не зовёт `is_auto_renewable()`; lease истекает на сервере; следующий запрос с guard → 401. Тихий отказ целой категории.

**S3 (ADR-0088 D3 «hand-roll allowed» × A3).** Шаги: (1) power-user пишет T3 `impl Credential` + `impl Refreshable` + руками `IsRefreshable::VALUE=true`; (2) забывает/ошибается в `fn policy`, возвращает `RefreshStrategy::Static`; (3) компилируется (policy — свободный метод). Факт: `registry.is_refreshable()=true`, но любой будущий policy-reader сочтёт cred статичным → рассинхрон, который ADR-0088 Pain 1 объявлял устранённым. Ни E0046, ни parity-assert не срабатывают.

**S4 (ADR-0088 D3 registry-sync probe).** Шаги: (1) разработчик читает registry.rs:300 doc-коммент про сверку `CredentialRegistry`/`StateProjectionRegistry`/`CredentialDispatch`; (2) две из трёх таблиц удалены (D3 impl-note); (3) реальный риск-рассинхрон теперь `CredentialRegistry` vs `DispatchOps<B,PS>`. Факт: инвариант-проба документирует мёртвую тройку, не покрывает живую пару registry↔DispatchOps; плагин с записью в registry без op-closure в DispatchOps проходит «sync»-пробу и падает в рантайме при первом resolve.

**S5 (DESIGN §11 «new SaaS = provider config, not new type» × A2).** Шаги: (1) добавляют Vault как `RefreshStrategy::Lease` протокол; (2) регистрируют как data; (3) cred входит в renewal-окно. Факт: `perform_refresh` хардкодит только `OAuth2Credential::KEY`-ветку, дальше `<C as Refreshable>::refresh` — для Vault это сработает лишь если автор вручную реализовал refresh, но Lease-семантика (renew по lease_id, не по refresh_token) не диспетчеризуется RefreshStrategy ни на одной развилке. «Code-per-protocol» свёлся к «code-per-OAuth2, остальное самонаведением».

---

ВЕРДИКТЫ (D1–D8)

- **D1** caveat — один крейт + 3 контекста + injected ports корректны и acyclic (ADR-0092 import-evidence убедителен), НО «reqwest/sqlx не линкуются» держится на дисциплине композит-рута, а не на feature-gate/типах; transport behind `feature="rotation"` (resolver.rs:13) — это feature-gate, противоречащий «without feature-gates» из ADR-0092:156.
- **D2** disagree — policy-as-data НЕ реализован: policy не функция от state (макрос игнорит `_state`), resolver не читает policy вовсе; «policy then method» — вапорвейр (A1, A2, S1).
- **D3** disagree — macro выводит policy из методов лишь синтаксически (presence), но (a) не из state, (b) hand-roll рассинхронит policy↔capability без compile-gate, (c) расширение capability — breaking на публичной `register` сигнатуре (A3, A4, A6).
- **D4** caveat — code-per-protocol/config-per-provider верны как принцип, но в коде один OAuth2-протокол захардкожен по KEY, прочие протоколы не диспетчеризуются по RefreshStrategy; «один OAuth2Protocol» ещё монолит (oauth2.rs), §16 сам числит D1 not done (A2, S5).
- **D5** agree — Plane Law (zero routes в credential, injected RefreshTransport, узкий SSRF-seam внутри credential) архитектурно чист и подтверждён ADR-0092 narrow-seam обоснованием; единственный нюанс — узкий seam ещё не везде (api/transport vs engine/rotation дублировались, унификация заявлена не доказана).
- **D6** caveat — reactive-only + durable RefreshClaimRepo солидны; но L1 coalescer и CAS-пути живут в resolver с `#[allow(deprecated)]` на typed-id миграцию (resolver.rs:197,273) — переходное состояние, не финал; ADR-0092 open-risk (proactive 1.1 тянет coordinator к scheduler) реален.
- **D7** agree — values-only persistence + schema-from-types (HasSchema→metadata→catalog) согласованы симметрично Action/Resource/Credential; честно помечен gap (нет expression-pipeline в credential setup, activation-validate отложен).
- **D8** caveat — slots отдельно от parameters и `CredentialGuard<Scheme>` bind-to-output корректны, НО bind «differs only in refresh cadence» ложно: разная TTL-семантика двух протоколов под одной Scheme даёт тихий отказ renewal (A7, S2); unified `#[property]` = Phase 5 сахар — приемлемо как отложенное.

Ключевые файлы: `crates/credential/macros/src/credential_attr.rs:435-447` (policy-константа), `crates/credential/src/runtime/resolver.rs:209,536` (routing по expires_at + KEY-хардкод), `crates/credential/src/lifecycle.rs:168,178` (lease без inline-expiry → is_expired_at=false, is_auto_renewable некем не зовётся), `crates/credential/src/contract/registry.rs:128-133,300` (пять трейт-бандов в публичной сигнатуре + протухший sync-probe коммент), `crates/credential/src/contract/capability_report.rs` (второй источник capability).


===== critic-sec =====
I have enough across all attack surfaces. Compiling the adversarial verdict now.

---

АТАКИ

1. **Confused-deputy через ослабленный fingerprint (`resolve_for_slot` доверяет binding, store не пере-проверяет owner).** Механизм: `validate_credential_binding` читает foreign row через `store_load_raw` (минует owner-gate), сравнивает `owner_id` строку, и пакует `TenantFingerprint(owner_id)` в `ValidatedCredentialBinding`. На consume-site `resolve_for_slot` сверяет только `binding.fingerprint() == from_scope(scope)` — то есть «совпадает ли owner_id строка сама с собой», после чего зовёт `resolver.resolve::<C>(credential_id)`, а резолвер (`load_and_verify`) грузит по сырому `credential_id` БЕЗ всякой owner-проверки на уровне store. Защита держится исключительно на том, что engine построил binding из правильного scope. Любой второй путь, конструирующий scope из недоверенного источника (workflow-author-controlled org/workspace, повторно используемая ULID, кэш scope между tenant'ами), резолвит чужой секрет: fingerprint тривиально проходит, store не возражает. DESIGN это НЕ закрывает — наоборот, §9 «defence in depth» называет повторную проверку fingerprint достаточной, хотя это идемпотентная тавтология (сравнение значения с самим собой), а не независимый барьер. Реальный барьер (owner-scoped store-query на стороне резолвера) отсутствует.

2. **`owner_id` = неэкранированная конкатенация свободных строк → tenant collision на consume-path.** Механизм: ADR-0088 amendment вводит length-prefixed `Scope::credential_owner_id` чтобы убить `{org}/{ws}` vs `{org}:{ws}` коллизию. Но `TenantFingerprint(scope.owner_id())` и `owner_matches` сравнивают именно строку `owner_id`. Если хоть один продьюсер scope (api edge vs runtime) не прошёл через length-prefixed derivation (а ADR прямо пишет «api manual-enforcement arm dead, follow-up deletion»), `org="a",ws="b␞c"` и `org="a␞b",ws="c"` дают одинаковый ключ → cross-tenant чтение через `owner_matches`-true. DESIGN §9 объявляет «single owner_id format» целью, но не инвариантом-по-конструкции: нет типа `OwnerId`, который НЕВОЗМОЖНО собрать из сырой строки. «Помни вызвать canonical derivation» — это discipline, не enforcement.

3. **Второй composition root → permissive `RefreshTransport` обходит SSRF, как и предупреждает ADR — но кодом не закрыто.** Механизм: SSRF-валидация (`validate_token_endpoint`) живёт в credential ДО `post_token`. Это снимает «permissive transport bypass» для проверки строки URL. НО: (a) `token_url` приходит из stored `OAuth2State`, который писался при acquire/refresh из ответа IdP/конфига провайдера; pre-call проверка строки — синхронная, а DNS-rebind закрыт ТОЛЬКО если конкретный `ReqwestRefreshTransport::hardened()` ставит connect-time resolver. Дока transport.rs пишет «transport SHOULD add connect-time blocking» — SHOULD, не MUST, не enforced типом. Любой alt-root (CLI, test harness, будущий worker) инжектит голый reqwest без custom resolver → TOCTOU: `validate_token_endpoint` резолвит `evil.com`→1.2.3.4 (ок), reqwest при connect резолвит `evil.com`→169.254.169.254 (metadata). DESIGN/ADR описывают риск словами, но узкий seam НЕ несёт сам resolver — он несёт `url: String`, отдавая DNS на откуп impl. Defense держится на дисциплине одного «hardened» конструктора.

4. **Pending-store `get` — дыра в 4-мерном binding (replay/fixation OAuth state).** Механизм: trait документирует «4-dimensional token binding» и `consume`/`get_bound` валидируют (kind, owner, session, token). Но метод `get<P>(&self, token)` читает pending state ТОЛЬКО по token, без owner/session/kind — «for polling flows like device code». 32-байтный CSPRNG token — единственная защита `get`. Если token утёк (лог, error-path, реферер, прокси, общий device-code poller), атакующий читает чужой pending OAuth2State (PKCE verifier, client creds) через `get`, минуя 3 из 4 измерений. Device-code poll именно так и работает: повторные `get` без session. DESIGN §10 «one PKCE/state kernel» и pending-store doc заявляют binding-by-construction, но публичный `get` его структурно подрывает.

5. **Rotation race: revoke удаляет row, а in-flight refresh CAS воскрешает удалённый секрет.** Механизм: `revoke` делает provider-revoke → `lease.revoke_for_credential` → `store.delete(id)`. Конкурентный `resolve_with_refresh`/`refresh_inner` в этот момент держит `stored` (version N), выполняет IdP POST (30s timeout), затем CAS-write `PutMode::CompareAndSwap{expected_version: N}`. Если delete прошёл ПОСЛЕ load но ДО CAS, поведение зависит от store-семантики delete+CAS: при `PutMode::CompareAndSwap` на удалённую строку многие KV вернут NotFound/conflict — но `perform_refresh` имеет retry-loop, читающий `actual` из VersionConflict, и НЕ имеет арма на «row исчез». Хуже: если delete не бампит version-эпоху (а в in-memory это просто remove), CAS на «expected N» может воссоздать row (upsert-CAS) с уже отозванным, но свежеобновлённым refresh_token. Отозванный credential оживает с новым access_token. DESIGN §9/facade обсуждают concurrent refresh vs update (CAS), и concurrent refresh vs coalesce, но revoke-during-refresh (delete vs CAS-resurrect) не разобран ни в одном worked-сценарии; `perform_refresh` не консультирует tombstone.

6. **Circuit-breaker «serve stale-but-valid» отдаёт секрет вне early-window под отказом IdP.** Механизм: при `is_circuit_open` и `!truly_expired` резолвер делает `C::project(&state)` и возвращает handle на устаревший токен «within early-refresh window». Но окно входа в эту ветку — `needs_refresh==true` (т.е. уже в early-window), а отдаётся НЕпроверенный на отзыв токен сколько угодно раз, пока circuit открыт. Если IdP вернул `invalid_grant` потому что токен отозван на стороне провайдера (а не network blip), circuit-breaker маскирует отзыв и продолжает раздавать живой bearer вплоть до hard-expiry. `reauth_required` ставится только на ReauthRequired-исходе, а circuit открывается по `record_failure`, который не различает «отозван» и «5xx». DESIGN §9 хвалит fallback-on-interrupt (`aws-credential-types`), но не ограничивает его терминальными ошибками на resolver-пути (в facade `is_transient_failure` есть, в `resolver.rs` circuit-ветке — нет).

7. **Plugin-supplied `AuthScheme`/`Credential` как канал эксфильтрации; dup-KEY закрыт, dup-Scheme — нет.** Механизм: §15.6/registry делают dup-KEY фатальным (хорошо). Но: (a) capability выводится из «наличия методов» (`compute_capabilities`), плагин волен реализовать `Testable::test`, который шлёт state на свой endpoint — `test` зовёт `ops.test(...&stored.data...)` с расшифрованными байтами; SSRF-валидация есть только на OAuth2 token_url, не на произвольный provider-network в plugin `test`/`refresh`/`acquire`. (b) `project()` плагина получает `&State` (plaintext) и возвращает Scheme — плагин может сложить весь refresh_token в «output scheme», который потом уедет в исходящий запрос action. (c) macro-policy supply-chain: policy выводится из методов, значит вредоносный плагин, объявив `category=Static` но реализовав скрытую сетевую логику в `project`, обходит «runtime сначала policy потом метод» — policy управляет ТЕМ КАКОЙ lifecycle-метод звать, а не тем что делает project/test. DESIGN §11 запрещает «browser/token HTTP в плагине», но не перекрывает плагину обычный outbound в test/acquire — нет egress-allowlist на plugin-network. Это противоречит заявленному «SSRF hardened» как свойству подсистемы: оно держится только на OAuth2-ветке.

АЛЬТЕРНАТИВА (радикально другая декомпозиция)

**Capability-token resolver вместо string-id + scope-fingerprint.** Сейчас все read-операции принимают `(scope, id: &str)` и каждая руками делает `load_owned`/owner-match — owner-проверка размазана по ~8 методам, а `resolve_for_slot` её вообще пропускает (доверяет binding). Радикальная замена: store-port принимает не `id: &str`, а `OwnerScopedKey` (privately-constructed newtype, инкапсулирующий length-prefixed owner_id + id), и НЕ имеет метода `get(&str)` вовсе. Тогда: (1) резолвер физически не может загрузить чужой row — нет API; owner-isolation становится свойством типа store-port, а не дисциплиной facade; (2) `ValidatedCredentialBinding` несёт `OwnerScopedKey`, а не голую строку + tautological fingerprint; (3) collision-by-construction исчезает, потому что единственный конструктор `OwnerScopedKey` — это length-prefixed derivation. Trade-offs: ломает store-port сигнатуры по всему workspace (api/storage/engine), удваивает работу миграции; durable backends должны индексировать по составному ключу (но это И ЕСТЬ нужный owner-scoped query, который DESIGN §list честно отложил как O(N)); теряется удобство «resolve by bare id» для админ-инструментов (нужен explicit `AdminScopedKey` с аудит-флагом). Это сдвигает defense с «помни вызвать load_owned» на «нельзя выразить незащищённый доступ» — ровно паттерн, который пользователь требует (type-enforce, не discipline).

СЦЕНАРИИ (фейл-сценарии для worked scenarios, точные шаги)

S1 (confused-deputy): tenant B запускает workflow с `slot_bindings{db -> "cred_<ULID-of-A>"}`; если engine собрал scope из workflow-предоставленного org/workspace (или scope tenant A переиспользован из пула), `validate_credential_binding` для несоответствия даст ScopeMismatch — НО если owner_id строки совпали по коллизии (см. S3) ИЛИ binding пришёл уже-валидированным из переиспользованного контекста, `resolve_for_slot` делает fingerprint==fingerprint (true) → `resolver.resolve` грузит cred A без owner-проверки → guard с секретом A в action B.

S2 (SSRF/rebind): провайдер `evil` зарегистрирован с `token_url=https://idp.evil.com/token`; `evil.com` DNS TTL=0, отвечает 1.2.3.4 на первый lookup. CLI/worker root инжектит `ReqwestRefreshTransport` без custom resolver. Refresh: `validate_token_endpoint` резолвит→1.2.3.4 (public, ок); reqwest connect резолвит→169.254.169.254; POST refresh_token+client_secret уходит на metadata-endpoint/internal service. Эксфильтрация секрета + SSRF.

S3 (tenant collision): orgA создаёт cred с `org="acme", ws="prod:db"`; если api-edge всё ещё на сепараторном формате (ADR: «manual-enforcement arm dead, follow-up deletion»), owner_id=`acme:prod:db`. orgB с `org="acme:prod", ws="db"` даёт тот же `acme:prod:db`. `owner_matches`→true → B видит/удаляет/рефрешит cred A через `get`/`list`/`delete`.

S4 (revoke vs refresh resurrect): T0 replica1 `resolve_with_refresh` грузит cred (v=5), уходит в 30s IdP POST. T1 replica2 `revoke` → provider-revoke OK → `store.delete`. T2 replica1 IdP вернул новый token, делает CAS{expected=5}. На in-memory store delete не сохраняет tombstone-эпоху → CAS воспринимает отсутствие как «version 0/упсерт» в зависимости от impl → отозванный cred воскрешён с рабочим access_token; action продолжает аутентифицироваться отозванным секретом.

S5 (pending replay): device-code flow: client опрашивает `pending.get(token)` без session (как разрешено). Token утёк в shared-poller лог. Атакующий из другого session/owner зовёт `get(token)` → читает OAuth2 pending (PKCE verifier, client_id/secret) минуя owner/session/kind binding; завершает чужой flow или ворует client creds.

ВЕРДИКТЫ (D1–D8)

- **D1** (один крейт, 3 BC, reqwest/sqlx не линкуются): **agree-with-caveat** — `cargo tree`-инвариант хорош, но «не линкуется reqwest» ≠ «SSRF закрыт»: defense ушла за seam, который несёт `String` а не resolver (см. A3); проверь cargo tree В CI как gate, иначе regression невидим.
- **D2** (один pipeline 5 операций): **disagree-as-stated** — пять entry point схлопнули в facade, но `resolver.resolve` (slot-path) и `facade.refresh` (mgmt-path) — ДВА разных CAS/owner-режима: slot-path owner-проверку не делает вовсе. «Один pipeline» на бумаге, два контракта изоляции в коде.
- **D3** (policy-as-data, capability compile-gated, policy→метод): **caveat** — policy гейтит КАКОЙ метод звать, не ЧТО метод делает; plugin `project/test/acquire` вне policy-контроля и без egress-ограничений (A7). Не моат против вредоносного плагина.
- **D4** (code-per-protocol, провайдеры=data): **agree** — структурно верно; единственный shape-риск: `token_url` как data в state делает SSRF-поверхность зависимой от записи в state, усиливая важность connect-time resolver.
- **D5** (OAuth Plane Law, zero routes в credential, RefreshTransport injected): **caveat** — разделение планов чистое, но «SSRF stays inside credential» истинно лишь для string-проверки; DNS-rebind по признанию самого ADR закрывается в transport-impl, т.е. вне credential. Заявление «defense not exportable» неточно.
- **D6** (reactive-only, L1 coalescer + durable RefreshClaimRepo): **caveat** — coalescing/CAS разобраны тщательно, НО revoke-during-refresh (delete vs CAS-resurrect, A5/S4) и circuit-breaker-serves-revoked (A6) — не покрыты; reactive-only не спасает от этих гонок.
- **D7** (values-only persistence, схема из типов): **agree** — `$expr`-refusal на typed deserialize + schema-from-type корректны; persistence-модель не открывает атак, найденных здесь.
- **D8** (consumer binding слоты→`CredentialGuard<Scheme>`, слоты отдельно от parameters): **agree-with-caveat** — bind-to-output-scheme и разделение slot/parameters верны; но `ValidatedCredentialBinding` несёт строковый fingerprint, а не owner-scoped key — binding type «доказывает» проверку, которая на consume-site вырождается в тавтологию (A1). Тип-доказательство слабее заявленного.

Ключевые файлы: `crates/credential/src/runtime/resolver.rs` (owner-проверка отсутствует на slot-path; circuit-breaker serves stale), `crates/credential/src/service/facade.rs` (`resolve_for_slot` tautological fingerprint, revoke=delete без tombstone-vs-refresh), `crates/credential/src/service/binding.rs` (`TenantFingerprint`=сырой owner_id), `crates/credential/src/runtime/refresh/transport.rs` (DNS-rebind = SHOULD, не enforced), `crates/credential/src/runtime/refresh/token_refresh.rs` (SSRF только pre-call string), `crates/credential/src/pending_store.rs` (`get` минует 4-D binding).


===== critic-dx =====
I now have enough verified evidence. The action macro uses `#[action]`/`input = FooInput` — there is no `#[property]` and no `unified` mode. `register::<C>()` requires all five `IsX` bounds by hand for T3. OAuth2 provider-config-as-data does not exist. The macro infers capability from method *name strings*. Producing my adversarial review.

---

АТАКИ

1. **`#[credential]` инферит capability по СТРОКОВОМУ имени метода — не по типу.** Механизм: `classify_items` (credential_attr.rs:522-547) матчит `ident.to_string()` против списка `"refresh"/"revoke"/"test"/...`. Сценарий поломки: автор пишет `async fn refresh_token(...)` или `fn reauth(...)` вместо `refresh` — это не «typo одного из известных», это разумная альтернатива-имя. Макрос его НЕ узнаёт → бросает `unrecognized method` (хорошо), НО если автор пишет helper `async fn test_connection` он отлетит как «unknown», а `fn test` молча станет `Testable` capability с боевой dispatch-семантикой. Имя метода теперь — публичный security-sensitive контракт без типовой защиты. serde/clap матчат по `#[serde(rename)]`/типу поля, не по сигнатуре функции; bon — по builder-методам, генерируемым из полей. Здесь «capability = наличие функции с магическим именем» — это самый хрупкий из возможных каналов.

2. **«policy выводится из методов» ложь для половины реальных credential — и макрос это знает.** Механизм: synthesized policy (строки 416-449) хардкодит `expires_at: None, lease: None`. Для OAuth2 (refresh-strategy зависит от живого `refresh_token` в state) и для любого `Dynamic`/Leased макрос ЗАПРЕЩАЕТ синтез (строки 211-218) и требует ручной `fn policy`. То есть для всех нетривиальных типов (OAuth2, Vault, STS, k8s) D3-обещание «macro выводит policy» не выполняется — автор пишет policy руками, как в oauth2.rs:622. Получается: магия работает только для StaticSecret (где policy и так тривиальна), и отключается ровно там, где была бы полезна. Чистый negative-value: сложность ради случая, который её не требует.

3. **T1 «unified derive» и `#[property]` НЕ СУЩЕСТВУЮТ — это vaporware-таблица.** Механизм: §7 таблица обещает `#[property]` единым для Action/Resource/Credential, а DESIGN сам помечает Phase 5 «после runtime green». Проверка кода: action-макрос экспортирует `#[action]`+`input = FooInput`, нет ни `#[property]`, ни `unified` (lib.rs action/macros). Сценарий: AI-агент читает DESIGN §7, генерит `#[derive(Action)] #[action(unified)] { #[property] chat_id: String }` → не компилируется, `unknown attribute property`. Документ описывает три tier'а T1/T2/T3 и worked-examples как существующие (`api_key.rs style`), но T1 (unified) для credential тоже не реализован — реальный api_key.rs использует T2 (`#[credential]` на impl), а bearer_token.rs — T3 (ручной impl + 5 ручных `IsX`). Таблица tier'ов выдаёт желаемое за текущее.

4. **T3 не «выживает без макроса» — он требует 5 boilerplate-impl, которые НИЧЕГО не проверяют.** Механизм: `register::<C>()` (registry.rs:122-133) требует bound на все пять `IsRefreshable/...`. bearer_token.rs:68-82 — это пять ручных `const VALUE: bool = false`. Если автор T3 случайно напишет `IsRefreshable = true` но не сделает `impl Refreshable` — capability_report.rs:51 признаёт: ошибка всплывёт только на dispatch-сайте (E0277) в ДРУГОМ крейте, далеко от объявления. Хуже обратное (`= false` при наличии impl): registry молча недо-репортит capability, refresh структурно недостижим — НЕ compile error, а тихий рантайм-даунгрейд. Это в точности тот silent-self-attestation, который §15.8 якобы закрыл; макрос закрывает, ручной путь открывает заново. serde/clap не имеют «ручного зеркала», которое можно рассинхронить — derive единственный источник.

5. **Двойной макро-путь (`#[derive(Credential)]` И `#[credential]` attr) — два несовместимых ментальных API в одном крейте.** Механизм: derive (credential.rs) использует `properties = T`/`protocol = T` + `capabilities(refreshable)` ФЛАГИ; attr (credential_attr.rs) инферит из методов и ЗАПРЕЩАЕТ флаг capabilities. DESIGN §16 говорит «Legacy `#[derive(Credential)]` удалить если attr покрывает всё», но он не удалён. Сценарий для AI: агент смешивает — `#[credential(key=.., capabilities(refreshable))] impl X` → attr-парсер не знает `capabilities`, упадёт; или `#[derive(Credential)]` + ручной `impl Refreshable` без флага → derive не эмитит `IsRefreshable=true` → недо-репорт. Два макроса с противоположной философией (declare-флагом vs infer-методом) на одном слове «credential» — гарантированный источник cargo-cult-ошибок.

6. **OAuth2-монолит остаётся монолитом; «protocol+config-as-data» (D4) не реализован.** Механизм: grep по `OAuth2ProviderConfig`/`provider(`/`register_provider` — ноль попаданий. oauth2.rs всё ещё ~1480 строк с тремя grant-types, всеми HTTP-disabled заглушками (`oauth2_http_transport_disabled` ×5) и единственным `OAuth2Credential`. DESIGN §11 «config per provider github/slack» — чистый proposal. Текущая реальность: чтобы добавить GitHub, автор всё ещё пишет credential-тип или прокидывает FieldValues. Anti-pattern, который DESIGN запрещает («new OAuth2Credential per API»), — ровно то, что код сейчас вынуждает, потому что альтернативы (provider-registry) нет.

7. **`metadata()` дублируется в каждом credential — symmetric-API обещание не доходит до authoring.** Механизм: api_key.rs:76-86 и bearer_token.rs:36-46 — побайтово одинаковый builder-boilerplate (`.key(credential_key!(..)).name(..).schema(schema_of::<Properties>()).pattern(..).icon(..).build().expect(..)`). attr-макрос УМЕЕТ синтезировать metadata из args (credential_attr.rs:235-295), но api_key.rs всё равно пишет его руками — значит даже эталонные builtins не пользуются собственным сахаром. Для AI-агента это ловушка: он копирует api_key.rs (с ручным metadata) и `.expect("...")` в библиотечном коде, размножая panic-сайты, хотя infallible `for_credential` путь существует.

АЛЬТЕРНАТИВА

**Радикально: убить инференс-по-имени-метода; capability = тип ассоциированного объекта, объявленный явно один раз; никакого attr+derive дуэта.**
Вместо «макрос читает имена методов» — один derive по полю-маркеру или один struct-typed registration:
```
#[derive(Credential)]
struct Oauth2 { #[lifecycle] policy: RefreshPair, ... }
```
где `RefreshPair`/`StaticSecret` — это ТИПЫ (zero-size), каждый реализует трейт `Lifecycle` с ассоциированными `Refresh`/`Revoke` стратегиями. Capability перестаёт быть «есть ли функция с именем X» и становится «какой тип у поля lifecycle» — проверяемо компилятором, нерассинхронизируемо, и identical путь для T1 и T3 (T3 = тот же трейт без derive). Trade-off: теряем «политика зависит от живого state» как метод — нужно вынести state-зависимую часть в `fn refresh_strategy(&state)->RefreshStrategy` на самом lifecycle-типе (OAuth2 кейс), что и так уже руками пишется. Плюс: один источник capability вместо трёх (метод-присутствие + 5 IsX + policy). Минус: один обязательный derive вместо «любой impl Credential» — но текущий «любой impl» всё равно требует 5 ручных IsX, так что свобода иллюзорна.

**Вторая декомпозиция (если резать меньше):** удалить `#[derive(Credential)]` СЕЙЧАС (не «если attr покроет»), оставить только attr-макрос, и сделать attr-макрос требующим `#[capability]`-аннотацию НА методе (`#[refresh] async fn whatever`) вместо магического имени — тогда имя метода свободно, а capability явна и span-локальна. Trade-off: чуть многословнее, но убивает атаку №1 и №5 целиком.

СЦЕНАРИИ (фейлы worked-examples)

1. **OAuth2 worked-example §10 (Plane B) недостижим в коде.** Шаги: следуй sequence-диаграмме → `ApiCred->Runtime: token POST через injected RefreshTransport`. Реально: oauth2.rs `refresh`/`continue_resolve`/`resolve` для ClientCredentials/AuthCode все возвращают `oauth2_http_transport_disabled()`. Ни один OAuth2 flow не завершается внутри показанного контракта; «прервёшься на token POST» = ошибка, не токен. Worked-scenario не воспроизводим end-to-end в этой ветке.

2. **§7.Action unified example не компилируется.** Шаги: скопируй `#[derive(Action)] #[action(key="send.message", unified, output=SendOutput)] struct { #[property] chat_id, #[resource] slack, #[credential] token }`. Реально: `unified` и `#[property]` не парсятся action-макросом → E0«unknown». Автор застрял на первой строке примера, который DESIGN подаёт как целевой DX.

3. **§7.Credential example «policy выводится» для OAuth-подобного.** Шаги: автор пишет `#[credential(key="gh", category=RefreshPair)] impl Gh { type State=OAuth2State; ... async fn refresh(...) }` без `fn policy`. Реально: синтез даёт `expires_at:None, refresh:RefreshToken` всегда — теряется ReAcquire-ветка при отсутствии refresh_token. Engine будет звать refresh там, где надо re-acquire → `ReauthRequired` петля. Чтобы избежать — нужен ручной policy, т.е. example вводит в заблуждение.

4. **T3-автор рассинхронит IsX (тихий даунгрейд).** Шаги: пишу ручной `impl Credential` + `impl Refreshable`, копирую блок IsX из bearer_token.rs (все false), забываю поправить `IsRefreshable`. Реально: компилируется, регистрируется, `iter_compatible` исключает мой credential, refresh недостижим — ноль ошибок, тихий прод-баг (capability_report.rs:54-57 это прямо признаёт).

5. **AI-агент добавляет GitHub как «new OAuth2Credential».** Шаги: агент видит oauth2.rs, копирует в `github_oauth2.rs`, меняет KEY. Реально: ровно anti-pattern §11; провайдер-как-данные не существует, так что «правильный» путь физически недоступен — агент структурно вынужден нарушить собственный гайд DESIGN.

ВЕРДИКТЫ

- **D1** (один крейт, 3 BC, reqwest/sqlx не линкуются) — agree. ADR-0092 import-evidence солидна, граф ацикличен, dyn-инъекция уже паттерн репо. Caveat: «3 bounded contexts» пока папки, не enforced-границы (нет crate-firewall, что ADR сам признаёт как accepted negative).
- **D2** (один pipeline acquire/continue/refresh/revoke/test) — agree-with-caveat. Цель верна (16 §: 4 entry points → 1), но в текущем коде resolver + facade + DispatchOps всё ещё сосуществуют; это TODO, не сделано. Не выдавать за выполненное.
- **D3** (policy-as-data + macro выводит policy) — **disagree**. Инференс по имени метода хрупок (атака 1), синтез policy отключён для всех нетривиальных типов (атака 2), capability имеет три рассинхронизируемых зеркала на ручном пути (атака 4). «Macro выводит policy» фактически ложно. Переделать на typed-lifecycle.
- **D4** (code-per-protocol, config-per-provider) — **disagree (как заявление о факте); agree как цель.** Реализации ноль (grep пуст), OAuth2-монолит цел, anti-pattern вынужден. Это спецификация, а DESIGN местами подаёт её настоящим временем.
- **D5** (OAuth Plane Law, zero routes в credential) — agree. Разделение планов чистое, credential не монтирует HTTP, RefreshTransport-инъекция корректна. Caveat: вся логика сейчас — `transport_disabled()` заглушки; «логика state/PKCE в credential» есть (continue_resolve валидирует state/PKCE), но завершение flow нигде не живёт в этой ветке.
- **D6** (reactive-only refresh 1.0) — agree. Согласуется с ADR-0084; open-risk (proactive затащит scheduler-coupling назад в Exec) честно зафиксирован в ADR-0092.
- **D7** (values-only persistence, схема из типов) — agree. Симметрия schema_of::<Properties/Input/Config> реальна в коде; values-only хранение обосновано. Caveat: activation-time cross-check параметров против registry — «planned», т.е. дыра между catalog-схемой и сохранёнными values остаётся до Phase 5.
- **D8** (consumer binding, slots отдельно от parameters, `#[property]` = Phase 5 сахар) — agree-with-caveat. Разделение slot/parameter корректно и survive-TypedDAG аргумент валиден. Caveat: `#[property]`/unified — vaporware (атака 3); пока это сахар, которого нет, и worked-examples §7 нельзя выдавать за рабочий DX.

Ключевые файлы: `crates/credential/macros/src/credential_attr.rs` (инференс по имени, синтез policy), `crates/credential/src/credentials/oauth2.rs` (монолит + transport-disabled заглушки), `crates/credential/src/contract/capability_report.rs:51-57` (признание тихого даунгрейда на ручном пути), `crates/credential/src/contract/registry.rs:122-134` (5 IsX-bound для T3), `crates/action/macros/src/lib.rs` (нет `#[property]`/`unified`), `crates/credential/src/credentials/api_key.rs` vs `bearer_token.rs` (T2 vs T3, дублирование metadata).


===== player-n8n =====
I have solid grounding. The deepwiki answer confirms a key nuance the DESIGN.md §15 table glosses over. Let me compile the final verdict.

[grounded: deepwiki] — n8n CredentialsHelper / OauthService / ICredentialType / extends oAuth2Api / preAuthentication / authenticate verified against n8n-io/n8n.

ВЕРДИКТЫ D1..D8

D1 (один крейт, 3 BC + injected ports, no reqwest/sqlx) — **agree, caveat.** Совпадает с ADR-0092 (acyclic граф, ports). Caveat: ADR-0092 фиксирует, что decorators Encryption/Cache/Audit ОСТАЛИСЬ в nebula-storage (revert step 3), а не в credential — DESIGN §9 «Ports» это не отражает. 3 BC в одном крейте без compile-firewall: touch контракта рекомпилит runtime+management (ADR прямо это «accepted»). Защити границы модульной видимостью, иначе BC размоются.

D2 (один CredentialRuntime pipeline) — **agree.** Прямо лечит «4 resolve entry points» (§16). n8n валидирует: CredentialsHelper — единственный runtime-оркестратор pre-auth/auth (см. ОПЫТ). Caveat: §16 говорит «4 entry points», но D2 объединяет 5 операций — убедись, что test (credential probe) не тащит сетевой I/O в pipeline без RefreshTransport-порта.

D3 (policy-as-data + capability compile-gated; macro выводит policy; policy перед методом) — **agree, strong.** Это ровно n8n-урок: OAuth2 без refresh_token → ReAcquire, не слепой refresh(). ADR-0088 D3 = capability-as-sub-trait-membership совместимо. Caveat: «macro выводит policy из наличия методов» хрупко — две credential с одним набором методов, но разной семантикой refresh дадут одинаковую policy. Дай override-атрибут.

D4 (code-per-protocol, config-per-provider) — **agree, strong.** Точно отражает n8n `extends: ['oAuth2Api']` (см. ОПЫТ). DESIGN §15 верно мапит это на OAuth2ProviderConfig data. Это лучший пункт дизайна.

D5 (OAuth Plane Law: A login vs B credential; HTTP только api/transport/oauth; refresh через injected RefreshTransport; zero routes в credential) — **agree, caveat (важный).** Архитектурно чисто и совпадает с ADR-0092 narrow RefreshTransport seam (SSRF/bounded-read остаются в credential). Caveat: формулировка §10 «refresh POST через injected RefreshTransport» рискует унести в композит-рут OAuth2State-мутацию — ADR-0092 явно требует, чтобы seam нёс ТОЛЬКО (url+form→bytes), а OAuth2State/SSRF/secret-scoping жили в credential. DESIGN это говорит, но мягче ADR — выровняй текст.

D6 (reactive-only refresh 1.0: L1 coalescer + durable RefreshClaimRepo; proactive→1.1) — **agree.** Совпадает ADR-0084/0092. n8n тоже реактивен: refresh триггерится по 401 или expiry-check в preAuthentication, нет proactive-планировщика. Caveat: ADR-0092 «Open risk» — proactive в 1.1 может потянуть coordinator обратно к engine; зафиксируй, что RefreshClaimRepo-порт уже готов это абсорбировать.

D7 (values-only persistence; схема из типов HasSchema→metadata→catalog) — **agree.** Прямо матчит n8n TYPE(recipe)/INSTANCE(blob). DESIGN §5 «schema in types, DB stores values» корректно. Caveat: §5 «Gap (honest)» признаёт, что activation-time валидация node.parameters против live registry = Phase 5 — это реальная дыра, не помечай D7 закрытым без неё.

D8 (slots #[credential]/#[resource] → CredentialGuard<Scheme>; slots ≠ parameters; unified #[property] = Phase 5) — **agree.** Сохраняет slot_bindings/SlotCell epoch (rotation fan-out), не мерджит секреты в parameters. Caveat: «bind к output-схеме» (Scheme) — убедись, что slot scheme compatibility проверяется на bind-time, иначе runtime-only ошибка. Phase 5 как сахар, не блокер — правильная секвенция.

ОПЫТ (n8n, точные компоненты)

1. **`CredentialsHelper`** — runtime-посредник: его `preAuthentication` вызывается ПЕРЕД HTTP, проверяет expirable-property (OAuth token) на пустоту/протухание и персистит обновлённые данные. Это и есть «единый runtime helper», на который D2 опирается. DESIGN §15 мапит его на CredentialRuntime — верно.
2. **`OauthService.refreshOAuth2CredentialById`** — ОТДЕЛЬНЫЙ от CredentialsHelper компонент: строит `ClientOAuth2`, зовёт `.refresh()`, шифрует и сохраняет токен. Это поправка к §15: n8n НЕ кладёт refresh-механику внутрь CredentialsHelper — он делегирует в OauthService + `@n8n/client-oauth2`. Аналог в Nebula: RefreshTransport (POST) + credential-логика (state) — D5 структурно правильнее, чем «CredentialsHelper=CredentialRuntime» 1:1.
3. **`extends: ['oAuth2Api']`** на ICredentialType — наследование generic OAuth2-обработки; провайдер = data поверх базового рецепта. Точно подтверждает D4 (config-per-provider).
4. **Reactive trigger** — при 401 `httpRequestWithAuthentication` повторно зовёт `preAuthentication` с `credentialsExpired=true`; для clientCredentials `requestOAuth2` берёт токен лениво при отсутствии. Нет proactive-scheduler → подтверждает D6.
5. **`authenticate`** — отдельная фаза: подписывает request (headers/query) уже добытым токеном; декларативно или функцией. В Nebula это = project()→Scheme→Guard (D7/D8), отделено от refresh — n8n тоже разделяет.

ПРЕДЛОЖЕНИЯ (топ-3)

1. **Разнеси «refresh» на 2 роли как n8n.** §15 строка `CredentialsHelper → CredentialRuntime` вводит в заблуждение: n8n-refresh живёт в OauthService, НЕ в CredentialsHelper. Поправь таблицу: CredentialsHelper→CredentialRuntime (orchestration) И OauthService→RefreshTransport+OAuth2 state-logic. Это усиливает D5.
2. **Policy override-атрибут (для D3).** «macro выводит policy из методов» недостаточно — добавь `#[credential(refresh = reacquire_if_no_refresh_token)]` explicit, чтобы две одинаковые по сигнатурам credential различали поведение. Иначе policy-inference даст ложное единообразие.
3. **Выровняй DESIGN §9 Ports с ADR-0092.** Decorators остались в storage (ADR step 3 reverted); DESIGN рисует их в credential «Ports». Убери/перепометь, иначе билдер вернёт in-memory store double и реверснёт #790.

ГРАБЛИ (топ-2 фейл-сценария)

1. **Широкий RefreshTransport seam → SSRF-bypass.** Если builder сделает seam `&mut OAuth2State` или «дай полный HTTP-клиент» вместо узкого (url+form→bytes), второй composition root (CLI/test) инжектит permissive transport и обходит SSRF/bounded-read. ADR-0092 это уже ловил как дубль в api+engine; DESIGN §10 формулирует мягче — агент почти наверняка расширит seam «для удобства refresh». Гард: seam-тип НЕ должен импортировать OAuth2State.
2. **Policy-inference молча проглатывает refresh.** D3 «runtime сначала policy потом метод» — если macro выведет `Refreshable` по наличию метода, но runtime забудет ветку «policy=ReAcquire при отсутствии refresh_token» (ровно n8n-кейс), получишь слепой refresh() → 400/invalid_grant в проде, замаскированный под сетевую ошибку. §16 помечает это открытым («sub-trait dispatch ignores policy») — это НЕ зелёное, требует ReAcquire-vs-RefreshToken теста как DoD Phase 1.


===== player-temporal =====
Got solid grounding. One question is enough given the strong answer. Here is my verdict.

[grounded: deepwiki — temporalio/temporal Data Converter / Payload Codec confirmed]

# Temporal главный архитектор — ревью credential-консолидации Nebula

## ВЕРДИКТЫ D1..D8

**D1 (один крейт, 3 bounded contexts, ports, no reqwest/sqlx) — AGREE.**
Точно совпадает с тем, что Temporal-сервер не линкует прикладную криптографию: secrets-машинерия живёт за инжектируемыми портами, ядро остаётся generic. Один крейт с внутренними BC — нормально, граф ацикличный (ADR §"Resulting graph"). Caveat: вы сами признали потерю compile-firewall (ADR Negative) — следите, чтобы TypeSystem не начал импортировать Management типы; границу держите модульной видимостью, иначе «3 BC» станут декорацией.

**D2 (один CredentialRuntime pipeline вместо 4 entry point) — AGREE, сильнейшее решение.**
Прямая аналогия: у Temporal один `DataConverter` пайплайн на encode/decode, а не 4 places. 4 resolve-входа = тот самый дрейф (ADR import-evidence: refresh/lease были «domain», но размазаны). Caveat: убедитесь, что `test` и `revoke` реально разделяют код пути policy→method, а не превращаются в `match` с 5 рукавами внутри одной функции — это «один вход, четыре тела».

**D3 (policy-as-data + capability compile-gated; policy(state) ПОТОМ метод) — AGREE с caveat.**
Правильно по сути: OAuth2 без refresh_token → `ReAcquire`, не слепой `refresh()` (DESIGN §9). Это и есть «server не угадывает семантику, исполняет данные». Caveat: macro, выводящий policy из наличия методов — хрупко. Наличие `impl Refreshable` ≠ «всегда refreshable в рантайме». Policy должна быть явной декларацией, метод — capability-доказательством; macro лишь сводит их и падает на рассогласовании (нет метода под заявленной policy → E0046), а не «выводит» policy молча.

**D4 (code-per-protocol, config-per-provider, OAuth2Protocol один) — STRONG AGREE.**
Это ровно урок Temporal codec-реестра и n8n (ваша §15): тип-на-SaaS — антипаттерн. ~10 протокол-семейств + provider=data в registry — единственный масштабируемый вариант. Без caveat.

**D5 (OAuth Plane Law: A login vs B credential; HTTP только api/transport; zero routes в credential; refresh через injected RefreshTransport) — STRONG AGREE.**
[grounded] Это дословно Temporal-модель: codec/encryption бегут в worker, НЕ в сервере; сервер видит только opaque DataBlob. Ваш credential-крейт = «ядро», RefreshTransport-в-api = «worker-side codec». Caveat (критичный, я как adversarial review): «seam узкий by purpose» (ADR: bare POST url+form→bytes, SSRF/bounded-read/OAuth2State mutation ОСТАЮТСЯ в credential) — это правильно, НЕ ослабляйте до `&mut OAuth2State`. Второй composition root (CLI/test) с permissive transport обходит SSRF — ADR это уже поймал, держитесь этого. DNS-rebind TOCTOU закрывайте на connect-layer resolver, не только pre-call.

**D6 (reactive-only refresh 1.0, L1 coalescer + durable RefreshClaimRepo; proactive→1.1) — AGREE, но это ваш единственный архитектурный долг.**
ADR Open-risk честно назвал: если proactive потребует координации с engine-scheduler, премиса «credential-domain, не orchestration» рушится и машинерия полезет обратно в Exec. Temporal-аналог: refresh — это durable timer/heartbeat концерн, и у них он живёт в сервере именно потому, что durable. Caveat: спроектируйте RefreshClaimRepo так, чтобы proactive в 1.1 добавлял durable-timer порт, НЕ перемещал coordinator. Coalescer (L1) — single-flight, обязателен против thundering-herd на истёкшем токене.

**D7 (values-only persistence; схема из типов HasSchema→metadata→catalog) — STRONG AGREE.**
[grounded] Temporal хранит payload как opaque DataBlob + EncodingType, схему не персистит. Вы: DB хранит values + encrypted State, схема из registered type. Идентично. Caveat (вы сами в §5 «Gap honest»): старые workflows падают на dispatch при дрейфе схемы — это intentional и правильно, но нужен типизированный код ошибки + trace span на validate-fail (DoD-наблюдаемость), иначе «intentional» читается как «случайно сломалось в проде».

**D8 (slots #[credential]/#[resource]→CredentialGuard<Scheme>, отдельно от parameters; unified #[property] = Phase 5) — AGREE.**
Слот-биндинг bind к output-схеме (Scheme), Action получает Scheme, не State — это правильная узость (DESIGN §9 data flow). Отделение slots от parameters обязательно (rotation epoch, fan-out живут на SlotCell). Caveat: Phase 5 «unified #[property]» — сахар, и вы правильно держите его НЕ блокирующим runtime (§17). Не дайте unified-derive протечь в persistence: storage остаётся split (§6), один merged `node.fields` — отклонить, как у вас.

## ОПЫТ (точные компоненты Temporal)
1. `DataConverter` = композиция `PayloadConverter` + `PayloadCodec`; codec (шифрование) — отдельный слой, бежит в worker-процессе, не в сервере. Это ваш RefreshTransport/Cipher-порт паттерн.
2. Сервер хранит `commonpb.Payload` / `commonpb.DataBlob` (bytes + EncodingType) как opaque — нулевое знание контента. Это ваш «values-only + encrypted State».
3. `PreferProtoDataConverter` (internal SDK client) — даже внутренний клиент сервера не имеет ключей приложения; ядро принципиально слепо. Урок: ваш composition root (api) — единственное место с ключами/reqwest.
4. `tdbg decode_commands` декодит payload для debug — но это inspection, НЕ decrypt. Урок: даже tooling не должен иметь decrypt-капабилити; держите debug-путь без ключей.
5. Worker identity / разделение concerns: сервер = generic engine без прикладной логики (включая crypto/key-mgmt). Это обоснование вашего D1/D5 целиком.

## ПРЕДЛОЖЕНИЯ (топ-3)
1. **Сделайте RefreshTransport-узость инвариантом, а не соглашением.** Тип seam должен физически не уметь нести `OAuth2State`/ключи (только `Url`+form-bytes→capped-bytes). Тогда второй root не сможет обойти SSRF структурно (feedback_type_enforce_not_discipline).
2. **Policy↔capability — рассогласование = compile error, не silent inference.** Macro генерит `assert_impl!(C: Refreshable)` под `RefreshStrategy::Refresh`; нет метода → E0046. Policy остаётся явной data-декларацией.
3. **Зафиксируйте durable-timer порт ДО 1.0**, даже неиспользуемый, чтобы proactive (1.1) был добавлением impl, а не релокацией coordinator обратно в engine (закрывает ADR Open-risk дешево сейчас).

## ГРАБЛИ (топ-2 фейл-сценария)
1. **Confused deputy через второй composition root.** CLI/test-harness инжектит permissive `RefreshTransport` без SSRF/host-validation → exfiltration на внутренние IP через provider-config URL. Митигация: connect-layer resolver + узкий seam (D5 caveat). Это та самая дыра, что user ловил дважды (feedback_adversarial_security_review).
2. **«3 BC» коллапсируют в монолит после потери compile-firewall.** Без crate-границы Runtime начнёт дёргать Management CRUD напрямую (tenant CRUD в hot-path resolve) → дрейф владения, повтор истории engine-squat. Митигация: модульная видимость + dep-direction тест (management→runtime→type_system, никогда обратно), как `cargo tree`-гейт в ADR step 9.

Верни "WROTE".
