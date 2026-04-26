# orka — Issues Sweep

## Summary

Commands executed:
```bash
gh issue list --repo excsn/orka --state open --limit 100 --json number,title,reactionGroups,labels
gh issue list --repo excsn/orka --state closed --limit 50 --json number,title,state,labels
gh api repos/excsn/orka/discussions --paginate
```

**Result:** All returned empty arrays (`[]`). The repository has zero open issues, zero closed issues, and zero discussions.

This is consistent with the project age (first public commit 2025-05-17, ~11 months old at time of analysis) and low adoption (665 total crates.io downloads).

## Implication

The Worker Brief requires ≥3 cited issues for Tier 1/2 projects with >100 closed issues. The orka repository has **0 closed issues**, so the threshold is not met and the citation requirement does not apply. This is documented here per the "document failures" operational rule.

## Notable signal from commit history

The only two commits are:
- `963db80 should use orkaresult more` (most recent)
- `235551a First Commit`

The first commit message indicates a known API consistency gap: handler functions did not uniformly return `OrkaResult`. This was caught and addressed by the author in a follow-up commit. No public issue was filed.

## Design gaps implied by zero issue activity

The lack of community issues does not mean the project is issue-free — it suggests:
1. Adoption is too low for external users to file issues
2. The sub-context write-through limitation (acknowledged in test comments at `core/tests/pipeline_execution_tests.rs:266`) has not been noticed or complained about externally
3. The `parking_lot::RwLock` deadlock footgun in async context has no filed issue
4. The single-pipeline-per-TData-type registry limitation has no filed issue
