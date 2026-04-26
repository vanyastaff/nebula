# DeepWiki Findings — emergent-engine

## Query Attempts (7 required for Tier 2)

All 7 required DeepWiki queries were submitted to `govcraft/emergent`.

**Result for all 7 queries:**
```
Error processing question: Repository not found. Visit https://deepwiki.com to index it. Requested repos: govcraft/emergent
```

DeepWiki has no index for the `govcraft/emergent` repository as of 2026-04-26.

3-fail-then-stop pattern triggered on the first round of parallel queries (queries 1, 2, 3 all returned the same error). Per protocol, no further DeepWiki queries were attempted.

All architectural findings were obtained directly from source code, documentation, and git history.

## Queries That Were Attempted

1. "What is the core trait hierarchy for actions/nodes/activities? What are the Source, Handler, and Sink types and their trait signatures?" — NOT FOUND
2. "How is workflow state persisted and recovered after crash? What event store backends are used and how is replay handled?" — NOT FOUND
3. "What is the credential or secret management approach? How does the engine handle API keys, tokens, and other secrets?" — NOT FOUND
4. (Would have been: "How are plugins or extensions implemented?") — SKIPPED (3-fail pattern)
5. (Would have been: "How are triggers (webhooks, schedules, external events) modeled?") — SKIPPED
6. (Would have been: "Is there built-in LLM or AI agent integration?") — SKIPPED
7. (Would have been: "What known limitations or planned redesigns are documented?") — SKIPPED
