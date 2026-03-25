# Active Work
Updated: 2026-03-25

## In Progress
- **nebula-credential** phases 3-7: derive macros (3), provider adapters (4), moka cache (5), test infra (6), protocol stubs (7)
- **nebula-engine/runtime**: blocked on resource system — credential/resource injection not wired
- **Desktop app** (Tauri): `apps/desktop/` — replaces old egui nebula-app

## Recently Completed
- **nebula-parameter** v3 migration (2026-03-25): full rewrite from v2 (Field enum + Schema) to v3 (Parameter struct + ParameterType enum + ParameterCollection). All consumers migrated.

## Blocked / Parked
- **nebula-engine full execution**: needs resource/credential DI from nebula-resource to stabilize first

## Next Up
- nebula-credential Phase 3 (derive macros)
- nebula-resource stabilization (unblock engine)
