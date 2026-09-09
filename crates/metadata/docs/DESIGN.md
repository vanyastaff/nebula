# nebula-metadata — design

| Field | Value |
|-------|-------|
| **Status** | **Stable** (issue 996, 2026-09-08: `README.md`/`docs/MATURITY.md` status flipped `frontier` → `stable`; §6 doc-debt below — accumulated since ADR-0090 — resolved in the same change: dedup'd defaults, `schema_arc()` removed, dead `MissingRequiredField` made reachable, README/AGENTS refreshed) |
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
| `trait Metadata { type Key; fn base() }` — остальные 10 аксессоров default-делегируют (key/name/description/schema/version/icon/documentation_url/tags/maturity/deprecation) — `schema_arc()` УДАЛЁН issue 996 (`metadata.schema().clone()` полностью заменяет) | `src/base.rs:195-251` |
| `BaseCompatError<K>` — enum `KeyChanged`/`VersionRegressed`/`SchemaChangeWithoutMajorBump` | `src/compat.rs:22-48` |
| `validate_base_compat(current, previous)` — ключ immutable, версия монотонна, schema-change ⇒ major bump | `src/compat.rs:61-84` |
| `Icon` — untagged enum `None`/`Inline(String)`/`Url{url}`; `inline()`/`url()`/`as_inline()`/`as_url()`/`is_none()` | `src/icon.rs:18-67` |
| `MaturityLevel` — `Experimental`/`Beta`/`Stable`(default)/`Deprecated` — `is_unstable()`/`is_deprecated()` УДАЛЕНЫ issue 996 (0 in-workspace вызовов) | `src/maturity.rs:17-27` |
| `DeprecationNotice` — `since: Version` + опц. `sunset`/`replacement`/`reason` + builder | `src/deprecation.rs:21-67` |
| `defaults.rs` (private, НЕ re-export) — `default_version`/`is_default_version`/`is_default_maturity`, написаны один раз issue 996 вместо дублей в `base.rs`+`manifest.rs` | `src/defaults.rs` |
| `PluginManifest` — контейнер-дескриптор (key/name/version/group/description/icon/color/tags/author/license/homepage/repository/nebula_version/maturity/deprecation), приватные поля + геттеры | `src/manifest.rs:110-274` |
| `PluginManifestBuilder` — `PluginManifest::builder(key, name)`; `build()` нормализует ключ, форсит deprecation⇒Deprecated независимо от порядка вызовов, и (issue 996) отвергает пустое/whitespace-only `name` | `src/manifest.rs:277-469` |
| `ManifestError` — `MissingRequiredField`/`InvalidKey(PluginKeyParseError)`, `derive(Classify)` — `MissingRequiredField` теперь ДОСТИЖИМ issue 996 (`build()` возвращает его для `name`) | `src/manifest.rs:64-81` |
| `lib.rs` — плоский re-export всего вышеперечисленного (кроме `defaults` — приватный `mod`) | `src/lib.rs:29-34` |

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

Пункты 1-8 — **✅ RESOLVED issue 996** (2026-09-08, frontier→stable), оставлены как запись истории
долга и как doc-debt чек-лист на будущее, а не как открытые задачи. Пункт 9 — новый, найденный тем же
review pass'ом и намеренно **не исправленный** (вне утверждённого объёма) — принятый, а не молчаливый
долг.

1. **✅ RESOLVED. README устарел vs Cargo.toml.** Было: `README.md:29-31` утверждал «зависит только от
   nebula-schema, semver, serde, thiserror», но в deps есть `nebula-core` и `nebula-error` (`Cargo.toml:15-16`),
   оба нужны `manifest.rs`. Исправлено: README §Role теперь перечисляет все шесть зависимостей.
2. **✅ RESOLVED. README устарел vs код.** Было: секция Public API не упоминала `PluginManifest`/`ManifestError`/
   `PluginManifestBuilder`/`PluginDependency`; README писал «`nebula-plugin::PluginManifest`», хотя манифест уже
   жил ЗДЕСЬ. Исправлено: Public API перечисляет все re-export'ы `lib.rs`; §Consumers прямо говорит, что
   `PluginManifest` живёт в этом крейте и `nebula_plugin::PluginManifest` — лишь re-export.
3. **✅ RESOLVED. Устаревший rationale переноса.** Было: `src/manifest.rs:11-14` объяснял перенос нуждой
   `nebula-plugin-sdk` («zero engine-side deps, canon §7.1»), но pivot 2026-06-09 (ADR-0091, in-process registry)
   отказался от out-of-process plugin-sdk — крейт `nebula-plugin-sdk` больше не существует. Исправлено:
   module-doc `manifest.rs` теперь обосновывает размещение через ADR-0018 (container-descriptor split at the
   Core layer) и упоминает ретирмент `nebula-plugin-sdk` как историю, а не как живое обоснование.
4. **✅ RESOLVED. Дубли мелких хелперов.** Было: `default_version`/`is_default_version`/`is_default_maturity`
   продублированы байт-в-байт в `base.rs` и `manifest.rs`. Исправлено: вынесены в приватный `src/defaults.rs`
   (`pub(crate) fn`), оба модуля используют `crate::defaults::{...}`; `#[expect(clippy::trivially_copy_pass_by_ref)]`
   написан один раз.
5. **✅ RESOLVED. `Metadata::schema_arc()` несостыковка имени.** Было: обещал Arc именем, возвращал дешёвый
   `ValidSchema`-clone. Исправлено: метод удалён (0 in-workspace вызовов подтверждено) — `metadata.schema().clone()`
   полностью его заменяет (`ValidSchema` сам — `Clone`-обёртка над `Arc`).
6. **✅ RESOLVED. Мёртвый вариант ошибки.** Было: `ManifestError::MissingRequiredField` существовал, но builder
   никогда его не возвращал — оба обязательных поля передавались в `builder()` позиционно. Исправлено (TDD,
   red→green): `PluginManifestBuilder::build()` теперь отвергает пустое/whitespace-only `name`, возвращая
   `MissingRequiredField { field: "name" }`; вариант остался (не удалён), стал достижим by-construction.
7. **✅ RESOLVED (запись была сама устаревшей). Шаблонный AGENTS-пункт.** Пункт долга описывал строку
   `AGENTS.md:24` «cross-crate calls go through nebula-eventbus» как нерелевантную для чисто типового крейта.
   На момент issue 996 эта строка в `AGENTS.md` уже отсутствовала — сам код был исправлен раньше, а именно эта
   запись §6 осталась висеть как стале-долг. Убрано отсюда.
8. **✅ RESOLVED (наполовину сама запись была неверна). Устаревший статус/дата.** Было: пункт долга
   утверждал «`AGENTS.md`/`README.md` status: frontier, last-reviewed 2026-04-19 — не обновлялись после
   ADR-0090». Проверено (`git show HEAD:crates/metadata/AGENTS.md` до issue 996): у `AGENTS.md` НЕТ
   YAML-frontmatter вообще — ни `status`, ни `last-reviewed` там никогда не было; только `README.md`
   когда-либо их нёс. Пункт долга был неверен насчёт `AGENTS.md` уже на момент написания, и переписывание
   этой записи в §6 issue 996 расширило неточность, а не исправило её — тот же класс stale-записи, что и
   пункт 7 выше. Исправлено: `README.md` frontmatter → `status: stable`, `last-reviewed: 2026-09-08`;
   `docs/MATURITY.md` API-stability cell для `nebula-metadata` → `stable` (issue 996); `AGENTS.md` не
   получил и не нуждается в `status`/`last-reviewed` — у него никогда не было этих полей.
9. **⚠️ ACCEPTED DEBT (issue 996 review, найдено не исправлено — вне утверждённого объёма).**
   `PluginManifestBuilder::build()` (`src/manifest.rs:436`) валидирует `self.name.trim().is_empty()`,
   но хранит `name: self.name` (`src/manifest.rs:452`) — **нетримленным**. Соседняя строка нормализует
   `key` через `normalize_key()` перед сохранением; `name` такой нормализации не получает, так что
   `PluginManifest::builder("slack", "  Slack  ")` проходит валидацию (не пусто после trim) и хранит имя
   с ведущими/хвостовыми пробелами. Тримминг при сохранении — за пределами утверждённого объёма issue 996
   (задача была «сделать `MissingRequiredField` достижимым», не «нормализовать хранимое имя») и намеренно
   НЕ исправлен в этом изменении. Зафиксировано здесь как явно принятый долг, а не молчаливый.

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

Пять из шести пунктов, ранее перечисленных здесь (README/AGENTS refresh, устаревший rationale переноса, дедуп
хелперов, судьба `schema_arc()`, мёртвый `MissingRequiredField`), закрыты issue 996 — см. §6 (все восемь пунктов
долга отмечены `✅ RESOLVED`). Ниже — только решённый ранее открытый вопрос single-public-sdk, записанный как
решение, а не как вопрос.

- **РЕШЕНО (single-public-sdk re-export set), issue 996.** `nebula-sdk`'s prelude (`crates/sdk/src/prelude.rs`)
  реэкспортирует **всю** публичную поверхность этого крейта: `BaseMetadata`, `Metadata`, `Icon`, `MaturityLevel`,
  `DeprecationNotice`, `BaseCompatError`, `validate_base_compat`, `PluginManifest`, `PluginManifestBuilder`,
  `ManifestError`, `PluginDependency` — не только "малое" подмножество (`BaseMetadata`/`Metadata`/`Icon`/
  `MaturityLevel`), которое рассматривалось как альтернатива. Мотив: `BaseCompatError<K>` — payload варианта
  `Base(..)` у всех трёх `MetadataCompatibilityError` enum'ов (action/credential/resource), которые сам prelude
  уже реэкспортирует; без `BaseCompatError` потребитель мог получить значение ошибки, но не мог назвать тип его
  payload'а. `PluginManifestBuilder` уже именуется как параметр consumer'ом
  (`crates/plugin/tests/frozen_registry.rs`), значит был достижим только анонимно через method-chaining, а не
  именуемо. См. `crates/sdk/docs/DESIGN.md` — issue 1000 (сжатие prelude в persona-модули) обязано сохранить
  достижимость этого набора, а не решать вопрос заново.
