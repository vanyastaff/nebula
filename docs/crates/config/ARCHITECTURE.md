# Architecture

## Problem Statement

- Business problem:
  - Nebula crates need a consistent way to load, validate, and refresh runtime configuration across environments.
- Technical problem:
  - avoid per-crate ad-hoc config logic, inconsistent precedence rules, and unsafe runtime overrides.

## Current Architecture

- module map:
  - `core/`: `Config`, `ConfigBuilder`, source model, errors/results, traits
  - `loaders/`: file/env/composite loaders
  - `validators/`: schema/function/composite/noop validators
  - `watchers/`: file/polling/noop watchers
- data/control flow:
  1. `ConfigBuilder` collects sources/defaults/components
  2. sources loaded concurrently by loader
  3. values merged by priority into one JSON tree
  4. validator chain runs
  5. optional watcher/reload loop updates in-memory state
- known bottlenecks:
  - large nested config merges under frequent reloads
  - user-defined validation complexity in hot paths

## Target Architecture

- target module map:
  - keep current split; improve contract docs and test rigor before structural refactor
- public contract boundaries:
  - `ConfigBuilder` for assembly
  - `Config` for runtime access and mutation
  - `ConfigLoader`/`ConfigValidator`/`ConfigWatcher` as extension points
- internal invariants:
  - source precedence must remain deterministic
  - merge must be idempotent for same input set
  - reload must either fully succeed or preserve previous valid state

## Design Reasoning

- trade-off 1: flexibility vs determinism
  - chosen: flexible source types with explicit priority ordering.
- trade-off 2: dynamic JSON tree vs compile-time config structs
  - chosen: dynamic core storage + typed retrieval bridges.
- rejected alternatives:
  - compile-time-only typed config tree as single model (too rigid for plugin/runtime extensibility).

## Comparative Analysis

References: n8n, Node-RED, Activepieces/Activeflow, Temporal/Airflow ecosystem practices.

- Adopt:
  - layered source precedence (defaults + file + env overrides), common in automation platforms.
  - hot-reload/watcher semantics for long-running orchestrators.
- Reject:
  - implicit/undocumented precedence rules (causes production misconfiguration).
  - silently coercing invalid values without explicit errors.
- Defer:
  - remote config source first-class support (`Remote/Database/KeyValue`) until reliability/security model is hardened.

## Breaking Changes (if any)

- none now.
- future candidates:
  - stronger typed path model
  - stricter merge conflict policy options

## Open Questions

- should reload support transactional staging hooks per consumer crate?
- should source priority become fully user-configurable at runtime?
