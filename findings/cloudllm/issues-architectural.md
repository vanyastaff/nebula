# cloudllm — Issues Summary

Total issues observed: 26 (25 closed, 1 open). Low volume — solo project.

## Open issues

| # | Title | Date |
|---|-------|------|
| 54 | Github Copilot provider | 2026-03-12 |

## Architecturally relevant closed issues

| # | Title | Significance |
|---|-------|-------------|
| 53 | Add Planner abstraction for agent orchestration (policy, memory, tools, streaming) | Major architectural addition — Planner trait added |
| 50 | Document AnthropicAgentTeams orchestration mode | New multi-agent mode |
| 34/32 | Keep per-provider clients hot | Performance: avoid re-creating HTTP clients |
| 26 | Arena/bump allocation for message bodies | Performance: bumpalo arena introduced in LLMSession |
| 24 | Reuse provider-ready payloads | Performance: serialization reuse |
| 22 | Trim before you transmit | Context trimming happens before serialization, not after |
| 17 | Support first-class streaming support | Streaming `MessageChunkStream` added |
| 16 | Cache per-message token estimates | Token cache to avoid re-estimation |
| 9 | Suggested low-latency improvements | Batch of perf improvements (parent of #10–#30) |
| 4 | Create abstractions for LLM clients to talk to one another | Orchestration engine genesis |
| 7 | Create Claude client | Claude provider added |
| 6 | Grok Client | Grok provider added |
| 5 | Gemini Client | Gemini provider added |
| 3 | Groq Integration | Groq (fast inference) — closed but status unclear; Groq is NOT in the current codebase |

## Notable patterns from issues

1. A focused performance sprint (issues #9–#30, all in September-October 2025) tuned LLM session allocation, cloning, and trimming. This signals the author noticed real latency in hot paths.
2. The Planner abstraction (#53) was a significant refactor, separating single-turn orchestration from the Agent identity — good separation of concerns added retroactively.
3. Provider expansion (Claude, Grok, Gemini) happened early (September 2025) — the multi-provider vision was present from the start.
4. Only 1 open issue — either very low usage or the author self-manages the backlog closely.
