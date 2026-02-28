# Architecture

## Problem Statement

- business problem:
  - multi-tenant SaaS workflow platform needs strict isolation and predictable fair-use controls.
- technical problem:
  - centralize tenant context, partition strategy, and quota policy so all runtime services enforce the same rules.

## Current Architecture

- module map:
  - current codebase has no `crates/tenant`; logic is distributed across `core/resource/credential/storage` and docs.
- data/control flow:
  - tenant-related fields travel through context/scope in other crates without a dedicated policy owner.
- known bottlenecks:
  - policy duplication and inconsistent enforcement risks across services.

## Target Architecture

- target module map:
  - `manager`: tenant registry and lifecycle state
  - `context`: tenant context extraction/validation/injection
  - `quota`: usage accounting and hard/soft limit enforcement
  - `partition`: data/resource partition strategy abstraction
  - `policy`: admission and override policies
  - `audit`: tenant decision trail/events
- public contract boundaries:
  - runtime-facing contracts for context retrieval and quota checks
  - storage/resource-facing contracts for partition and limit mapping
- internal invariants:
  - every request/execution path has validated tenant context (or explicit system scope)
  - quota checks are monotonic and race-safe
  - isolation policy cannot be bypassed by caller-provided metadata

## Design Reasoning

- key trade-off 1:
  - centralized tenant crate improves consistency but introduces a cross-cutting dependency.
- key trade-off 2:
  - strict isolation defaults increase safety but may require explicit override flows for internal ops.
- rejected alternatives:
  - leaving tenant policy distributed across all crates was rejected due to long-term drift risk.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal, Prefect, Airflow.

- Adopt:
  - explicit tenant/workspace boundaries and quota governance.
  - policy-as-contract approach with auditability.
- Reject:
  - implicit global context mutation without authoritative validation boundary.
- Defer:
  - fully dynamic per-tenant topology orchestration in first release.

## Breaking Changes (if any)

- change:
  - migration of tenant-related ad-hoc logic from other crates into dedicated contracts.
- impact:
  - runtime/api/storage/resource integration points need adapter updates.
- mitigation:
  - staged adapter layer and dual-read/dual-enforcement rollout.

## Open Questions

- Q1: should quota accounting be strict real-time or eventually consistent per operation type?
- Q2: which partition strategy should be default for first production release?
