# acts — Issues Sweep

## Repository Stats

- Total closed issues: 7 (very low — project has minimal public issue traffic)
- Total open issues: 2
- Total issues: 9

Note: With fewer than 100 closed issues the ≥3 citation requirement is applied to all available issues. All 9 are cited below.

---

## Open Issues

### #16 — Cache learning
**URL:** https://github.com/yaojianpin/acts/issues/16
**Reactions:** 0
**Summary:** Relates to understanding or improving the Moka LRU cache behavior for in-flight process instances. Cache size is configurable via `config.cache_cap` (default 1024). No architectural detail disclosed publicly.

### #10 — Add version on a Model/Package
**URL:** https://github.com/yaojianpin/acts/issues/10
**Reactions:** 0
**Summary:** Request to add versioning support to workflow models and packages. Current `Workflow` struct has a `ver: i32` field (`acts/src/model/workflow.rs:32`) but packages track `version: &'static str` in metadata only. No migration support, no version routing. This is a significant missing feature for production use: deploying a new workflow version while old instances run is unaddressed.

---

## Closed Issues

### #13 — Add Diesel to handle SQL databases
**URL:** https://github.com/yaojianpin/acts/issues/13
**Reactions:** 0
**Summary:** Request for Diesel ORM support. Resolved by adding `acts-postgres` plugin using `sea-query` + `sqlx` rather than Diesel. Architectural decision: sea-query was chosen over Diesel, likely due to simpler async story.

### #12 — Process state never change
**URL:** https://github.com/yaojianpin/acts/issues/12
**Reactions:** 0
**Summary:** Bug where process state was not being updated when root task completed. Fixed in v0.15.0. Exposed a gap in the state machine logic: task completion did not propagate to the containing process correctly.

### #9 — Be able to load multiple packages
**URL:** https://github.com/yaojianpin/acts/issues/9
**Reactions:** 0
**Summary:** Request to register multiple packages at once. Led to the `inventory`-based compile-time registration approach in v0.16.0 and `register_package` on `Extender`.

### #8 — Input information
**URL:** https://github.com/yaojianpin/acts/issues/8
**Reactions:** 0
**Summary:** Clarification request about how act inputs/outputs work. Indicates documentation gap — the inputs/outputs/params/options distinction in act definition is non-obvious.

### #7 — Publish 0.13.3
**URL:** https://github.com/yaojianpin/acts/issues/7
**Reactions:** 0
**Summary:** Release process issue. Not architecturally significant.

### #4 — Event to store Vars without changing status
**URL:** https://github.com/yaojianpin/acts/issues/4
**Reactions:** 0
**Summary:** Request to update process variables from an event without transitioning act state. Led to `SetProcessVars` EventAction (`acts/src/export/executor/act_executor.rs:set_process_vars`) and later `$set_process_var()` in JavaScript.

### #3 — Use Strum
**URL:** https://github.com/yaojianpin/acts/issues/3
**Reactions:** 0
**Summary:** Suggestion to use the `strum` crate for enum-to-string conversions. Adopted — `strum` is a workspace dependency used for `TaskState`, `MessageState`, `EventAction`, `ActRunAs`, `ActPackageCatalog` enums.

---

## Architectural Observations from Issues

1. **Version management gap** (issue #10): No workflow versioning or migration story — a real limitation for production deployments where model changes must be backward-compatible.
2. **State machine correctness** (issue #12, fixed v0.15.0): The process/task state propagation had a bug, suggesting the state machine logic is hand-rolled and fragile.
3. **Documentation gaps** (issue #8): The acts/params/inputs/outputs/options layering is confusing enough to generate support requests — DX concern for integrators.
4. **Low community engagement**: Zero reactions on all issues suggests this is primarily a solo project with few external users in production.
