# Architecture

## Overview

`nebula-log` is a cross-cutting infra crate with no business-domain dependency.
It builds a process-wide tracing pipeline with pluggable formatting, destinations,
and optional telemetry.

## Module Map

```
nebula-log/src/
|
|-- lib.rs                    public API, init functions, prelude
|-- core/                     LogError, LogResult
|-- config/                   config schema, env parsing, presets, writer settings
|-- builder/                  logger build pipeline, reload, telemetry wiring
|-- writer.rs                 destination creation, fanout, failure policy, rolling
|-- format.rs                 logfmt + format adapters
|-- layer/context.rs          context fields helpers
|-- timing.rs                 Timer, Timed, TimerGuard
|-- observability/            events, hooks, registry, context/resource helpers
|-- telemetry/                OTLP + Sentry integration (feature-gated)
```

## Build Pipeline

1. Resolve config (`explicit > env > preset`) for startup path.
2. Validate config compatibility and feature constraints.
3. Build filter layer (`EnvFilter`, optionally reloadable).
4. Build writer(s) and file guards.
5. Build format layer (`Pretty|Compact|Json|Logfmt`).
6. Optionally attach telemetry layers (`telemetry`/`sentry` features).
7. Install subscriber globally.
8. Return `LoggerGuard` (RAII owner for runtime resources).

## Data/Control Flow

```
Config -> Builder -> Filter + Format + Writer + (Telemetry)
                                  |
                                  v
                         tracing subscriber
                                  |
                     log events + spans emitted
```

## Concurrency and Lifecycle

- Tracing subscriber is global to process.
- Hook registry is shared and thread-safe.
- `LoggerGuard` owns teardown-critical resources.
- Dropping guard finalizes resources (including optional telemetry provider/shutdown path).

## Design constraints

- No `unsafe`.
- Feature-gated optional dependencies to keep base footprint small.
- Deterministic initialization precedence.
