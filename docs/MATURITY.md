---
name: Nebula crate maturity dashboard
description: Manual per-crate state dashboard. Edited in PRs that change a crate's API stability, test coverage, doc state, engine integration, or SLI-readiness.
status: skeleton
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
| nebula-action        |   |   |   |   |   |
| nebula-api           |   |   |   |   |   |
| nebula-core          |   |   |   |   |   |
| nebula-credential    |   |   |   |   |   |
| nebula-engine        |   |   |   |   |   |
| nebula-error         |   |   |   |   |   |
| nebula-eventbus      |   |   |   |   |   |
| nebula-execution     |   |   |   |   |   |
| nebula-expression    |   |   |   |   |   |
| nebula-log           |   |   |   |   |   |
| nebula-metrics       |   |   |   |   |   |
| nebula-plugin        |   |   |   |   |   |
| nebula-plugin-sdk    |   |   |   |   |   |
| nebula-resilience    |   |   |   |   |   |
| nebula-resource      |   |   |   |   |   |
| nebula-runtime       |   |   |   |   |   |
| nebula-sandbox       |   |   |   |   |   |
| nebula-schema        |   |   |   |   |   |
| nebula-sdk           |   |   |   |   |   |
| nebula-storage       |   |   |   |   |   |
| nebula-system        |   |   |   |   |   |
| nebula-telemetry     |   |   |   |   |   |
| nebula-validator     |   |   |   |   |   |
| nebula-workflow      |   |   |   |   |   |

Cell values populate in Pass 4 (crate sweep).
