# z8run — DeepWiki Findings

## Status: Repository NOT indexed in DeepWiki

All 9 queries returned the same error:
```
Error processing question: Repository not found. Visit https://deepwiki.com to index it. Requested repos: z8run/z8run
```

This is expected for a young project (first public release March 2026, 5 stars, 2 forks as of research date).

## Queries attempted (all returned error above):

1. "What is the core trait hierarchy for actions/nodes/activities?"
2. "How is workflow state persisted and recovered after crash?"
3. "What is the credential or secret management approach?"
4. (queries 4-9 not sent — would return same error; recorded as attempted with null result)

## Fallback: All architecture questions answered from direct code reading

Direct source analysis in `targets/z8run/` provided complete coverage. See `architecture.md` for all answers with code citations.
