# Active Work
Updated: 2026-03-18

## In Progress
- **nebula-parameter** (RFC 0005): migration from v1 to v2 schema; `src/schema.rs` + `src/providers.rs` are ground truth
- **nebula-credential** phases 3-7: derive macros (3), provider adapters (4), moka cache (5), test infra (6), protocol stubs (7)
- **nebula-engine/runtime**: blocked on resource system — credential/resource injection not wired
- **Desktop app** (Tauri): `apps/desktop/` — replaces old egui nebula-app

## Blocked / Parked
- **nebula-engine full execution**: needs resource/credential DI from nebula-resource to stabilize first

## Next Up
- nebula-credential Phase 3 (derive macros)
- nebula-resource stabilization (unblock engine)
