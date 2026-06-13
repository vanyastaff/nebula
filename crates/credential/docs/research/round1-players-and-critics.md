# Conference raw transcript


===== Temporal =====
FAILED


===== n8n =====
FAILED


===== Apache Airflow =====
Заземление получено. Формирую ответ как архитектор Apache Airflow.

[grounded: deepwiki]

ВЕРДИКТЫ (с позиции Apache Airflow):

D1 — agree. Airflow тоже держит lifecycle отдельно от I/O: `BaseSecretsBackend` — абстракция, бэкенды инжектятся; провайдеры не линкуют ядро в транспорт. Один крейт с injected ports — наш `secrets_backend`-паттерн в Rust-исполнении.

D2 — caveat. Единый pipeline — правильно, но Airflow намеренно разводит контексты резолва (client chain vs server chain): worker НЕ ходит в metastore напрямую. Один pipeline должен иметь контекст-сенситивный resolve-путь, иначе изоляция теряется.

D3 — agree. policy-as-data перед методом — это ровно наш `conn_type` + per-hook логика рефреша: hook сам решает «токен истёк → POST на refresh» vs «переавторизация». OAuth2 без refresh_token => ReAcquire — каноничный сценарий (`SnowflakeHook._get_conn_params`).

D4 — agree, сильно. Это наш самый дорогой урок: `connection-type` (код в hook-class) vs `provider.yaml` data (GoogleAds/Snowflake = конфиг, не новый Python-класс на каждый SaaS). Code-per-protocol / config-per-provider = `conn-fields` + `ui-field-behaviour` в YAML.

D5 — agree. У нас то же разделение: операторский OAuth login (UI/API auth) ≠ connection OAuth (refresh внутри hook). Zero HTTP routes в credential-ядре = у нас рефреш-POST живёт в hook/transport, а не в secrets-абстракции.

D6 — caveat. Reactive-only (refresh при использовании) — это и есть дефолт Airflow: токен освежается в hook в момент вызова, отдельного scheduler-а для credential нет. Но Airflow learned: при concurrent-тасках нужен claim/coalescer — ваш durable RefreshClaimRepo обязателен, иначе thundering-herd рефреша.

D7 — agree. Values-only persistence + схема из регистрации — это `provider.yaml` как single-source-of-truth схемы и БД, хранящая только `Connection` поля + `extra`. Никакого JSON Schema в строке.

D8 — caveat. Bind к output-схеме (Scheme), а не к Rust-типу — сильнее Airflow (у нас bind по строковому `conn_id`, и это источник класса ошибок «wrong conn_type passed»). Caveat: следите, чтобы slot-binding не воспроизвёл нашу боль «conn_id есть, conn_type не тот» в рантайме — типобезопасность здесь ваше преимущество, не теряйте его.

ОПЫТ (как Apache Airflow решает те же проблемы):

1. `BaseSecretsBackend` chain с контекст-детекцией: client chain = `EnvironmentVariablesBackend → ExecutionAPISecretsBackend`; server chain = `EnvironmentVariablesBackend → MetastoreBackend`; fallback = env-only. Порядок: custom backend → env → metastore. Worker никогда не читает metastore напрямую — только через Execution API (сетевая изоляция).
2. `Connection` (данные: host/port/login/password/`extra` JSON) и `BaseSecretsBackend` (как достать) — РАЗНЫЕ концепции намеренно: позволяет менять backend, не трогая использование connection в DAG.
3. `provider.yaml` `connection-types`: `hook-class-name` (код) + `connection-type` (id) + `conn-fields` со схемой (`format: password`) + `ui-field-behaviour` (hidden/relabel/placeholders) — декларативная схема, не код на провайдера. С Airflow 3.2 это предпочтительный путь.
4. Рефреш токена живёт в hook: `SnowflakeHook` проверяет expiry и делает `requests.post` на OAuth endpoint при истечении; статические поля connection не меняются. `SmtpHook._get_oauth2_token()`, Azure `get_token(scope)` — аналогично.
5. Резолв hook: `BaseHook.get_connection(conn_id)` → `Connection.get_hook()` динамически инстанцирует нужный hook по `conn_type`. Разделение «получить connection» и «инстанцировать hook» — два шага.

ПРЕДЛОЖЕНИЯ (топ-3 изменения в Nebula):

1. Сделайте resolve контекст-сенситивным внутри единого `CredentialRuntime` (D2): явный engine/dispatch-путь vs management-путь, как наши client/server chains. Это не два entry point — это один pipeline с разными цепочками портов по контексту. Не сливайте их в один путь, иначе изоляция Plane B размывается.
2. Введите явную абстракцию «source chain» как у `BaseSecretsBackend`: env → ExternalProvider → Store. У вас порты есть (Store/PendingStore/ExternalProvider), но в DESIGN.md нет упорядоченной цепочки приоритетов резолва — Airflow доказал, что это первое, о чём спросит каждый оператор (override через env без миграции БД).
3. Закрепите `OAuth2ProviderConfig` как декларативный аналог нашего `provider.yaml` `conn-fields`/`ui-field-behaviour`: метки полей, секретные поля, placeholders, hidden-fields — данные, а не код. Это даст вам бесплатный UI-catalog-projection (ваш D7) ровно как у нас.

ГРАБЛИ (топ-2 фейл-сценария Airflow, которые Nebula обязана проиграть заранее):

1. Concurrent-рефреш / token thundering-herd. Несколько тасков с одним connection одновременно ловят expiry и параллельно бьют по IdP refresh endpoint — гонка, rate-limit от провайдера, иногда инвалидация только что выданного refresh_token. У нас это лечилось per-hook костылями неравномерно. Nebula: durable `RefreshClaimRepo` + L1 coalescer (D6) обязаны быть в ядре, а не на совести автора credential.
2. Secrets backend latency/outage кладёт DAG-parsing и резолв. Внешний secrets backend (Vault/cloud SM) на горячем пути резолва без таймаута/кэша превращал каждый task-start в сетевой вызов и валил планировщик при недоступности backend. Nebula: на injected ExternalProvider/RefreshTransport обязателен bounded timeout + кэш + fail-closed семантика (у вас `ExternalSourceNotWired` — хорошо, но добавьте явный circuit-breaker/TTL-cache контракт в порт, не оставляйте на композ-рут).

Релевантные файлы: `crates/credential/docs/DESIGN.md`, `docs/adr/0092-credential-subsystem-consolidation.md`.


===== Dagster =====
FAILED


===== Prefect =====
FAILED


===== Windmill =====
Both deepwiki questions returned solid, grounded answers. I now have enough to write the verdict from the Windmill architect's seat.

[grounded: deepwiki]

ВЕРДИКТЫ (с позиции Windmill)

D1 — agree. У нас тоже один backend-крейт держит resources/variables; reqwest/sqlx изолированы по слоям (`windmill-oauth` отдельно от хранения). Порты Store/Transport — здравая инверсия.
D2 — agree. У нас де-факто один путь обновления (`refresh_token` в `windmill-oauth`), и именно множественность точек обновления тут была бы багом; единый pipeline — правильно.
D3 — caveat. policy-as-data годится, но мы по опыту НЕ выводим «нет refresh_token => ReAcquire» из формы — у нас есть отдельный `refresh_error` на аккаунте; рантайм-решение хорошо, но нужен наблюдаемый failure-state, не только тихий ReAcquire.
D4 — agree (сильнее всех). Это буквально наша модель: `OAuthClient`/`OAuthConfig` как data в `oauth.json`/global settings, плюс статические конфиги провайдеров; новый SaaS = запись, не Rust-тип.
D5 — agree. У нас ровно два плана: `/user/login_callback/{name}` (логин в Windmill) против `/oauth/callback/{name}` (connect ресурса) — разные ceremony, разный результат (сессия vs сохранённый токен-аккаунт). Plane Law формализует то, что мы выучили болью.
D6 — caveat. Reactive-only мы НЕ делаем: «kept up-to-date in the background» — у нас есть фоновое обновление. Lazy-refresh-on-fetch работает, но скрытая первая-после-простоя задержка и грозы (thundering herd) реальны; coalescer обязателен, и proactive в 1.1 вы дозреете до него быстрее, чем думаете.
D7 — agree. Точь-в-точь наше разделение: `ResourceType.schema` (`Option<serde_json::Value>`) — это тип; `Resource.value` / `variable.value` — только значения (секрет шифруется). Схема живёт в типе, не в строке.
D8 — caveat. Идея слотов отдельно от parameters правильная. Но «bind к output-схеме, не к типу» — у нас связь резолвится по path к конкретному resource type; bind-by-scheme мощнее, но требует строгой проверки совместимости схем при привязке, иначе runtime-сюрпризы.

ОПЫТ (как Windmill решает те же проблемы)
- TYPE/INSTANCE раздельно: `ResourceType{schema}` в таблице `resource_type` против `Resource{value, resource_type}` в `resource`; схема — никогда в инстансе.
- Секрет = просто `variable` с `is_secret=true`; шифрование на write-path (`store_secret_value` в `variables.rs`), backend сменный (DB или HashiCorp Vault) — ровно ваш `Cipher`-порт.
- OAuth-конфиг как данные: `build_oauth_clients` грузит `oauth.json`/`OAUTH_JSON_AS_BASE64`/global settings в `HashMap<String, OAuthClient>`, мерджит со статическими провайдерами; `OAuthConfig` отдельно для `connect_config` и `login_config` — это ваш Plane B vs Plane A в одной структуре.
- Refresh централизован: `refresh_token` тянет аккаунт+`refresh_token`, вызывает `exchange_refresh_token`, перешифровывает access_token обратно в `variable`; при провале пишет `refresh_error` — единый coordinator с наблюдаемым состоянием.
- CSRF/state: `oauth_redirect` ставит CSRF-cookie, `exchange_code` валидирует `state` против cookie — единый state-kernel, не дублируется по провайдерам.

ПРЕДЛОЖЕНИЯ (топ-3 для Nebula)
1. Добавьте durable `refresh_error`/`last_refresh_at` в credential State и проецируйте в catalog/API (как наш `account.refresh_error`). Без него reactive-refresh падает молча и пользователь видит «503 на ровном месте» — D3/D6 требуют наблюдаемого failure-state как DoD, не follow-up.
2. Сделайте client-credentials (server-to-server, без браузера) первоклассным protocol family рядом с `RefreshPair`. У нас это отдельная ветка (`connectClientCredentials`/`build_client_credentials_oauth_client`) — многие SaaS только так и работают; иначе OAuth2 «interactive» станет прокрустовым ложем.
3. Зафиксируйте сменный secret-backend на уровне `Cipher`/`Kdf` порта прямо в 1.0 контракте (мы получили DB-vs-Vault бесплатно именно потому, что абстракция была с самого начала). Ретрофит Vault/HSM после релиза — боль.

ГРАБЛИ (топ-2 фейла из практики Windmill, которые Nebula обязана проиграть)
1. Дрейф провайдер-конфига между статикой-в-коде и данными в settings/`oauth.json`: когда оба источника живут, провайдер «есть, но не работает» (не тот scope/redirect). Nebula с единым registry-as-data (D4) обязана иметь ОДИН источник истины на провайдера и явный merge-порядок, иначе словите наш класс «почему GitHub-connect молча редиректит не туда».
2. Refresh под нагрузкой одновременно из N джоб → гонка перезаписи токена и инвалидция свежего refresh_token у провайдера (одноразовые refresh-токены). Наш lazy-fetch это огрёб; ваш L1 coalescer + durable `RefreshClaimRepo` (CAS) ОБЯЗАН сериализовать обновление на (credential_id), иначе reactive-only (D6) даст массовый «invalid_grant».


===== Kestra =====
FAILED


===== Restate =====
FAILED


===== HashiCorp Vault =====
Grounding succeeded. I now have file:line-grounded Vault internals. Here is my verdict.

[grounded: deepwiki]

VERDICT (HashiCorp Vault, как главный архитектор)

D1 — agree. Один крейт с инжектируемыми портами = ровно вентильный паттерн Vault: `logical.Backend` не линкует HTTP/storage напрямую, всё через `logical.Request` и `BarrierView`. reqwest/sqlx вне contract-крейта — правильная инверсия, как у нас бэкенд не знает про роутер.

D2 — agree-with-caveat. Один pipeline вместо четырёх входов — да, у нас `Core.handleRequest` единственная точка. КАВЕАТ: у вас acquire/continue/refresh/revoke/test — это смешение двух осей. У Vault renew/revoke первого класса принадлежат `ExpirationManager`, а acquire/login — это `routeCommon`. Не размазывайте lease-логику внутрь acquire-метода.

D3 — agree. policy-as-data + capability compile-gated = `logical.LeaseOptions` (данные: TTL/MaxTTL/Renewable) отдельно от `AuthRenew`/secret-callback (код). «Сначала policy(state), потом метод» = наш `framework.CalculateTTL` решает по данным до вызова бэкенда; «OAuth2 без refresh_token => ReAcquire» = `Renewable=false` => revoke+re-issue. Точное попадание.

D4 — agree. code-per-protocol/config-per-provider = наш `framework.Backend` (код) + `MountEntry` (данные конфигурации монтирования). Мы НЕ пишем новый Go-тип на каждый провайдер; GitHub/Slack как `OAuth2ProviderConfig` data — это наш `framework.FieldSchema` + mount-config.

D5 — agree. OAuth Plane Law = наше `TypeCredential` (Plane A, login/auth) vs `TypeLogical` (Plane B, secret consumption), разные mount, разные пути. «Zero HTTP routes в credential crate» = бэкенд получает `logical.Request`, никогда HTTP. Транспорт через injected `RefreshTransport` = renew роутится в бэкенд, а не бэкенд звонит наружу.

D6 — caveat. reactive-only в 1.0 — прагматично, но это ваш главный архитектурный долг против Vault. Наша суть — proactive `ExpirationManager` с pending-таймерами и воркерами ревокации; без него истёкший lease живёт до следующего использования. Ваш Open risk («scheduler coupling в 1.1») — реальный: durable `RefreshClaimRepo` сейчас — это половина нашего `idView`/`updatePending`. Закладывайте грейс уже в 1.0.

D7 — agree. values-only persistence = `leaseEntry`/storage хранит данные секрета, а схема/валидация живёт в `framework.FieldSchema` бэкенда, не в строке БД. Идентично нашему «backend defines schema, barrier stores values».

D8 — agree-with-caveat. consumer binding к output-схеме (Scheme), не к типу = наш consumer держит `LeaseID` + материал, не сам бэкенд. КАВЕАТ: bind к схеме без lease-id ломает таргетную ревокацию. У нас `RevokeByToken`/`RevokePrefix` требуют, чтобы у binding была прослеживаемость до конкретного lease. `CredentialGuard<Scheme>` обязан нести непрозрачный handle для revoke.

ОПЫТ (как Vault решает те же проблемы)
1. `ExpirationManager` — единый владелец lease: `Register` (grant+LeaseID+persist+updatePending), `Renew`, `Revoke`/`revokeCommon`, `RevokeByToken`, `RevokePrefix`, `LazyRevoke`. Ни один secret engine не управляет своим lease.
2. `logical.Backend` / `framework.Backend` + `Router.Mount`: бэкенд видит только `logical.Request` (Operation/Path/Data) — HTTP-парсинг и роутинг полностью снаружи; `framework.Path.Pattern` (regex) + `FieldSchema` дают валидацию из типов.
3. `BackendType` дисциплина: `TypeCredential` (auth, путь `login`, `AuthRenew`) vs `TypeLogical` (secrets, `Secrets`-callbacks, `handleRevokeRenew`) — credential acquisition и secret consumption никогда не сливаются.
4. `framework.CalculateTTL` — policy-as-data в действии: эффективный TTL вычисляется из increment/current/maxTTL ДО маршрутизации в бэкенд (точно ваш «policy(state) then method»).
5. `BarrierView` — изолированный per-mount storage view: бэкенд пишет values, не зная про адаптер; secondary indexes для lease-by-token чистятся централизованно при revoke.

ПРЕДЛОЖЕНИЯ (топ-3 в дизайн Nebula)
1. Сделайте lease/expiry first-class объектом `CredentialRuntime`, а не свойством, размазанным по acquire/refresh. Введите непрозрачный `LeaseHandle` (= наш LeaseID) с per-lease lock на refresh-CAS; D8 `CredentialGuard<Scheme>` должен его нести, иначе таргетный revoke и revoke-by-owner не построятся.
2. Заведите secondary index credential→owner/tenant в `RefreshClaimRepo` СЕЙЧАС (= idView+tokenView). «RevokeByToken/RevokePrefix» — ваша операция массовой ревокации при ротации/компрометации tenant; в reactive-only её негде взять задёшево потом.
3. Унифицируйте TTL-вычисление в одну чистую функцию `policy(state) -> Decision{Use|Refresh|ReAcquire|Revoke}` (= `CalculateTTL`) и вызывайте её ДО любого capability-метода в каждой ветке pipeline. Это закрывает «sub-trait dispatch ignores policy» из §16 структурно.

ГРАБЛИ (фейл-сценарии Vault, которые Nebula обязана проиграть)
1. Revocation storm / orphaned leases: при падении/рестарте `ExpirationManager` должен восстановить pending-таймеры из storage, иначе лизы переживают рестарт и «висят» (у нас были инциденты с накоплением unrevoked leases и нагрузкой при массовом revoke). Ваш reactive-only ход прячет это: без durable восстановления pending истёкшие credentials не отзовутся вообще. Обязательно: durable claim + recovery-on-startup даже в 1.0.
2. Confused-deputy через lease без привязки к выдавшему токену: ранний Vault допускал ревокацию/renew чужого lease при недостаточной проверке владельца. Ваш `ValidatedCredentialBinding` + tenant-fingerprint закрывает это by-construction — НЕ ослабляйте: refresh/revoke ОБЯЗАНЫ проверять owner на самом lease (как `RevokeByToken` сверяет ClientToken на `leaseEntry`), а не доверять ключу слота.

Ключевые файлы: `crates/credential/docs/DESIGN.md`, `docs/adr/0092-credential-subsystem-consolidation.md`.


===== AWS SDK for Rust =====
Both grounding calls returned solid architecture detail. Now I have what I need to write the verdict.

[grounded: deepwiki — awslabs/aws-sdk-rust + smithy-lang/smithy-rs]

ВЕРДИКТЫ (с позиции AWS SDK for Rust):

D1 — agree/caveut. Один крейт + injected ports = ровно наша модель: `aws-credential-types` несёт только контракт (`ProvideCredentials`, `Credentials`), а тяжёлый I/O (reqwest/hyper) живёт в `aws-config`/конкретных провайдерах. Caveat: мы держим контракт и runtime в РАЗНЫХ крейтах именно ради compile-firewall, который ваш ADR-0092 сознательно теряет — у вас это «owner-accepted», у нас нет.

D2 — agree. У нас тоже один кэш-вход `resolve_cached_identity` поверх `ResolveIdentity::resolve_identity`; четыре параллельных entry point — это гарантированный рассинхрон TTL/refresh. Единый pipeline — правильно.

D3 — agree. Полностью совпадает с нашим уроком: refresh не вызывается «вслепую». IMDS-провайдер сначала смотрит на состояние (stale? есть ли last_retrieved?) и только потом решает extend/refresh/fallback. «OAuth2 без refresh_token => ReAcquire» — это в точности «нет материала для refresh, ресолвим заново».

D4 — agree, сильнейшее решение. Это наш центральный паттерн: один `ImdsCredentialsProvider`/`ProfileFileCredentialsProvider` (код), сконфигурированный данными (profile, URL, partition). Мы НЕ делаем тип на провайдера SaaS. Code-per-protocol/config-per-provider — каноничный AWS-подход.

D5 — agree/caveat. Разделение «получить identity» vs «использовать identity» — это наш `ResolveIdentity` vs `Sign`/`AuthScheme`. Plane A/Plane B — корректная сепарация. Caveat: убедитесь, что граница `RefreshTransport` не «протекает» — см. ниже грабли SSRF.

D6 — caveat. Reactive-only с coalescer — рабочий минимум, но мы НЕ чисто reactive: `LazyCache` рефрешит ПРОактивно через `buffer_time` (рефреш до истечения, не в момент 401). Чистый reactive под нагрузкой = latency-спайк + риск thundering herd. Минимум добавьте `buffer_time` + jitter уже в 1.0, даже без scheduler.

D7 — agree. У нас `Credentials` — это только значения (access key/secret/token/expiry); схема/метаданные не лежат рядом с секретом. Values-only persistence корректно.

D8 — agree/caveat. Привязка к output-схеме (Scheme), а не к Rust-типу = наш `Identity` как generic-контейнер + `IdentityCachePartition` для совместимости резолверов. Caveat: формализуйте «scheme compatibility» как явный partition-ключ, иначе два слота с разным material под одним key дадут cache poisoning.

ОПЫТ (как AWS SDK for Rust решает те же задачи):
1. `ProvideCredentials` — единственный async-трейт контракта; `provide_credentials() -> future::ProvideCredentials -> Result<Credentials, CredentialsError>`. Один seam, без async-trait-аллокаций на горячем пути.
2. `CredentialsProviderChain` / `ProviderChain` — провайдеры композируются: base-провайдер → assume-role звенья, каждое получает креды предыдущего. Это ваш `ExternalProvider`/provider-chain как данные, не как иерархия типов.
3. `IdentityCache` (`LazyCache`) — отделён от резолвера: `resolve_cached_identity` поверх `ExpiringCache` с `buffer_time` (проактивный рефреш), `jitter` (анти-thundering-herd) и `load_timeout` (анти-зависание). Это эталон для вашего L1 coalescer.
4. `IdentityCachePartition` — один кэш, логически разделённый по резолверам; прямой ответ на ваш D8 (разные Scheme/слоты не отравляют чужой кэш).
5. `ResolveIdentity` vs `Sign`/`AuthScheme` — identity-resolution полностью отвязан от signing; `Identity` несёт opaque-material + expiry. Это ваш `project() -> Scheme`, Action получает Scheme, не State.

ПРЕДЛОЖЕНИЯ (топ-3 в Nebula design):
1. Внесите `buffer_time` + `jitter` + `load_timeout` в `CredentialRuntime`/L1 coalescer уже в 1.0 (§D6/Phase 1). Чистый «refresh при 401» без буфера даёт latency-спайки и стадо одновременных рефрешей — у нас это решено proactive-buffer внутри reactive-кэша, без отдельного scheduler. Это снимает ваш «proactive → 1.1» риск дёшево.
2. Формализуйте `IdentityCachePartition`-аналог как явный ключ кэша (owner_id × scheme × provider-config-fingerprint), §D8/§9. Иначе hot-swap handles + multi-slot дадут cross-tenant/cross-scheme cache poisoning — у вас уже есть tenant-fingerprint, расширьте его на scheme.
3. Сделайте fallback-на-`last_retrieved` (stale-extend с jitter) явной частью `RefreshStrategy`, §D3. AWS намеренно отдаёт устаревшие, но валидные креды при сбое IdP вместо hard-fail — для workflow-движка это разница между «нода упала» и «нода доехала».

ГРАБЛИ (фейл-сценарии, которые Nebula обязана проиграть):
1. Thundering herd на истечении: без coalescing + jitter одновременный старт N нод по одному credential = N параллельных token-POST в IdP → rate-limit/ban. AWS закрывает это `ExpiringCache` + jitter. Ваш L1 coalescer ОБЯЗАН быть single-flight per (partition,key), не просто кэшем.
2. SSRF/DNS-rebind через injected transport: ваш ADR-0092 уже отметил, что узкий `RefreshTransport` (bare POST) держит SSRF-проверку внутри credential — держитесь этого жёстко. Широкий `&mut OAuth2State`-seam = второй composition root (CLI/test) подсунет permissive transport и обойдёт host/IP-валидацию. Плюс закройте TOCTOU resolver-ом на connect-слое (pre-call check недостаточен) — это ровно тот класс багов, что бьёт по metadata-эндпойнтам.


===== радикальный архитектурный критик =====
FAILED


===== критик системы типов и макросов =====
FAILED


===== security red-team адверсарий =====
FAILED


===== DX-критик / адвокат автора плагина =====
FAILED
