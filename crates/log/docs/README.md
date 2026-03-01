# nebula-log docs

Internal engineering docs for `nebula-log`.

## Purpose

`nebula-log` is the canonical logging/observability layer used by Nebula crates.
It standardizes:
- tracing initialization
- structured event formatting
- writer routing and failover behavior
- optional OTLP/Sentry integration

## Intended audience

- Core/infrastructure maintainers
- Runtime/engine developers integrating logging
- Developers changing config/env contracts

## Document map

- [ARCHITECTURE.md](./ARCHITECTURE.md) - module boundaries and runtime pipeline
- [API.md](./API.md) - public contract and initialization semantics
- [Integration.md](./Integration.md) - integration patterns for engine/runtime/services
- [OPERATIONS.md](./OPERATIONS.md) - production setup and troubleshooting
- [RELIABILITY.md](./RELIABILITY.md) - failure handling guarantees and limitations
- [MIGRATION.md](./MIGRATION.md) - compatibility policy and upgrade checklist

## Canonical references

- Crate root docs: `crates/log/src/lib.rs`
- User-facing entrypoint: `crates/log/README.md`
- Changelog: `crates/log/CHANGELOG.md`
