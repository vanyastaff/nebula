# nebula-schema — design

| Field | Value |
|-------|-------|
| **Status** | Frontier — ядро (lint → validate → resolve) стабильно; периферия (UI-hints, JSON Schema export Phase 4) в pragmatic-baseline |
| **Layer** | Core — типизированная конфигурационная поверхность для всех интеграционных концептов |
| **Redesign role** | **Не перестраивается** (потребитель-инфраструктура). Затронут косвенно: предоставляет схему-валидацию write-path кредов (ADR-0052 P4), `schema_of` как единственный путь схем Action/Credential (ADR-0052 P3), secret-типы на стыке credential-rewrite |
| **Related** | [ADR-0080](../../../docs/adr/0080-schema-validation-platform.md) (schema & validation platform, absorbs 0052/0058-0064), PRODUCT_CANON L1-3.5 / L1-4.5, [README](../README.md), siblings: nebula-validator, nebula-expression |

---

## 1. Назначение и границы

`nebula-schema` — это **типизированная конфигурационная схема** для всех интеграционных
концептов (Actions, Credentials, Resources); прямая замена удалённого крейта
`nebula-parameter`. Центральная ценность — proof-token pipeline «lint → validate → resolve»,
где пропустить шаг невозможно на уровне типов (канон-инварианты L1-3.5, L1-4.5):
`Schema::builder().build() -> ValidSchema`, затем `ValidSchema::validate -> ValidValues`,
затем `ValidValues::resolve(ctx).await -> ResolvedValues`.

**Владеет:** draft-моделью схемы (`Schema`/`SchemaBuilder`), unified-перечислением полей
`Field` (String/Number/Boolean/Secret/Select/Object/List/Mode/Computed/Dynamic/Notice/File/Code),
typestate-цепочкой proof-token (`ValidSchema`/`ValidValues`/`ResolvedValues`), wire-форматом
значений (`FieldValues`/`FieldValue`, ключ `$expr`), структурным линтом, канонической
таксономией ошибок валидации (`ValidationError`/`ValidationReport`/`STANDARD_CODES`),
secret-типами с zeroize (`SecretString`/`SecretBytes`/`SecretValue`), реестром async-загрузчиков
опций/записей (`Loader`/`LoaderRegistry`) и опциональным экспортом JSON Schema Draft 2020-12.

**ЯВНО НЕ делает:** не является движком правил валидации — программные предикаты и
декларативные `Rule` живут в `nebula-validator` (реэкспортируется отсюда для авторов схем);
не вычисляет выражения — резолюция делегируется caller-supplied `ExpressionContext`
(реализуется `nebula-expression`); не рендерит UI-формы — несёт UI-hints как данные, рендер
снаружи; **не делает KDF/hashing** (удалено как слабый дубликат Argon2id из nebula-credential,
project_schema_no_kdf — в коде KDF отсутствует, re-add запрещён).

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `Schema` / `SchemaBuilder`; `MAX_SCHEMA_DEPTH: u8 = 64` | `src/schema.rs:41` / `:418` / `:694` |
| `Field` (unified enum); `ModeVariant` / `ComputedReturn` / `NoticeSeverity` | `src/field.rs:790` / `:593` / `:714` / `:768` |
| `ValidSchema` / `ValidValues` / `ResolvedValues` / `ResolvedLookup`; `SchemaFlags`; `FieldHandle` | `src/validated.rs:85` / `:410` / `:602` / `:610` / `:43` / `:54` |
| `FieldValues` / `FieldValue`; `EXPRESSION_KEY = "$expr"`; `try_set_raw` (panic-вариант `set_raw` удалён) | `src/value.rs:306` / `:35` / `:17` |
| `ValidationError` / `ValidationReport` / `ValidationErrorBuilder` / `Severity` / `STANDARD_CODES` | `src/error.rs:22` / `:151` / `:92` / `:12` / `:284` |
| `FieldKey(Arc<str>)`; макрос `field_key!` (compile-time-валидация) | `src/key.rs:20`; `macros/src/lib.rs:43` |
| `HasSchema` + `schema_of<T>()` (единственный путь схем Action/Credential, ADR-0052 P3); `HasSelectOptions` | `src/has_schema.rs:20` / `:37` / `:44` |
| `#[derive(Schema)]` / `#[derive(EnumSelect)]` | `macros/src/lib.rs:71` / `:82` |
| `ExpressionContext` / `Expression` / `ExpressionAst` / `EvalFuture` (BoxFuture-алиас вместо async_trait) | `src/expression.rs:43` / `:79` / `:64` / `:15` |
| `SecretString` / `SecretBytes` (Zeroizing); `SecretValue`; `SecretWire`; `SECRET_REDACTED`; feature `audit-secret-expose` | `src/secret.rs:27` / `:129` / `:203` / `:248` / `:18` |
| `Loader<T>` / `LoaderRegistry` / `LoaderContext` / `OptionLoader` / `RecordLoader` | `src/loader.rs:183` / `:304` / `:35` / `:276` / `:278` |
| `FieldCollector` (typed-closure DSL: Object/List/Group + leaf-билдеры) | `src/builder/mod.rs:41` |
| `VisibilityMode` / `RequiredMode` / `ExpressionMode`; виджеты по семействам | `src/mode.rs:10` / `:32` / `:54`; `src/widget.rs` |
| `FieldPath` / `PathSegment`; `Transformer`; `InputHint` | `src/path.rs:43` / `:13`; `src/transformer.rs:14`; `src/input_hint.rs:20` |
| `ValidSchema::json_schema()` + `JsonSchemaExportError` (feature `schemars`); `SCHEMA_WIRE_VERSION: u16 = 1` | `src/json_schema.rs:22`; `src/lib.rs:247` |
| Re-export `nebula_validator::{Predicate, Rule}` | `src/lib.rs:231` |

## 3. Зависимости и зависимые

- **Зависит от (workspace):** `nebula-validator` (предикаты/правила), `nebula-expression`
  (резолюция), `nebula-schema-macros` (вложенный proc-macro крейт `macros/`).
- **Внешние:** serde, serde_json, indexmap, smallvec, regex, zeroize, hex, tracing, subtle,
  schemars (opt).
- **Зависимые (8 потребителей — один из самых нагруженных Core-крейтов):** nebula-metadata,
  nebula-api (с feature `schemars`, разрешено ADR-0052 P4), nebula-action, nebula-engine,
  nebula-resource, nebula-credential, nebula-plugin, nebula-sdk.

## 4. Внутренняя архитектура

Поток данных следует трём фазам proof-token pipeline и разнесён по модулям:

- **draft → ValidSchema:** `schema.rs` строит draft-`Schema` через `SchemaBuilder`; `lint.rs`
  прогоняет структурные проходы (дубликаты ключей, кросс-полевые инварианты, max-depth);
  `build()` возвращает `ValidSchema` или `ValidationReport`.
- **ValidSchema → ValidValues:** `validated.rs` держит typestate-цепочку; `value.rs` несёт
  wire-формат `FieldValues`/`FieldValue` (строгий ингест ключей, `$expr`); `context.rs`
  строит `PredicateContext` для validator (visibility/required); единственное schema→validator
  пересечение — `validate_rules_with_ctx` + `resolve_field_policies`.
- **ValidValues → ResolvedValues:** `expression.rs` — seam `ExpressionContext` (реализуется
  nebula-expression), async-резолюция через `EvalFuture` (BoxFuture-алиас, без async_trait).
- **Поддержка:** `field.rs` (unified `Field` + все виды полей, крупнейший модуль);
  `builder/` (mod/object/list/group — typed-closure DSL); `secret.rs` (zeroize + subtle
  const-time eq + redacted Debug); `loader.rs` (реестр async-загрузчиков опций select/записей);
  `has_schema.rs` (Rust-тип → схема); `json_schema.rs` (экспорт Draft 2020-12 + `x-nebula-*`,
  feature-gated); примитивы `key.rs`/`path.rs`/`mode.rs`/`option.rs`/`widget.rs`/
  `input_hint.rs`/`transformer.rs`; pub(crate) `field_tree.rs`/`rule_ref.rs`.
- **macros/:** `field_key!`, `#[derive(Schema)]`, `#[derive(EnumSelect)]`.
- **Тесты:** seam-контракты (required-emitter, root-rule-scrub, single-crossing, proof-token
  custody, security codes), ~30 trybuild compile_fail, proptest, insta, 6 criterion-бенчей.

## 5. Инварианты и контракты

- **L1-4.5 — proof-token by-construction.** `ValidValues`/`ResolvedValues` — compile-time-evident
  токены: нельзя вызвать `resolve` без `ValidValues`, нельзя читать резолвнутые поля без
  `ResolvedValues`. Никаких runtime-флагов (`validated.rs`).
- **Единственное пересечение schema→validator** (ADR-0052 P2). Все правила пересекают границу
  ровно один раз через `validate_rules_with_ctx` + `resolve_field_policies`; код провалившегося
  `Rule` (`min_length`, `max`, `invalid_format`, …) пробрасывается verbatim — крейт не делает
  namespace-remap; schema-owned структурные коды (`type_mismatch`, `items.*`, `option.*`,
  `mode.*`, `expression.*`, `required`) неизменны.
- **`schema_of<T>()` — единственный путь схем Action/Credential** (ADR-0052 P3); per-trait
  `*_schema`-методы устранены (`has_schema.rs`).
- **Строгий ингест ключей.** `FieldValues::from_json` отклоняет невалидные ключи объекта
  кодом `invalid_key`, а не молча роняет их (`value.rs`).
- **Expression-required поля** отклоняют литералы кодом `expression.required` на validate-time.
- **Секреты by-construction.** `SecretString`/`SecretBytes` — Zeroizing; const-time равенство
  через subtle; Debug редактирован (`SECRET_REDACTED`); экспонирование секрета — за feature
  `audit-secret-expose` (`secret.rs`).
- **Гигиена:** `#![forbid(unsafe_code)]`, `missing_docs` = warn, TODO/FIXME в `src/` отсутствуют.

## 6. Известные напряжения / долг (честно)

1. **Единственный `#[deprecated]`** — `ValidValues::raw_values` («use `raw()` instead»,
   `src/validated.rs:430`). Кандидат на снос — реальных причин держать нет.
2. **Двойной синтаксис rule-ссылок.** Легаси-форма `$root.foo` поддерживается рядом с
   JSON Pointer (`src/rule_ref.rs:17`, `src/lint.rs:88`, `:644`) — кандидат на унификацию на
   один синтаксис.
3. **Мягкий обходной путь вокруг ADR-0052 P3.** Baseline `HasSchema` impl для `FieldValues`
   (`src/has_schema.rs:73` и `:105`) помечен как «legacy code paths / типы, ещё не объявившие
   реальную схему» — частично подтачивает инвариант «`schema_of` — единственный путь». Должен
   уйти, когда все потребители объявят реальные схемы.
4. **Стейл-строка в AGENTS.md.** `AGENTS.md:29` «Cross-crate calls go through nebula-eventbus»
   сомнительна для Core-крейта без зависимости на eventbus (скопировано из общего шаблона);
   README/AGENTS в остальном согласованы с кодом.

## 7. Роль в пост-0092 credential/resource модели

`nebula-schema` — **потребитель-инфраструктура**, которая сама не перестраивается коллапсом
credential/resource-крейтов, но несёт несколько load-bearing швов для новой модели:

- **Write-path валидация кредов (ADR-0052 P4).** Объединённый `nebula-credential` (contract +
  runtime + `CredentialService` facade + builtin-типы) валидирует `data` через схему **до
  persist**. Схема приходит через `schema_of<Scheme>()` — единственный путь схем Credential
  (P3); per-trait `*_schema` устранены. Этот шов не меняется при коллапсе крейтов: то, что
  `credential-runtime`/`builtin`/`testutil`/`vault` удалены и слиты в один крейт, не трогает
  контракт «values-only persistence + схема из зарегистрированных типов».
- **Цепочка обнаружения схем.** Авторская связка такова: тип реализует `HasSchema` →
  `nebula-metadata` собирает метаданные → `nebula-api` отдаёт каталог. nebula-schema —
  фундамент этой цепочки; именно поэтому nebula-api получил право зависеть на nebula-schema
  с feature `schemars` (P4), не таща при этом нижнеуровневые типы в DTO.
- **Secret-типы на стыке.** `SecretString`/`SecretValue`/`SecretBytes` — это типы секретов,
  которые credential-rewrite использует на швах ввода/хранения. **Граница чёткая:**
  nebula-schema владеет *типами* секрета (zeroize, redacted Debug, const-time eq), но **не**
  крипто-примитивами — AES-256-GCM/Argon2id и порты `Cipher`/`Kdf` живут в `nebula-crypto`
  (ADR-0088/0092). KDF/hashing сюда не возвращается (project_schema_no_kdf).
- **Resource-конфиги.** `nebula-resource` (владелец per-slot rotation fan-out, SlotCell,
  Manager, topology) использует схемы конфигов ресурсов через тот же proof-token pipeline и
  `schema_of`. Биндинг-модель «слоты (`slot_bindings`) отдельно от параметров» означает, что
  схема описывает *параметры* концепта, а не binding-слоты — это разделение остаётся за
  пределами nebula-schema (в action/resource-авторинге).
- **Что НЕ меняется:** ядро pipeline, таксономия кодов, единственное validator-пересечение,
  proof-token custody. Конференц-коррекции credential (policy(&State)-routing, OwnerScopedKey,
  узкий типизированный RefreshTransport seam, lease first-class) — это контракты внутри
  credential-рантайма; nebula-schema их **не** реализует и от них не зависит.

## 8. Forward design / открытые вопросы

- **Снять `#[deprecated]` `raw_values`** (см. §6.1) — чистый low-risk шаг, удаляет единственный
  deprecated-элемент публичной поверхности.
- **Унифицировать rule-ссылки** на один синтаксис (JSON Pointer), удалив легаси `$root.foo`
  (§6.2) — затрагивает `rule_ref.rs` + `lint.rs`, нужен сценарий миграции для авторов схем.
- **Закрыть baseline `HasSchema` для `FieldValues`** (§6.3): как только все потребители
  объявят реальные схемы, удалить legacy-impl и сделать «`schema_of` — единственный путь»
  истинным by-construction, а не by-convention.
- **Phase 4 JSON Schema export → stable.** Сейчас в pragmatic-baseline; стабилизировать набор
  `x-nebula-*` расширений (expression/required/visibility modes, root rules, UI/runtime hints)
  и зафиксировать их как версионированный контракт (сейчас `SCHEMA_WIRE_VERSION: u16 = 1`).
- **Phase-5 unified authoring (`#[property]`) — NOT-YET-BUILT.** Унифицированный авторинг
  полей/слотов через атрибуты ещё не существует; когда он появится, derive-поверхность
  (`#[derive(Schema)]`/`field_key!`) — естественная точка интеграции, но это отдельная фаза.
- **Поправить стейл-строку eventbus в AGENTS.md** (§6.4) — документационный долг, не код.
- **Риск нагрузки:** 8 потребителей делают любую breaking-правку публичной поверхности дорогой;
  изменения proof-token-типов и кодов ошибок должны идти через ADR-цикл (как ADR-0052), а не
  ad-hoc.
