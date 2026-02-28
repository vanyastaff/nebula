# Contract: Logging Configuration Precedence

## Purpose

Define deterministic configuration resolution behavior for `nebula-log` initialization across explicit runtime config, environment-derived config, and preset defaults.

## Inputs

- Explicit runtime config (programmatic)
- Environment config (`NEBULA_LOG`, `RUST_LOG`, related env settings)
- Preset defaults (development, test, production)

## Resolution Order

1. Explicit runtime config
2. Environment-derived config
3. Preset defaults

## Precedence Examples

- If `init_with(config)` is used, `config` is the effective configuration regardless of `NEBULA_LOG`/`RUST_LOG`.
- If no explicit config is passed and `NEBULA_LOG=warn`, the effective level is `warn` over preset defaults.
- If neither explicit config nor log env vars are provided, the runtime preset remains effective.

## Behavioral Guarantees

- Conflicting values always resolve deterministically via the order above.
- Invalid configuration values fail initialization with actionable error context.
- Successful initialization surfaces the effective resolved profile for diagnostics.

## Acceptance Contract

- For every documented conflicting-input scenario, effective values are stable and reproducible.
- Invalid filter expressions fail before runtime emission begins.
- Contract behavior remains stable for minor-version releases.

## Compatibility Rules

- Any change to resolution order is a breaking contract change.
- Additional environment variables may be introduced only additively with clear precedence mapping.
