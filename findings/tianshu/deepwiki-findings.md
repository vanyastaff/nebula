# Tianshu-rs — DeepWiki Query Results

## Query log

3 consecutive failures → stopped per 3-fail-stop protocol.

| Query | Response |
|-------|----------|
| Q1: "What is the core trait hierarchy for actions/nodes/activities?" | Error: "Repository not found. Visit https://deepwiki.com to index it." |
| Q2: "How is workflow state persisted and recovered after crash?" | Error: "Repository not found. Visit https://deepwiki.com to index it." |
| Q3: "What is the credential or secret management approach?" | Error: "Repository not found. Visit https://deepwiki.com to index it." |
| Q4-Q7 | Not attempted — 3-fail-stop triggered |

## Reason for failure

Tianshu-rs (https://github.com/Desicool/Tianshu-rs) has not been indexed by DeepWiki. The repository has 2 stars and was likely published too recently for automatic indexing.

## Impact on analysis

All findings were derived from direct source code inspection (git clone --depth 50). The 3K+ word architecture.md is based on primary source evidence with path:line citations throughout.
