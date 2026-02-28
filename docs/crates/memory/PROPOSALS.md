# Proposals

## P001: Unified config schema

Idea:
- introduce a crate-level `MemoryRuntimeConfig` that composes allocator/pool/cache/budget configs.

Benefit:
- simpler integration in engine/runtime bootstrap.

Potential break:
- constructor APIs may shift toward builder-based configuration.

## P002: Policy-driven allocator selection

Idea:
- route allocations through policy (`LatencyOptimized`, `ReuseOptimized`, `ThroughputOptimized`).

Benefit:
- runtime can switch memory strategy without touching call sites.

Potential break:
- direct allocator usage patterns may need migration to selector APIs.

## P003: Budget-to-backpressure integration

Idea:
- connect memory budget pressure events to queue/scheduler backpressure decisions.

Benefit:
- avoids OOM-like conditions by reducing admission rate earlier.

Potential break:
- behavior changes in overload scenarios (intentional but visible).

## P004: Consistent async trait surface

Idea:
- stabilize async wrappers around pool/cache/budget with one trait family.

Benefit:
- less ad-hoc async integration for runtime code.

Potential break:
- existing async support module APIs may be replaced or renamed.

## P005: Extract experimental modules

Idea:
- move unstable/highly experimental capabilities into sibling crates behind explicit adoption.

Benefit:
- tighter core crate with clearer support guarantees.

Potential break:
- import paths and feature flags for experimental APIs will change.
