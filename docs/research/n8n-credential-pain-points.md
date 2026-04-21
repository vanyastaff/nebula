# n8n Credentials & Auth — Field Report of Pain Points

> Выжимка реальных жалоб пользователей n8n на credentials/auth: GitHub issues (open + closed),
> community forum, discussions. Дополняет [n8n-auth-architecture.md](./n8n-auth-architecture.md)
> — там «как устроено», тут «что ломается и почему».

## Метаданные исследования

- **Последняя сверка:** 2026-04-20
- **Источники:**
  - `github.com/n8n-io/n8n` issues (open + closed, 11 keyword queries, ~300 titles scanned, 2023–2026)
  - `community.n8n.io` — форумные треды
  - DeepWiki семантический обзор
- **Охват:** backend + user-visible UX; Cloud-specific vs self-hosted отмечено где релевантно.
- **Провенанс:** каждое утверждение подкреплено issue-URL или forum-thread URL. Там где
  проблема воспроизведена несколько раз — указано «confirmed», где сообщено единично — «reported».

---

## Executive Summary — топ-5 болевых областей

В порядке убывания частоты упоминаний:

1. **OAuth2 refresh token handling хрупкий.** Десятки closed issues, много open.
   Microsoft/Azure «token expires after 1 hour», Google Drive `Dummy.stack.replace`
   маскирует real errors, Generic OAuth2 Client Credentials *никогда не рефрешит*,
   и подтверждённый **race condition** на concurrent refresh (one-time refresh
   token consumed twice). Юзеры вынуждены нажимать «reconnect» руками.

2. **`N8N_ENCRYPTION_KEY` rotation практически не поддержана.** Нет in-place rotation.
   Попытки → `error:1C800064:Provider routines::bad decrypt`, instance не бутится,
   enterprise-фичи (External Secrets, SSO, Environments) ломаются независимо.
   Документированная процедура = export-decrypted → wipe → reimport → downtime + потеря
   секретов если старый ключ уже потерян.

3. **SSO конфиг хрупкий и оставляет orphan state в DB.** SAML toggle'ы тихо откатываются,
   error «Cannot switch SAML login enabled state when other method is active» требует
   `DELETE FROM settings WHERE key='features.oidc'` напрямую в Postgres,
   license expiry не выключает SSO — lockout.

4. **OAuth flow на n8n Cloud специфично часто ломается.** HTTP-vs-HTTPS redirect,
   `redirect_uri_mismatch`, «verified for youtube.upload scope» блоки,
   `Dummy.stack.replace` placeholder утечка в real errors. Это столько же shared-OAuth-app
   проблема, сколько code problem.

5. **MFA/login lockouts с минимальной recovery ergonomics.** Нет 2FA-поля на login screen
   после update, 2FA timer expires до того как юзер успеет сохранить recovery codes,
   `n8n mfa:disable --email=...` — единственный escape hatch (требует shell access).

Вторично, но заметно: community-node credentials утекают между workflow;
git-pull стирает OAuth creds; missing «transfer credential» UX; external-secrets
expression parser ломается на bracket-notation; public API keys изначально
стораились unencrypted в workflow JSON.

---

## 1. OAuth2 refresh-token failures

Самая объёмная категория. Паттерны:

### Refresh-token rotation не персистится
- [#25926](https://github.com/n8n-io/n8n/issues/25926) — Generic OAuth2 не сохраняет
  *новый* `refresh_token` из refresh-response → следующий refresh использует
  consumed token → `invalid_grant`. Помечен can't-reproduce, но симптомы matched у других providers.

### Client Credentials flow не рефрешит
- [#17450](https://github.com/n8n-io/n8n/issues/17450), [#18517](https://github.com/n8n-io/n8n/issues/18517), [#24405](https://github.com/n8n-io/n8n/issues/24405)
  — когда сервер возвращает 403 (а не 401) на expiry, `preAuthentication` не детектит
  «token stale» и не рефрешит.

### Race condition на concurrent refresh
- [#13088](https://github.com/n8n-io/n8n/issues/13088) — **confirmed**. HTTP Request
  node с батчем items одновременно hit'ит refresh; первый consumes one-time refresh
  token, остальные fail'ятся. Workaround: «use batch size 1». Нет mutex'а. Issue stale.

### Microsoft/Azure 1-hour expiry
- [#26453](https://github.com/n8n-io/n8n/issues/26453) (open), [#22544](https://github.com/n8n-io/n8n/issues/22544),
  десятки forum-threads включая [Microsoft Azure OAuth2 refresh token not working](https://community.n8n.io/t/microsoft-azure-oauth2-refresh-token-not-working-token-expires-after-1-hour/257628).
- Error маскируется `dummy.stack.replace is not a function` — см. [#23182](https://github.com/n8n-io/n8n/issues/23182),
  [#28055](https://github.com/n8n-io/n8n/issues/28055) — placeholder из
  error-stringification bug утекает в UI.

### Redirect URL всегда HTTP на Cloud
- [#26066](https://github.com/n8n-io/n8n/issues/26066) — Google reject'ит потому что
  registered URIs — HTTPS.
- [#23565](https://github.com/n8n-io/n8n/issues/23565), [#23568](https://github.com/n8n-io/n8n/issues/23568).

### Proxy не honour'ится на refresh
- [#28225](https://github.com/n8n-io/n8n/issues/28225) — `@n8n/client-oauth2` неявно
  передаёт `proxy: false` в axios, corporate proxies ломаются.

### 200-with-error-body не детектится как auth failure
- [#23410](https://github.com/n8n-io/n8n/issues/23410) — provider возвращает HTTP 200
  с `{"code":401,...}`, n8n treats as success → refresh не триггерится.

**Корреляция с архитектурой:** refresh — JIT в `preAuthentication` → stateless → нет
server-side lock, нет retry ladder, нет «healthy credential» background poller.
Write path для `oauthTokenData` без optimistic-lock колонки → два worker'а refresh'ащих
одновременно racing на одной `credentials_entity.data` row.

---

## 2. Encryption key rotation & loss

### Критичные open issues
- [#22478](https://github.com/n8n-io/n8n/issues/22478) (open, Nov 2025) — enterprise
  features (External Secrets, SSO, Environments) ломаются после `ENCRYPTION_KEY`
  rotation, потому что эти фичи хранят data encrypted старым ключом, который
  rotation-скрипт не трогает.
- [#20175](https://github.com/n8n-io/n8n/issues/20175), [#14596](https://github.com/n8n-io/n8n/issues/14596)
  — `N8N_ENCRYPTION_KEY_FILE` не работает (secret-file mount pattern ломается).
- [#8287](https://github.com/n8n-io/n8n/issues/8287) —
  `error:1C800064:Provider routines::bad decrypt` после update (likely `.n8n/config` loss).

### Форум: users bleed out данные
- [Updating N8N_ENCRYPTION_KEY (thread 165334)](https://community.n8n.io/t/updating-n8n-encryption-key/165334)
  — procedure «export decrypted → change key → reimport».
- [Persistent bad decrypt after environment reset (thread 240548)](https://community.n8n.io/t/persistent-bad-decrypt-error-with-credentials-even-after-complete-environment-reset-and-correct-encryption-key-configuration/240548)
  — `bad decrypt` даже после complete environment reset.
- [Problem updating n8n on Docker — losing encryption key (thread 157972)](https://community.n8n.io/t/problem-updating-n8n-on-docker-losing-data-encryption-key/157972)
  — юзеры теряют ключ полностью при Docker updates.
- [#25684](https://github.com/n8n-io/n8n/issues/25684) (closed, feature request) —
  documented что community edition не имеет secure secret store для node-internal use,
  так что юзеры хардкодят API keys в workflow JSON.

**Корреляция:** `credentials_entity.data` — single ciphertext string без versioning /
`kek_id` / JSON envelope (см. architecture doc). Нет способа migrate per-credential.
Worker nodes refuse to boot если env var mismatch с файлом (designed safety, но также
означает что Docker restart с опечаткой = dead instance).

---

## 3. SSO orphan state & license transitions

### Главный кейс
- [#19066](https://github.com/n8n-io/n8n/issues/19066) (closed, 20+ комментариев) —
  «SAML Test работает, Save не работает». Root cause: **предыдущий OIDC trial config
  всё ещё в `settings` таблице**, SAML save reject'ится `setSamlLoginEnabled` с
  `Cannot switch SAML login enabled state when another method is active (current: oidc)`.
  **Fix юзеру сказали:** `DELETE FROM settings WHERE key = 'features.oidc'` напрямую в Postgres.
  Мейнтейнер ответил: *«Sounds like we should introduce a CLI command to disable auth methods»*
  — **ещё не сделано**.

### Остальные
- [#25969](https://github.com/n8n-io/n8n/issues/25969) — SAML metadata endpoint 404
  пока SAML не активирован (deliberate, но сюрпризит юзеров).
- [#19907](https://github.com/n8n-io/n8n/issues/19907), [#18673](https://github.com/n8n-io/n8n/issues/18673)
  — SSO не отключается когда license expire'ится → юзеры lockout.
- [#17399](https://github.com/n8n-io/n8n/issues/17399) — OIDC fail'ится когда IdP
  enforce'ит `state` parameter.
- [#18298](https://github.com/n8n-io/n8n/issues/18298) — Okta OIDC reject'ит потому что
  n8n не шлёт state.
- [#25984](https://github.com/n8n-io/n8n/issues/25984) — OIDC new-user login fail'ится
  после switch на «Instance and project roles».
- [#25166](https://github.com/n8n-io/n8n/issues/25166) — proxy не применяется на
  discovery request.

**Корреляция:** SSO config stored as `settings` KV rows (нет schema validation,
referential integrity, cross-row transitions). License downgrade не трогает
`settings` row. Нет `/rest/sso/disable` management endpoint.

---

## 4. Credential sharing / transfer / lifecycle

- [#21558](https://github.com/n8n-io/n8n/issues/21558) (open) — «Credential sharing
  settings gone missing after upgrading to v1.118.1». User-visible regression.
- [#21382](https://github.com/n8n-io/n8n/issues/21382) — Credential sharing issue.
- [#26499](https://github.com/n8n-io/n8n/issues/26499) — **Каждый Git pull из production
  стирает Gmail OAuth2 credentials.** Юзеры manually re-auth после каждого deploy.
  Потому что source-control sync treats credentials как overwritable, но OAuth
  токены сами не живут в git.
- [#24091](https://github.com/n8n-io/n8n/issues/24091) — credential-sharing dropdown
  unscrollable на macOS.
- [#19798](https://github.com/n8n-io/n8n/issues/19798) — credential export crashes с
  «Cannot read properties of undefined (reading 'slug')».

**Корреляция:** `shared_credentials` sharing project-based, но git-sync (source-control
модуль) оперирует `credentials_entity` row без концепта «runtime-only tokens».
`oauthTokenData` blob внутри `data` стирается на import.

---

## 5. MFA & login recovery

- [#25831](https://github.com/n8n-io/n8n/issues/25831) (open, 2026) — **Can't login
  with MFA enabled, no 2FA field on login screen** после auto-update на 2.7.5.
  Много duplicates.
- [#22637](https://github.com/n8n-io/n8n/issues/22637) — 2FA setup times out до
  того как юзер успеет записать recovery codes.
- [#14275](https://github.com/n8n-io/n8n/issues/14275), [#13244](https://github.com/n8n-io/n8n/issues/13244)
  — 2FA setup causes login 404 после update.
- [#11806](https://github.com/n8n-io/n8n/issues/11806) — 2FA fail'ится из-за
  container timezone mismatch (TOTP needs wall-clock within ±30s).
- [#7907](https://github.com/n8n-io/n8n/issues/7907) — использование TOTP consume'ит
  backup codes (logic bug).

**Recovery — только CLI:** `n8n mfa:disable --email=user@example.com`.
Self-service «email me a recovery code» path нет, если сам не сохранил.

---

## 6. API keys

- [#25684](https://github.com/n8n-io/n8n/issues/25684) — **Workflow-level API keys
  stored unencrypted** когда node не имеет built-in credential type (юзеры хардкодят
  в workflow JSON). Major security gap для community tier.
- [#26642](https://github.com/n8n-io/n8n/issues/26642) — API key scopes не respected.
- [#21054](https://github.com/n8n-io/n8n/issues/21054) — JWT `iat` set to future
  timestamp (clock skew → token treated as future-dated).
- [#16134](https://github.com/n8n-io/n8n/issues/16134) — bug на permissioning для API key.
- [#20354](https://github.com/n8n-io/n8n/issues/20354) — blank screen на API key page.

**Корреляция:** API keys — JWTs с `exp` claim внутри токена. Нет dedicated `expiresAt`
колонки (см. architecture doc) → expiry cleanup парсит каждый token; также нет
per-key revocation list кроме row deletion.

---

## 7. External Secrets (Vault, Azure KV, GCP, 1Password)

- [#28516](https://github.com/n8n-io/n8n/issues/28516) (open) — 2.9.0 ломает Azure
  Key Vault credentials когда secret name использует bracket-notation с дефисами
  (e.g. `$secrets.azureKeyVault["postgres-n8n-data"]`). Root cause:
  `extractProviderKeysFromExpression` regex в
  `packages/cli/src/credentials/external-secrets.utils.ts` — dot-notation regex
  использует `(?=\.)` lookahead, не матчит mixed `.` + `[...]` forms.
  Workaround: конвертить все `.` в `[...]`.
- [#28151](https://github.com/n8n-io/n8n/issues/28151) (open) — Azure Key Vault
  reload failure.
- [#24273](https://github.com/n8n-io/n8n/issues/24273) — Azure KV test connection
  всегда 400.
- [#24828](https://github.com/n8n-io/n8n/issues/24828) (open) — HashiCorp Vault:
  «Could not load secrets» в runtime.
- [#24057](https://github.com/n8n-io/n8n/issues/24057) — GCP connection broken в 2.2.0+.
- [#20033](https://github.com/n8n-io/n8n/issues/20033) — GCP secret без «latest»
  version crashes.
- [#18053](https://github.com/n8n-io/n8n/issues/18053) — Vault subpath не supported.

**Корреляция:** expression-time resolution (см. architecture doc) means каждый node
execution re-resolves secrets через cache. Provider downtime surfaces как runtime
errors. Нет staleness-aware retry; нет provider-health circuit breaker. Regex
fragility — exact kind of bug that comes from a stringly-typed expression DSL
instead of AST.

---

## 8. Community-node credentials

- [#27833](https://github.com/n8n-io/n8n/issues/27833) (closed) — **Community node
  credentials не изолированы per workflow — все workflows резолвятся на последнюю
  saved credential.** High-severity data-leak class bug.
- [#23877](https://github.com/n8n-io/n8n/issues/23877) — community OAuth2 nodes
  игнорят user-entered scope values.

**Корреляция:** `no-credential-reuse` ESLint rule (в package linter, см. architecture doc)
— *static* check, runtime isolation отсутствовала. Намекает что
credential-type-as-metadata scales surface-wise, но tests для cross-package
leakage не enforced'ились.

---

## 9. LDAP

Notably low-volume по сравнению с OAuth2. Большинство issues касаются *LDAP workflow node*,
не LDAP *auth integration*:

- [#15604](https://github.com/n8n-io/n8n/issues/15604) — enterprise feature activation bug.
- [#18598](https://github.com/n8n-io/n8n/issues/18598) — high-user-count installations:
  stale LDAP users никогда не removed, share masks break. Прямо surface'ит
  «no group-to-role mapping + weak disable sync» gap.
- [#15737](https://github.com/n8n-io/n8n/issues/15737) — excessive memory на LDAP
  sync (loads all users в памяти).
- [#13462](https://github.com/n8n-io/n8n/issues/13462) — неправильная email validation
  для LDAP users.

---

## 10. Корреляционная таблица — проблема → root cause → Nebula mitigation

| Проблема | Root cause в n8n | Nebula mitigation |
|---|---|---|
| Refresh-token rotation lost | Нет optimistic lock / upsert на `credentials_entity.data`; write-after-read | Dedicated `credential_tokens` с `version: i64` + `UPDATE … WHERE version = ?` |
| Refresh race на concurrent requests | Нет per-credential mutex | `tokio::sync::Mutex` в `Arc<DashMap<…>>` keyed by `credential_id` или Postgres `pg_try_advisory_xact_lock` |
| `dummy.stack.replace` маскирует errors | Error-stringification placeholder leak | Structured `NebulaError` + `Classify`; никогда не interpolate stack/template strings |
| Key rotation breaks enterprise | Один global key, без versioning, encrypted blobs scattered по таблицам без `kek_id` | Envelope format `{version, kek_id, iv, ct}` на *каждой* encrypted колонке, per-table rotation walker |
| SSO orphan state | `settings` KV без cross-row transitions; нет CLI disable | Dedicated `auth_provider` table per provider + `nebula-cli auth disable saml \| oidc \| ldap` |
| License expiry leaves SSO enabled | Feature-flag checked только на login | License check в auth middleware, не только at login; fail-safe to email login |
| Git-pull wipes OAuth tokens | Source-control treats `credentials_entity.data` как sync target | Split: `credential_config` (git-syncable) vs `credential_runtime` (never synced) |
| MFA lockout recoverable только через CLI | Нет self-service recovery flow | Recovery codes + signed email recovery tokens; rate-limited |
| External secret regex fragility | Stringly regex в `extractProviderKeysFromExpression` | Parse expressions в AST; validators operate on AST, не strings |
| Community nodes leaking credentials | Только static ESLint rule; нет runtime check | Credential resolver берёт `(workflow_id, node_id)` tuple, не только credential name; reject cross-package |
| Workflow-hardcoded API keys unencrypted | Community tier без secret store | All credentials через single encrypted store с day 1 — никакого «community limitation» tier |
| TOTP clock-skew failures | Нет tolerance window и NTP check на boot | ±1 step window + clear error «server clock skew >30s» at boot |
| LDAP memory blowup на sync | Loads all users в памяти | Streaming cursor + batched upsert |

---

## 11. Quick wins для Nebula

Каждый — примерно ~10 строк кода, закрывают класс жалоб целиком:

1. **Envelope все encrypted blobs** как `{version:u8, kek_id:u32, iv:[u8;12], ct:Vec<u8>}`
   (serde + postcard). Cost: ~15 lines. Устраняет весь класс #22478.
2. **Version column на каждом row с refreshable token** (`credential_tokens.version`).
   Update с `WHERE version = ?`, retry on conflict. Убивает #25926 + race class.
3. **Keyed mutex для OAuth refresh** (`Arc<DashMap<CredentialId, Arc<Mutex<()>>>>`).
   One critical section per credential, zero chance of token reuse. Убивает #13088.
4. **Classified error → user-facing message с fixed vocabulary.** Без templated
   placeholders. Убивает `dummy.stack.replace` family.
5. **`nebula auth providers list|disable <id>` CLI** с day 1. Убивает
   `DELETE FROM settings` workaround из #19066.
6. **Split `credential_config` vs `credential_runtime` tables.** Git-sync трогает
   только первый. Убивает #26499.
7. **Expression AST, не regex.** Убивает #28516 + все future shape-mismatch bugs.
8. **Feature-flag check в auth middleware**, не только at login. Убивает #19907.
9. **«Configured but unreachable» health probe** на external secret providers,
   surfaced в UI. Убивает silent-failure half из #28151, #24828.
10. **Rotation walker CLI** (`nebula credentials rotate-key --from=kek_42 --to=kek_43`)
    который iterate'ит every encrypted row across every table. Ten lines per table.
    Убивает весь ENCRYPTION_KEY pain forever.

---

## 12. Источники

### GitHub issues (most-cited, отсортированы по теме)

**OAuth refresh:**
- [#25926 Generic OAuth2 rotated refresh_token не persisted](https://github.com/n8n-io/n8n/issues/25926)
- [#26453 Microsoft Outlook OAuth2 refresh fail после ~1h](https://github.com/n8n-io/n8n/issues/26453)
- [#13088 Race condition в OAuth token refresh](https://github.com/n8n-io/n8n/issues/13088)
- [#17450 OAuth2 Client Credentials не рефрешит на 403](https://github.com/n8n-io/n8n/issues/17450)
- [#23410 Credential refresh не triggered когда status 200 но body 401](https://github.com/n8n-io/n8n/issues/23410)
- [#28225 OAuth2 refresh behind corporate proxy](https://github.com/n8n-io/n8n/issues/28225)
- [#26066 OAuth redirect URL всегда HTTP](https://github.com/n8n-io/n8n/issues/26066)

**Encryption:**
- [#22478 Enterprise features break после rotating ENCRYPTION_KEY](https://github.com/n8n-io/n8n/issues/22478)
- [#20175 N8N_ENCRYPTION_KEY_FILE не working](https://github.com/n8n-io/n8n/issues/20175)

**SSO:**
- [#19066 SAML save не persist (OIDC orphan в settings)](https://github.com/n8n-io/n8n/issues/19066)
- [#25969 SAML metadata endpoint не working](https://github.com/n8n-io/n8n/issues/25969)
- [#18298 OIDC missing state → Okta failure](https://github.com/n8n-io/n8n/issues/18298)
- [#19907 SSO не disabled при license expiry](https://github.com/n8n-io/n8n/issues/19907)

**MFA:**
- [#25831 Can't login с MFA enabled](https://github.com/n8n-io/n8n/issues/25831)
- [#22637 2FA setup times out перед saving recovery codes](https://github.com/n8n-io/n8n/issues/22637)

**API keys:**
- [#25684 Workflow API keys stored unencrypted](https://github.com/n8n-io/n8n/issues/25684)
- [#26642 API key scopes не respected](https://github.com/n8n-io/n8n/issues/26642)

**External Secrets:**
- [#28516 External secrets break на bracket notation](https://github.com/n8n-io/n8n/issues/28516)
- [#28151 External Secrets — Azure Key Vault reload fails](https://github.com/n8n-io/n8n/issues/28151)
- [#24828 HashiCorp Vault: Could not load secrets](https://github.com/n8n-io/n8n/issues/24828)

**Sharing / Git-sync:**
- [#26499 Git pull стирает Gmail OAuth credential](https://github.com/n8n-io/n8n/issues/26499)
- [#21558 Credential sharing settings gone после upgrade](https://github.com/n8n-io/n8n/issues/21558)

**Community nodes:**
- [#27833 Community node credentials не isolated per workflow](https://github.com/n8n-io/n8n/issues/27833)

**LDAP:**
- [#18598 LDAP: old users не removed](https://github.com/n8n-io/n8n/issues/18598)
- [#15737 Excessive memory usage после ldap sync](https://github.com/n8n-io/n8n/issues/15737)

### Community forum

- [Updating N8N_ENCRYPTION_KEY (thread 165334)](https://community.n8n.io/t/updating-n8n-encryption-key/165334)
- [Persistent bad decrypt после environment reset (thread 240548)](https://community.n8n.io/t/persistent-bad-decrypt-error-with-credentials-even-after-complete-environment-reset-and-correct-encryption-key-configuration/240548)
- [Problem updating n8n on Docker — losing encryption key (thread 157972)](https://community.n8n.io/t/problem-updating-n8n-on-docker-losing-data-encryption-key/157972)
- [Critical: unable to recover workflows from old DB (thread 198838)](https://community.n8n.io/t/critical-issue-unable-to-recover-workflows-from-old-database-after-config-file-corruption/198838)
- [Microsoft Azure OAuth2 refresh 1-hour expiry (thread 257628)](https://community.n8n.io/t/microsoft-azure-oauth2-refresh-token-not-working-token-expires-after-1-hour/257628)
- [OAuth2 Client Credentials не refreshing (thread 127129)](https://community.n8n.io/t/oauth2-client-credentials-not-refreshing-expired-tokens/127129)
- [GitHub #17450 cross-posted (thread 163067)](https://community.n8n.io/t/github-issue-17450-oauth2-client-credentials-in-n8n-does-not-refresh-token-after-expiry-403-error/163067)

---

## 13. Связь с остальной документацией

- Архитектурный reference: [`n8n-auth-architecture.md`](./n8n-auth-architecture.md) —
  как устроено в n8n код/DB/REST.
- Для Nebula-ADR по credential design: каждый пункт §11 (Quick wins) должен либо
  попасть в ADR «Credential encryption envelope», либо закрыться тестом.
- Для **Nebula STYLE** — §10 содержит anti-pattern `dummy.stack.replace`, stringly
  regex для expressions, in-memory LDAP sync — стоит занести в `docs/STYLE.md`
  когда будут разбираться соответствующие crate'ы.
