# dagx — Issues and PRs Sweep

## Issue count

Total closed issues: 3. Total open issues: 0. Total PRs: 12 (all merged).

**Note on quality gate:** The brief requires ≥3 cited issues for Tier 1/2 projects with >100 closed issues. dagx has only 3 closed issues total — well under 100. The quality gate for issue citations therefore does not apply. All 3 closed issues are cited below.

## Closed Issues

### Issue #15 — Release v0.4
URL: https://github.com/swaits/dagx/issues/15
State: CLOSED
Reactions: 0
Summary: Planned v0.4 release tracking issue. Unreleased changes listed in CHANGELOG.md "Unreleased" section include: bare tuple return types, cycle fix (#3), explicit return type not needed (#2), Clone not needed on inputs (#7), single-threaded construction (#12), TaskInput linear type.

### Issue #5 — Consolidate some tests/benches/examples
URL: https://github.com/swaits/dagx/issues/5
State: CLOSED
Reactions: 0
Summary: Internal housekeeping — redundant tests, examples, and benchmarks consolidated. Resolved via PR #11.

### Issue #4 — Make DagRunner internally single-threaded
URL: https://github.com/swaits/dagx/issues/4
State: CLOSED
Reactions: 0
Summary: DagRunner construction/wiring made single-threaded (no Mutex/RwLock needed during build phase). Resolved via PR #12. This changed the public API: `run()` now consumes the `DagRunner` and returns `DagOutput`.

## Notable PRs (architectural signal)

### PR #3 — Address loophole in cycle prevention
URL: https://github.com/swaits/dagx/pull/3
Summary: An earlier version had a loophole where cycles could theoretically be expressed. This PR closed it by ensuring `TaskHandle` has no `depends_on()` method and the builder is consumed (moved) on wiring. Documents that cycle prevention is a first-class design goal.

### PR #7 — Remove clone bound from task data
URL: https://github.com/swaits/dagx/pull/7
Summary: Removed `Clone` requirement from task inputs. Changed `TaskInput` to use a linear type pattern for dependency extraction, replacing the previous cloning approach.

### PR #12 — Make DagRunner construction single-threaded
URL: https://github.com/swaits/dagx/pull/12
Summary: Removed `parking_lot` dependency; `DagRunner` is now single-threaded during construction. `run()` consumes `DagRunner`, eliminating concurrent run footguns.

### PR #14 — Remove Clone/Copy from TaskHandle
URL: https://github.com/swaits/dagx/pull/14
Summary: `TaskHandle` no longer implements `Clone` or `Copy`, preventing repeat output fetches from `DagOutput::get()`.
