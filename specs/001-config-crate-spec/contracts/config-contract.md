# Contract: Config Runtime and Consumer Surface

## Stable Public Interfaces

- `ConfigBuilder`
- `Config`
- `ConfigSource`, `ConfigFormat`, `SourceMetadata`
- `ConfigLoader`, `ConfigValidator`, `ConfigWatcher`
- `ConfigError`, `ConfigResult`

## Behavioral Contract

- Source precedence and merge ordering are deterministic.
- Validation is mandatory before startup/reload activation.
- Invalid reload does not replace active valid configuration.
- Typed retrieval by path returns deterministic success/failure categories.
- Load/reload diagnostics include source-level context without exposing sensitive values.

## Compatibility Rules

- Minor release:
  - Additive source/validator/watcher extensions only.
  - Existing precedence, path, and typed retrieval semantics remain unchanged.
- Major release:
  - Required for precedence behavior changes.
  - Required for path traversal semantic changes.
  - Required for behavior-significant validation gate changes.
- Deprecation:
  - Maintain deprecated aliases/accessors for at least one minor cycle where feasible.
  - Publish migration mapping before removal.

## Required Contract Tests

- Precedence matrix fixtures for layered source combinations.
- Merge determinism fixtures for identical inputs across repeated loads.
- Typed retrieval compatibility fixtures for representative consumer paths.
- Reload atomicity checks ensuring last-known-good retention on validation failure.
- Security-oriented diagnostics checks for sensitive-value redaction.
