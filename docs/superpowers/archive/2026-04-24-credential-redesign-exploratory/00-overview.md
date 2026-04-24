---
name: credential redesign — exploratory draft overview
status: draft — exploratory, NOT a spec
date: 2026-04-24
authors: Claude (with tech-lead, security-lead, rust-senior agent review rounds)
scope: cross-cutting — nebula-credential, nebula-storage, nebula-engine, nebula-api, nebula-resource, nebula-action, nebula-core, nebula-schema
supersedes: []
related:
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md
  - docs/superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0031-api-owns-oauth-flow.md
  - docs/adr/0032-credential-store-canonical-home.md
  - docs/adr/0033-integration-credentials-plane-b.md
  - docs/research/n8n-credential-pain-points.md
  - docs/research/n8n-auth-architecture.md
---

# credential redesign — exploratory draft (overview)

## Что это такое

Серия MD файлов фиксирующих **текущее мышление** о потенциальном редизайне `nebula-credential`. Это **не спек**, **не plan**, **не ADR**. Это notes с acknowledged open questions и honest pointers на unresolved problems.

**Scope этого драфта:** зафиксировать где мы сейчас в обсуждении, что решено, что открыто, какие у нас есть pain points от real n8n data, и какой следующий step имеет смысл.

**Не scope:**
- Final trait shapes (они ломаются под compile-test — см. known-gaps)
- Implementation timeline
- Writing spec / plan / ADR — прежде чем prototype спайк покажет что trait shape вообще работает.

## Почему мы здесь

После того как P6-P11 credential cleanup phases landed (storage-owns-persistence, engine-owns-orchestration, api-owns-OAuth ceremony, trait/impl split, Plane B vocabulary), выглядит что архитектура "готова". Но:

1. **Feature gates** (`credential-oauth` в nebula-api) всё ещё висят как "rollout only" артефакт.
2. **ADR-0031** accepted 4 дня назад с обоснованием "n8n parity". Это обоснование слабое при ближайшем рассмотрении — n8n-shape не идеал, а просто реальность одного проекта.
3. **n8n field data** (428 credential types в production, корреляционная таблица pain points в [n8n-credential-pain-points.md](../../../research/n8n-credential-pain-points.md)) показывает **известные классы регрессий** которые текущий Nebula design частично не защищает:
   - Concurrent refresh race (n8n #13088) — в Nebula частично mitigated (in-proc RefreshCoordinator), но multi-replica не покрыт
   - Encryption key rotation pain (n8n #22478) — envelope format + walker недокументирован
   - SSO orphan state (n8n #19066) — Plane A, out of scope
   - Git-pull wipes tokens (n8n #26499) — нужен config/runtime split в storage schema
   - Community node credential leak (n8n #27833) — нужен workflow_id invariant в resolver
4. **Предложенный мной redesign** в conversation (три раунда response) — **paper design**. Пользователь выявил 37 findings (см. `05-known-gaps.md`), из которых ~8 BROKEN (API не typechecks или таксономически ошибочен), ~15 RESOLVABLE but недодуманный, ~4 NEW dimensions которые я пропустил, ~10 DETAILS.

## Что эти заметки дают

- **Карту layered responsibilities** (`02-layer-map.md`) — кто чем владеет, чтобы не дублировать между nebula-core / schema / credential / storage / engine / api
- **Type system direction** (`01-type-system-draft.md`) — с honest holes: `ctx.credential<C>()` ambiguity, dyn-safety для Credential с 4 assoc types, Pattern 1 vs Pattern 2 дефолт
- **Flows** (`03-flows.md`) — concrete interaction sequences для основных сценариев
- **Schemes catalog** (`04-schemes-catalog.md`) — 12 auth patterns с injection mechanics
- **Known gaps** (`05-known-gaps.md`) — 37 findings triage: blocker / resolvable / new dimension / detail
- **Prototype plan** (`06-prototype-plan.md`) — spike scope перед writing spec

## Позиционирование относительно канона

- **PRODUCT_CANON.md §3.5** (integration model) — this draft preserves стр Credential / AuthScheme / Resource binding; добавляет капабилити-based binding для multi-auth services.
- **PRODUCT_CANON.md §4.5** (operational honesty) — этот draft **не** добавляет новый public surface без implementation; prototype spike сначала, public surface только после.
- **PRODUCT_CANON.md §12.5** (secrets and auth) — preserves AAD binding, Zeroize, redaction invariants. Adds: per-field sensitivity уже есть в nebula-schema (`SecretField`), не изобретаем.
- **PRODUCT_CANON.md §13.2** (rotation/refresh seam) — вопрос multi-replica refresh race с IdP refresh_token rotation (n8n #13088 класс) не полностью решён существующим `RefreshCoordinator` в nebula-core. Это вероятно требует новый trait interface или новый storage repo.
- **PRODUCT_CANON.md §0.2** (canon revision triggers) — если мы решим что OAuth2 HTTP ceremony должен уехать из api обратно в engine (supersede ADR-0031), это `capability lag` trigger, требует new ADR.

## Ключевые tensions которые эти файлы фиксируют

1. **`ctx.credential::<C>()` shape — ambiguous при multiple slots одного типа.** Либо binding по полю (runtime lookup), либо type-driven (compile-time uniqueness). Не оба. Документ держит обе формы как обсуждаемые.

2. **Pattern 1 (concrete per-service types) vs Pattern 2 (service-specific trait для multi-auth).** Реальные данные n8n: большинство популярных services — multi-auth (Bitbucket, Jira, Shopify, GitHub, Slack, Stripe, HubSpot, Notion, Salesforce). Pattern 2 — default, Pattern 1 — minority. Это переопределяет shape trait'ов.

3. **Sealed trait vs plugin extensibility.** Goal — 400+ third-party credentials. `sealed::Sealed` запрещает out-of-crate impls. Нельзя иметь оба.

4. **AcceptsBearer / SchemeInjector — новый trait независимый от core's AuthScheme.** Core's AuthScheme — classification (pattern + expires_at). SchemeInjector — injection mechanics (inject/sign/tls/connection). Два concepts с похожим именем — путает.

5. **Multi-replica refresh race с refresh_token rotation** (n8n #13088 класс). Два engine instances могут запросить refresh одновременно. Первый получает new refresh_token, но replica-2 всё ещё держит старый в cache → permanent failure. In-proc RefreshCoordinator этого не решает. Storage-backed claim repo = решение, но дополнительная новая инфра.

6. **ProviderRegistry (operator-managed OAuth endpoints) vs user credential config.** Закрывает SSRF-через-user-config (n8n не имеет такой защиты). Но операционно усложняет: seeding, Microsoft multi-tenant templating, registry update vs existing credentials.

7. **Multi-step credential flows** (Salesforce JWT, session login) — accumulator state между шагами не определён. `PendingStore` сейчас только для PKCE verifier + CSRF, не generic per-step accumulator.

8. **Trigger ↔ credential** — в моём изначальном дизайне не рассмотрено. Trigger actions (IMAP watcher, webhook signature verification) имеют свой lifecycle credential integration который нужен.

## Что решено (базовый консенсус после research + agent review)

- `nebula-credential` остаётся **pure contract + primitives crate**. HTTP полностью out (это уже так сегодня — см. security-lead reality check).
- `CredentialStore` trait и DTOs остаются в `nebula-credential` (per ADR-0032); impls — в `nebula-storage`.
- `nebula-core::AuthScheme` — classification, нельзя смешивать с injection.
- `SecretString` / `SecretBytes` / `SecretField` — используем из `nebula-schema`, не изобретаем заново.
- `Guard` / `TypedGuard` — используем из `nebula-core`.
- Plane A / Plane B separation (per ADR-0033) — хороший, не трогаем.
- §12.5 crypto primitives (AES-256-GCM + AAD) — bit-for-bit preserve.

## Что надо решить (в порядке приоритета)

1. **Type system shape** — решить через prototype spike с 5+ real credential types которые компилируются.
2. **Pattern default** — Pattern 1 vs Pattern 2 vs смесь с generic fallback (OAuth2Api catch-all).
3. **Multi-replica refresh coordination** — durable claim repo или другой механизм.
4. **Multi-step flow state model** — PendingStore shape для N-step accumulation.
5. **ProviderRegistry bootstrap + versioning** — desktop/self-hosted/cloud divergence.
6. **Trigger credential integration** — новое dimension которое требует трактования.
7. **SchemeInjector vs AuthScheme** — naming + separation discipline.

## Next step рекомендация

**Prototype spike** — throwaway Cargo project который пробует trait shapes на реальных примерах (SlackOAuth2, BitbucketPat, AwsSigV4+STS, PostgresConnection, SalesforceJwtMultiStep, MtlsClient + action + resource + 3 usage points). Iterate until compiles и looks sane. Потом writing spec.

Детали в `06-prototype-plan.md`.

## Файловая карта

| Файл | Содержимое | Статус |
|---|---|---|
| `00-overview.md` | Этот файл. Context, non-goals, tensions, next step. | Это я |
| `01-type-system-draft.md` | Trait shape draft с open holes. | Не final |
| `02-layer-map.md` | Cross-crate responsibility map. | Рабочее |
| `03-flows.md` | Concrete flows: create/resolve/refresh/rotate/multi-step. | Draft |
| `04-schemes-catalog.md` | 12 auth patterns + injection mechanics. | Рабочее |
| `05-known-gaps.md` | 37 findings triage + resolution direction. | Honest |
| `06-prototype-plan.md` | Spike scope, deliverables, dispatch plan. | Concrete |

## Disclaimer

Этот draft **не** представляет собой approved design. Любой файл в `drafts/` может:
- Содержать ошибки, которые мы ещё не поймали
- Иметь противоречия между файлами
- Быть отброшенным целиком после prototype spike

Продакшен decisions — только через ADR / spec / plan в `docs/adr/` / `docs/superpowers/specs/` / `docs/superpowers/plans/`. Эти drafts — источник гипотез, не decisions.
