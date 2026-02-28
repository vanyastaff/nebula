# API

## Public Surface

- stable APIs:
  - planned `LocaleManager`, `LocaleContext`, `Translator`, `MessageKey` contracts
  - planned middleware helpers for locale negotiation
- experimental APIs:
  - runtime hot-reload of translation bundles
  - per-tenant override catalogs
- hidden/internal APIs:
  - internal bundle caching/indexing details

## Usage Patterns

- API resolves locale from request and user context.
- runtime/action/validator render user-facing strings through translator API.
- plugin loader registers plugin locale catalogs from `locales/` directory.
- errors map to message keys with interpolation parameters.

### Plugin Auto-Discovery Contract

- if plugin root contains `locales/`, loader scans files matching `<lang-tag>.ftl` (for example `en-US.ftl`, `ru-RU.ftl`)
- each file must pass:
  - locale tag parse validation
  - Fluent syntax validation
  - key namespace validation (must start with plugin namespace)
  - required key-set checks for declared plugin UI/action keys
- on validation failure:
  - plugin remains loadable
  - locale bundle is skipped
  - diagnostics emitted to log/telemetry with plugin id + locale + error details

## Minimal Example

```rust
// planned API sketch
let locale = locale_manager.resolve(&request_meta, &user_profile)?;
let text = locale_manager.t(&locale, "welcome", params! { "user" => "John" })?;
```

## Advanced Example

```rust
// planned API sketch
let ctx = LocaleContext::new("ru-RU").with_fallback("en-US");
let msg = translator.render_error(&ctx, error_key, error_params)?;
```

## Plugin Locale Example

```rust
// planned API sketch
locale_manager.register_plugin_catalog(
    "plugin.telegram",
    "ru-RU",
    include_str!("../locales/ru-RU.ftl"),
)?;

let text = locale_manager.t(
    &ctx,
    "plugin.telegram.node.send_message.title",
    params! {},
)?;
```

## Error Semantics

- retryable errors:
  - transient catalog backend/cache loading failures.
- fatal errors:
  - invalid catalog format or broken interpolation contract.
- validation errors:
  - unknown locale code, missing required interpolation parameter, invalid key format.

## Compatibility Rules

- what changes require major version bump:
  - key namespace semantics and interpolation behavior
  - locale negotiation precedence contract
- deprecation policy:
  - key aliases and compatibility windows for at least one minor release
