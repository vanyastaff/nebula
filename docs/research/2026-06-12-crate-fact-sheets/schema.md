# nebula-schema — fact sheet

## Назначение
Типизированная конфигурационная схема для всех интеграционных концептов (Actions, Credentials, Resources); замена удалённого `nebula-parameter`. Реализует proof-token pipeline «lint → validate → resolve»: `Schema::builder().build() -> ValidSchema`, `ValidSchema::validate -> ValidValues`, `ValidValues::resolve(ctx).await -> ResolvedValues` — пропустить шаг невозможно на уровне типов (канон-инварианты L1-3.5, L1-4.5). Плюс secret-типы (zeroize) и опциональный экспорт JSON Schema Draft 2020-12.

## Публичная поверхность
- `Schema` / `SchemaBuilder` — src/schema.rs:41 / :418; `MAX_SCHEMA_DEPTH: u8 = 64` — src/schema.rs:694
- `Field` (unified enum: String/Number/Boolean/Secret/Select/Object/List/Mode/Computed/Dynamic/Notice/File/Code) — src/field.rs:790; `ModeVariant` :593, `ComputedReturn` :714, `NoticeSeverity` :768
- `ValidSchema` — src/validated.rs:85; `ValidValues` :410; `ResolvedValues` :602; `ResolvedLookup` :610; `SchemaFlags` :43; `FieldHandle` :54
- `FieldValues` / `FieldValue` — src/value.rs:306 / :35; `EXPRESSION_KEY = "$expr"` :17; строгий ингест ключей (`invalid_key`), `try_set_raw` (panic-вариант `set_raw` удалён)
- `ValidationError` / `ValidationReport` / `ValidationErrorBuilder` / `Severity` / `STANDARD_CODES` — src/error.rs:22/:151/:92/:12/:284
- `FieldKey(Arc<str>)` — src/key.rs:20; макрос `field_key!` (compile-time-валидация) — macros/src/lib.rs:43
- `HasSchema` трейт + `schema_of<T>()` — src/has_schema.rs:20/:37 (единственный путь схем Action/Credential, ADR-0052 P3); `HasSelectOptions` :44
- `#[derive(Schema)]` / `#[derive(EnumSelect)]` — macros/src/lib.rs:71/:82 (attrs/derive_enum/derive_schema/type_infer)
- `ExpressionContext` трейт / `Expression` / `ExpressionAst` / `EvalFuture` (BoxFuture-алиас вместо async_trait) — src/expression.rs:43/:79/:64/:15
- Secret: `SecretString` / `SecretBytes` (Zeroizing) — src/secret.rs:27/:129; `SecretValue` :203; `SecretWire` :248; `SECRET_REDACTED` :18; feature `audit-secret-expose`
- `Loader<T>` / `LoaderRegistry` / `LoaderContext` / `OptionLoader` / `RecordLoader` — src/loader.rs:183/:304/:35/:276/:278 (async-загрузчики опций/записей)
- Builder DSL (typed closures): `FieldCollector` — src/builder/mod.rs:41; `ObjectBuilder`/`ListBuilder`/`GroupBuilder` + leaf-билдеры (String/Number/Boolean/Secret/Select/Code)
- Режимы: `VisibilityMode` / `RequiredMode` / `ExpressionMode` — src/mode.rs:10/:32/:54; виджеты по семействам — src/widget.rs
- `FieldPath` / `PathSegment` — src/path.rs:43/:13; `Transformer` — src/transformer.rs:14; `InputHint` — src/input_hint.rs:20
- `ValidSchema::json_schema()` + `JsonSchemaExportError` (feature `schemars`) — src/json_schema.rs:22; `SCHEMA_WIRE_VERSION: u16 = 1` — src/lib.rs:247
- Re-export `nebula_validator::{Predicate, Rule}` — src/lib.rs:231 (для derive-расширения и авторов схем)

## Workspace-зависимости
Зависит от: `nebula-validator`, `nebula-expression`, `nebula-schema-macros` (вложенный proc-macro крейт `macros/`); внешние: serde, serde_json, indexmap, smallvec, regex, zeroize, hex, tracing, subtle, schemars (opt).
От него зависят: nebula-metadata, nebula-api (с feature `schemars`; разрешено ADR-0052 P4), nebula-action, nebula-engine, nebula-resource, nebula-credential, nebula-plugin, nebula-sdk. Итого 8 потребителей — один из самых нагруженных Core-крейтов.

## Структура модулей
- `schema.rs` — draft-модель `Schema`/`SchemaBuilder`, вход в proof-token pipeline
- `validated.rs` — typestate-цепочка `ValidSchema`/`ValidValues`/`ResolvedValues`
- `field.rs` — unified `Field` enum + все виды полей (крупнейший модуль)
- `builder/` (mod/object/list/group) — typed-closure DSL поверх полей
- `lint.rs` — структурные линт-проходы (дубликаты ключей, кросс-полевые инварианты)
- `error.rs` — `ValidationError`/`ValidationReport`, канонические коды
- `value.rs` — `FieldValues`/`FieldValue`, wire-формат, `$expr`
- `expression.rs` — `ExpressionContext` seam (реализуется nebula-expression)
- `context.rs` — построение `PredicateContext` для validator (visibility/required)
- `secret.rs` — секреты с zeroize + subtle (const-time eq) + redacted Debug
- `loader.rs` — реестр async-загрузчиков опций select/записей
- `has_schema.rs` — `HasSchema`/`schema_of` связка Rust-тип → схема
- `json_schema.rs` — экспорт Draft 2020-12 + `x-nebula-*` extensions (feature-gated)
- `key.rs`, `path.rs`, `mode.rs`, `option.rs`, `widget.rs`, `input_hint.rs`, `transformer.rs` — типы-примитивы
- `field_tree.rs`, `rule_ref.rs` — pub(crate) обход дерева / парсинг rule-ссылок
- `macros/` — proc-macro: `field_key!`, `#[derive(Schema)]`, `#[derive(EnumSelect)]`
- Тесты: seam_*-контракты (required-emitter, root-rule-scrub, single-crossing, proof-token custody, security codes), ~30 trybuild compile_fail, proptest, insta; 6 criterion-бенчей

## Напряжения
- `ValidValues::raw_values` — `#[deprecated(note = "use raw() instead")]` — src/validated.rs:430; единственный deprecated, можно снести
- Легаси-форма rule-ссылок `$root.foo` поддерживается рядом с JSON Pointer — src/rule_ref.rs:17, src/lint.rs:88, :644 (двойной синтаксис, кандидат на унификацию)
- `has_schema.rs:73` и :105 — baseline `HasSchema` impl для `FieldValues` помечен как «legacy code paths / типы, ещё не объявившие реальную схему» — мягкий обходной путь вокруг ADR-0052 P3 «schema_of — единственный путь»
- AGENTS.md:29 «Cross-crate calls go through nebula-eventbus» — сомнительно для Core-крейта без зависимости на eventbus (скопировано из общего шаблона); README/AGENTS в остальном согласованы с кодом
- TODO/FIXME в src/ отсутствуют; `#![forbid(unsafe_code)]`, `missing_docs` warn — гигиена высокая

## Роль в credential/resource redesign
Потребитель-инфраструктура, сам не перестраивается. Связи: (1) credential write-path валидирует `data` через схему до persist (ADR-0052 P4), `schema_of` — единственный путь схем Credential; (2) `SecretString`/`SecretValue` — типы секретов, которые credential-rewrite использует на стыке; (3) resource использует схемы конфигов. Запрет re-add KDF/hashing (project_schema_no_kdf) остаётся в силе — в коде KDF нет. Коллапс крейтов за sole-public-sdk схему не затрагивает напрямую.
