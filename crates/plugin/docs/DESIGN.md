# nebula-plugin — design

| Field | Value |
|-------|-------|
| **Status** | Active — README `status: partial` устарел (forward-notice 2026-04-20 пора снять; slice B влит) |
| **Layer** | Composition / registration unit (in-process; над `nebula-action`/`nebula-credential`/`nebula-resource`) |
| **Redesign role** | **Затронут косвенно** — чистый потребитель/индексатор credential- и resource-фасадов; собственной credential/resource-логики не содержит, но dyn-поверхность обоих каскадирует сюда. |
| **Related** | ADR-0091 (out-of-process retired, in-process Plugin Distribution Unit), ADR-0027 (`ResolvedPlugin`, namespace-инвариант, registry-аксессоры), ADR-0092 (credential consolidation), PRODUCT_CANON §3.5 / §7.1 / §13.1 |

---

## 1. Назначение и границы

`nebula-plugin` — это **in-process Plugin Distribution Unit** (ADR-0091): трейт
`Plugin` объединяет actions / credentials / resources под одной версионированной
идентичностью, чтобы движок мог каталогизировать их без переизобретения
per-integration регистрации. Плагин — это единица **регистрации**, а не единица
размера: полноценная интеграционная крейта и микро-плагин используют один контракт.

**Владеет:** трейтом `Plugin` (object-safe контейнер runnable trait-объектов);
`ResolvedPlugin` (eager-кэш компонентов с инвариантом неймспейса `{plugin.key()}.`);
in-memory `PluginRegistry` (`PluginKey -> Arc<ResolvedPlugin>`); таксономией
`PluginError` / `ComponentKind`; derive-макросом `#[derive(Plugin)]`
(подкрейт `nebula-plugin-macros`); парсером `plugin.toml`.

**Явно НЕ делает:** не исполняет компоненты и не содержит credential/resource-логики
— только хранит и индексирует их trait-объекты; не выполняет process/WASM-изоляцию
(out-of-process retired, ADR-0091 / canon §12.6); не персистит (registry чисто
in-memory, durability — в `nebula-storage`); не отвечает за thread-safety
(`PluginRegistry` без внутреннего лока — `RwLock` навешивает вызывающий, registry.rs:12).
По решению sole-public-sdk (публичен только `nebula-sdk`) крейт **internal** — его
поверхность можно ломать без внешнего semver-обязательства.

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `trait Plugin` (object-safe): `manifest()`, `key()`/`version()` форвардят в manifest | `src/plugin.rs:27` |
| `Plugin::actions() -> Vec<Arc<dyn ActionFactory>>` | `src/plugin.rs:40` |
| `Plugin::credentials() -> Vec<Arc<dyn AnyCredential>>` | `src/plugin.rs:48` |
| `Plugin::resources() -> Vec<Arc<dyn ResourceDescriptor>>` | `src/plugin.rs:56` |
| `Plugin::on_load` / `on_unload` (default no-op) | `src/plugin.rs:69 / 76` |
| `ResolvedPlugin` + `from(impl Plugin)` (вызывает списки ровно один раз, валидирует префикс + within-plugin дубли) | `src/resolved_plugin.rs:29 / 57` |
| `ResolvedPlugin::action()/credential()/resource()` — O(1) lookup | `src/resolved_plugin.rs:94..105` |
| `ResolvedPlugin::actions()/credentials()/resources()` — итераторы | `src/resolved_plugin.rs:109..121` |
| `PluginRegistry` + `register` (fail при дубле key) | `src/registry.rs:35 / 46` |
| `PluginRegistry::get/contains/remove/clear/iter/len` | `src/registry.rs:56..83` |
| `all_actions/all_credentials/all_resources` (плоские, для bulk-регистрации движком на старте) | `src/registry.rs:95..119` |
| `resolve_action/credential/resource` по полному ключу (O(plugins); интроспекция/каталог) | `src/registry.rs:126..153` |
| `PluginError` (`derive(Classify)`, коды `PLUGIN:*`) + `ComponentKind` | `src/error.rs:28 / 7` |
| `PluginManifest` / `PluginManifestBuilder` / `ManifestError` — re-export | `src/manifest.rs:10` (канон — `nebula-metadata`) |
| `PluginKey` — re-export из `nebula-core` | `src/lib.rs:46` |
| `#[derive(Plugin)]` (атрибуты `key/name/description/version/group`, генерирует `manifest()`) | `macros/src/lib.rs:41` |
| `plugin_toml::parse_plugin_toml(&Path) -> PluginTomlManifest` (читает `[nebula].sdk` VersionReq + опц. `[plugin].id`-guard) | `src/plugin_toml.rs:107` |
| `PluginTomlManifest` / `PluginTomlError` | `src/plugin_toml.rs:19 / 32` |

`PluginError` варианты: `NotFound`, `AlreadyExists`, `InvalidManifest(#[from] ManifestError)`,
`NamespaceMismatch`, `DuplicateComponent`. `lib.rs` несёт `#![forbid(unsafe_code)]`,
`#![warn(missing_docs)]`.

## 3. Зависимости и зависимые

- **Deps (workspace):** `nebula-plugin-macros` (path=macros), `nebula-core`,
  `nebula-error` (derive), `nebula-metadata`, `nebula-action`, `nebula-credential`,
  `nebula-resource`. **Внешние:** `semver`, `serde`, `thiserror`, `toml="1"`.
  **Dev:** `nebula-schema`, `nebula-workflow`, `insta`, `rstest`, `tempfile`.
- **Зависимые:** `nebula-engine` (engine/Cargo.toml:30), `nebula-api` (api/Cargo.toml:30),
  `nebula-sdk` (sdk/Cargo.toml:22). Движок — главный потребитель `all_*`
  (bulk-регистрация на старте); api — `resolve_*` (интроспекция/каталог).

## 4. Внутренняя архитектура

- `src/lib.rs` — фасад + re-exports.
- `src/plugin.rs` — базовый трейт `Plugin`.
- `src/resolved_plugin.rs` — `ResolvedPlugin` + три `build_*_index` (одно прохождение
  списков `Plugin`, проверка namespace-префикса и within-plugin дублей, построение
  eager-карт для O(1) lookup).
- `src/registry.rs` — `PluginRegistry` (карта `PluginKey -> Arc<ResolvedPlugin>`);
  плоские `all_*` фолдят по всем плагинам, `resolve_*` ищут по полному ключу.
- `src/error.rs` — `PluginError` / `ComponentKind` + ручной `PartialEq`.
- `src/manifest.rs` — чистый re-export shim из `nebula-metadata`.
- `src/plugin_toml.rs` — единственный `pub mod` (НЕ re-export в корень); парсер `plugin.toml`.
- `macros/` (`nebula-plugin-macros`) — derive `Plugin`.

Поток данных: автор пишет `impl Plugin` (или `#[derive(Plugin)]`) →
`ResolvedPlugin::from` фиксирует компоненты один раз и валидирует инварианты →
`PluginRegistry::register(Arc<ResolvedPlugin>)` → движок читает `all_*`,
интроспекция читает `resolve_*`.

## 5. Инварианты и контракты

- **Namespace-инвариант (canon §13.1, ADR-0027).** Каждый ключ компонента начинается
  с `{plugin.key()}.`; нарушение → `NamespaceMismatch`. Проверяется в
  `ResolvedPlugin::from` **до** попадания в registry — by-construction, не by-convention.
- **No within-plugin duplicates.** Дубль ключа внутри плагина → `DuplicateComponent`
  (с `ComponentKind`), ловится при построении индексов.
- **Single source of truth (canon §7.1 / §13.1).** `impl Plugin` — единственный
  runtime-источник того, что регистрируется; нет вторичного манифеста, дублирующего
  `fn actions/credentials/resources`. Списки вызываются ровно один раз
  (`ResolvedPlugin::from`, resolved_plugin.rs:57).
- **Registry-дедупликация по key.** `register` падает при дубле `PluginKey`
  (`AlreadyExists`), registry.rs:46.
- **Cross-plugin dependency rule (in-process, ADR-0091).** Типы чужого плагина
  доступны только через `Cargo.toml [dependencies]`; замкнутость зависимостей
  обеспечивает компилятор на этапе линковки.
- **Thread-safety — НЕ инвариант крейта.** `PluginRegistry` без внутреннего лока;
  синхронизация — на вызывающем.

## 6. Известные напряжения / долг

1. **`plugin_toml` vs in-process модель (противоречие).** Doc-комментарий
   `src/plugin_toml.rs:8-10` говорит про «spawning the plugin binary», IPC round-trip,
   wire protocol — наследие out-of-process модели, retired ADR-0091. README:51 при
   этом заявляет «Not responsible for `plugin.toml` parsing… belongs to pre-compile
   tooling (`cargo-nebula`)» — прямое противоречие с наличием `pub mod plugin_toml`
   в этом же крейте. Нужно решение: либо парсер уезжает в tooling, либо doc/README выправляются.
2. **README устарел (forward-notice 2026-04-20).** README:13-15 утверждает, что код
   «still exports `PluginType`/`PluginVersions` and defines `PluginManifest` locally»;
   slice B влит, `manifest.rs` давно re-export. Notice пора снять, `status: partial` → пересмотреть.
3. **README vs сигнатуры.** README:32 пишет `actions() -> Vec<Arc<dyn Action>>` и
   `resources() -> Vec<Arc<dyn AnyResource>>`; в коде — `ActionFactory`
   (`src/plugin.rs:40`) и `ResourceDescriptor` (`src/plugin.rs:56`). Документация
   отстаёт от фактических dyn-поверхностей.
4. **Обрубленные doc-комментарии** (следы вычищенных ссылок на план):
   `src/plugin_toml.rs:1` «parsing per» (оборвано), `src/plugin.rs:37-39`
   («per …\n\n is `Sized`…»), `src/resolved_plugin.rs:10` «See and `docs/pitfalls.md`»,
   `src/manifest.rs:1` «canonical in `nebula-metadata` ( follow-up,».
5. **Двойная stringly-поверхность ключа credential.** `resolved_plugin.rs:157-160`
   сознательно игнорирует `AnyCredential::credential_key()` (`&str` из `KEY` const)
   в пользу типизированного `metadata().base.key` — две поверхности ключа сосуществуют
   в `nebula-credential` (см. §7).
6. **`lib.rs:16-17`** описывает `PluginManifest` как локальный тип с builder API,
   не упоминая, что это re-export — мелочь, но вводит в заблуждение.

## 7. Роль в пост-0092 credential/resource модели

`nebula-plugin` — **потребитель обоих фасадов**, без собственной credential/resource-логики.
Он завязан на три dyn-поверхности нижних слоёв и просто хранит/индексирует их trait-объекты,
поэтому любой сдвиг этих поверхностей каскадирует сюда механически.

- **Credential seam.** `Plugin::credentials()` отдаёт `Vec<Arc<dyn AnyCredential>>`
  (`src/plugin.rs:48`); индексы `ResolvedPlugin` ключуют их по `metadata().base.key`.
  Пост-ADR-0092 `nebula-credential` стал единым крейтом (contract + runtime + facade +
  builtin; `credential-runtime`/`builtin`/`testutil`/`vault` удалены). Для plugin это
  меняет **источник** dyn-типа, но не контракт регистрации: `AnyCredential` остаётся
  индексируемым trait-объектом. Если rewrite двинет dyn-поверхность (scheme-enum /
  Protocol-модель, судьба `credential_key()`), правки локализованы в
  `plugin.rs` / `resolved_plugin.rs` / `registry.rs` (`all_credentials`/`resolve_credential`).
- **Key-поверхность.** Уже сегодня крейт сознательно предпочитает типизированный
  `metadata().base.key` поверх stringly `credential_key()` (resolved_plugin.rs:157-160).
  Это **ранний голос** в пользу схлопывания двойной key-поверхности в `nebula-credential`:
  когда rewrite выберет единственный канонический ключ, plugin уже на правильной стороне шва.
- **Resource seam.** `Plugin::resources()` отдаёт `Vec<Arc<dyn ResourceDescriptor>>`
  (`src/plugin.rs:56`). Resource-redesign (Resource = 2 assoc types, per-slot rotation
  fan-out, SlotCell — всё в `nebula-resource`) plugin **не касается**: он индексирует
  дескрипторы, а не активирует ресурсы и не участвует в bind-population. Меняется
  трейт-объект `ResourceDescriptor` — меняются только индексы здесь.
- **Что остаётся неизменным.** Namespace-инвариант, single-source-of-truth-регистрация,
  registry-дедуп и in-process cross-plugin closure (ADR-0091) ортогональны
  credential/resource-rewrite. Plugin остаётся **каталогизатором**: компоненты декларируют
  слоты (`#[credential]`/`#[resource]`) и получают `CredentialGuard<Scheme>` на уровне
  action/resource, но это происходит **ниже** plugin — он лишь поставляет типы в каталог
  (values-only persistence, schema из зарегистрированных типов через `HasSchema` →
  `nebula-metadata` → API-каталог). Façade-уровня credential (policy(&State)-routing,
  OwnerScopedKey, RefreshTransport, lease) plugin не видит и видеть не должен.

## 8. Forward design / открытые вопросы

1. **Снять forward-notice и поднять статус.** README forward-notice (2026-04-20) и
   `status: partial` отражают пред-slice-B состояние; код уже на целевой поверхности.
   Выправить README:13-15 / :32 под фактические `ActionFactory`/`ResourceDescriptor` и
   re-export `PluginManifest`, затем `partial → stable`.
2. **Судьба `plugin_toml`.** Решить противоречие §6.1: либо парсер `plugin.toml`
   переезжает в pre-compile tooling (`cargo-nebula`) согласно README/canon §7.1, либо
   doc-комментарий с IPC/wire-наследием переписывается под in-process реальность.
   Сейчас `pub mod plugin_toml` существует вопреки заявленному non-goal.
3. **Финализировать key-поверхность вслед за credential-rewrite.** Когда
   `nebula-credential` схлопнёт двойную key-поверхность, убрать комментарий-обоснование
   в resolved_plugin.rs:157-160 и зафиксировать единственный канонический ключ.
4. **Очистить обрубленные doc-комментарии** (§6.4) — следы вычищенных plan-ссылок,
   ломают rustdoc-читаемость; не оставлять plan-id в коде.
5. **Риск каскада dyn-поверхностей.** Поскольку три метода `Plugin` буквально возвращают
   `Arc<dyn ...>` нижних крейтов, любой breaking-change их трейт-объектов ломает компиляцию
   здесь. Это приемлемо (крейт internal, sole-public-sdk), но `ResolvedPlugin`/`PluginRegistry`
   тесты должны идти в одном PR с credential/resource dyn-сдвигами.
