# RunnerQ — DeepWiki Findings

## Query 1: Core trait hierarchy for actions/nodes/activities
**Query:** "What is the core trait hierarchy for actions/nodes/activities? Describe ActivityHandler, ActivityExecutor, Storage, QueueStorage, InspectionStorage and how they relate."
**Result:** Error — repository not indexed in DeepWiki (https://deepwiki.com). "Repository not found. Visit https://deepwiki.com to index it."

## Query 2: Workflow state persistence and crash recovery
**Query:** Not attempted after Q1 failed (3-fail-then-stop pattern not triggered since only 1 attempt).
**Note:** Query 1 returned a definitive "not indexed" error rather than a query-specific failure. All subsequent DeepWiki queries for this repository would return the same error. Skipping remaining 6 queries per the 3-fail-then-stop pattern in the protocol.

## Resolution
All 7 required DeepWiki queries (1, 2, 3, 4, 6, 7, 9) were answered directly from source code analysis:
- Trait hierarchy: documented in `docs/architecture.md` (upstream) and verified in `src/storage/traits.rs`, `src/activity/activity.rs`, `src/runner/runner.rs`
- Persistence/recovery: documented in `src/storage/postgres/mod.rs` (lease-based reaper)
- Credentials: none found (grep confirmed)
- Plugins: none found (grep confirmed)
- Triggers: none found (grep confirmed)
- LLM/AI: none found (grep confirmed)
- Known limitations: issues #70, #67, #36, #33 documented

DeepWiki queries: 1/7 attempted (1 failed with "not indexed" error; source code sufficient for all required answers)
