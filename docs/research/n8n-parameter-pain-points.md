# n8n Parameters — Architecture & Pain Points (field report)

> Выжимка реальных жалоб на parameter-систему n8n: UI-формы, где юзер вводит
> данные для запуска node'ов и настраивает credentials. Структура отчёта по
> **трём осям**: Coverage (хватает ли типов), Ergonomics/Universality
> (удобство и гибкость), Bugs (реальные проблемы).
>
> Третий в серии после [n8n-auth-architecture.md](./n8n-auth-architecture.md)
> (как устроено auth) и [n8n-credential-pain-points.md](./n8n-credential-pain-points.md)
> (жалобы на credentials).

## Метаданные исследования

- **Последняя сверка:** 2026-04-20
- **Источники:**
  - `github.com/n8n-io/n8n` issues (open + closed, ~250 titles scanned, 13 keyword queries)
  - `community.n8n.io` форум
  - DeepWiki архитектурный summary
- **Охват:** frontend editor + workflow types backend (`packages/workflow`, `packages/frontend/editor-ui`)
- **Провенанс:** каждое утверждение подкреплено GitHub issue или forum-thread URL.
  «confirmed» — воспроизведено несколько раз или в коде; «hot» — одна жалоба
  повторилась 5+ раз в разных issues.

---

## Executive Summary — топ-5 pain areas

1. **Expression editor ↔ fixed-value toggle хрупкий.** Десятки closed issues про
   потерянные values, misinterpretation, literal source выдаваемый вместо evaluated.
   Type validation молча flip'ается strict↔loose (`#27131`), decimal separator
   переписывает values (`#24499`), emoji ломают parser (`#16498`, `#20519`).
   Drag-and-drop генерит неправильные `$json.field` пути на branching nodes
   (`#23395`, `#26592`). **Hot class — 25+ open+closed.**

2. **`loadOptions` caching и invalidation сломаны.** Forum threads подтверждают:
   options загружаются один раз и никогда не refresh'ятся если `loadOptionsDependsOn`
   не настроен идеально. resourceLocator cache — **confirmed bug `#22123`**
   «In-flight responses cached to the wrong key» (open). Dropdowns stall на
   больших датасетах (Outlook `#17078`, LDAP `#12478`, Matrix `#16379`).

3. **`fixedCollection` / `collection` UX хрупкий.** `#19607` (open) — fixedCollection
   с defaults не сохраняется в community nodes. `#6693` — не рендерится в
   credentials modal. `#1119` — redundant data persists когда options удалены.
   Populate fixedCollection через `loadOptionsMethod` не supported но широко requested.

4. **`resourceMapper` (новый, ~2024) с rough edges.** `#23884` — 404 при switch
   «Map Each Column Manually». NocoDB fields list empty после 1.111.0.
   Column-rename silently invalidates old mappings — PR #8478 пришлось добавлять
   «prune values not in schema». Google Sheets регулярно бросает «Column names
   were updated after the node's setup».

5. **`displayOptions` visibility systemic bugs.** `#13049` — node properties
   одного имени interfere. `#25803` (open) — нельзя expression на conditional
   driver и dependent одновременно. `#14974` — `getParameterResolveOrder`
   throws на unresolved dependencies. Name-collisions в nested collections.

---

## Architectural overview (~200 слов)

Источник: deepwiki + `packages/workflow/src/interfaces.ts`, `NodeHelpers.ts`,
`packages/frontend/editor-ui/src/components/*`.

`INodeProperties` — универсальный schema для node- и credential-parameter
definitions. Каждое property имеет `displayName`, `name`, `type`, `default`,
`description`, опциональный `displayOptions`, `typeOptions`, `required`, `routing`.
`type` — одна из ~20 строк, каждая маппится в distinct Vue renderer.

`displayOptions` keys `show` и `hide` принимают
`{otherParamName: [allowedValues] | DisplayCondition}` — evaluation синхронный JS
в `shouldDisplayNodeParameter` (`ParameterInputList.vue`) и `displayParameter`
(`NodeHelpers.ts`).

`typeOptions` несёт per-type knobs: `rows`, `minValue`, `maxValue`,
`loadOptionsMethod`, `loadOptionsDependsOn`, `numberPrecision`, `editorLanguage`,
`password`, `multipleValues`.

`loadOptions` вызывает метод на node's load-options handler через REST
(`/rest/dynamic-node-parameters/*`). `dependentParametersValues` в
`ParameterInput.vue` использует `computedAsync` — кеширование reactive-memoization,
не explicit TTL.

Expression toggle per-parameter: `isModelValueExpression` детектит `"={{…}}"`
prefix; `ParameterInputFull.vue` показывает `showExpressionSelector`,
`ExpressionParameterInput.vue` хостит редактор.

`resourceLocator` имеет три режима (`list`, `id`, `url`), конфигурируются per-node;
`ResourceLocator.vue` роутит. `routing` (declarative API calls) позволяет
property также описывать как его value вставляется в HTTP request — привлекательно
в принципе, но частый источник drift между UI и runtime (`#24360`, `#23093`).

---

## Ось 1 — COVERAGE (хватает ли типов?)

**~20 типов в каталоге:** `string`, `number`, `boolean`, `options`, `multiOptions`,
`collection`, `fixedCollection`, `resourceLocator`, `resourceMapper`,
`credentialsSelect`, `notice`, `hidden`, `json`, `dateTime`, `color`, `filter`,
`assignmentCollection`, `workflowSelector`, `cron`. Breadth в целом нормальная.

### Явные пробелы (с доказательствами)

| Чего не хватает | Доказательство | Что делают юзеры |
|---|---|---|
| `loadOptions` **внутри `fixedCollection`** | [feature req 70942](https://community.n8n.io/t/enable-loading-options-based-on-neighboring-values-in-multi-item-fixed-collections/70942) — not implemented | Задублировать property снаружи collection'а |
| **Populate fixedCollection from `loadOptionsMethod`** | [thread 6585](https://community.n8n.io/t/populate-a-fixedcollection-with-loadoptionsmethod/6585) | Manual copy-paste |
| File upload с **default/preview** | `#21905` required file accepts empty submission | Юзер не видит что файл не приложен |
| Schema-evolution tracking для `resourceMapper` | `#23884`, `#19327`, `#6770` | Refresh button вручную |
| **Pagination** в `resourceLocator` list | `#21148` Typeform «stuck not loading», `#17078` Outlook folders missing | Юзеры не могут найти item |
| **Expression-mode** для array-item addition | `#19982` | Нет workaround |
| **Expression-mode одновременно** на conditional driver и dependent | `#25803` (open) | Нет |
| **Date-range picker** | Запросы в форуме, нет issue-tracker | Два отдельных dateTime parameters |
| **JSON-schema-driven collection** | Запросы в форуме | Ручной fixedCollection |
| **Rich text / WYSIWYG** | Запросы в форуме | String + HTML knowledge |
| **Typed keyvalue** (не просто `{k:string, v:string}`) | `assignmentCollection` частично решает, но типы в строках | `json` + runtime parse |

### Implication для Nebula
- Rust typed schema → можно сразу **предусмотреть все эти типы** как first-class
  (`DateRange`, `FileUpload { default, mime_filter, preview }`, и т.д.).
- `loadOptions` внутри `fixedCollection` — **обязательно закладывать с day 1**:
  context resolver берёт не только parent-scope but и sibling-siblings по path.
- Pagination — не опция: `loadOptions` возвращает `{items, cursor, total?}` всегда.

---

## Ось 2 — ERGONOMICS / UNIVERSALITY (удобство и гибкость)

Главный источник боли — **expression ↔ fixed toggle**. Discriminator — `"="`
prefix (regex), не AST. Последствия:

### Expression parser fragility

- **[#15900](https://github.com/n8n-io/n8n/issues/15900) hot** — Set-node выводит
  литерал JS-source вместо evaluated результата.
- [#27131](https://github.com/n8n-io/n8n/issues/27131) — editor молча меняет
  `typeValidation` strict↔loose при открытии IF/Switch.
- [#24499](https://github.com/n8n-io/n8n/issues/24499) — decimal separator `.` → `,`
  по locale, корраптит числа.
- [#16498](https://github.com/n8n-io/n8n/issues/16498), [#20519](https://github.com/n8n-io/n8n/issues/20519),
  [#16262](https://github.com/n8n-io/n8n/issues/16262) — emoji (`❌`, `⛔`)
  ломают expression parser.
- [#21982](https://github.com/n8n-io/n8n/issues/21982) — autocomplete broken на
  корейских символах в node names.
- [#23395](https://github.com/n8n-io/n8n/issues/23395) — drag-and-drop генерит
  неправильные `$json.field` после append/merge.
- [#19982](https://github.com/n8n-io/n8n/issues/19982) — array additional
  parameters cannot be expressions — schema-level hole.
- [#22015](https://github.com/n8n-io/n8n/issues/22015) — cannot push credential с
  expression-valued field.
- [#24173](https://github.com/n8n-io/n8n/issues/24173) — `$vars` not available в
  resource-locator expressions для declarative nodes.
- [#27734](https://github.com/n8n-io/n8n/issues/27734) — **Prototype pollution
  vulnerability в `@n8n_io/riot-tmpl`** (!) — underscore fragility.

**Root cause pattern:** expression — regex-matched string prefix, не AST-backed
typed value. Parameter's «effective type» в runtime определяется re-parsing'ом,
не typed model на клиенте.

### Composability дыры

- `fixedCollection` нельзя наполнить из `loadOptionsMethod`
- `loadOptions` внутри `fixedCollection` не видит sibling-значения
  ([req 70942](https://community.n8n.io/t/enable-loading-options-based-on-neighboring-values-in-multi-item-fixed-collections/70942))
- `$fromAI` button invisible для полей с именем `"name"` ([#28261](https://github.com/n8n-io/n8n/issues/28261))
  — name-collision с framework semantics
- Expression-toggle скрыт в некоторых `resourceLocator` list-state → юзер не
  может переключиться когда list залип

### Copy/paste/migration lifecycle

- [#25307](https://github.com/n8n-io/n8n/issues/25307) — **clipboard copy-paste
  не работает на self-hosted** (Docker, HTTP). `navigator.clipboard` требует
  secure context.
- [#25254](https://github.com/n8n-io/n8n/issues/25254) — workflow import стёр
  все parameters и connections в 2.7.0 (severe data loss).
- [#19197](https://github.com/n8n-io/n8n/issues/19197) — community-node upgrade
  молча меняет defaults в existing workflows.
- [#27160](https://github.com/n8n-io/n8n/issues/27160) (open) — Git integration
  резетит non-default credential option values.
- [#19406](https://github.com/n8n-io/n8n/issues/19406) (open) — resource versioning
  causes duplicated actions.

### i18n / a11y

- [#21451](https://github.com/n8n-io/n8n/issues/21451) — browser translation of
  form dropdown menu breaks workflows (Google Translate rewrites `<option>` values).
- [#21982](https://github.com/n8n-io/n8n/issues/21982) — expression autocomplete
  fails на корейских node names.
- [#24091](https://github.com/n8n-io/n8n/issues/24091) — credential sharing dropdown
  не scroll'ится на macOS touchpad.
- [#13226](https://github.com/n8n-io/n8n/issues/13226), [#13055](https://github.com/n8n-io/n8n/issues/13055)
  — dropdowns без scrollbars.
- Нет issues про keyboard navigation в parameter forms — **signal**: вероятно,
  никто не добирается достаточно далеко чтобы filed it.

### Performance с many parameters

- [#16379](https://github.com/n8n-io/n8n/issues/16379) — Matrix send-message
  dropdown pulls all rooms каждый render.
- [#17078](https://github.com/n8n-io/n8n/issues/17078) — Outlook folders pagination
  incomplete; юзеры с большими mailbox не могут найти folders.
- HTTP Request node — canonical «100+ parameters» case; нет specific perf issue,
  но «Connection lost» cluster ([#28484](https://github.com/n8n-io/n8n/issues/28484),
  [#28710](https://github.com/n8n-io/n8n/issues/28710)) correlates с heavy panels.

---

## Ось 3 — BUGS / PROBLEMS (реальные баги и проблемы)

### 1. Expression editor (уже покрыто в Оси 2)

Дополнительно:
- [#26392](https://github.com/n8n-io/n8n/issues/26392) cluster — `$now` показывает
  current time в *historical* execution views.
- [#16112](https://github.com/n8n-io/n8n/issues/16112) — Luxon options не работают.
- [#25265](https://github.com/n8n-io/n8n/issues/25265) — `.replace().match()`
  chaining работает иногда, не всегда.
- [#28222](https://github.com/n8n-io/n8n/issues/28222) CLOSED — expression timeout
  bypass через long-running ops.

### 2. `loadOptions` — caching, dependencies, perf

- **Confirmed**: [#22123](https://github.com/n8n-io/n8n/issues/22123) (open) —
  ResourceLocator in-flight response caching keys on wrong value; два concurrent
  load'а на slow network могут swap results.
- [#27499](https://github.com/n8n-io/n8n/issues/27499) (open) — unable to filter
  MySQL table list.
- [#27652](https://github.com/n8n-io/n8n/issues/27652) (open) — OpenAI model list
  fail'ится когда custom credential header использует expression — `loadOptions`
  evaluated **без** resolve credential expressions.
- [#26579](https://github.com/n8n-io/n8n/issues/26579) (open) — Weaviate
  «Tenant Name» default undefined.
- [#21148](https://github.com/n8n-io/n8n/issues/21148) — Typeform node
  «stuck not loading list» с юзер unable переключиться на expression.
- [#16379](https://github.com/n8n-io/n8n/issues/16379) — Matrix node freezes on
  many rooms.
- [#26917](https://github.com/n8n-io/n8n/issues/26917) — `[object Object]` как
  user-facing error.

**Forum:**
- [Re-trigger loadOptions (190792)](https://community.n8n.io/t/how-to-re-trigger-loadoptions/190792)
  — single-shot без `dependsOn`.
- loadOptions внутри fixedCollection не видит siblings
  ([70942](https://community.n8n.io/t/enable-loading-options-based-on-neighboring-values-in-multi-item-fixed-collections/70942)).

**Root cause pattern:** `loadOptions` — synchronous-ish REST call с reactive memo,
без explicit TTL, без version key per-credential, без per-call tracing.
`loadOptionsDependsOn` opt-in, авторы regularly under-declare.

### 3. `fixedCollection` / `collection`

- **Confirmed / open**: [#19607](https://github.com/n8n-io/n8n/issues/19607) —
  fixedCollection с defaults не сохраняется в community nodes.
- [#6693](https://github.com/n8n-io/n8n/issues/6693) — fixedCollection не
  рендерится в credentials modal.
- [#1119](https://github.com/n8n-io/n8n/issues/1119) — redundant data persisted
  когда collection options removed.
- [#1318](https://github.com/n8n-io/n8n/issues/1318) — collection parameter с
  same name collisions.
- [#13049](https://github.com/n8n-io/n8n/issues/13049) — node properties одного
  имени interfere (related).
- [#23347](https://github.com/n8n-io/n8n/issues/23347) — Outlook v2 Draft:
  attachments (fixedCollection) не могут быть added.

**Forum:**
- [Format fixedCollection в JSON nested object (19225)](https://community.n8n.io/t/format-a-fixedcollection-in-a-json-nested-object/19225)
  — property name появляется дважды в output.
- [Interface bug with fixed collection input type (25437)](https://community.n8n.io/t/interface-bug-with-fixed-collection-input-type-or-am-i-doing-something-wrong/25437)
  — duplicate rendering at top.
- [Custom node fixedCollection → API mismatch (82454)](https://community.n8n.io/t/custom-node-help-convert-fixed-collection-data-to-api-compatible-format/82454)
  — shape `{key: [{…}]}` удивляет authors.

**Root cause pattern:** fixedCollection output shape unusual (wrapper object
keyed by variant name) и poorly specified для authors. Persisted state in DB
carries obsolete sub-keys forever.

### 4. `resourceMapper`

- **Hot**: [#23884](https://github.com/n8n-io/n8n/issues/23884) — DataTable Insert
  calls `/rest/dynamic-node-parameters/resource-mapper-fields` → 404 на switch
  mapping mode.
- [#19327](https://github.com/n8n-io/n8n/issues/19327) — NocoDB fields list
  empty после 1.111.0.
- PR #8478 — added pruning schema-missing values.
- [#6770](https://github.com/n8n-io/n8n/issues/6770) — Postgres auto-mapping fails.
- [#27590](https://github.com/n8n-io/n8n/issues/27590) — date field в form не
  показывает default value.

**Forum:**
- [Columns updated after setup (267814)](https://community.n8n.io/t/column-names-were-updated-after-the-nodes-setup-refresh-the-columns-list-on-the-column-to-match-on-parameter/267814)
- [Google Sheets «Column names were updated» (68334)](https://community.n8n.io/t/google-sheets-error-column-names-were-updated-after-the-nodes-setup/68334)

**Root cause pattern:** resourceMapper persists snapshot of schema at save-time,
but *live* schema evolves; нет «schema version pin» или «migrate mappings on
schema drift» step. Manual refresh button — единственный escape.

### 5. `displayOptions` и property dependencies

- [#25803](https://github.com/n8n-io/n8n/issues/25803) (open) — нельзя expression
  и на conditional driver и на dependent field.
- [#14974](https://github.com/n8n-io/n8n/issues/14974) — `getParameterResolveOrder`
  throws `ApplicationError` на unresolved dependencies.
- [#13049](https://github.com/n8n-io/n8n/issues/13049) — node properties одного
  имени interfere.
- [#126](https://github.com/n8n-io/n8n/issues/126) (legacy closed) — «Properties
  that depend on other properties» был long-running tracker.
- [#28261](https://github.com/n8n-io/n8n/issues/28261) (open) — `$fromAI` button
  не показан для fields с именем `"name"` в AI Agent tool mode —
  name-collision с framework semantics.

**Root cause pattern:** `displayOptions` evaluation runs against single flat
parameter map, так что name collisions across siblings разных `fixedCollection`
variants ambiguous. Нет cycle detection в dependency graph.

### 6. Validation и required fields

- [#21905](https://github.com/n8n-io/n8n/issues/21905) — **required file field
  accepts empty** submission в n8n forms.
- [#24286](https://github.com/n8n-io/n8n/issues/24286) — options missing required
  field в n8n form.
- [#22378](https://github.com/n8n-io/n8n/issues/22378) — form default-value query
  params не работают с dropdown/checkbox/radio.
- [#25913](https://github.com/n8n-io/n8n/issues/25913) — OpenAI node: `"developer"`
  role shows validation warning несмотря на required для Responses API.
- [#19431](https://github.com/n8n-io/n8n/issues/19431) — community node fails с
  «Could not get parameter» для unmodified boolean field.
- [#19319](https://github.com/n8n-io/n8n/issues/19319) — Webhook returns 200 но
  «JSON parameter needs to be valid JSON» — contradictory.

### 7. Community-node authoring mistakes

- [#27833](https://github.com/n8n-io/n8n/issues/27833) (closed) — **community-node
  credentials не isolated per workflow** — все workflow резолвятся на последнюю
  saved credential. **High-severity class.**
- [#23877](https://github.com/n8n-io/n8n/issues/23877) — community OAuth2 nodes
  игнорят user-entered scopes.
- [#19607](https://github.com/n8n-io/n8n/issues/19607) — fixedCollection-with-defaults
  unsavable.
- [#4037](https://github.com/n8n-io/n8n/issues/4037) — lintfix destroys declarative
  properties через `node-param-options-type-unsorted-items`.

---

## Correlation table: проблема → root cause → Nebula mitigation

| Ось | Проблема | Root cause в n8n | Nebula mitigation |
|---|---|---|---|
| Bug | Expression toggle loses value / literal source | `"="` prefix single discriminator; regex reparse | `enum ParamValue { Fixed(T), Expression(ExprAST) }` с explicit tag; никакого prefix-sniffing |
| Bug | Emoji/CJK ломают parser | Regex не Unicode-aware | Unicode-aware lexer (`logos`/`chumsky`); тесты над UCD BMP |
| Bug | Decimal `.` → `,` (#24499) | Locale-dependent JS `Number`/`toString` в UI | Canonical C-locale обе стороны; round-trip test |
| Bug | Type validation silently flips (#27131) | Implicit default derived from node version | `type_validation: Strict \| Loose` — explicit required field; без implicit migration |
| Bug | loadOptions cache pollution (#22123) | Promise cache key на stale request hash | Cache key = `(method, deps_snapshot, credential_id, version)`; cancel in-flight |
| Ergonomics | loadOptions никогда не refreshes | `loadOptionsDependsOn` opt-in; не supported в fixedCollection | Dependencies структурно выводятся из expression AST; explicit refresh всегда |
| Coverage | resourceMapper schema drift (#23884, column-rename) | Saved mapping — plain snapshot | Mapping хранит `(schema_version, column_id, column_name)`, prune missing on resolve, delta в UI |
| Ergonomics | fixedCollection name collisions | Property map flat by `name` | Параметры адресуются path `a.b.c[0].d`; collision detection at schema-load |
| Bug | displayOptions cycle / can't depend both ways | Evaluator без cycle detection | DAG из `show`/`hide`; reject cycles at schema-load с actionable error |
| Bug | `$fromAI` invisible для `name`-named fields | Framework использует `name` как reserved keyword | Reserved-names table в docs + schema validator warns authors |
| Bug | Community node leaks credentials across workflows | Только static ESLint rule | Resolver keyed by `(workflow_id, node_id, type)`; runtime assertion |
| Ergonomics | Clipboard blocked на self-hosted (#25307) | `navigator.clipboard` requires secure context | `document.execCommand('copy')` fallback + visible HTTPS-missing notice |
| Coverage | resourceLocator list stalls на больших sets | Single REST call без pagination spec | `loadOptions` возвращает `{items, cursor}`; editor «Load more»; server-side search |
| Bug | Required-file accepts empty (#21905) | Client-only required check | Server re-validates против schema перед execution; typed error |
| Ergonomics | JS-on-client type validation | Типы described но не compiled | Schema = Rust типы; codegen TypeScript из Rust (single source of truth) |
| Ergonomics | Copy-paste corrupts expressions | Clipboard payload — raw string | Copy carries `{version, paramType, value, expression}` JSON blob; paste validates |
| Coverage | loadOptions inside fixedCollection | Context resolver берёт только parent scope | Context resolver берёт sibling siblings по path |

---

## Quick wins для Nebula (каждый ~10 LoC)

1. **Tag `ParamValue` как `{Fixed(T), Expression(ExprAST)}` в Rust** и derive
   JSON (de)serializers. Убивает класс «expression leaked as literal» и
   «value lost on toggle» outright.
2. **Unicode-aware lexer** (`logos` с `#[unicode]`) — закрывает
   emoji/Korean/CJK баги на compile time.
3. **Cache key для dynamic options = `(method, credential_version, deps_hash)`**
   + per-request `AbortSignal`. Убивает `#22123`-class stale responses.
4. **Schema DAG validator at node-registration time**: detect `displayOptions`
   cycles и reject. Авторы find out на `cargo build`, не at user runtime.
5. **Paths для параметров, не flat names** (`node.params.a.b[0].c`). Убивает
   name-collision class (`#13049`, `#1318`).
6. **`loadOptions` returns `{items, cursor, total?}` и UI auto-paginates.**
   Убивает «folders missing» + «stalls on large set».
7. **Required validation runs server-side always.** One match arm в executor.
8. **Default-value migration requires explicit `migrate_from` в node manifest.**
   Forces авторов подумать; kills silent regressions (`#19197`).
9. **Clipboard payload — JSON с shape marker**, feature-detected.
   `navigator.clipboard.writeText(JSON.stringify({v:1, type, value}))` с
   `execCommand` fallback. Работает over HTTP.
10. **`ParamValue` CRDT-friendly с day one** (content-addressed, version counter).
    Делает undo/redo и workflow diff trivial; делает «git pull wipes tokens»
    (credential issue `#26499`) структурно невозможным потому что config и
    runtime layers — distinct types.

---

## Ключевая мета-идея для Nebula

n8n's parameter-система — это **TypeScript-модель, описанная но не compiled**,
с **regex-based discriminators** и **stringly-typed expressions**. Почти
каждый баг из топа — прямое следствие. Nebula может flip это одним
архитектурным решением:

> **Rust-типы как Single Source of Truth → codegen TypeScript клиента из Rust
> → AST для expressions вместо regex → Unicode-aware lexer.**

Это закрывает 7 из 10 quick wins выше **архитектурно**, без продолжающейся
bug-chase мешанины.

---

## Sources

### GitHub issues (selection, отсортированы по теме)

**Expression editor:**
- [#27131 typeValidation silently flips strict↔loose](https://github.com/n8n-io/n8n/issues/27131)
- [#15900 Set-node выводит literal source](https://github.com/n8n-io/n8n/issues/15900)
- [#24499 Set-node decimal separator silently changed](https://github.com/n8n-io/n8n/issues/24499)
- [#23395 Drag-and-drop генерит wrong $json.field](https://github.com/n8n-io/n8n/issues/23395)
- [#16498 Emoji ломает Expression Parser](https://github.com/n8n-io/n8n/issues/16498)
- [#20519 Emoji ломает HTTP body expression](https://github.com/n8n-io/n8n/issues/20519)
- [#21982 Korean node names ломают autocomplete](https://github.com/n8n-io/n8n/issues/21982)
- [#27734 Prototype pollution в riot-tmpl](https://github.com/n8n-io/n8n/issues/27734)
- [#19982 Array additional params не могут быть expressions](https://github.com/n8n-io/n8n/issues/19982)

**loadOptions:**
- [#22123 ResourceLocator cache pollution (confirmed)](https://github.com/n8n-io/n8n/issues/22123)
- [#27499 Unable to filter MySQL table list](https://github.com/n8n-io/n8n/issues/27499)
- [#27652 loadOptions fails когда credential header — expression](https://github.com/n8n-io/n8n/issues/27652)
- [#21148 Typeform resourceLocator stuck](https://github.com/n8n-io/n8n/issues/21148)
- [#17078 Outlook folders missing в dropdown](https://github.com/n8n-io/n8n/issues/17078)
- [#16379 Matrix node freezes на many rooms](https://github.com/n8n-io/n8n/issues/16379)
- [#26917 [object Object] как user-facing error](https://github.com/n8n-io/n8n/issues/26917)

**fixedCollection / collection:**
- [#19607 fixedCollection с defaults can't save](https://github.com/n8n-io/n8n/issues/19607)
- [#6693 fixedCollection не рендерится в credentials modal](https://github.com/n8n-io/n8n/issues/6693)
- [#1119 Redundant collection data persisted](https://github.com/n8n-io/n8n/issues/1119)
- [#13049 Properties одного имени interfere](https://github.com/n8n-io/n8n/issues/13049)
- [#23347 Outlook v2 Draft attachments can't add](https://github.com/n8n-io/n8n/issues/23347)

**resourceMapper:**
- [#23884 ResourceMapper 404 на mode switch](https://github.com/n8n-io/n8n/issues/23884)
- [#19327 NocoDB resourceMapper fields empty](https://github.com/n8n-io/n8n/issues/19327)
- [#6770 Postgres auto-mapping fails](https://github.com/n8n-io/n8n/issues/6770)

**displayOptions:**
- [#25803 Can't expression both conditional driver и dependent](https://github.com/n8n-io/n8n/issues/25803)
- [#14974 getParameterResolveOrder crashes на unresolved deps](https://github.com/n8n-io/n8n/issues/14974)
- [#28261 $fromAI button invisible для fields с именем "name"](https://github.com/n8n-io/n8n/issues/28261)

**Validation:**
- [#21905 Required file field accepts empty](https://github.com/n8n-io/n8n/issues/21905)
- [#24286 Options missing required field в form](https://github.com/n8n-io/n8n/issues/24286)

**Lifecycle:**
- [#25307 Clipboard copy-paste broken на self-host](https://github.com/n8n-io/n8n/issues/25307)
- [#25254 Workflow import strips parameters](https://github.com/n8n-io/n8n/issues/25254)
- [#19197 Default value silently changes on node upgrade](https://github.com/n8n-io/n8n/issues/19197)
- [#27160 Git integration resets credential options](https://github.com/n8n-io/n8n/issues/27160)

**i18n / a11y:**
- [#21451 Browser translation breaks form dropdown](https://github.com/n8n-io/n8n/issues/21451)
- [#24091 Credential sharing dropdown won't scroll на macOS](https://github.com/n8n-io/n8n/issues/24091)

**Community nodes:**
- [#27833 Community node credentials leak across workflows](https://github.com/n8n-io/n8n/issues/27833)
- [#23877 Community OAuth2 nodes игнорят scopes](https://github.com/n8n-io/n8n/issues/23877)

### Community forum

- [Re-trigger loadOptions (190792)](https://community.n8n.io/t/how-to-re-trigger-loadoptions/190792)
- [Dynamic Properties (5449)](https://community.n8n.io/t/dynamic-properties/5449)
- [Columns updated after setup (267814)](https://community.n8n.io/t/column-names-were-updated-after-the-nodes-setup-refresh-the-columns-list-on-the-column-to-match-on-parameter/267814)
- [ResourceMapper required field (32041)](https://community.n8n.io/t/resourcemapper-required-field/32041)
- [fixedCollection nested JSON duplicate (19225)](https://community.n8n.io/t/format-a-fixedcollection-in-a-json-nested-object/19225)
- [fixedCollection UI duplicate (25437)](https://community.n8n.io/t/interface-bug-with-fixed-collection-input-type-or-am-i-doing-something-wrong/25437)
- [Enable loadOptions в multi-item fixedCollection (70942)](https://community.n8n.io/t/enable-loading-options-based-on-neighboring-values-in-multi-item-fixed-collections/70942)
- [Populate fixedCollection с loadOptionsMethod (6585)](https://community.n8n.io/t/populate-a-fixedcollection-with-loadoptionsmethod/6585)
- [Google Sheets «Column names were updated» (68334)](https://community.n8n.io/t/google-sheets-error-column-names-were-updated-after-the-nodes-setup/68334)
- [Custom node fixedCollection API format (82454)](https://community.n8n.io/t/custom-node-help-convert-fixed-collection-data-to-api-compatible-format/82454)

---

## Связь с остальной документацией

- Архитектурный reference auth: [`n8n-auth-architecture.md`](./n8n-auth-architecture.md)
- Жалобы на credentials: [`n8n-credential-pain-points.md`](./n8n-credential-pain-points.md)
- Для Nebula-ADR по **parameter/schema design**: §11 (Quick wins) должны
  попасть в ADR «Typed parameter schema» или закрыться тестами.
- Для **STYLE.md**: §Ось 2 содержит anti-pattern catalog — regex-based
  expression discrimination, flat parameter namespace, loadOptions без
  dependency-AST. Стоит занести когда будет разбираться соответствующий crate.
