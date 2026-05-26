# Observability stack (local dev)

`docker-compose.observability.yml` brings up the minimal collector + Jaeger
combo that exercises the nebula-server OTLP pipelines wired in
`crates/api/src/telemetry_init.rs` (traces) and `crates/metrics/src/otlp.rs`
(metrics).

## Running

```bash
task obs:up      # docker compose up -d on this file
# … run nebula-server, exercise the API …
task obs:down    # docker compose down
```

The Taskfile loads `deploy/.env` (and `deploy/.env.example` as a fallback) so
`OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317` is the default for
`nebula-server` started from this repo.

## Ports

| Port  | Service          | Purpose                                     |
| ----- | ---------------- | ------------------------------------------- |
| 4317  | otel-collector   | OTLP/gRPC ingest (traces + metrics + logs)  |
| 4318  | otel-collector   | OTLP/HTTP ingest                            |
| 55679 | otel-collector   | zpages (debug UI for the collector itself)  |
| 16686 | jaeger-all-in-one| Jaeger UI                                   |

Internal-only: otel-collector forwards spans to Jaeger's OTLP receiver at
`jaeger:4317` over the compose network. The deprecated Jaeger native gRPC
port (`:14250`) is intentionally unused — the otelcol `jaeger` exporter
that spoke it was removed in v0.105.

## Verifying a span lands in Jaeger

1. `task obs:up` and wait ~5s for both containers to settle.
2. Start nebula-server with `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317`
   (the default once `.env` is in place).
3. Hit a Nebula route that produces a span — e.g.
   `curl http://localhost:8080/api/v1/healthz` (or any auth-free endpoint).
4. Open `http://localhost:16686` and pick **Service: nebula-api** from the
   dropdown. The most recent request span should appear within ~1s.

The collector also runs the `debug` exporter, so spans / metrics / logs are
printed to its stdout in the meantime. `docker logs nebula-otel-collector -f`
shows the live feed when the Jaeger UI is unreachable.

## Integration test

`crates/api/tests/otlp_one_root_span.rs` gates on `OTEL_E2E_TEST=1` and
exercises the API → control queue → engine → action chain against in-memory
OTel exporters (no collector required — `task obs:up` is NOT a prerequisite
for the test path).

The test does not read `OTEL_EXPORTER_OTLP_ENDPOINT`; it always wires the
in-memory exporters so assertions stay hermetic. To validate the live
operator path against real otelcol-contrib + Jaeger, run nebula-server with
`OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317` (the default once
`.env` is in place) while `task obs:up` is running, then exercise an API
route and inspect the Jaeger UI per the steps above.
