# nebula-plugin — fact sheet

## Назначение
In-process Plugin Distribution Unit (ADR-0091): трейт `Plugin` объединяет actions/credentials/resources
под версионированной идентичностью (`PluginManifest`, канонический дом — nebula-metadata).
`ResolvedPlugin` — eager-кэш компонентов с инвариантом неймспейса `{plugin.key()}.`;
`PluginRegistry` — in-memory `PluginKey -> Arc<ResolvedPlugin>`. Плюс парсер `plugin.toml` (SDK-совместимость до загрузки).

## Публичная поверхность
- `Plugin` (trait, object-safe) — src/plugin.rs:27; `manifest()`, `actions() -> Vec<Arc<dyn ActionFactory>>` (:40), `credentials() -> Vec<Arc<dyn AnyCredential>>` (:48), `resources() -> Vec<Arc<dyn ResourceDescriptor>>` (:56), `on_load`/`on_unload` (default no-op, :69/:76), `key()`/`version()` форвардят в manifest
- `ResolvedPlugin` — src/resolved_plugin.rs:29; `from(impl Plugin)` (:57) вызывает списки ровно один раз, валидирует префикс неймспейса + within-plugin дубли; O(1) `action()/credential()/resource()` (:94-:105); итераторы `actions()/credentials()/resources()` (:109-:121)
- `PluginRegistry` — src/registry.rs:35; `register` (fail при дубле key, :46), `get/contains/remove/clear/iter/len` (:56-:83), плоские `all_actions/all_credentials/all_resources` (:95-:119, для bulk-регистрации engine на старте), `resolve_action/credential/resource` по полному ключу (O(plugins), интроспекция/каталог, :126-:153). Thread-safety на вызывающем (RwLock снаружи)
- `PluginError` — src/error.rs:28; варианты `NotFound`, `AlreadyExists`, `InvalidManifest(#[from] nebula_metadata::ManifestError)`, `NamespaceMismatch`, `DuplicateComponent`; `#[derive(Classify)]` с кодами `PLUGIN:*`
- `ComponentKind` (Action/Credential/Resource) — src/error.rs:7
- `PluginManifest`, `PluginManifestBuilder`, `ManifestError` — re-export из nebula-metadata (src/manifest.rs:10); канонический дом там, здесь — source-compat
- `PluginKey` — re-export из nebula-core (src/lib.rs:46)
- `#[derive(Plugin)]` — proc-macro в подкрейте `nebula-plugin-macros` (macros/src/lib.rs:41); атрибуты `#[plugin(key, name, description, version, group)]`, генерирует `manifest()`
- `plugin_toml::parse_plugin_toml(&Path) -> PluginTomlManifest` — src/plugin_toml.rs:107; читает `[nebula].sdk` (VersionReq, обязателен) + опциональный `[plugin].id`-guard
- `PluginTomlManifest` (src/plugin_toml.rs:19), `PluginTomlError` (Missing/Io/InvalidToml/MissingSdkConstraint/InvalidSdkConstraint, :32)

## Workspace-зависимости
Deps: nebula-plugin-macros (path=macros), nebula-core, nebula-error(derive), nebula-metadata, nebula-action, nebula-credential, nebula-resource; внешние: semver, serde, thiserror, toml="1". Dev: nebula-schema, nebula-workflow, insta, rstest, tempfile.
Кто зависит: **nebula-engine** (crates/engine/Cargo.toml:30), **nebula-api** (crates/api/Cargo.toml:30), **nebula-sdk** (crates/sdk/Cargo.toml:22).

## Структура модулей
- `src/lib.rs` — фасад + re-exports; `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `src/plugin.rs` — базовый трейт `Plugin`
- `src/resolved_plugin.rs` — `ResolvedPlugin` + три `build_*_index` (namespace/dup-валидация)
- `src/registry.rs` — `PluginRegistry`
- `src/error.rs` — `PluginError`/`ComponentKind` + ручной PartialEq
- `src/manifest.rs` — чистый re-export shim из nebula-metadata
- `src/plugin_toml.rs` — единственный `pub mod` (не re-export в корень); парсер plugin.toml
- `macros/` (nebula-plugin-macros) — derive `Plugin` (lib.rs / plugin.rs / plugin_attrs.rs)
- `tests/` — derive_plugin.rs, plugin_toml_parse.rs, resolved_plugin.rs

## Напряжения
- **plugin_toml vs in-process модель**: doc-комментарий src/plugin_toml.rs:8-10 говорит про «spawning the plugin binary», IPC round-trip и wire protocol — наследие out-of-process модели, retired ADR-0091. README:51 при этом заявляет «Not responsible for plugin.toml parsing… belongs to pre-compile tooling (cargo-nebula)» — прямое противоречие с наличием pub-модуля `plugin_toml` в этом же крейте.
- **README устарел (forward-notice 2026-04-20)**: README:13-15 утверждает, что код «still exports PluginType/PluginVersions and defines PluginManifest locally» — slice B уже влит, manifest.rs давно re-export; notice пора снять, status `partial` — пересмотреть.
- **README vs сигнатуры**: README:32 пишет `actions() -> Vec<Arc<dyn Action>>` и `resources() -> Vec<Arc<dyn AnyResource>>`; в коде — `ActionFactory` (src/plugin.rs:40) и `ResourceDescriptor` (src/plugin.rs:56).
- **Обрубленные doc-комментарии** (следы вычищенных ссылок на план): src/plugin_toml.rs:1 «parsing per» (предложение оборвано), src/plugin.rs:37-39 («per …\n\n is `Sized`…»), src/resolved_plugin.rs:10 «See and `docs/pitfalls.md`», src/manifest.rs:1 «canonical in `nebula-metadata` ( follow-up,».
- **Двойная stringly-поверхность ключа credential**: resolved_plugin.rs:157-160 сознательно игнорирует `AnyCredential::credential_key()` (&str из KEY const) в пользу типизированного `metadata().base.key` — две поверхности ключа сосуществуют в nebula-credential.
- lib.rs:16-17 описывает `PluginManifest` как локальный тип с builder API, не упоминая, что это re-export (мелочь, но вводит в заблуждение).

## Роль в credential/resource redesign
Прямо затронут как **потребитель обоих фасадов**: `Plugin::credentials()` и индексы ResolvedPlugin
завязаны на `nebula_credential::AnyCredential` (+ его `metadata().base.key`), `Plugin::resources()` —
на `nebula_resource::ResourceDescriptor`. Любая смена dyn-поверхности credential-rewrite
(scheme-enum/Protocol, merge runtime→credential, судьба `credential_key()`) и resource-redesign
(Resource=2 assoc types, ResourceDescriptor) каскадирует сюда: plugin.rs, resolved_plugin.rs, registry.rs (all_*/resolve_*).
Сам крейт логики credential/resource не содержит — только хранит/индексирует их trait-объекты.
Через sole-public-sdk решение (только nebula-sdk публичен) nebula-plugin — internal: его поверхность можно ломать свободно.
