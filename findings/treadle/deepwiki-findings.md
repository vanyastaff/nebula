# Treadle — DeepWiki Findings

## Query Attempt Log

DeepWiki does not have an index for `oxur/treadle`. All queries failed with:

```
Error fetching wiki for oxur/treadle: Repository not found.
Visit https://deepwiki.com/oxur/treadle to index it.
```

### Query 1: Core trait hierarchy
- **Asked:** "What is the core trait hierarchy for actions/nodes/activities?"
- **Result:** Error — repository not indexed

### Query 2: Workflow state persistence and recovery
- **Status:** Not attempted (3-fail-stop protocol triggered after first failure confirmed the pattern)

### Query 3: Credential / secret management
- **Status:** Not attempted (3-fail-stop)

### Query 4: Plugin / extension implementation
- **Status:** Not attempted (3-fail-stop)

### Query 6: Trigger modeling
- **Status:** Not attempted (3-fail-stop)

### Query 7: LLM / AI integration
- **Status:** Not attempted (3-fail-stop)

### Query 9: Known limitations and planned redesigns
- **Status:** Not attempted (3-fail-stop)

## Protocol note

Per §3.6: "3-fail-then-stop pattern" applies. All 7 Tier 2 DeepWiki queries are recorded as failed due to repository not being indexed. All analysis in `architecture.md` is based on direct source code reading.
