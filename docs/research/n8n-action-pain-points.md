# n8n Action Nodes — Coverage, Ergonomics & Bugs (field report)

> Пятый в серии peer-research про n8n. Разбор **action-node** системы
> — т.е. того, что делает работу в середине workflow: HTTP Request, Code,
> Set, If/Switch, Merge, Loop/SplitInBatches, Execute Workflow, AI Agent,
> Tool-wrappers, integration-ноды (Gmail/Slack/Sheets/Postgres и ~400
> остальных). **Триггеры** покрыты в отдельном отчёте — здесь не дублируем.
>
> Структура по трём осям: Coverage / Ergonomics / Bugs.
>
> Связанные файлы:
> - [n8n-auth-architecture.md](./n8n-auth-architecture.md)
> - [n8n-credential-pain-points.md](./n8n-credential-pain-points.md)
> - [n8n-parameter-pain-points.md](./n8n-parameter-pain-points.md)
> - [n8n-trigger-pain-points.md](./n8n-trigger-pain-points.md)

> **⚠️ Scope reminder.** Всё содержимое ниже — описание **n8n** как
> peer-research subject. Никаких утверждений про Nebula-архитектуру здесь
> НЕТ; Nebula-mitigation вынесены в correlation-table и Quick wins sections
> отдельно.

## Метаданные исследования

- **Последняя сверка:** 2026-04-20
- **Источники:**
  - `github.com/n8n-io/n8n` issues (~400 titles scanned, 17 keyword queries)
  - `community.n8n.io` форум
  - DeepWiki architectural summary
- **Охват:** `INodeType` contracts, versioning, execute semantics,
  `pairedItem`, binary data, Code/task-runner subsystem, AI Agent ecosystem,
  top SaaS integrations, community-node pitfalls
- **Провенанс:** «hot» = жалоба повторилась 5+ раз; «confirmed» =
  воспроизведено в коде или issue

---

## Executive Summary — топ-5 pain areas

1. **Code node task-runner timeouts доминируют в recent issue stream.**
   `#25171`, `#25319`, `#25986`, `#23381`, `#23430`, `#25356`, `#27148`,
   `#27353`, `#27838`, `#28625` — «Task request timed out after 60 seconds
   / not matched to a runner». Since the 2.x task-runner default, *simple*
   JS code node runs intermittently time out. `#25281` даже simple Python
   times out on Cloud. **Hot + confirmed runtime-stability class.**

2. **`pairedItem` breakage endemic.** «Paired item data for item from node
   X is unavailable» — один из самых-ищемых форумных errors. Confirmed:
   `#14568` Item linking breaks in Execute Sub-workflow, `#27767` AI Agent
   v3.1 `$getPairedItem` lineage error on If-node false branch, `#15981`
   Merge `$('NodeName').item` fails for non-first inputs, `#24534` Set
   node loses `.item` context after pin-data, `#4507` Postgres default
   query mode breaks pairedItems, `#12558` Continue-on-error loses
   reference. Class структурная — n8n не может track provenance across
   custom JS или many integration nodes.

3. **Node versioning creates silent, unbounded back-compat debt.**
   `nodeVersions` array + `defaultVersion` означает каждый workflow pins
   to version at save time; upgrades не migrate. Issues: `#27131` Editor
   silently flips `typeValidation` loose→strict on IF/Switch, `#17481`
   Switch drops Set data on specific routes, `#22489` «This model is not
   supported in 2.2 version of the Agent node», `#27726` AI Agent JSON
   output format change broke every workflow. Forum [thread 190031](https://community.n8n.io/t/update-broke-ai-agent-node/190031):
   «connectors missing entirely» after update. **Hot.**

4. **HTTP Request node — universal fallback, но с десятками edge cases.**
   Canonical «just use HTTP» escape hatch has real pitfalls. `#26925` large
   ints rounded in responses (JSON.parse precision loss), `#27439` retries
   on HTTP 200 with error body, `#27735` cookies lost on 302 redirect,
   `#27782` fails on empty JSON, `#27724` no XML parsing, `#26044` DELETE
   body stripped, `#16005` Bearer auth broken with pagination, `#27533`
   multipart boundary mismatch, `#23815` can't attach multiple binaries,
   `#27122` form-data >2MB broken. Plus pagination bugs
   ([thread 72427](https://community.n8n.io/t/troubles-with-pagination/72427))
   и Retry-on-Fail inconsistencies
   ([thread 110925](https://community.n8n.io/t/retry-on-fail-doesnt-work-for-http-request-node/110925)).

5. **AI Agent / Tool wrappers — fast-churn class где каждый upgrade
   ломает что-то.** `#24397` AI Agent tool v3 broken after AI Node updates,
   `#21740` AI Agent v3 incompatible with tool v2.2, `#18561` Model
   Selector not recognized as ChatModel, `#28215` googleGemini crashes on
   multi-turn tool call, `#26202` AI Agent receives empty tool response
   from `usableAsTool` community nodes (FIFO queue ordering), `#27805`
   memory node doesn't work with tools. Framework evolving too fast для
   workflow durability guarantees to hold.

**Secondary:** Merge node behavioral regressions (`#7775`, `#3949`,
`#13182`, `#19001`, `#19624` — «does not wait for both inputs», «outputs
only branch 1»); Google Sheets crashes/daily OAuth disconnects (`#26460`,
`#20946`, `#15490`, `#17261`, `#20372`); If/Switch bugs (`#23862` «If node
contents overrides adjacent If node», `#27693` «forwards true to false
branch»); binary data memory leaks (`#8028` fills up filesystem, `#21746`
too large to save); community-node credentials leaking across workflows
(`#27833`, covered in credentials report but origin is in node resolution).

---

## Architectural Overview (~200 слов)

Источник: DeepWiki + `packages/workflow/src/interfaces.ts`, `node-helpers.ts`,
`packages/nodes-base/nodes/*/`.

**`INodeType`** имеет два пути: **declarative** (metadata-only via
`INodeTypeDescription.routing` describing HTTP request generation) и
**programmatic** (implements `execute()`, `executeSingle()` or
`supplyData()`). Declarative preferred для pure REST wrappers; programmatic
wins для anything custom.

**Versioning** explicit. `VersionedNodeType` holds `nodeVersions: Record<number, INodeType>`
map и `defaultVersion`. Workflow pins `typeVersion` at save time;
`NodeTypes.getNodeType(name, version)` routes to exact implementation.
Migrations — author's problem — most nodes реализуют через
`if (this.getNode().typeVersion < N)` branches inline.

**`execute()`** receives `INodeExecutionData[]` и returns
`INodeExecutionData[][]` (outer = output branch). `continueOnFail()`
toggles error-as-data vs throw. `alwaysOutputData` forces empty outputs.
Multi-output nodes declare `outputs: [...]` и route via branch index.

**`pairedItem: { item: number } | number[]`** на каждом output item links
back to input index. `WorkflowExecute.assignPairedItems` auto-fills trivial
cases (1:1 or single input); anything non-trivial — author-responsibility.

**Binary data** — `IBinaryData` refs, resolved by `BinaryDataManager`
(memory, filesystem, or S3). Expression access via `$binary.<propertyName>`.

**Code node:** `JsTaskRunnerSandbox` или `PythonTaskRunnerSandbox` (Pyodide).
Task runners can be in-process (`internal`) или separate-process (`external`,
WebSocket broker). Helpers `$input.all()`, `$('NodeName').item`, `$json`,
`$binary`, `$now`, `$workflow`.

**AI Agent и Tool nodes** use `supplyData()` чтобы plug into a LangChain
agent as Runnables. `usableAsTool: true` marks обычный node как также
exposable as a tool.

---

## Ось 1 — COVERAGE (какие ноды есть и чего не хватает)

### Core taxonomy (из `packages/nodes-base/nodes/*/`)

~400 integrations + core primitives:

- **Transform:** Set/Edit Fields, Filter, Sort, Aggregate, Summarize,
  Split Out, Item Lists, Remove Duplicates, Compare Datasets
- **Flow:** If, Switch, Merge, SplitInBatches/Loop, Wait, NoOp,
  StopAndError, Execute Workflow, Respond to Webhook
- **Code:** JavaScript, Python, HTML Extract, XML/JSON, Crypto, DateTime,
  Markdown
- **HTTP:** Request (universal fallback), FTP/SFTP
- **Files:** Read/Write Disk, Convert, Compression, Extract
- **Databases:** Postgres, MySQL, MSSQL, MongoDB, Oracle, Snowflake,
  ClickHouse, BigQuery, DynamoDB, Redis
- **Messaging:** AMQP, MQTT, Kafka, RabbitMQ, SQS
- **100+ SaaS integrations**
- **40+ AI/LangChain nodes**

### Declarative vs programmatic

Most SaaS integrations — declarative (resource/operation → routing).
HTTP Request, Code, AI Agent, Merge, Loop — programmatic. PR `#4037`
lintfix destroying declarative properties hints at fragility of the
declarative metadata.

### Multi-operation mega-nodes

Gmail (50+ ops), Google Sheets, Notion, HubSpot — `resource` + `operation`
dropdowns. Это explodes parameter surface (см. parameter report —
`displayOptions` hell stems from exactly this pattern). Alternative
«atomic nodes per op» used by some community authors, но n8n core
предпочитает mega-nodes.

### AI Agent / Tool class (since 2024)

Agent, Tools Agent, ReAct, SQL Agent, multiple Chat Models (OpenAI,
Anthropic, Gemini, Ollama, Groq, Mistral, xAI, Bedrock), Vector Stores
(Pinecone, Qdrant, Weaviate, PGVector, Supabase, Milvus, MongoDB Atlas,
Chroma), Memory (Buffer, Window, Postgres, Redis, MongoDB, Zep), Document
Loaders, Text Splitters, Output Parsers, Retrievers, Rerankers
(missing — `#17942`).

### Явные coverage gaps (что юзеры запрашивают, но нет)

| Чего не хватает | Доказательство | Что делают юзеры |
|---|---|---|
| **Lightweight data-transform primitives** | `#22264` JMESPath projection fails, `#7017` expression editor chokes on JMESPath | Ручной Code node |
| **Native stream processors** | Нет streaming windowing / aggregation | SplitInBatches + ручной state |
| **Typed DB clients** | Postgres `#20078` date operator errors, `#21098` parameters >5 break, `#16354` comma breaks query, `#24291` boolean Update sends empty | Raw SQL query mode |
| **Reliable bulk-ops** | `#26569` WooCommerce 20× amplification, `#26571` Apify Cartesian product | Code node orchestration |
| **First-class webhook response shaping** | Respond-to-Webhook covers many cases, но `#21140` ignores Content-Length, `#25982` streaming broken, `#25972` hangs with two triggers | Custom Code + HTTP Response |
| **Retryable idempotent keys on any node** | Нет generic «retry until success with dedup key» operator | Code node |

### Community ecosystem

700+ packages на npm. Pain:
- `#27106` update leaves broken `node_modules`
- `#27833` credential leak across workflows
- `#27401` «Invalid version: beta» on npm dist-tag
- `#26026` duplicate name collision crashes install
- `#19431` community nodes can't `getParameter` for unmodified booleans
- `#22706` Windows install fails
- `#24796`/`#24797` admin-only install friction

---

## Ось 2 — ERGONOMICS / UNIVERSALITY

### Node versioning (hot)

- Version = save-time *pin*, не live. Нет migration framework — каждый
  node автор writes `if (typeVersion < N)` branches или ships новый class.
  Многие не делают: `#24397` AI Agent tool v3 broken after AI node updates,
  `#17481` Switch behavior changes on load with certain routes, `#22489`
  model not supported in 2.2.
- Breaking-change doc exists (`breaking-changes` module, `#28572`) но не
  gate save или warn users.
- **Cross-ref parameter report:** когда author changes parameter default,
  workflows silently pick up new value (`#19197`). Same root — no schema
  versioning on parameters.

### Execute semantics

- `execute()` gets all items, returns `[[...]]`. Batch size — per-author
  convention, не framework primitive.
- `continueOnFail()` partially works:
  - `#25581` ERPNext doesn't respect it
  - `#20321` GraphQL sends to **both** outputs
  - `#23813` 400 returned as success
  - `#18908` NocoDB doesn't output payload on error
  - `#26199` Oracle timeout doesn't go to error path
  - `#15272` BigQuery loops when «continue on error» enabled
- `alwaysOutputData` — separate flag which most authors ignore.
- **Error output branching:** some nodes declare 2 outputs (success/error),
  others route inline. Inconsistent UX. `#28095` Apify routes errors to success.

### Item linking (`pairedItem`) — hot

- Trivial 1:1 auto-filled. Custom Code, aggregating, splitting, or
  sub-workflow calls lose it:
  - `#14568` Execute Sub-workflow loses it
  - `#4507` Postgres default query mode breaks
  - `#27767` AI Agent v3.1 throws `$getPairedItem` lineage error on If node false branch
- Error message «Paired item data for item from node X is unavailable» —
  один из most-filed forum posts:
  - [39875](https://community.n8n.io/t/error-missing-paireditem-data/39875)
  - [125941](https://community.n8n.io/t/why-im-getting-this-errror-paired-item-data-for-item-from-node-create-list-of-labels-is-unavailable-ensure-create-list-of-labels-is-providing-the-required-output/125941)
  - [164324](https://community.n8n.io/t/paired-item-data-for-item-from-node-is-unavailable-ensure-node-is-providing-the-required-output-item-0/164324)
  - [245326](https://community.n8n.io/t/paired-item-data-for-item-from-node/245326)
  - [228022](https://community.n8n.io/t/how-is-paireditem-supposed-to-work-in-code-node/228022)
- Workaround: `.first()` / `.all()[i]`. **Hot.**

### Binary data handling

- Три режима: memory (default), filesystem, S3.
  - `#8028` filesystem mode fills up disk с no reaper.
  - `#21746` «Large binary execution data results in too large a string
    to save execution state to db» — DB execution_data size limit trap.
- `#26968` S3 binary socket pool exhaustion causes hang (open).
- `#18175`, `#20939`, `#25567` multipart upload regressions.
- **Nodes silently drop binary** (most Code/Set nodes unless
  `Include Other Input Fields`): `#17432` Set drops, `#16673` Set forwarding
  feature request («Set Node should forward binaries by default» — long-standing).
- Cross-ref parameter report: binary fields — type `binary`, но expression
  UX diverges от `string`.

### Code node (hot runtime bugs)

- **Task-runner subsystem — #1 recent regression source.** 10+ issues
  про «Task request timed out after 60 seconds / runner not matched»:
  `#25171`, `#25319`, `#25986`, `#23381`, `#23430`, `#25356`, `#27148`,
  `#27353`, `#27838`, `#28625`.
- `#25544` Phantom Internal Runner Spawns in External Mode Queue Workers
  — blocks BOTH JS & Python.
- `#15628` `N8N_RUNNERS_ENABLED=true` causes memory leak.
- `#26926` cannot import installed npm package in Code node under task runner.
- `#24307` `$evaluateExpression` returns null in Code node.
- **Security:** `#26708` (closed as «proposal») Secure Execution Sandbox
  request — current sandbox — Node.js `vm` с hardening, acknowledged partial.
- Python (Pyodide) has own Fork Server + SecurityValidator но `#25281`
  times out on simple code.

### AI Agent / Tool wrappers (hot, high churn)

- **Sub-node pattern:** Memory, Tool, Model connect via `supplyData()`.
  Sensitive to LangChain upgrades.
- `#17979` AI Agent loop context persistence
- `#22112` memory pollution via full intermediate tool outputs stored in Redis
- `#27805` memory doesn't work with tools
- `#24042` tool errors fail entire workflow instead of being returned to agent
- `#26202` FIFO queue ordering bug in `usableAsTool` community nodes
- `#25966` AI Agent sends node name to LLM instead of tool name
- `#15528` degraded perf on Kubernetes
- `#26640` Gemini implicit caching broken
- `#25655` tokenUsageEstimate discarded
- Cross-ref parameter report: `$fromAI` button missing для fields named
  `"name"` (`#28261`).

### Expression scope внутри node

- `$input.all()`, `$input.first()`, `$('Node Name').item`, `$json`,
  `$binary`, `$now`, `$workflow`, `$vars`, `$execution`.
- `#24173` `$vars` не в resource-locator expressions для declarative nodes
- `#16112` `$now` returns current time даже в historical execution view
- `#15981` Merge `$('NodeName').item` fails для non-first inputs
- Cross-ref parameter report: regex-based expression discrimination,
  нет AST.

### Node rename consequences

Workflow references `$('Old Name').item` everywhere. Rename triggers
find-and-replace который misses quoted forms. `#21982` autocomplete breaks
on Korean names. Нет structural rename.

### Batch size / concurrency per node

- Нет framework-level concurrency knob. Some nodes (HTTP Request, OpenAI,
  AI Agent) expose batch size в их own UI.
- `#21376` «Done» output fires per-iteration в Split-in-Batches
- `#20630` freezes на 45k items
- `#21817` infinite loop → heap OOM
- `#28488` Wait node breaks `concurrency=1` в queue mode

---

## Ось 3 — BUGS / PROBLEMS

### Version-bump silent breakage

- `#22489` AI Agent v2.2 rejects models
- `#17481` Switch v3 drops Set data
- `#27131` typeValidation flips loose→strict
- `#27070` Switch regex `|` converted to newline
- `#18992` Sheets node v4.7 crashes с `Cannot read properties of undefined`
- [Forum 249698](https://community.n8n.io/t/upgrade-n8n-version-from-1-123-10-to-2-2-4-didnt-show-old-workflows/249698)
  «upgrade didn't show old workflows»

### Code sandbox / prototype

- `#27734` Prototype pollution в `@n8n_io/riot-tmpl` (expression engine,
  не Code node itself)
- `#16404` `pdf-lib` / `puppeteer` fail даже с
  `N8N_RUNNERS_ALLOW_PROTOTYPE_MUTATION=true`
- `#28222` Expression timeout bypass via long-running ops
- `#26865` n8n 2.11.2 crashes с SIGSEGV on ARM64 / Node 24 в Merge v3
  (`#26859`, `#26853`, `#26863`)

### Binary data corruption & exhaustion

- `#21746` too large to save execution state
- `#8028` filesystem fills up
- `#19405` returns `filesystem-v2` string as base64
- `#28354` Telegram strips Chinese punctuation из filenames

### Item linking lost → wrong data routing

Все `pairedItem` issues выше. Плюс:
- `#18181`, `#18180` HTTP Request node fails resolve expressions from
  trigger output including webhooks and chat triggers

### Integration-node drift

- **Google Sheets — #1 integration complaint:**
  - `#26460` daily OAuth disconnect
  - `#20946` «dummy.stack.replace is not a function» masking real errors
  - `#16750` quota after update
  - `#20900` dynamic doc ID fails on rerun
  - `#18349` «Get Rows» failing all workloads
  - `#20372` Append writes only to first sheet with two inputs
- **Postgres:**
  - `#20388` password с `$$` breaks
  - `#16354` comma in param breaks query
  - `#25408` «executes successfully but doesn't persist to Supabase»
  - `#21223` not using dedicated error output
  - `#23666` hangs indefinitely on SELECT
- **LinkedIn** `#28660` deprecated `LinkedIn-Version 20250401 → 426 NONEXISTENT_VERSION`
  — provider API deprecation not tracked by node
- **Monday** `#26071` deprecated API
- **Todoist** `#28441` token auth broken on 2.14.2 → 410

**Class:** core ships snapshot of provider API, нет freshness tracking.

### continueOnFail inconsistent

Listed в Execute Semantics. Class-wide.

### Memory leaks с large datasets

- `#20124` Workflow Editor freezes на >100 nodes
- `#16862` v1.99.1 memory leak in Code-node workflows
- `#15269` significant leaks from Code node use
- `#15628` runners memory leak OOM
- `#1583` Redis node leak
- `#27980` MongoDB Chat Memory leaks MongoClient instances

### Merge node specifically (cluster)

`#7775`, `#3949`, `#13182`, `#19001`, `#19624`, `#14986`, `#15334`, `#17529`,
`#18429`, `#18465`, `#19393`, `#26853`, `#26859`, `#26863` —
«doesn't wait for both inputs», «only outputs branch 1», «crashes on ARM64»,
«missing Merge-by-Index», «inputs gone after update». **Один из highest-churn nodes.**

---

## Correlation Table

| Ось | Проблема | Root cause в n8n | Nebula mitigation |
|---|---|---|---|
| Bug | Code node task-runner timeouts | External task broker с fragile discovery; нет warm pool | In-process `wasmtime` / `deno_core` sandbox pool sized by workflow concurrency; p99 latency visible as SLI |
| Bug | Sandbox prototype pollution | Node.js `vm` не security boundary | WASM sandbox с day one; memory/CPU limits per task |
| Ergonomics | `pairedItem` lineage loss | Author-responsibility, string-typed, нет validation | `ItemLineage` — typed edge в execution DAG, managed by engine, не node |
| Ergonomics | AI Agent churn breaks workflows | Sub-node wiring через stringly `supplyData` | Typed traits per sub-node role (`ChatModel`, `Memory`, `Tool`) с semver-stable API |
| Ergonomics | Node versioning silent breakage | Нет migration framework | `migrate_v(old, new, params) -> Result<Params>` required at registration; workflow save records all referenced migrations |
| Ergonomics | Binary data fills disk / too-large exec state | Нет size cap, нет TTL, execution payload stored inline | Binary as content-addressed blobs; execution row stores only hash; TTL reaper |
| Bug | Merge node behavioral regressions | Mode-specific JS с mutable state | Merge — pure operator на typed streams; modes — enum + functional |
| Ergonomics | continueOnFail inconsistent | Per-node author choice | Core contract: `Result<Outputs, NodeError>`; engine decides routing; `onError: Fail \| Route \| Retry` as schema |
| Bug | HTTP Request edge cases (int precision, XML, 302 cookies, empty body) | JS JSON.parse + ad-hoc body handling | Typed body decoder pipeline: `Option<Decoder>` chain, `serde_json` с `arbitrary_precision`, XML via `quick-xml`, cookie jar shared с retries |
| Bug | Provider API drift (LinkedIn/Monday/Todoist) | Node ships API snapshot | Node manifest pins provider-API version; freshness check в CI; deprecation warnings в UI |
| Ergonomics | Mega-nodes (Gmail 50 ops) | Один file, один set of displayOptions | Per-operation module file + typed inputs; UI composes из registry |
| Ergonomics | `$('Name').item` fragile на rename | Name-as-identifier | Stable internal node IDs; `$('Name')` — UI alias resolved к ID at save |
| Bug | SplitInBatches infinite loop / OOM | State-machine modelled via side effects | Loop — typed operator с bounded iteration count в schema; engine-enforced |
| Ergonomics | Community-node ecosystem instability | Runtime `require()` of npm packages | WASM plugin sandbox + typed plugin manifest (already Nebula direction per ADR) |
| Ergonomics | Memory leaks с large datasets | Node-by-node hoarding of references | Streaming item pipeline с backpressure; bounded channel между nodes |

---

## Quick Wins для Nebula (каждый ~10 LoC)

1. **`pairedItem` как typed `ItemLineage<InputId>`**, managed by engine.
   Every output item carries it automatically; custom Rust nodes get
   helpers. Убивает целый forum category.
2. **`#[derive(Migration)]` macro on `NodeParams`** с versioned structs.
   `V1 -> V2` — function; engine runs on workflow load. Workflows никогда
   silently не break on upgrade.
3. **Typed `OnError` enum в schema:** `Fail | RouteTo(OutputId) | Retry { max, backoff }`.
   Engine routes. Нет author-by-author implementation.
4. **Binary data — content-addressed blob** (`sha256` + store ref).
   Execution row stores hash only. 10-line reaper job deletes unreferenced.
   Убивает `#8028` + `#21746` class.
5. **`wasmtime`-based sandbox для Code node + community plugins** —
   CPU/memory limits enforced per call. Убивает timeout class +
   sandbox-escape class.
6. **Stable node IDs, не names.** Rename updates aliases. `$('Old Name')`
   still works; `$node_id_<uuid>` — canonical form. Убивает
   rename-breaks-references class.
7. **Typed HTTP client в core** (`reqwest` + `serde_json` с arbitrary
   precision + `quick-xml` + shared cookie jar). Каждый integration node
   использует это, не axios-ish per-node. Убивает HTTP Request edge-cases fleet.
8. **Streaming item pipeline via `tokio::sync::mpsc` с bounded capacity**
   — backpressure enforced. Убивает memory-leak-on-large-dataset class.
9. **Sub-node typed traits:** `trait ChatModel`, `trait Memory`, `trait Tool`.
   AI Agent depends on traits, не concrete nodes. Upgrade one model crate
   без breaking workflows.
10. **Operation-per-file для integration nodes,** registered via
    `inventory` crate. Gmail не exists as one 50-op mega-node — это 50
    small nodes в `gmail` namespace. UI groups them back at display time.
    Убивает `displayOptions` hell class at the source (также covered в
    parameter report).

---

## Ключевая мета-идея

Action-ноды n8n страдают от **author-responsibility model**: `pairedItem`,
`continueOnFail`, version migrations, error routing, concurrency — всё
это по-разному реализуется каждым автором. Это объясняет почему:
- одни nodes respect `continueOnFail`, другие нет
- `pairedItem` breaks в каждом нестандартном case
- version upgrades silent break workflows
- integration nodes drift с provider APIs
- community ecosystem нестабилен

Nebula должна flip это:

> **Engine-managed contracts с typed traits. Authors пишут только business
> logic; lineage, error routing, concurrency, versioning, binary lifecycle —
> ответственность engine'а.**

Это не квартет quick wins — это architecture-level shift от
metadata-declaration к typed-interface.

---

## Sources

### GitHub issues (selected)

**Code / task runner:**
- [#25171](https://github.com/n8n-io/n8n/issues/25171), [#25319](https://github.com/n8n-io/n8n/issues/25319),
  [#25986](https://github.com/n8n-io/n8n/issues/25986), [#23381](https://github.com/n8n-io/n8n/issues/23381),
  [#27353](https://github.com/n8n-io/n8n/issues/27353), [#27838](https://github.com/n8n-io/n8n/issues/27838),
  [#28625](https://github.com/n8n-io/n8n/issues/28625) (timeouts)
- [#25544](https://github.com/n8n-io/n8n/issues/25544) (phantom internal runner spawns)
- [#26926](https://github.com/n8n-io/n8n/issues/26926) (npm package import)
- [#26708](https://github.com/n8n-io/n8n/issues/26708) (secure sandbox proposal)
- [#15628](https://github.com/n8n-io/n8n/issues/15628) (runners memory leak)

**`pairedItem`:**
- [#14568 Execute Sub-workflow loses item linking](https://github.com/n8n-io/n8n/issues/14568)
- [#27767 AI Agent v3.1 $getPairedItem lineage error](https://github.com/n8n-io/n8n/issues/27767)
- [#15981 Merge $('NodeName').item fails](https://github.com/n8n-io/n8n/issues/15981)
- [#24534 Set node loses .item context after pin-data](https://github.com/n8n-io/n8n/issues/24534)
- [#4507 Postgres default query mode breaks pairedItems](https://github.com/n8n-io/n8n/issues/4507)
- [#12558 Continue-on-error loses reference](https://github.com/n8n-io/n8n/issues/12558)

**Versioning:**
- [#22489 AI Agent v2.2 rejects models](https://github.com/n8n-io/n8n/issues/22489)
- [#27131 typeValidation flips loose→strict](https://github.com/n8n-io/n8n/issues/27131)
- [#17481 Switch v3 drops Set data](https://github.com/n8n-io/n8n/issues/17481)
- [#27726 AI Agent JSON output format breaking change](https://github.com/n8n-io/n8n/issues/27726)
- [#24397 AI Agent tool v3 broken](https://github.com/n8n-io/n8n/issues/24397)
- [#21740 AI Agent v3 incompatible с tool v2.2](https://github.com/n8n-io/n8n/issues/21740)
- [#28572 Breaking-change doc module](https://github.com/n8n-io/n8n/issues/28572)

**HTTP Request:**
- [#26925 Large ints rounded in responses](https://github.com/n8n-io/n8n/issues/26925)
- [#27439 Retries on 200 с error body](https://github.com/n8n-io/n8n/issues/27439)
- [#27735 Cookies lost на 302 redirect](https://github.com/n8n-io/n8n/issues/27735)
- [#27782 Fails on empty JSON](https://github.com/n8n-io/n8n/issues/27782)
- [#27724 No XML parsing](https://github.com/n8n-io/n8n/issues/27724)
- [#26044 DELETE body stripped](https://github.com/n8n-io/n8n/issues/26044)
- [#16005 Bearer auth broken с pagination](https://github.com/n8n-io/n8n/issues/16005)
- [#27533 Multipart boundary mismatch](https://github.com/n8n-io/n8n/issues/27533)
- [#23815 Can't attach multiple binaries](https://github.com/n8n-io/n8n/issues/23815)
- [#27122 Form-data >2MB broken](https://github.com/n8n-io/n8n/issues/27122)

**AI Agent / Tools:**
- [#28215 googleGemini crashes on multi-turn tool call](https://github.com/n8n-io/n8n/issues/28215)
- [#26202 AI Agent receives empty tool response (FIFO queue ordering)](https://github.com/n8n-io/n8n/issues/26202)
- [#27805 Memory node doesn't work с tools](https://github.com/n8n-io/n8n/issues/27805)
- [#24042 Tool errors fail entire workflow](https://github.com/n8n-io/n8n/issues/24042)
- [#22112 Memory pollution via full intermediate tool outputs](https://github.com/n8n-io/n8n/issues/22112)
- [#26640 Gemini implicit caching broken](https://github.com/n8n-io/n8n/issues/26640)
- [#25966 AI Agent sends node name к LLM instead of tool name](https://github.com/n8n-io/n8n/issues/25966)

**Merge:**
- [#7775](https://github.com/n8n-io/n8n/issues/7775),
  [#3949](https://github.com/n8n-io/n8n/issues/3949),
  [#13182](https://github.com/n8n-io/n8n/issues/13182),
  [#15981](https://github.com/n8n-io/n8n/issues/15981),
  [#26853](https://github.com/n8n-io/n8n/issues/26853),
  [#26863](https://github.com/n8n-io/n8n/issues/26863)

**If/Switch:**
- [#23862 If node overrides adjacent If](https://github.com/n8n-io/n8n/issues/23862)
- [#27693 If forwards true to false branch](https://github.com/n8n-io/n8n/issues/27693)
- [#27070 Switch regex `|` → newline](https://github.com/n8n-io/n8n/issues/27070)
- [#25971](https://github.com/n8n-io/n8n/issues/25971)

**Split in Batches / Loop:**
- [#21376 Done output fires per-iteration](https://github.com/n8n-io/n8n/issues/21376)
- [#21817 Infinite loop → heap OOM](https://github.com/n8n-io/n8n/issues/21817)
- [#20630 Freezes on 45k items](https://github.com/n8n-io/n8n/issues/20630)
- [#16465](https://github.com/n8n-io/n8n/issues/16465)

**Binary data:**
- [#8028 Filesystem mode fills up disk](https://github.com/n8n-io/n8n/issues/8028)
- [#21746 Too large to save execution state](https://github.com/n8n-io/n8n/issues/21746)
- [#26968 S3 binary socket pool exhaustion](https://github.com/n8n-io/n8n/issues/26968)
- [#16673 Set Node should forward binaries by default](https://github.com/n8n-io/n8n/issues/16673)
- [#17432 Set drops binary](https://github.com/n8n-io/n8n/issues/17432)

**Integration drift:**
- [#28660 LinkedIn deprecated API](https://github.com/n8n-io/n8n/issues/28660)
- [#26071 Monday deprecated API](https://github.com/n8n-io/n8n/issues/26071)
- [#28441 Todoist token auth broken](https://github.com/n8n-io/n8n/issues/28441)
- [#26460 Google Sheets daily OAuth disconnect](https://github.com/n8n-io/n8n/issues/26460)
- [#20946 Google Sheets dummy.stack.replace masks errors](https://github.com/n8n-io/n8n/issues/20946)

**Community nodes:**
- [#27106 Update leaves broken node_modules](https://github.com/n8n-io/n8n/issues/27106)
- [#27833 Credential leak across workflows](https://github.com/n8n-io/n8n/issues/27833)
- [#26026 Duplicate name collision crashes install](https://github.com/n8n-io/n8n/issues/26026)
- [#19431 Community nodes can't getParameter for unmodified booleans](https://github.com/n8n-io/n8n/issues/19431)

### Community forum (hot threads)

- [Paired item unavailable (39875)](https://community.n8n.io/t/error-missing-paireditem-data/39875)
- [Item linking issues after Code (89235)](https://community.n8n.io/t/item-linking-issues-after-a-code-node/89235)
- [How is pairedItem supposed to work (228022)](https://community.n8n.io/t/how-is-paireditem-supposed-to-work-in-code-node/228022)
- [Update broke AI Agent (190031)](https://community.n8n.io/t/update-broke-ai-agent-node/190031)
- [Complex Workflows Unresponsive After Major Upgrade (91985)](https://community.n8n.io/t/complex-workflows-unresponsive-after-major-upgrade/91985)
- [HTTP Request hangs indefinitely (185220)](https://community.n8n.io/t/http-request-node-hangs-indefinitely-during-web-scraping-timeout-not-applied/185220)
- [Retry on Fail doesn't work (110925)](https://community.n8n.io/t/retry-on-fail-doesnt-work-for-http-request-node/110925)
- [HTTP 3-min timeout (33087)](https://community.n8n.io/t/http-request-3-min-timeout/33087)
- [Simple memory loses context (281016)](https://community.n8n.io/t/simple-memory-node-loses-context-between-messages/281016)
- [AI Agent has no memory of tool calls (147455)](https://community.n8n.io/t/ai-agent-has-no-memory-of-tool-calls-in-its-history/147455)
- [Troubles with pagination (72427)](https://community.n8n.io/t/troubles-with-pagination/72427)

---

## Связь с остальной документацией

- Auth architecture: [`n8n-auth-architecture.md`](./n8n-auth-architecture.md)
- Credential pain points: [`n8n-credential-pain-points.md`](./n8n-credential-pain-points.md)
- Parameter pain points: [`n8n-parameter-pain-points.md`](./n8n-parameter-pain-points.md)
- Trigger pain points: [`n8n-trigger-pain-points.md`](./n8n-trigger-pain-points.md)
- Для Nebula-ADR по **node execution contracts** — §Quick wins 1, 2, 3, 8, 9
  — кандидаты на отдельный ADR «Engine-managed node contracts».
- Для **STYLE.md** — §Ось 2 содержит anti-pattern catalog:
  author-responsibility for lineage, stringly-typed sub-node wiring,
  per-node concurrency ad-hoc, rename-breaks-references.
