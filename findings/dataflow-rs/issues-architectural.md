# dataflow-rs — Issues Sweep

## Summary

As of 2026-04-26, dataflow-rs has only **8 total issues** (all closed, 0 open). This is well below the ≥100 threshold that requires ≥3 cited issues per the protocol — the small issue count reflects the project's youth and narrow scope. All 8 issues are noted below.

**Note:** The quality gate "≥3 cited issues for Tier 1 projects with >100 closed issues" does NOT apply here. dataflow-rs has 8 total issues, not >100. All are documented for completeness.

---

## All Issues (Chronological)

### Issue #1 — Multiple Mappings in Same Task Overwrites Parent Object
- **URL:** https://github.com/GoPlasmatic/dataflow-rs/issues/1
- **State:** CLOSED
- **Created:** 2025-08-18
- **Labels:** (none)
- **Reactions:** N/A
- **Description:** Setting `path: "temp_data"` at root level replaced the entire `temp_data` object rather than merging new fields. Bug confirmed in early versions. Fixed in v2.1.x.

**Architectural significance:** Reveals the implicit mutation model — `Message.context` is a single `serde_json::Value` tree modified in place. Root-level assignment semantics were undefined initially. The fix (merge not replace) required explicit object detection in `set_nested_value`. This is a fundamental edge case for any data transformation engine.

---

### Issues #2–#8 — Feature Requests (self-filed)
These 7 issues were filed by the maintainer on 2026-02-22 as a batch to document planned v2.1.1 enhancements. All closed in the same release.

| # | Title | Architectural Impact |
|---|-------|---------------------|
| #2 | Pre-sort Workflows at Construction Time | Priority ordering moved from runtime to `Engine::new()` — zero overhead dispatch |
| #3 | Workflow Lifecycle Fields | Added `status`/`channel`/`version`/`tags`/`created_at`/`updated_at` to `Workflow` struct |
| #4 | Log Function (Built-in) | Added `log` to `FunctionConfig` enum and `InternalExecutor` |
| #5 | Engine Reload Helper | Added `Engine::with_new_workflows()` for hot-reload, reusing function registry `Arc` |
| #6 | Filter/Gate Function with Pipeline Control Flow | Added `filter` function with `halt`/`skip` semantics, `FILTER_STATUS_HALT` and `FILTER_STATUS_SKIP` constants |
| #7 | Channel-Based Routing & Status Filtering | Added `channel_index: HashMap<String, Vec<usize>>` for O(1) channel dispatch |
| #8 | Typed Config Variants for Integration Functions | Added `HttpCallConfig`, `EnrichConfig`, `PublishKafkaConfig` as typed `FunctionConfig` variants |

---

## Architectural observations from issue history

1. **Self-filed roadmap issues** — issues #2–#8 are classic self-filed batch-close patterns. The maintainer uses the issue tracker as a changelog anchor for releases, not as a community feedback mechanism.

2. **No community bug reports** — Only issue #1 came from actual usage. This suggests either very low user adoption testing the engine in production, or the narrow IFTTT/library-first scope avoids runtime failures common in server-heavy projects.

3. **No design discussions** — No architecture proposals, no RFC discussions, no multi-maintainer debate. All design decisions are implicit in code.

4. **Rate of feature addition** — All 8 issues (post-initial) were closed in a single release (v2.1.1 on 2026-02-22), indicating the maintainer builds in isolation then batch-publishes.
