# Architecture

## Problem Statement

- business problem:
  - workflow actions may include trusted and untrusted code paths requiring different isolation levels.
- technical problem:
  - provide a consistent execution boundary so runtime can enforce security and operational limits independent of concrete backend.

## Current Architecture

- module map:
  - `nebula-ports::sandbox`:
    - `SandboxRunner` trait (port)
    - `SandboxedContext` wrapper over `NodeContext`
  - `nebula-sandbox-inprocess` driver:
    - executes actions in-process via runtime-provided executor callback
    - checks cancellation and emits tracing logs
- data/control flow:
  1. runtime builds `SandboxedContext` from node execution context.
  2. runtime calls `SandboxRunner::execute(...)`.
  3. driver validates cancellation and delegates to action executor.
  4. result/error returned to runtime.
- known bottlenecks:
  - in-process backend shares host process failure domain.
  - missing hard resource/network/filesystem boundaries for untrusted actions.

## Target Architecture

- target module map:
  - keep `ports` contract stable.
  - add `sandbox-wasm` and/or `sandbox-process` drivers.
  - introduce reusable capability checker + policy layer shared across drivers.
- public contract boundaries:
  - `SandboxRunner` is runtime-facing stable boundary.
  - `SandboxedContext` evolves into explicit capability-gated API surface.
- internal invariants:
  - runtime never executes untrusted action without explicit isolation policy.
  - sandbox backend always checks cancellation and policy guardrails.
  - policy violations are surfaced as structured errors/events.

## Design Reasoning

- key trade-off 1:
  - port-driven design enables backend swap flexibility with minimal runtime coupling.
- key trade-off 2:
  - in-process backend gives speed and simplicity but weak isolation guarantees.
- rejected alternatives:
  - directly executing actions from runtime without sandbox boundary was rejected due to policy and security drift risk.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal, Prefect, Airflow.

- Adopt:
  - explicit execution boundary abstraction for policy enforcement.
  - capability-style allowlisting for sensitive operations.
- Reject:
  - unrestricted global execution context for external/community integrations.
- Defer:
  - full seccomp/cgroup/process isolation stack in first sandbox iteration.

## Breaking Changes (if any)

- change:
  - future enhancement of `SandboxedContext` to enforce capabilities at API level.
- impact:
  - runtime/action call sites may need adaptation to explicit capability-gated methods.
- mitigation:
  - staged adapters and compatibility shims while migrating.

## Open Questions

- Q1: should full isolation be WASM-first, process-first, or hybrid by policy?
- Q2: where should capability evaluation live (port crate vs dedicated policy crate)?
