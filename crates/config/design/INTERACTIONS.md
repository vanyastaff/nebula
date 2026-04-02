# nebula-config — Consumer Interactions

## Downstream Consumer Requirements

All components that read configuration from nebula-config are considered
**downstream consumers**. They must adhere to the following contract:

### consumer CI requirements

1. **Tests must not hardcode config values** that could change between
   deployments. Use `Config::default()` or test-specific builders.
2. **Schema version checks**: consumers must call `ensure_compatible()` on
   deserialized configs before use.
3. **Reload handling**: consumers subscribed to config updates must handle
   the case where a reload is rejected and the previous config remains active.

## Notification Contract

Config changes are signalled via the `nebula-eventbus`. Consumers subscribe
to `ConfigChangedEvent` and receive the new `Config` snapshot only after it
has passed validation.

The event payload includes:
- `previous_version`: schema version of the old config.
- `new_config`: the fully validated new `Config`.

## Contract for downstream consumer requirements

- Consumers **must not** cache raw field values across reloads without
  re-reading from the config handle.
- Consumers **must** treat a missing reload notification as a no-op (the
  previous config is still valid).
- Consumers **should** log the config version they are operating with for
  observability.

## Versioning Impact

When a **major** config version change occurs (see DECISIONS.md), downstream
consumers must update their deserialization logic before receiving the new
config. The event bus will publish `ConfigIncompatibleEvent` if a consumer
tries to apply a config with an unsupported schema version.
