# kotoba-workflow — DeepWiki Findings

## Query results

All three queries returned the same error: "Repository not found. Visit https://deepwiki.com to index it."

| Query # | Question | Result |
|---------|----------|--------|
| 1 | "What is the core trait hierarchy for actions/nodes/activities? How is the Actor trait defined and how do processes get dispatched?" | Error: Repository not found |
| 2 | "How is workflow state persisted and recovered after crash? What storage backends are used?" | Error: Repository not found |
| 3 | "What is the credential or secret management approach?" | Error: Repository not found |

## Protocol action

Three consecutive failures — DeepWiki augmentation stopped per §3.6 3-fail-stop protocol.

The repository `com-junkawasaki/kotoba` is not indexed in DeepWiki. All architectural findings were derived from direct source code inspection.
