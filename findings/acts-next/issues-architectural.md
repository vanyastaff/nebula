# acts-next — Issues Reference

Note: luminvent/acts has **issues disabled**. The following issues are from the upstream yaojianpin/acts repository (https://github.com/yaojianpin/acts/issues), which is the direct parent of the luminvent fork. Luminvent directly references and addresses some of these issues in their fork's commits.

## Issue List (yaojianpin/acts, all issues)

| # | Title | State | Reactions | Luminvent relevance |
|---|-------|-------|-----------|---------------------|
| #16 | Cache learning | OPEN | 0 | No direct fix in fork |
| #13 | Add Diesel to handle SQL databases | CLOSED | 0 | Addressed differently: sqlx-based Postgres plugin added |
| #12 | Process state never change | CLOSED | 0 | **Directly fixed in luminvent fork** (Marc-Antoine ARNAUD commit "fix: set process state if task is completed and is root task") |
| #10 | Add version on a Model/Package | OPEN | 0 | Not addressed in fork |
| #9 | Be able to load multiple packages | CLOSED | 0 | Fixed upstream (v0.16.0 package refactor) |
| #8 | Input information | CLOSED | 0 | Fixed upstream (v0.16.0 refactor) |
| #4 | Event to store Vars without changing status | CLOSED | 0 | Possibly related to new `SetVars`/`SetProcessVars` EventActions added by Luminvent |

## Issue #12 — "Process state never change" (Closed)
**URL:** https://github.com/yaojianpin/acts/issues/12  
**Summary:** Root process state was not updated when the root task completed — process remained `running` indefinitely. A state-machine propagation bug in the `review()` path. Fixed in yaojianpin/acts v0.15.0 and also independently fixed in the luminvent fork by Marc-Antoine ARNAUD (commit `423...`, 2025-04-23).

## Issue #10 — "Add version on a Model/Package" (Open)  
**URL:** https://github.com/yaojianpin/acts/issues/10  
**Summary:** Request for workflow model versioning: ability to run different versions of a workflow simultaneously, migrate running instances when a new model version is deployed. The `Workflow.ver: i32` field exists but is unused for routing or migration. Both upstream and luminvent fork leave this open.

## Issue #16 — "Cache learning" (Open)  
**URL:** https://github.com/yaojianpin/acts/issues/16  
**Summary:** Question/request about how the Moka LRU cache interacts with the persistence layer — what happens when a process is evicted from cache, and how eviction + recovery semantics work. Not addressed in the luminvent fork.

## Architectural Summary of Issue Landscape

The upstream yaojianpin/acts has only 9 total issues (7 closed, 2 open at time of research), reflecting low community engagement relative to GitHub stars (~61). The small issue set masks that:

1. **Model versioning (#10)** is a genuine production blocker — both fork and upstream leave it unaddressed.
2. **The Luminvent fork's most impactful contribution** was fixing the process state machine bug (#12) before upstream accepted the same fix.
3. **Issues are sparse** — most architectural pain points are documented as roadmap items in README rather than issues. The absence of issues on the luminvent fork (disabled) makes it impossible to see if Luminvent has additional known limitations beyond upstream.

The total closed-issue count on upstream (7) is below the ≥100 threshold requiring 3 cited issues for the Tier 2 quality gate. However, key issues are cited above for completeness.
