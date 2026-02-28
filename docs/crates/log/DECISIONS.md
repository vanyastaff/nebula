# Decisions

## D-001: tracing-first Observability

Status: accepted

Decision:
- Use `tracing` + `tracing-subscriber` as primary abstraction.

Reason:
- structured events/spans, ecosystem maturity, async compatibility.

## D-002: Feature-gated Integrations

Status: accepted

Decision:
- Keep telemetry/file/metrics/sentry optional behind feature flags.

Reason:
- minimal core footprint, controllable binary size, deployment-specific enablement.

## D-003: Panic-isolated Hook Dispatch

Status: accepted

Decision:
- Catch panics in hook lifecycle and event dispatch.

Reason:
- one faulty hook must not break entire logging path.

## D-004: Async-safe Context Propagation

Status: accepted

Decision:
- Use task-local context in async mode, thread-local in sync mode.

Reason:
- preserve context across `.await` while maintaining zero-cost path for non-async setups.

## D-005: Config-first Initialization

Status: accepted

Decision:
- expose explicit `Config` + presets (`from_env`, `development`, `production`) rather than hardcoded global behavior.

Reason:
- predictable operations in high-load and multi-environment deployments.
