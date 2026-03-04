# Architecture

## Problem Statement

- **Business problem:** Action and plugin authors need one dependency and one import surface to build nodes, parameters, and tests without depending on every nebula-* crate.
- **Technical problem:** Provide stable prelude, builders, and test utilities that stay in sync with action/runtime contract and do not introduce new domain logic.

## Current Architecture

- **Module map:** lib, prelude, builders (node, parameter, workflow, trigger), testing (context, mock, harness, assertions). Feature flags: testing, builders, codegen, dev-server.
- **Data/control flow:** Author uses prelude or builders → compiles to Action-compatible type → engine/runtime consume. Tests use TestContext/MockExecution → run node in isolation.
- **Known bottlenecks:** Prelude stability and macro/output compatibility need formalization (ROADMAP Phase 1).

## Target Architecture

- **Target module map:** Same; optional codegen and dev-server behind features. No orchestration or runtime in sdk.
- **Public contract boundaries:** Prelude, builders, testing module are stable; feature-gated optional parts documented.
- **Internal invariants:** SDK is facade only; no workflow execution or credential resolution in sdk.

## Design Reasoning

- **Trade-off:** Single prelude simplifies authors but ties sdk to core/action/macros versions; we version sdk with platform and document compatibility.
- **Rejected:** Multiple preludes per layer — would confuse authors (CONSTITUTION D-001).

## Comparative Analysis

Sources: n8n (node dev docs), Temporal SDK.

- **Adopt:** Single SDK entry point, prelude, test utilities for local runs (n8n/Temporal style).
- **Reject:** SDK running engine or hosting runtime.
- **Defer:** Optional codegen and dev-server; keep minimal default features.

## Breaking Changes (if any)

- Prelude or builder contract break: major; see MIGRATION.md.

## Open Questions

- Formal compatibility matrix (sdk X ↔ core/action Y) and release cadence.
