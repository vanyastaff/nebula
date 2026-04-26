# dagx — Structure Summary

## Crate count and layout

dagx is a 3-crate workspace:

| Crate | Role | LOC |
|-------|------|-----|
| `dagx` | Core library — DAG runner, task trait, builder, scheduler | ~1641 |
| `dagx-macros` | Proc-macro crate — `#[task]` attribute macro | 307 |
| `dagx-test` | Internal test helper — `task_fn` closure factory + `ExtractInput` trait | 210 |

Total source LOC (src/ only, excluding tests/examples/benches): ~2158 lines across 11 files.
Test LOC (tests/): ~2552 lines, 22 test files.
Benchmarks: ~8 files under benches/, using criterion 0.7.

No `tokei` binary available; line counts from `wc -l`.

## Dependency graph

Production dependencies are minimal:
- `futures-util 0.3` (FuturesUnordered, StreamExt, FutureExt::catch_unwind) — `std` features only
- `dagx-macros 0.3.1` (optional, gated behind `derive` feature — on by default)
- `tracing 0.1` (optional, gated behind `tracing` feature — off by default)

Dev dependencies (tests + benchmarks):
- `tokio 1`, `smol 2`, `async-executor 1.13`, `pollster 0.4`, `futures-executor 0.3` (runtime matrix)
- `criterion 0.7` (benchmarks)
- `dagrs 0.5` (comparison benchmarks)
- `test-case 3.3.1`, `tracing-subscriber 0.3`

No external graph crate (no petgraph). The DAG topology is represented purely through in-memory `HashMap<NodeId, Vec<NodeId>>` (adjacency lists), with cycle prevention via the type system.

## Feature flags

| Flag | Default | Effect |
|------|---------|--------|
| `derive` | on | enables `#[task]` proc-macro via `dagx-macros` |
| `tracing` | off | enables structured `tracing` spans/events through all execution paths |

## Test count (approximate)

22 integration test files + inline unit test modules (`src/**/*tests*`). Tests cover:
- Cycle prevention (compile_fail proofs), boundaries (deep chains, zero tasks, type limits)
- Parallelism timing, spawning, thread proofs
- Runtime matrix (tokio, smol, async-executor, pollster, futures-executor)
- Error propagation and recovery patterns
- Tracing (with/without feature)
- Custom type handling (v0.3.1 regression area)

## Last activity

Created: 2025-10-07. Latest release: v0.3.1 (2025-10-10). Repository updated: 2026-04-23. Stars: 19. Forks: 1.
