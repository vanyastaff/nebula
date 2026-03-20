# Resource Plans — Naming Audit

Generated: 2026-03-20

## 1. "Driver" Terminology

**Baseline count: 1 occurrence across all 9 plan files.**

| File | Line | Context |
|------|------|---------|
| `03-infrastructure.md` | 192 | `// Deref → R::Lease → driver-specific API:` |

**Recommendation:** Replace "driver-specific" with "resource-specific" or "backend-specific" to align with project terminology (the codebase uses "Resource" not "Driver").

## 2. Naming Consistency Analysis

### "handle" — Established, Consistent ✅
Used consistently across all files to mean a caller-facing wrapper (`ResourceHandle<R>`, `ReleaseQueueHandle`, `EventStreamHandle`). Files: 01-core, 02-topology, 03-infrastructure, 05-manager, 06-action-integration, 07-implementation, 08-correctness, 09-topology-guide.

### "guard" — Established, Consistent ✅
Used consistently for RAII drop-guard patterns (`LeaseGuard<L>`, `Drop guard` on `RecoveryTicket`). Files: 04-recovery-resilience, 07-implementation, 08-correctness. No overlap with "handle" — different concepts.

### "instance" — Established, Consistent ✅
Used consistently to mean a single runtime object (connection, client, process). Frequently paired with "runtime instance". Files: 01-core, 02-topology, 03-infrastructure, 04-recovery-resilience, 06-action-integration, 08-correctness, 09-topology-guide.

### "runtime" — Established, Consistent ✅
Used consistently as the associated type `Resource::Runtime` — the internal managed object. Files: all 9.

## 3. Summary

| Term | Occurrences | Status |
|------|-------------|--------|
| `driver` | 1 | ❌ Inconsistent — should be `resource-specific` or `backend-specific` |
| `handle` | ~25 | ✅ Consistent |
| `guard` | ~14 | ✅ Consistent |
| `instance` | ~40+ | ✅ Consistent |
| `runtime` | ~50+ | ✅ Consistent |

**Action items:**
1. Fix `03-infrastructure.md:192` — replace "driver-specific" with "resource-specific"
