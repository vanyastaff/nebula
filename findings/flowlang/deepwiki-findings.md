# DeepWiki Findings — flowlang

Tier 2 protocol: queries 1, 2, 3, 4, 6, 7, 9 attempted (or 3-fail-stop).

## Query 1
"What is the core trait hierarchy for actions/nodes/activities?"
RESULT: Error — "Repository not found. Visit https://deepwiki.com to index it."

## Query 2
"How is workflow state persisted and recovered after crash?"
RESULT: Error — same (repository not indexed).

## Query 3
"What is the credential or secret management approach?"
RESULT: Error — same (repository not indexed).

3 consecutive failures reached — stopping per 3-fail-stop rule.
All findings from direct source-code and README analysis.
