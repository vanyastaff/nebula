# Architecture

## Problem Statement

- **Business problem:** Workflow platforms (n8n-class) need unified observability: structured logs, traces, metrics, and context propagation across engine, runtime, API, and plugins.
- **Technical problem:** Provide a zero-config-to-production logging layer that scales from development to high-throughput deployments without coupling to domain logic.

## Current Architecture

- **Module map:**
  - `core/` — `LogError`, `LogResult`, result extension traits
  - `config/` — config schema (`Config`, `Format`, `Level`, writer/display/fields presets)
  - `builder/` — subscriber construction, filter reload, telemetry attachment
  - `writer.rs` — writer instantiation (stderr/stdout/file/multi)
  - `layer/` — context and field layers in tracing pipeline
  - `timing.rs` + `macros.rs` — low-overhead timing for sync/async
  - `observability/` — event model, hook registry, context/resource-aware hooks
  - `metrics/` (feature-gated) — metrics facade and timing helpers
  - `telemetry/` (feature-gated) — OpenTelemetry/Sentry integration

- **Data/control flow:**
  - Init: `Config` → `LoggerBuilder` → tracing subscriber + layers → `LoggerGuard`
  - Log path: `info!`/`span!` → tracing → fmt layer → writer(s)
  - Observability: `emit_event` → registry → hooks (panic-isolated)

- **Known bottlenecks:**
  - hooks execute inline; slow hooks increase tail latency
  - `WriterConfig::Multi` falls back to first writer only
  - size-based rolling not implemented

## Target Architecture

- **Target module map:** Same structure; add `HookPolicy` (inline vs bounded-async) and typed event keys.
- **Public contract boundaries:**
  - `nebula-log` is a leaf infra crate; no domain crates as dependencies
  - consumers: `core`, `action`, `config`, `credential`, `expression`, `memory`, `resilience`
- **Internal invariants:**
  - no `unsafe` (`#![forbid(unsafe_code)]`)
  - hook dispatch never panics (catch_unwind)
  - context propagation preserves across `.await` in async mode

## Design Reasoning

- **tracing-first:** Structured events/spans, ecosystem maturity, async compatibility. Rejected: raw `log` crate (no spans, weaker structure).
- **Feature-gated integrations:** Minimal core footprint, controllable binary size. Rejected: monolithic build with all backends.
- **Panic-isolated hooks:** One faulty hook must not break logging. Rejected: inline panic propagation.
- **Config-first init:** Predictable behavior across environments. Rejected: hardcoded global defaults.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces/Activeflow, Temporal/Prefect/Airflow.

- **Adopt:** Structured logging (JSON/logfmt), span-based tracing, env-based config (common in Temporal/Prefect). OTLP export for cloud observability.
- **Reject:** Node-RED style in-process only; n8n’s Node.js-specific logging. No adoption of language-specific log formats.
- **Defer:** Distributed trace sampling policies (delegate to OTLP collector); alerting rules (consumer responsibility).

## Breaking Changes (if any)

- P-002 (typed event names): migration path via dual API and deprecation.
- P-003 (context ID types from `nebula-core`): typed constructors added; string constructors deprecated.

## Open Questions

- Q1: Should hook budget policy be opt-in or default for high-throughput deployments?
- Q2: Preferred deprecation window for string-only event names (6 or 12 months)?
