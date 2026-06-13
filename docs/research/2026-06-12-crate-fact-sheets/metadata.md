# nebula-metadata — fact sheet

## Назначение
Core-слой крейт с общими метаданными «каталожных листьев» (action / credential / resource):
`BaseMetadata<K>` (общий префикс: key, name, description, schema, version, ornaments) + трейт
`Metadata` с default-делегацией, плюс generic compat-правила (`validate_base_compat`) и
`PluginManifest` — дескриптор плагина-контейнера (НЕ компонует BaseMetadata, ADR-0018).

## Публичная поверхность
- `BaseMetadata<K>` — общий префикс метаданных; `#[non_exhaustive]`, serde-flatten-композиция — src/base.rs:31
- `BaseMetadata::new(key, name, description, schema)` + builder-цепочка `with_version/with_icon/with_tags/add_tag/with_documentation_url` — src/base.rs:74-147
- `mark_experimental/mark_beta/mark_stable/with_maturity/deprecate/with_deprecation` (deprecation ⇒ maturity=Deprecated) — src/base.rs:151-191
- `trait Metadata { type Key; fn base() }` — остальные 11 аксессоров default-делегируют (key/name/description/schema/version/schema_arc/icon/documentation_url/tags/maturity/deprecation) — src/base.rs:206-268
- `BaseCompatError<K>` — enum: `KeyChanged` / `VersionRegressed` / `SchemaChangeWithoutMajorBump` — src/compat.rs:22-48
- `validate_base_compat(current, previous)` — key immutable, version monotonic, schema-change ⇒ major bump — src/compat.rs:61-84
- `Icon` — untagged enum `None`/`Inline(String)`/`Url{url}`; `inline()/url()/as_inline()/as_url()/is_none()` — src/icon.rs:18-67
- `MaturityLevel` — `Experimental/Beta/Stable(default)/Deprecated` + `is_unstable()/is_deprecated()` — src/maturity.rs:17-43
- `DeprecationNotice` — `since: Version` + опц. `sunset/replacement/reason` (свободные строки) + builder — src/deprecation.rs:21-67
- `PluginManifest` — контейнер-дескриптор плагина: key(PluginKey)/name/version/group/description/icon/color/tags/author/license/homepage/repository/nebula_version/maturity/deprecation, все поля приватные + геттеры — src/manifest.rs:78-228
- `PluginManifestBuilder` — `PluginManifest::builder(key, name)`; `build()` нормализует ключ (lowercase, space→_) и форсит deprecation⇒Deprecated независимо от порядка вызовов — src/manifest.rs:231-396
- `ManifestError` — `MissingRequiredField` / `InvalidKey(PluginKeyParseError)`, с `nebula_error::Classify` — src/manifest.rs:24-37
- lib.rs: flat re-export всего перечисленного — src/lib.rs:25-30; `#![forbid(unsafe_code)]` + `#![warn(missing_docs)]`

## Workspace-зависимости
Deps (Cargo.toml:14-20): `nebula-core` (PluginKey), `nebula-error` (+derive, Classify), `nebula-schema` (ValidSchema), `semver` (+serde), `serde`, `thiserror`. Dev: serde_json, insta, pretty_assertions, rstest.
Зависят от него: `nebula-action`, `nebula-credential`, `nebula-resource`, `nebula-plugin`, `nebula-sdk` (crates/{action,credential,resource,plugin,sdk}/Cargo.toml).

## Структура модулей
- `src/lib.rs` — проводка модулей + плоские re-export'ы (31 строка)
- `src/base.rs` — `BaseMetadata<K>` + трейт `Metadata` (default-делегация)
- `src/compat.rs` — `BaseCompatError<K>` + `validate_base_compat` (generic-правила; entity-specific — у потребителей)
- `src/icon.rs` — `Icon` (единственное валидное представление иконки)
- `src/maturity.rs` — `MaturityLevel`
- `src/deprecation.rs` — `DeprecationNotice`
- `src/manifest.rs` — `PluginManifest` + builder + `ManifestError` (перенесён из nebula-plugin, «slice B»)

## Напряжения
- README устарел vs Cargo.toml: README.md:29-31 «Only depends on nebula-schema, semver, serde, thiserror» — но в deps есть `nebula-core` и `nebula-error` (Cargo.toml:15-16), оба нужны manifest.rs.
- README устарел vs код: секция Public API (README.md:35-51) не упоминает `PluginManifest`/`ManifestError`; README.md:98 говорит «`nebula-plugin::PluginManifest`» — манифест уже живёт ЗДЕСЬ (src/manifest.rs:10-14).
- Возможно устаревший rationale: src/manifest.rs:11-14 объясняет перенос нуждой `nebula-plugin-sdk` («zero engine-side deps, canon §7.1») — но pivot 2026-06-09 отказался от out-of-process plugin-sdk; обоснование переноса повисло (сам перенос безвреден).
- Дубли мелких хелперов: `default_version`/`is_default_version` (base.rs:9-14 и manifest.rs:47-52), `is_default_maturity` (base.rs:64 и manifest.rs:55) — копипаста внутри одного крейта.
- `Metadata::schema_arc()` (base.rs:240-242) — имя обещает Arc, возвращает `ValidSchema` (дешёвый clone); легкая API-несостыковка имени и сигнатуры.
- `ManifestError::MissingRequiredField` (manifest.rs:28) — вариант существует, но builder его никогда не возвращает (оба обязательных поля передаются в `builder()` позиционно); мёртвый вариант.
- AGENTS.md:24 «Cross-crate calls go through nebula-eventbus» — нерелевантный шаблонный пункт для чисто типового крейта без вызовов.
- AGENTS.md status: frontier, last-reviewed 2026-04-19 — после ADR-0090 (metadata остаётся отдельным Core-крейтом, PR #784) статус/дата не обновлены.

## Роль в credential/resource redesign
Крейт — общая база, а не мишень редизайна. ADR-0090 зафиксировал: nebula-metadata ОСТАЁТСЯ отдельным Core-крейтом (не сливать в core), симметричный by-value `metadata()` API уже сделан по Action/Credential/Resource (PR #784 merged). В credential-rewrite и resource-teardown (ADR-0093) поверхность крейта не меняется; при single-public-sdk pivot он становится приватной impl-деталью, реэкспортируемой через `nebula-sdk` (sdk уже зависит от него).
