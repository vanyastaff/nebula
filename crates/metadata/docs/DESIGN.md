# nebula-metadata — design

| Field | Value |
|-------|-------|
| **Status** | Frontier (AGENTS-метка устарела; контент-стабильный Core-крейт после ADR-0090) |
| **Layer** | Core / cross-cutting (нет восходящих зависимостей; листовой набор типов) |
| **Redesign role** | **Не мишень редизайна, общая база.** ADR-0090 закрепил: `nebula-metadata` ОСТАЁТСЯ отдельным Core-крейтом (не сливать в `nebula-core`); симметричный by-value `metadata()` API по Action/Credential/Resource уже сделан (PR #784). Поверхность крейта не меняется ни в credential-rewrite (ADR-0088/0092), ни в resource-teardown (ADR-0093). |
| **Related** | ADR-0018 (plugin = контейнер-дескриптор), ADR-0090 (отдельный Core-крейт + by-value API), PRODUCT_CANON §3.5 (one pattern, five concepts), canon-инвариант L2-3.5 |

---

## 1. Назначение и границы

`nebula-metadata` владеет общими метаданными «каталожных листьев» (action / credential /
resource). Каждый такой лист несёт один и тот же префикс: типизированный ключ, имя, описание,
канонический input-schema, версию и каталожные «украшения» (icon, documentation_url, tags,
maturity, deprecation). Крейт даёт этот префикс как конкретный тип `BaseMetadata<K>` плюс трейт
`Metadata` с default-делегацией аксессоров — чтобы бизнес-слойные крейты КОМПОНОВАЛИ общую базу
через `#[serde(flatten)]`, а не переобъявляли её с несовместимыми именами полей. Отдельно крейт
держит дескриптор плагина-контейнера `PluginManifest` (slice B, перенесён из `nebula-plugin`).

**Владеет:** `BaseMetadata<K>` и трейт `Metadata`; малые типы-украшения (`Icon`, `MaturityLevel`,
`DeprecationNotice`); entity-agnostic compat-правила (`validate_base_compat` + `BaseCompatError<K>`:
ключ неизменен, версия монотонна, schema-break требует major-bump); `PluginManifest` + builder +
`ManifestError`.

**ЯВНО НЕ делает:** не валидирует schema-контент (это `nebula-schema`, тип `ValidSchema` приходит
готовым); не несёт entity-specific compat-правила (inputs/outputs/pattern и т.п. — у потребителей,
они оборачивают `BaseCompatError`); НЕ компонует `BaseMetadata` в `PluginManifest` — плагин это
контейнер, а не схематизированный лист, у него нет input-schema (ADR-0018); не делает cross-crate
вызовов (чисто типовой крейт — `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`).

## 2. Публичная поверхность

| Item | Где |
|------|-----|
| `BaseMetadata<K>` — общий префикс; `#[non_exhaustive]`, serde-flatten-композиция | `src/base.rs:31` |
| `BaseMetadata::new(key, name, description, schema)` + builder `with_version`/`with_icon`/`with_tags`/`add_tag`/`with_documentation_url` | `src/base.rs:74-147` |
| `mark_experimental`/`mark_beta`/`mark_stable`/`with_maturity`/`deprecate`/`with_deprecation` (deprecation ⇒ maturity=Deprecated) | `src/base.rs:151-191` |
| `trait Metadata { type Key; fn base() }` — остальные 11 аксессоров default-делегируют (key/name/description/schema/version/schema_arc/icon/documentation_url/tags/maturity/deprecation) | `src/base.rs:206-268` |
| `BaseCompatError<K>` — enum `KeyChanged`/`VersionRegressed`/`SchemaChangeWithoutMajorBump` | `src/compat.rs:22-48` |
| `validate_base_compat(current, previous)` — ключ immutable, версия монотонна, schema-change ⇒ major bump | `src/compat.rs:61-84` |
| `Icon` — untagged enum `None`/`Inline(String)`/`Url{url}`; `inline()`/`url()`/`as_inline()`/`as_url()`/`is_none()` | `src/icon.rs:18-67` |
| `MaturityLevel` — `Experimental`/`Beta`/`Stable`(default)/`Deprecated` + `is_unstable()`/`is_deprecated()` | `src/maturity.rs:17-43` |
| `DeprecationNotice` — `since: Version` + опц. `sunset`/`replacement`/`reason` + builder | `src/deprecation.rs:21-67` |
| `PluginManifest` — контейнер-дескриптор (key/name/version/group/description/icon/color/tags/author/license/homepage/repository/nebula_version/maturity/deprecation), приватные поля + геттеры | `src/manifest.rs:78-228` |
| `PluginManifestBuilder` — `PluginManifest::builder(key, name)`; `build()` нормализует ключ и форсит deprecation⇒Deprecated независимо от порядка вызовов | `src/manifest.rs:231-396` |
| `ManifestError` — `MissingRequiredField`/`InvalidKey(PluginKeyParseError)`, `derive(Classify)` | `src/manifest.rs:24-37` |
| `lib.rs` — плоский re-export всего вышеперечисленного | `src/lib.rs:25-30` |

## 3. Зависимости и зависимые

- **Deps** (`Cargo.toml:14-20`): `nebula-core` (для `PluginKey`), `nebula-error` (+derive, `Classify`),
  `nebula-schema` (`ValidSchema`), `semver` (+serde), `serde`, `thiserror`.
  Dev: `serde_json`, `insta`, `pretty_assertions`, `rstest`.
- **Зависимые:** `nebula-action`, `nebula-credential`, `nebula-resource`, `nebula-plugin`, `nebula-sdk`
  (`crates/{action,credential,resource,plugin,sdk}/Cargo.toml`).

## 4. Внутренняя архитектура

Семь модулей, чистая типовая декомпозиция без потоков выполнения:

- `src/lib.rs` — проводка модулей + плоские re-export'ы (31 строка).
- `src/base.rs` — `BaseMetadata<K>` + трейт `Metadata` (аксессоры default-делегируют через `base()`).
- `src/compat.rs` — generic compat-правила; entity-specific остаются у потребителей.
- `src/icon.rs` — `Icon` как единственное валидное представление иконки (заменил пару `Option<String>`).
- `src/maturity.rs` — `MaturityLevel`.
- `src/deprecation.rs` — `DeprecationNotice`.
- `src/manifest.rs` — `PluginManifest` + builder + `ManifestError`.

Поток данных: потребитель строит `BaseMetadata<K>` через `new()` + builder-цепочку, кладёт во встроенное
поле `base` своей конкретной метадаты, реализует `Metadata::base()` в одну строку и получает 11 аксессоров
бесплатно. На версионных переходах потребитель вызывает `validate_base_compat(current, previous)` и оборачивает
`BaseCompatError<K>` в свой entity-specific error-enum. `PluginManifest` строится отдельной веткой через
`PluginManifestBuilder`, минуя `BaseMetadata`.

## 5. Инварианты и контракты

- **Ключ immutable (compat).** `validate_base_compat` отвергает смену ключа (`KeyChanged`) — стабильность
  идентичности каталожного листа (canon §3.5, инвариант L2-3.5). `src/compat.rs:61-84`.
- **Версия монотонна.** Регресс версии ⇒ `VersionRegressed`; гарантирует forward-only эволюцию каталога.
- **Schema-break ⇒ major bump.** Любое изменение `schema` без мажорного bump ⇒ `SchemaChangeWithoutMajorBump` —
  semver-совместимость каталога by-construction на этом seam.
- **Deprecation ⇒ Deprecated by-construction.** И в `BaseMetadata` (`src/base.rs:151-191`), и в
  `PluginManifestBuilder::build()` (`src/manifest.rs:231-396`) установка deprecation форсит
  `maturity = Deprecated` независимо от порядка builder-вызовов — нельзя получить deprecated-но-не-Deprecated.
- **Нормализация ключа манифеста.** `PluginManifestBuilder::build()` приводит ключ (lowercase, `space→_`)
  перед валидацией через `PluginKey` — единый канон идентификатора плагина.
- **`Icon` как единственное валидное представление.** Untagged enum исключает невозможные состояния
  (нет одновременных `icon`/`icon_url`).
- **`#![forbid(unsafe_code)]`.** Безопасность памяти крейта тривиально доказуема.

## 6. Известные напряжения / долг (честно)

1. **README устарел vs Cargo.toml.** `README.md:29-31` утверждает «зависит только от nebula-schema, semver,
   serde, thiserror», но в deps есть `nebula-core` и `nebula-error` (`Cargo.toml:15-16`), оба нужны `manifest.rs`.
2. **README устарел vs код.** Секция Public API (`README.md:35-51`) не упоминает `PluginManifest`/`ManifestError`;
   `README.md:98` пишет «`nebula-plugin::PluginManifest`», хотя манифест уже живёт ЗДЕСЬ (`src/manifest.rs:10-14`).
3. **Устаревший rationale переноса.** `src/manifest.rs:11-14` объясняет перенос нуждой `nebula-plugin-sdk`
   («zero engine-side deps, canon §7.1»), но pivot 2026-06-09 отказался от out-of-process plugin-sdk. Обоснование
   переноса повисло (сам перенос безвреден — манифест остаётся уместным в Core).
4. **Дубли мелких хелперов.** `default_version`/`is_default_version` (`base.rs:9-14` и `manifest.rs:47-52`),
   `is_default_maturity` (`base.rs:64` и `manifest.rs:55`) — копипаста внутри одного крейта.
5. **`Metadata::schema_arc()` несостыковка имени.** `base.rs:240-242` обещает Arc именем, возвращает
   `ValidSchema` (дешёвый clone) — лёгкое расхождение имени и сигнатуры.
6. **Мёртвый вариант ошибки.** `ManifestError::MissingRequiredField` (`manifest.rs:28`) существует, но builder
   его никогда не возвращает — оба обязательных поля передаются в `builder()` позиционно.
7. **Шаблонный AGENTS-пункт.** `AGENTS.md:24` «cross-crate calls go through nebula-eventbus» нерелевантен для
   чисто типового крейта без вызовов.
8. **Устаревший статус/дата.** `AGENTS.md` status: frontier, last-reviewed 2026-04-19 — после ADR-0090 (PR #784)
   не обновлены; контент крейта по сути стабилен.

## 7. Роль в пост-0092 credential/resource модели

`nebula-metadata` — это **общий фундамент схематизации каталога**, на который опираются три перестраиваемых
поддомена, но сам он остаётся неизменным.

- **Credential (ADR-0088/0092).** После консолидации `nebula-credential` = один крейт (contract + runtime +
  `CredentialService` facade + builtin types; крейты credential-runtime/builtin/testutil/vault удалены).
  Его `CredentialMetadata` по-прежнему компонует `BaseMetadata<CredentialKey>` и оборачивает `BaseCompatError`.
  Криптография ушла в `nebula-crypto` (ADR-0088) — это НЕ затрагивает metadata: крейт никогда не касался secret-типов
  или шифрования. Seam «registered types → schema → API catalog» проходит через metadata: схема каталога берётся из
  зарегистрированных типов через `HasSchema → nebula-metadata → API catalog`, а не из per-instance persistence
  (values-only). То есть metadata — это путь, по которому schema конкретного scheme/типа добирается до каталога;
  при этом сам secret-материал и routing-by-`policy(&State)` к metadata отношения не имеют.

- **Resource (ADR-0093).** `nebula-resource` владеет per-slot rotation fan-out (`credential_fanout/`),
  `SlotCell`, Manager и топологией; teardown-контракт (reset/destroy fallible-async) — всё это поведенческое и
  metadata не касается. `ResourceMetadata` остаётся тонкой композицией `BaseMetadata<ResourceKey>` без
  entity-specific полей; единственная роль крейта здесь — давать стабильный typed-prefix и compat-правила для
  каталога ресурсов.

- **Consumer binding.** action/resource объявляют `#[credential]`/`#[resource]` слоты и получают
  `CredentialGuard<Scheme>`; слоты (`slot_bindings`) отделены от параметров; persistence — values-only, а схема
  поднимается из зарегистрированных типов через `HasSchema → nebula-metadata`. metadata здесь — источник schema/identity
  для каталога, не участник runtime-резолвинга (resolver/refresh/lease/rotation-state живут в credential/resource).

- **Что меняется / что остаётся.** Меняется только окружение: соседние крейты консолидируются и инвертируют
  порты. Сам `nebula-metadata` НЕ меняет поверхность — это и есть его роль в редизайне (стабильная общая база,
  ADR-0090). При single-public-sdk pivot крейт становится приватной impl-деталью, реэкспортируемой через
  `nebula-sdk` (sdk уже зависит от него) — публичным остаётся только то, что sdk реэкспортирует.

## 8. Forward design / открытые вопросы

- **Освежить README/AGENTS** под фактическое состояние: добавить `nebula-core`/`nebula-error` в раздел deps,
  включить `PluginManifest`/`ManifestError` в Public API, убрать «manifest живёт в nebula-plugin», обновить
  status/last-reviewed после ADR-0090. Низкий риск, чистый doc-debt.
- **Удалить устаревший rationale переноса манифеста** (`manifest.rs:11-14`): plugin-sdk-обоснование мертво после
  pivot 2026-06-09; заменить на «манифест — Core-уровневый дескриптор контейнера, ADR-0018». Сам перенос оставить.
- **Дедуп мелких хелперов** (`default_version`/`is_default_version`/`is_default_maturity`) в один внутренний
  модуль — устранить копипасту base.rs↔manifest.rs.
- **Решить судьбу `schema_arc()`**: либо вернуть реальный `Arc<...>`, либо переименовать в честное `schema()` /
  убрать — устранить расхождение имени и сигнатуры до того, как новые потребители завяжутся на текущую форму.
- **Удалить мёртвый `ManifestError::MissingRequiredField`** ИЛИ перевести обязательные поля манифеста с позиционных
  аргументов `builder()` на builder-сеттеры с проверкой в `build()` — тогда вариант станет достижимым by-construction.
- **Открытый вопрос (single-public-sdk).** При переходе на единственный публичный `nebula-sdk` нужно зафиксировать,
  какие именно типы metadata sdk реэкспортирует (вся поверхность vs только `BaseMetadata`/`Metadata`/`Icon`/`MaturityLevel`);
  непубличные станут свободны для рефакторинга без semver-ограничений. Решить до публикации sdk.
