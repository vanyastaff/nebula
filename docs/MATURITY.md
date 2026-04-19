---
name: Nebula crate maturity dashboard
description: Manual per-crate state dashboard. Edited in PRs that change a crate's API stability, test coverage, doc state, engine integration, or SLI-readiness.
status: accepted
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md, STYLE.md]
---

# Crate maturity dashboard

Updated manually in PRs that touch a crate's public surface, test bar, docs, engine integration, or observability. Reviewers check that this file stays truthful.

Legend:
- `stable` — end-to-end works, tested, safe to depend on.
- `frontier` — actively moving; breakage expected; do not add consumers without coordinating.
- `partial` — works for declared happy path; known gaps documented in the crate README.
- `n/a` — dimension does not apply to this crate.

| Crate | API stability | Test coverage | Doc completeness | Engine integration | SLI ready |
|---|---|---|---|---|---|
| nebula-action        | frontier | stable  | stable | partial (webhook sig covered; CheckpointPolicy planned; `ActionResult::Retry` gated behind `unstable-retry-scheduler`, #290) | n/a |
| nebula-api           | frontier | stable  | stable | partial (knife steps 3+5: Start/Cancel producers stable, #332/#330; engine-side Start/Resume/Restart dispatch wired via EngineControlDispatch — ADR-0008 A2; Cancel/Terminate dispatch still planned — A3) | partial |
| nebula-core          | frontier | stable  | stable | stable | n/a |
| nebula-credential    | frontier | stable  | stable | partial (rotation in integration tests) | n/a |
| nebula-engine        | partial  | stable  | stable | partial (ControlConsumer skeleton lands §12.2; Start/Resume/Restart dispatch via EngineControlDispatch — ADR-0008 A2; Cancel/Terminate dispatch planned — A3) | n/a |
| nebula-error         | stable   | stable  | stable | n/a | n/a |
| nebula-eventbus      | stable   | stable  | stable | n/a | n/a |
| nebula-execution     | stable   | stable  | stable | stable | partial |
| nebula-expression    | stable   | stable  | stable | stable | n/a |
| nebula-log           | stable   | stable  | stable | n/a | n/a |
| nebula-metrics       | stable   | stable  | stable | n/a | n/a |
| nebula-plugin        | partial  | stable  | stable | partial (registry wired; load path partial) | n/a |
| nebula-plugin-sdk    | partial  | stable  | stable | n/a | n/a |
| nebula-resilience    | stable   | stable  | stable | n/a | n/a |
| nebula-resource      | frontier | stable  | stable | partial (lifecycle visible; CAS guards partial) | n/a |
| nebula-runtime       | partial  | stable  | stable | stable | partial |
| nebula-sandbox       | partial  | stable  | stable | partial (process isolation; signing planned) | n/a |
| nebula-schema        | frontier | stable  | stable | stable | n/a |
| nebula-sdk           | partial  | stable  | stable | n/a | n/a |
| nebula-storage       | partial  | stable  | stable | stable | partial |
| nebula-system        | partial  | partial | stable | n/a | n/a |
| nebula-telemetry     | stable   | stable  | stable | n/a | n/a |
| nebula-validator     | frontier | stable  | stable | stable | n/a |
| nebula-workflow      | stable   | stable  | stable | stable | n/a |

---

## Review cadence

This file is a living dashboard. Reviewers check truthfulness on every PR that touches a crate's public surface, test suite, or docs. Canon §17 DoD includes "MATURITY.md row updated if the PR changes crate state."

Last full sweep: 2026-04-17 (Pass 4 of docs architecture redesign).
Last targeted revision: 2026-04-19 (Engine lifecycle cluster doc-sync: all 15 P1 issues landed; ADR-0015 lease-lifecycle promoted from proposed-0008 to accepted).
