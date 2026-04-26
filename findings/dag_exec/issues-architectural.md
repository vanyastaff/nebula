# Issues — dag_exec

Total issues: 6 (1 open, 5 closed).

## Open issues

### #6 — Error hardening & invariants cleanup
- **Label:** enhancement
- **State:** OPEN
- **Created:** 2026-02-17
- **URL:** https://github.com/reymom/rust-dag-executor/issues/6
- **Summary:** Error hardening and invariant tightening. Mentioned in README "Next" section as upcoming work. Covers: better error messages, invariant documentation, edge-case validation improvements.

## Closed issues (recent)

### #5 — Docs polish: rustdoc, README, and API story
- **Label:** documentation
- **Closed:** 2026-02-26

### #4 — docs/examples: merkle + pipeline examples
- **Label:** documentation
- **Closed:** 2026-02-26

### #3 — bench: criterion suite (sequential vs parallel, prune vs full)
- **Label:** performance
- **Closed:** 2026-02-22

### #2 — parallel: correctness + failure/RAII tests
- **Label:** tests
- **Closed:** 2026-02-21

### #1 — parallel: enforce max_in_flight via sync_channel + try_send
- **Label:** enhancement
- **Closed:** 2026-02-20
- **Summary:** Backpressure design issue. Landed as bounded sync_channel approach with per-worker queues and global max_in_flight cap.

## Note on issue volume

6 total issues — well below the ≥100 closed issues threshold that triggers the "cite ≥ 3" rule for Tier 1/2. Tier 3 citation rules do not require minimum issue count.
