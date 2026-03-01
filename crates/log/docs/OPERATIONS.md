# Operations

## Runtime controls

### Logging controls

- `NEBULA_LOG` / `RUST_LOG`
- `NEBULA_LOG_FORMAT` (`pretty|compact|json|logfmt`)
- `NEBULA_LOG_TIME`
- `NEBULA_LOG_SOURCE`
- `NEBULA_LOG_COLORS`

### Global identity fields

- `NEBULA_SERVICE`
- `NEBULA_ENV`
- `NEBULA_VERSION`
- `NEBULA_INSTANCE`
- `NEBULA_REGION`

### Telemetry/Sentry

- `OTEL_EXPORTER_OTLP_ENDPOINT`
- `SENTRY_DSN`
- `SENTRY_ENV`
- `SENTRY_RELEASE`
- `SENTRY_TRACES_SAMPLE_RATE`

## Production rollout checklist

1. Choose stable format (`json` or `logfmt`) for ingestion.
2. Configure destination policy (`BestEffort`/`FailFast`/`PrimaryWithFallback`).
3. Set service/env/version metadata fields.
4. Confirm OTLP/Sentry endpoints are reachable.
5. Validate startup in staging with effective env values.

## Quality gates

```bash
cargo test -p nebula-log
cargo test -p nebula-log --doc
cargo clippy -p nebula-log --all-targets --all-features --locked -- -D warnings
```

## Troubleshooting matrix

| Symptom | Likely cause | Action |
|---|---|---|
| init fails with filter error | invalid `NEBULA_LOG`/`RUST_LOG` | validate filter expression |
| file writer init failure | invalid path/permissions | check destination path and access |
| no OTLP exports | missing feature or endpoint | enable `telemetry`, verify endpoint |
| no Sentry events | missing `SENTRY_DSN` or feature | enable `sentry`, set DSN |
| missing structured fields | not set in config/env | set `fields.*` or env vars |
