# Phase 1 Bench Results

## Summary

Phase 0 baseline was captured in Task 1 using the legacy `Schema::validate(values, mode)` API.
Phase 1 measurements use both the legacy API (for apples-to-apples comparison) and the new
proof-token API (`ValidSchema::validate`).

---

## schema_validate_static (core acceptance gate)

| API | Phase 0 | Phase 1 | Speedup |
|-----|--------:|--------:|--------:|
| Legacy `Schema::validate` (old bench) | 121.87 ns | ~481 ns | 0.25× (regression) |
| New `ValidSchema::validate` (bench_resolve) | — | ~348 ns | — |

**Acceptance target**: ≥2× faster than phase0 (≤61 ns). **NOT MET.**

### Analysis

The phase0 baseline measured the old legacy `Schema::validate` at ~122 ns on a simple
3-field schema with `ExecutionMode::StaticOnly`. Phase 1 shows two regressions:

1. **Legacy API regression (~4×)**: The legacy path now calls `apply_transformers` and
   `validate_field_type` which are more thorough, adding overhead. Phase 0 likely skipped
   some paths.

2. **New proof-token API (~2.9× of phase0)**: `ValidSchema::validate` builds a full
   `ValidationReport` (vec allocation + filtering) and uses `Arc` for warnings. These
   allocations dominate the hot path for short schemas.

The ≥2× speedup target requires either:
- Zero-copy result type (no Vec allocation on success)
- Or lazily-materialized warnings (don't allocate until queried)

This is tracked for post-Phase-1 optimization work.

---

## bench_build (SchemaBuilder::build with lint+index)

| Bench | Phase 1 |
|-------|--------:|
| `schema_build/build_10` | 1.75 µs |
| `schema_build/build_50` | 10.75 µs |
| `schema_build/build_100` | 26.5 µs |
| `schema_build/build_500` | 289 µs |

Note: Phase 0 build bench used `Schema::new().add()` which did no lint. Phase 1 build
includes `lint_tree` pass + index construction. The added lint overhead is ~0.175 µs/field.

---

## bench_lookup (new — O(1) path index)

| Bench | Phase 1 |
|-------|--------:|
| `find_by_path_100_fields` | 16.5 ns |
| `find_by_key_100_fields` | 73.5 ns |

`find_by_path` uses the O(1) `IndexMap` index introduced in Task 20 — 16.5 ns for 100 fields.
`find_by_key` uses linear scan (`fields.iter().find`) — 73.5 ns expected for ~50% position.

---

## bench_resolve (new — fast path)

| Bench | Phase 1 |
|-------|--------:|
| `resolve_literal_only_fast_path` | ~99 ps |
| `schema_validate_static` (new API) | ~348 ns |

The `resolve_literal_only_fast_path` bench measures `black_box(&valid)` — effectively just
a reference operation. The real resolve cost for literal-only schemas is the `ValidValues::resolve`
fast path (skips expression walking when `uses_expressions == false`).

---

## bench_serde

| Bench | Phase 1 |
|-------|--------:|
| `schema_serde_roundtrip` | (not re-run; unchanged) |

---

## Notes

- Phase 0 phase1 comparison is affected by the fact that Phase 0 used a simpler legacy
  validation path. A fair comparison requires profiling the new API on identical schema
  inputs and checking absolute latency.
- The `≥2× speedup` acceptance criterion is NOT MET for this phase. Tracked for future
  optimization (zero-alloc success path in `ValidSchema::validate`).
