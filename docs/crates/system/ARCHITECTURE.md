# Architecture

## Problem Statement

- **Business problem:** Workflow engines need system awareness (CPU, memory, disk) for resource limits, backpressure, and health checks without platform-specific code.
- **Technical problem:** Cross-platform system information gathering with minimal dependencies, feature-gated footprint, and consistent error handling.

## Current Architecture

### Module Map

| Module | Purpose | Feature |
|--------|---------|---------|
| `core` | Error types, result extensions | always |
| `info` | SystemInfo aggregator, init, summary | always |
| `memory` | Memory pressure, info, low-level management | `memory` |
| `cpu` | CPU usage, features, topology, affinity | `sysinfo` |
| `process` | Process list, current, kill, priority | `process` |
| `network` | Interfaces, stats, config | `network` |
| `disk` | Disks, usage, pressure | `disk` |
| `utils` | Formatting, platform info | always |
| `prelude` | Re-exports | always |

### Data/Control Flow

1. **Init:** `init()` forces `LazyLock` initialization of `SystemInfo` and sysinfo backend
2. **Read path:** `SystemInfo::get()` returns cached `Arc<SystemInfo>`; `refresh()` updates cache
3. **Memory:** `memory::current()` calls `SystemInfo::current_memory()` (always fresh)
4. **Pressure:** Thresholds: Low (<50%), Medium (50–70%), High (70–85%), Critical (>85%)

### Known Bottlenecks

- `process::list()` can be expensive (~10ms for 100+ processes)
- sysinfo `refresh_*` calls block; no async variant
- NUMA detection simplified (single-node fallback on non-Linux)

## Target Architecture

- **Target module map:** Same; add `metrics` module when `metrics` feature stabilizes
- **Public contract boundaries:** `init()`, `SystemInfo`, `memory::*`, `cpu::*`, `process::*`, `network::*`, `disk::*`
- **Internal invariants:** All types `Send + Sync`; caching via `parking_lot::RwLock`; init once at startup

## Design Reasoning

- **Key trade-off 1:** sysinfo vs raw platform APIs — Adopted sysinfo for cross-platform coverage; accept some platform limitations
- **Key trade-off 2:** Sync-only — Keeps crate simple; async wrappers can live in consumers
- **Rejected alternatives:** Custom `/proc`/sysctl parsing (maintenance burden); heim (heavier dependency)

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal/Prefect/Airflow.

- **Adopt:** Pressure thresholds (n8n-style health checks); feature-gated modules (Node-RED plugins)
- **Reject:** Full async system info (overkill for polling use case)
- **Defer:** Prometheus metrics export; distributed system discovery

## Breaking Changes (if any)

- None planned; API follows semver

## Open Questions

- Q1: Should `metrics` feature emit OpenTelemetry/Prometheus directly?
- Q2: Add `component` (temperature sensors) to default feature set?
