# Activepieces — Peer Research (OSS TypeScript n8n-alternative)

> Восьмой в серии peer-research. Разбор **Activepieces**
> (`activepieces/activepieces`) — TypeScript OSS automation, позиционируется
> как «developer-first, type-safe pieces». ~21k stars, active community.
>
> **Почему важен для Nebula:** Activepieces позиционируется как «лучше n8n
> по DX». Их community complaints покажут — is pivot away from n8n спасает
> automatically или нет. **Customer migration data point.**

> **⚠️ Scope reminder.** Всё содержимое ниже — описание **Activepieces**
> как peer-research subject. Nebula-mitigation — в correlation-table и
> Quick wins секциях отдельно.

## Метаданные исследования

- **Последняя сверка:** 2026-04-20
- **Репо:** `activepieces/activepieces` (~21k stars)
- **Источники:** GitHub issues (open + closed), community forum, Reddit,
  HN, comparison blogs, DeepWiki
- **Провенанс:** «hot» = 5+ повторов; «confirmed» = в коде или reproduced

---

## Executive Summary — топ-5 pain areas

1. **Sandbox overhead dominates execution latency.** Activepieces'
   namespace + V8 isolation makes single trigger take ~15s где n8n
   does it в ~0.5-1s (community thread #3864). Maintainers had to ship
   «Workers 3.0» + unsandboxed worker-thread mode to close gap. Open
   request для QEMU microVMs (#11547).

2. **Community pieces break на upgrade, constantly.** `🧩 area/third-party-pieces`
   label — largest bug cluster. Shopify, Xero, Notion, Google Drive,
   Hubspot, Mastodon, IMAP, Facebook Pages, Trello — все have
   «piece stopped working after version X» issues open right now.

3. **«Code piece fails» — single most repeated bug title.** Five separate
   issues с that exact phrase (#11998, #11995, #10989, #10639, #9554) —
   in-flow code step fragile и poorly diagnosable.

4. **Upgrade friction real.** 0.65/0.68/0.73/0.80 all shipped с SQLite
   migrations, broken bun binaries, vanished flows, или MCP tool
   regressions (#8338, #9190, #10368, #10483, #12294).

5. **OAuth long-tail never gets fixed permanently.** Google 7-day refresh
   token, Xero OAuth, GoHighLevel, Monday, Facebook Pages, Hubspot
   paid-scope mismatch — every half-year новый OAuth flavor leaks.

---

## Architectural Overview (~200 слов)

Activepieces — TypeScript monolith split на три process roles:
- **Fastify API server**
- **BullMQ worker** (running engine)
- **React flow editor**

Storage — **PostgreSQL + Redis**.

**Pieces** — npm packages authored с `createPiece({ displayName, actions, triggers, auth })`,
using typed `Property.{ShortText, LongText, Array, StaticDropdown, Checkbox, …}`
DSL с `required` и `propsProcessor` validation.

**Triggers** pick explicit `TriggerStrategy.POLLING | WEBHOOK`; polling triggers
implement `onEnable/onDisable/run/test` + `pollingHelper` с deduplication;
webhook triggers add `onHandshake` для provider verification.

**Engine** — TS file compiled to JS, spawned inside sandbox (namespaces или V8
depending on mode); `pieceExecutor` injects `ActionContext` с `store`, `files`,
`connections`.

**`pieceSyncService`** polls Activepieces cloud every hour и installs new/updated
piece packages; platforms can also upload `.tgz` tarballs.

**Community Edition** runs all three roles в single Docker container; paid
editions separate workers. Horizontal scaling — manual — worker replicas
behind shared Redis queue, с no autoscaling или burst queue. Multi-tenancy
exists at platform/project level; pieces can be scoped per project.

---

## Ось 1 — COVERAGE

| Area | Status | Evidence |
|---|---|---|
| Piece catalog | ~280-400 pieces depending on count method (vs n8n 1100+) | DeepWiki; marketing says 375 |
| Trigger types | `POLLING` + `WEBHOOK` explicit enum, no `EVENT_SOURCE` или queue trigger | DeepWiki |
| Credential types | OAuth2 (client_credentials + authorization_code), Basic, Custom auth (typed `PieceAuth.CustomAuth`) | DeepWiki |
| OAuth refresh handling | Bug-prone: Google 7-day (#3067), client_credentials 1-hour (#7850), multiple app-creds только Enterprise (#6265) | — |
| AI / MCP | **First-class** — MCP server UI, `mcpTool` trigger, 400+ claimed MCP servers, но MCP UI hidden в 0.73 broke flows-as-tools (#12294) | #11040, #12294, #12381, #12455 |
| Form trigger / Respond | «Respond in Triggers» landed Sep 2025 (#9294), Human Input Chat UI has file-validation bug (#9399) | — |
| Community install | npm package + `.tgz` upload; auto-sync every hour via `pieceSyncService` | DeepWiki |
| Enterprise-only gating | Multi app-cred OAuth (#6265), some audit/governance, separated workers | — |
| Top community requests | Clay (#12132), Greenhouse (#12145), Postmark (#12108), native parallelism (#10980), bearer-token webhooks (#8060), sequential processing (#6844), multiple triggers per flow (#9690), Tauri embed (#10641) | — |

---

## Ось 2 — ERGONOMICS / UNIVERSALITY

### Piece DX — headline claim, mostly delivers

Typed `createPiece` / `createAction` / `createTrigger`, local hot-reload
через `nx`, CLI scaffolding (`create-trigger.ts`). Reviewers consistently
describe as «TypeScript way to write n8n nodes».

### Но typing leaky

Issue #12456 literally titled «Eliminate excessive `any` types in
community pieces для improved type safety». Framework typed; community
piece code не.

### Auth flow чище чем n8n

`PieceAuth.OAuth2({ authUrl, tokenUrl, scope })` declarative, против n8n's
split `credentials/*.ts` + `nodes/*/generic.ts`. Но каждая новая OAuth
provider variant (audience, client_credentials с custom grant, paid-scope
enforcement) hits новый bug.

### Expression / templating

Mustache-like `{{ step_1.output.field }}` — similar ergonomics to n8n но
much less fancy than n8n's `$json`/`$node` shortcuts. No built-in code
assistance для expressions. **Reddit:** «n8n expressions more powerful,
AP simpler».

### Schema validation

Type-driven, no Zod. `propsProcessor` at trigger time applies required +
custom validators; webhook payloads have **NO schema validation by default**
(#6749 open since Feb 2025).

### Sub-flows

«Callable subflow» exists но requires sequential processing workaround (#6844).

### Migration from n8n

**No importer.** Marketing pages say «similar concepts» но users rebuild
flows by hand.

### Deployment

Docker single-container — 1-command; splitting workers new (#7190 announcement
July 2025) и community-edition accessible. Helm chart uses Bitnami images
which just went commercial (#10795 open).

### Upgrade friction

Documented regressions в:
- 0.65 (SQLite broken)
- 0.68 (multiple small issues)
- 0.73 (bun binary + flow-list blank + google-provider crash)
- 0.80 (MCP UI hidden)

Template import across versions has failed (#9047).

---

## Ось 3 — BUGS / PROBLEMS

### Piece breakage on upgrade

Own recurring class. Many area-labeled bugs stale (open >6 months)
потому что community pieces have no dedicated maintainer — label
`🧩 area/third-party-pieces` has **dozens of open bugs**.

### Code step instability

- #10634 fails on curly braces
- #9554 fails на property named «target»
- #10989, #11995, #11998 generic «code piece fails»

Cross-cutting extensibility escape hatch и users blow themselves up.

### Trigger reliability

- #7366 Slack message trigger unreliable
- #7267 Telegram webhook+polling conflict
- #8609 Stripe duplicates webhooks
- #10158 Xero «intent to receive» broken
- #10633 Formbricks env-id missing
- #12436 handshake-based triggers fail

### OAuth refresh

- #3067 Google 7-day expiry
- #6867 client_credentials audience
- #7850 client_credentials 1-hour refresh missing
- #4665 custom-OAuth insecure-warning

Recurring потому что каждый provider имеет own grant flavor.

### Performance

- #11256 flow editor >60s load at 100+ steps
- #9729 alerts query unindexed
- #6818 publish flow timeouts
- #9477 large redis payloads
- #11204 OOM on piece sync
- Cloud «slow processing» incident (#4135)

### Docker / self-hosted

- #10276 `AP_PUBLIC_URL` undefined makes server crash с cryptic «Invalid URL»
- #12577 CORS header duplicated на `/sync`
- #6609 & #7136 SMTP_PORT must be set или invite crashes
- #8338 SQLite migration break
- #10368 bun binary broken
- #7310 SANDBOXED mode breaks Twitter piece

### Security audit findings

- #12234 — tool description injection + missing output sanitization на MCP path
- #12389, #12381 ecosystem around MCP-trust gaps

### Storage abuse

- #9398 «Get Storage on RUN level does not generate sample data»

---

## n8n ↔ Activepieces Migration Patterns — UNIQUE SECTION

### Почему юзеры мигрируют К Activepieces

- **Predictable flat pricing** ($25/mo unlimited tasks) vs n8n cloud's task
  metering — top stated cost reason.
- Flow builder described как «feels like modern app, не developer tool»;
  **AI-guided onboarding** vs n8n's blank canvas.
- **First-class MCP + AI-agent story** lands earlier и looks more polished
  чем n8n's, especially для flows-as-tools.
- Typed piece framework + npm distribution wins с developers который want
  package private connectors.
- **MIT license vs n8n's Sustainable Use License** — relevant для commercial
  redistribution.

### Почему юзеры мигрируют от Activepieces (назад к n8n или остаются)

- **Raw speed.** Thread #3864 — canonical data point: **n8n 0.5-1s/request
  vs AP ~15s/request** на identical hardware. Даже после «Workers 3.0» и
  unsandboxed mode, heavy workloads still prefer n8n. **Single most-cited
  objective reason.**
- **Integration count.** 1100+ nodes vs ~400 pieces. Long-tail enterprise
  integrations (Databricks, SAP, Salesforce variants) thinner на AP.
- **Expression power.** n8n `$json`, `$node`, `$workflow`, `$items` +
  «Code node» с JS + Python > AP's mustache + sandboxed code piece.
  Power users rebuild complex flows в n8n.
- **Community size.** n8n ~180k stars / AP ~21k stars. Для obscure problems,
  n8n's forum has answers, AP's doesn't.
- **Complex branching / multi-trigger flows.** AP только got multiple
  triggers issue open (#9690), sequential webhook handling open (#6844),
  native parallelism open (#10980) — все capabilities n8n already has.

### Hybrid pattern

Multiple sources: **users run AP для business-facing AI-agent workflows
и n8n для backend/ETL.** This is «AP — ease, n8n — power» framing.

### «I regret switching» stories

Cluster на:
- Piece breakage после upgrade (company-critical Shopify/HubSpot flows die)
- Code-step fragility
- «Slow UI» feeling at self-hosted community scale
- Help-wanted threads like «Self-Hosting Activepieces For Massive Scale»
  где responder asks «did you resolve it or give up?»

---

## Что Activepieces делает ПРАВИЛЬНО vs n8n

1. **Explicit `TriggerStrategy.POLLING | WEBHOOK` enum** — typed, не
   implicit. No «is this webhook trigger?» guessing.
2. **`PieceAuth` as first-class declarative object** с `OAuth2/BearerToken/CustomAuth`
   variants — чище чем n8n's split credentials/generic-auth files.
3. **Pieces — npm packages** — real distribution, real versioning, real
   registry. **n8n community nodes still second-class.**
4. **Hot reload для local piece dev.** n8n has nothing comparable.
5. **`pieceSyncService` с platform/project scoping** — install at platform,
   hide at project. Good multi-tenancy primitive n8n lacks.
6. **MCP-first mindset.** `mcpTool` trigger, MCP server UI (before 0.73
   hid it), «flows as tools». n8n bolted MCP on later.
7. **Flat pricing**, MIT license — enterprise adoption story simpler.
8. **Onboarding UX** — consistently wins в comparison blogs и G2 reviews
   для non-developers.

---

## Correlation Table

| # | Signal | Evidence | Ось | Nebula relevance |
|---|---|---|---|---|
| 1 | Sandbox cost dominates latency | thread #3864, #11547 | 3 | Validates choosing Wasm/process isolation boundary carefully; sandbox — first-class perf axis |
| 2 | Community piece rot on upgrade | `area/third-party-pieces` label | 3 | Plugin/integration versioning + contract tests matter more than catalog size |
| 3 | Code-step fragility | #9554, #10634, #10989, #11995, #11998 | 3 | Code-action must have strict input schema + good error surfacing |
| 4 | OAuth refresh long-tail | #3067, #6867, #7850, #4665 | 1+3 | Credential refresh must be pluggable per grant type, не one-size-fits-all |
| 5 | TriggerStrategy enum | DeepWiki | 2 | Explicit trigger kinds > implicit — matches Nebula's integration-model direction |
| 6 | npm package distribution | DeepWiki | 1+2 | Pieces-as-packages — right mental model; Nebula needs parallel для plugin crates |
| 7 | Webhook schema validation missing | #6749 | 1 | Inbound schema validation — differentiator, не optional |
| 8 | Upgrade-migration breakage | #8338, #9190, #10368, #10483 | 3 | Storage migrations need compatibility tests или lose install base |
| 9 | No n8n import path | WebSearch | 2 | «Importer from X» — real moat только если commit to it; AP chose not to |
| 10 | `any` type creep в community pieces | #12456 | 2 | Typed framework necessary но не sufficient; community enforcement hard |
| 11 | Self-hosted scale stories «did you give up?» | community thread #5405 | 3 | Scaling story needs ops docs + hard numbers, не just «horizontally scalable» |
| 12 | MCP security (tool injection) | #12234, #12381, #12389 | 3 | MCP safety + output sanitization will become table-stakes |

---

## Quick Wins для Nebula (10)

1. **Adopt explicit trigger-kind enum** (`Webhook | Polling | EventSource | Manual`).
   No implicit classification. Mirrors AP `TriggerStrategy` и aligns с
   `INTEGRATION_MODEL.md`.
2. **Credential/OAuth grant as typed variant**, не free-form config.
   Separate refresh strategy per grant (`authorization_code`, `client_credentials`,
   `device_code`). Keeps #7850/#3067 class off Nebula.
3. **Ship webhook schema-validation primitive на day one.** AP's #6749 open
   14 months. Nebula already has `nebula-validator`; wire в webhook receive
   at engine level.
4. **Contract tests per integration**, run на every release. AP's «piece
   broke после upgrade» crisis directly avoided making this non-optional
   в `nebula-action` / `nebula-plugin`.
5. **Bench sandbox overhead as published SLI.** AP discovered latency
   after users complained. **Nebula should measure engine+sandbox trip time
   per action и publish в `OBSERVABILITY.md`.**
6. **Typed property DSL с processors**, не JSON-schema strings. AP
   `Property.{ShortText, LongText, Array, StaticDropdown}` + `propsProcessor`
   — right shape; Rust version через enum + `schemars`-backed DSL strictly better.
7. **Plugin-distribution story** — cargo-registry-based или nebula-native
   registry, но **pick one и commit**. AP's npm model — their best DX lever.
8. **First-class «Respond in Trigger»** — AP took until Sep 2025 (#9294)
   to get it. Для webhook-as-API shapes — mandatory.
9. **Native parallelism + sequential-webhook modes as declared flow properties**,
   не workaround. AP has both as open issues (#10980, #6844). Nebula flow
   should declare its concurrency semantics.
10. **Version-isolated credential refresh path.** Keep `credential-refresh`
    out of action hot path так борked provider не takes down flow execution.
    AP mixes these и pays в bug cadence.

---

## Sources

### GitHub issues

Bug searches: `bug piece`, `n8n`, `oauth`, `webhook trigger`, `performance slow`,
`migration import`, `docker self-hosted`, `community piece custom`, `schedule cron`,
`MCP AI agent`, `credentials connection`, `sandbox isolate`.

**Specific issues:** #3067, #3307, #4665, #6265, #6609, #6749, #6818, #6844,
#6867, #7136, #7267, #7310, #7366, #7850, #8060, #8072, #8161, #8338, #8609,
#8938, #9047, #9103, #9190, #9294, #9398, #9399, #9453, #9477, #9572, #9690,
#9729, #10076, #10158, #10254, #10276, #10368, #10431, #10483, #10504, #10586,
#10633, #10634, #10641, #10795, #10959, #10980, #10989, #11040, #11044, #11132,
#11153, #11204, #11231, #11256, #11302, #11547, #11605, #11710, #11712, #11995,
#11998, #12086, #12125, #12132, #12145, #12234, #12294, #12321, #12381, #12386,
#12389, #12436, #12439, #12455, #12456, #12577, #12578, #12602.

### DeepWiki

`activepieces/activepieces` — piece framework architecture; community piece
distribution; `pieceSyncService`; Docker/worker topology; multi-tenancy.

### Activepieces community forum

- Thread 3864 (slow processing vs n8n, April 2024, maintainer abuaboud response)
- Thread 4135 (resolved slow processing в cloud)
- Thread 5405 (help wanted massive scale)
- Thread 7190 (separating workers announcement)
- Thread 10956 (Workers 3.0)

### Web comparisons

- activepieces.com/blog/activepieces-vs-n8n
- hostadvice 2026 comparison
- black bear media comparison
- openalternative.co
- booleanbeyond.com (AP vs n8n vs Windmill)
- flowlyn.com (OSS n8n alternatives)
- n8n community thread «Has anyone seen ActivePieces yet?» (22690)

### GitHub

`activepieces/activepieces` repo (~21k stars)

---

## Связь с остальной документацией

Эта серия теперь покрывает **8 peer products**:

| Файл | Фокус |
|---|---|
| [n8n-auth-architecture.md](./n8n-auth-architecture.md) | n8n auth (REST + DB + flow-диаграммы) |
| [n8n-credential-pain-points.md](./n8n-credential-pain-points.md) | n8n credentials pain |
| [n8n-parameter-pain-points.md](./n8n-parameter-pain-points.md) | n8n parameter UI pain |
| [n8n-trigger-pain-points.md](./n8n-trigger-pain-points.md) | n8n trigger pain |
| [n8n-action-pain-points.md](./n8n-action-pain-points.md) | n8n action-node pain |
| [temporal-peer-research.md](./temporal-peer-research.md) | Temporal (durable execution, Rust SDK) |
| [windmill-peer-research.md](./windmill-peer-research.md) | Windmill (Rust peer) |
| **activepieces-peer-research.md** (этот файл) | Activepieces (n8n migration data) |

**Meta-cross-references:**
- AP's explicit `TriggerStrategy` enum + `PieceAuth` declarative object —
  mirror эти ergonomics
- Temporal's Patch/GetVersion — solution для AP's upgrade breakage class
- Windmill's process-per-job durability — solution для AP's sandbox latency
- Все три агрее: **plugin/integration versioning + contract tests > catalog size**
