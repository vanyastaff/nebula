---
name: nebula-layer-dependency-guard
description: "Protect Nebula's one-way workspace layering. Use when adding crates, imports, or cross-crate integrations to verify dependency direction and eventbus-first decoupling."
---

# Nebula Layer Dependency Guard

Use this skill when touching crate boundaries in the Nebula workspace.

## Goals
- Preserve one-way layer dependencies
- Prevent circular imports between business crates
- Prefer `nebula-eventbus` for cross-cutting signals
- Keep `nebula-core` minimal and stable

## Layer Order (One-Way, Top to Bottom)

```
Infrastructure        nebula-storage
      ↓
Core                  nebula-core  (imported by everything; keep minimal)
      ↓
Cross-cutting         nebula-log, nebula-config, nebula-eventbus,
(safe at any layer)   nebula-metrics, nebula-telemetry, nebula-resilience,
                      nebula-system
      ↓
Business Logic        nebula-credential, nebula-resource, nebula-action,
                      nebula-plugin, nebula-parameter, nebula-expression,
                      nebula-workflow, nebula-execution, nebula-validator,
                      nebula-memory
      ↓
Execution             nebula-engine, nebula-runtime
      ↓
API / Application     nebula-api, nebula-webhook, nebula-sdk, nebula-macros,
                      nebula-auth
```

**Rule:** Arrows point downward only. A crate may import from its own layer or any layer below — never from a layer above.

## Checks
1. Does this new dependency point downward in the documented layer order?
2. Can this coupling be replaced with event publication/subscription via `nebula-eventbus`?
3. Is this type truly foundational, or should it live outside `nebula-core`?
4. Are storage-backend specifics leaking into non-infrastructure crates?

## Safe Patterns
- Publish domain events via `nebula-eventbus` instead of importing peer crate internals
- Add adapter traits in upper layers, implementations in lower layers
- Keep crate public APIs explicit and small
- Cross-cutting crates (`nebula-log`, `nebula-config`, `nebula-eventbus`, etc.) are always safe to import regardless of layer

## Block Conditions
- Circular dependency introduction
- Upward dependency (lower layer imports upper layer)
- Unreviewed `nebula-core` API widening (it's imported by all 25+ crates)
- Direct credential/resource coupling when eventbus decoupling is viable
- Business crate importing from execution or API layer
