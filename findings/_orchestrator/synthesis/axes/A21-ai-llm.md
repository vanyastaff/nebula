# A21 — AI / LLM Integration: Deep Cross-Project Analysis

**Strategic verdict for Nebula**: AI/LLM integration в Rust workflow space перешёл из "nice-to-have" в "expected feature" между 2024 и 2026. **7 из 27 исследованных проектов имеют first-class AI integration**, ещё 1 имеет documented design proposal (treadle v2). Defensive bet Nebula ("AI = generic actions + plugin LLM client + Surge orchestrator on ACP") нуждается в re-evaluation: конкуренты shipping LLM features today.

## Population

### Projects with shipped first-class AI

| Project | Position | Distinct architectural pattern |
|---------|----------|---------------------------------|
| **z8run** (T1) | n8n-style + WASM | **Node-based AI**: 10 dedicated AI nodes shipping (LLM/embeddings/vector store/agent/structured output/conversation memory/prompt template/text splitter/classifier/summarizer) |
| **runtara-core** (T2) | Compile-to-WASM durable | **First-class step + MCP server**: `AiAgent` step type + tool-call-as-edge labeling + conversation memory with SlidingWindow/Summarize compaction + structured output via provider-native schema + `rmcp 1.2` MCP server making platform AI-controllable |
| **tianshu** (T2) | LangGraph-alternative | **Coroutine-replay durability**: `ctx.step()` checkpoint + `LlmProvider` trait + `ToolRegistry` with `ToolSafety::ReadOnly`/`Exclusive` + `ManagedConversation` compaction + `ResilientLlmProvider` fallback chain |
| **cloudllm** (T3) | LLM client + agent SDK | **7-mode orchestration**: `ClientWrapper` 4 providers (OpenAI/Claude/Gemini/Grok) + `Agent` with `ToolProtocol` + 7 modes (Parallel/RoundRobin/Moderated/Hierarchical/Debate/Ralph/AnthropicAgentTeams) + MentisDB SHA-256 hash-chained persistent thought graph |
| **aofctl** (T3) | AI-first DevOps agent framework | **5 fleet modes**: 6 providers (Anthropic/OpenAI/Google/Groq/Ollama/Bedrock + Azure stub) + `AgentFleet` (Hierarchical/Peer/Swarm/Pipeline/Tiered) + JSON Schema validation with retry/fail/passthrough + regex prompt-injection detection |
| **rayclaw** (T3) | Single-agent runtime | **LLM-as-scheduler**: 3 backends (Anthropic native + OpenAI-compat + AWS Bedrock) + skill self-evolution via LLM-generated SKILL.md + sqlite-vec RAG + token tracking + prompt cache TTL pattern |
| **orchestral** (T3) | LLM-as-planner | **9-provider planner**: 6-iter replan loop with observations feedback + MCP bridge as plugin model + skill system (SKILL.md keyword-matched) + action-selector pre-filter (LLM pre-selects subset when catalog ≥ 30) |

### Projects with proposed but unshipped AI

| Project | Status | Detail |
|---------|--------|--------|
| **duroxide** (T1) | docs/proposals/llm-integration.md | Replay-safe LLM via history events `LlmRequested`/`LlmCompleted`. Architecturally sound — durable AI workflows. |
| **treadle** (T2) | v2 RetryBudget+QualityGate design | Stage / QualityGate (judges quality) / ReviewPolicy (decides next action) — designed for LLM pipeline patterns where outputs must meet criteria, with structured feedback to next attempt. |
| **rust-rule-engine** (T3) | aspirational only | Keywords "ai"/"ml" in Cargo.toml, env vars in .env.example, doc/example, but ZERO implementation. |
| **kotoba-workflow** (T2) | _archive/ only | Archived OpenAI client + Anthropic/Google stubs; not buildable. |

### Confirmed absent (verified by grep)

dataflow-rs, orka, acts, acts-next, runner_q, raftoral, dagx, fluxus, aqueducts-utils, ebi_bpmn, durable-lambda-core, deltaflow, dag_exec, emergent-engine (uses `claude -p` CLI-wrapping not native), temporalio-sdk.

## Architectural pattern catalog

### Pattern 1 — Node-based AI (z8run)
LLM as a built-in **node type** in the workflow graph. User wires LLM nodes alongside HTTP/code/branch nodes. Inputs: prompt, model name, params. Outputs: completion text. Pros: maps to no-code visual editor; user-controllable composition. Cons: stateful concerns (conversation memory) live outside the node; no durability semantics.

### Pattern 2 — First-class step + MCP server (runtara-core)
LLM as a dedicated **step kind** (`AiAgent`) with structured config (tools-as-edges, memory strategy, output schema). Plus the platform itself exposes an MCP server so external AI agents can drive workflow construction. Pros: type-safe configuration; AI-controllable platform. Cons: more invasive in core engine.

### Pattern 3 — Coroutine-replay (tianshu)
Workflow code = sequential async function. `ctx.step()` is the ONLY checkpoint primitive. LLM calls are normal `ctx.step()` invocations and gain durability automatically. Pros: simplest mental model; no DAG; LLM calls naturally durable. Cons: no graph-level concurrency control; harder to visualize.

### Pattern 4 — LLM-as-scheduler (rayclaw, orchestral, partial cloudllm)
The LLM **decides** what tool to call next. Workflow is unstructured tool-calling conversation. Pros: handles dynamic / unanticipated tasks. Cons: non-deterministic; harder to test; cost runaway risk; opposite of Nebula's typed-DAG philosophy.

### Pattern 5 — Multi-agent fleet coordination (cloudllm 7-mode, aofctl 5-mode)
Multiple agents collaborate via explicit topology (Hierarchical, Peer, Swarm, Pipeline, Tiered, Debate, AnthropicAgentTeams). Hand-off via shared memory or message passing. Pros: handles complex multi-step problems. Cons: non-trivial orchestration code; hard to debug; cost amplification.

### Pattern 6 — Replay-safe LLM events (duroxide proposal)
LLM call recorded as **history event** (`LlmRequested` / `LlmCompleted`). On replay, response is replayed from history rather than re-issued. Provides exactly-once LLM semantics in durable execution. Architecturally sound for durable AI workflows. **Pattern not yet implemented anywhere; raftoral's `checkpoint_compute!` is the closest live implementation.**

## Cross-cutting architectural patterns observed

### LLM provider abstraction (universal)
Every AI-first project has some `LlmProvider` / `Model` / `LlmClient` / `ClientWrapper` trait. Convergent design: 3-5 methods covering completion / streaming / tool-call. Multi-provider support is table stakes (4-9 providers per project; OpenAI + Anthropic + at least one local via Ollama is the minimum).

### Conversation memory + compaction (3 independent implementations)
- runtara-core: `SlidingWindow` / `Summarize`
- tianshu: 2 strategies in `ManagedConversation`
- cloudllm: 3 strategies (`Trim` / `SelfCompression` / `NoveltyAware`)

This convergence indicates conversation memory is a recognized first-class concern. Three-strategy minimum is the emerging norm.

### Tool calling with safety classification (1 leader)
**tianshu's `ToolSafety::ReadOnly`/`Exclusive`** is the only project that classifies tools by concurrency safety at the type level. This enables parallel ReadOnly tool execution while serializing Exclusive ones. Genuine borrow candidate.

### Structured output (3 implementations)
- aofctl: JSON Schema prompt injection + retry/fail/passthrough validation policies
- runtara-core: provider-native schema enforcement
- rayclaw: implicit via OpenAI-compat function calling

### MCP (Model Context Protocol) integration (6 projects)
- **Native server (host)**: runtara-core (rmcp 1.2 — workflow definitions exposed to AI clients)
- **MCP binary**: flowlang (`flowmcp` exposes loaded flows as LLM tools via JSON-RPC stdio)
- **MCP client (subprocess bridge)**: rayclaw, aofctl, orchestral, cloudllm
- **Sub-bridge / catalog planned**: aofctl (P0 issue #71)

This is a clear industry convergence pattern. **Workflow engines becoming AI-controllable via standardized protocol** is now a recognized direction.

### Multi-agent orchestration patterns (3 mature, multiple modes each)
- aofctl `AgentFleet`: Hierarchical / Peer / Swarm / Pipeline / Tiered (5)
- cloudllm `Orchestration`: Parallel / RoundRobin / Moderated / Hierarchical / Debate / Ralph / AnthropicAgentTeams (7)
- orchestral: 6-iter replan loop with observations feedback (single-agent + sub-agents)

cloudllm's `AnthropicAgentTeams` pattern (decentralized task-coordination via shared memory) and aofctl's Swarm/Pipeline are the most distinct patterns.

### Cost / safety controls (incomplete)
- **Cost tracking**: rayclaw (token tracking), aofctl (per-call counts) — implemented
- **Cost circuit breaker / budget**: aofctl has `max_cost_per_day` field but **no enforcement found** (genuine gap, also for Nebula to avoid)
- **Prompt injection detection**: aofctl (regex-based in `aof-conversational/src/sanitize.rs`)
- **Content filtering pre/post call**: not observed in any project

### RAG / vector store (1 leader)
**rayclaw** — `sqlite-vec` for embeddings. Lightweight in-process vector store. **cloudllm** has thought graph (MentisDB SHA-256 hash-chained) but not vector retrieval. Most projects skip RAG entirely or defer to user code.

## Verdict for Nebula's strategy

### Current Nebula bet (stated in NEBULA_CONTEXT.md)
> No first-class LLM abstraction yet. Strategic bet: AI workflows realized through generic actions + plugin LLM client. Surge (separate project) handles agent orchestration on ACP.

### Industry signal vs Nebula's bet

| Dimension | Industry signal (2026 Q2) | Nebula's bet |
|-----------|---------------------------|--------------|
| LLM provider trait | universal (every AI-first project) | will arrive via plugin |
| Conversation memory | 3 independent compaction strategies | not yet |
| Tool calling | universal + safety classification (tianshu) | not yet |
| MCP integration | 6 projects, multi-modal (server + bridge + client) | not yet |
| Multi-agent fleets | 2 mature implementations (5+7 modes) | will arrive via Surge |
| Replay-safe LLM | 1 proposal (duroxide), 1 close fit (raftoral) | implicit via durable actions |
| Cost tracking | implemented in 2-3 projects | not yet |
| Structured output | 3 implementations | not yet |
| RAG | 1 in-process (rayclaw sqlite-vec) | not yet |

**Conclusion**: Nebula's bet is _arguably correct_ for **Pattern 1 (node-based)** — generic actions + plugin LLM client can produce z8run-style 10-node catalog without core engine changes. But:

1. **MCP integration is core-level concern** — it's not just another action; it's an external-facing API surface that other AI agents use to drive Nebula. Cannot be left to plugins because it's about the **platform** itself, not workflow content. **Concrete actionable**: ship `nebula-mcp` binary that exposes registered actions as LLM tools (mirror of flowlang's `flowmcp`). Estimated effort: 1-2 weeks. Returns: zero-cost competitive parity with flowlang and partial parity with runtara-core.

2. **Replay-safe LLM events is a missed defensive bet** — Nebula's durable execution promise weakens if LLM calls aren't first-class history events. duroxide's proposal (`LlmRequested` / `LlmCompleted`) and raftoral's `checkpoint_compute!` semantic suggest this should be a core engine concern, not a plugin concern. **Concrete actionable**: extend the engine's append-only execution log to include AI-call events with a stable schema. Effort: 2-4 weeks for the core; plugin LLM client then targets this schema. Returns: durable AI workflows from day one.

3. **Tool safety classification (`ToolSafety::ReadOnly`/`Exclusive`)** is novel enough that adopting it would put Nebula ahead of all but tianshu. Cheap win for the LLM plugin SDK design.

4. **MCP server (Pattern 2) deserves a dedicated ADR** — the question of whether Nebula itself should be AI-controllable at the platform level (workflow construction, action discovery, execution control) is a strategic decision, not a feature flag. Recommend: ADR before next milestone.

5. **The "AI ≠ first-class kind"** stance is sustainable if and only if (1)-(3) above ship. Without them, Nebula's AI story will lag z8run / runtara / tianshu by a 12-18 month gap.

### What NOT to do

- **Don't ship a 6th sealed action kind for AI**. The plugin LLM client + replay-safe events approach is correct. Industry has 4 patterns; locking into one (Pattern 1) is premature.
- **Don't build multi-agent fleet modes in core**. cloudllm and aofctl's mode catalogs are interesting but should live in Surge or a higher-level orchestrator, not in the workflow engine.
- **Don't prioritize RAG primitives**. Only 1 of 27 has it (rayclaw sqlite-vec); most defer to user code. Plugin-level concern.
- **Don't replicate aofctl's `max_cost_per_day` field without enforcement** — that's user-facing safety theater. Either implement enforcement properly or don't expose the field.

## Borrow candidates ranked

| Pattern | Source | Effort | Strategic value |
|---------|--------|-------:|-----------------|
| MCP server binary (`nebula-mcp`) | flowlang `flowmcp`, runtara-core rmcp | 1-2w | ⭐⭐⭐ Industry parity, AI-controllable platform |
| Replay-safe LLM events in execution log | duroxide proposal | 2-4w | ⭐⭐⭐ Durable AI workflows, defensive bet |
| `ToolSafety::ReadOnly`/`Exclusive` for plugin LLM SDK | tianshu | 1w | ⭐⭐ Concurrency-safe tool calling |
| Conversation memory compaction strategy enum | runtara/tianshu/cloudllm convergence | 1-2w | ⭐⭐ Standard feature for plugin LLM client |
| LLM error 5-category classifier | rayclaw | 1w | ⭐⭐ Better resilience integration |
| Prompt cache TTL semantic | rayclaw | 0.5w | ⭐ Token cost optimization |
| `OutputSchemaSpec` retry-on-validation | aofctl | 1w | ⭐⭐ Structured output reliability |
| Action-selector pre-filter | orchestral | 1-2w | ⭐ Scaling to large action catalogs |
| `Supervisor` resilience primitive (agent crash/restart) | aofctl | 1w (extends nebula-resilience) | ⭐⭐ Closes existing gap |

Total recommended core engine work: 6-12 weeks for the high-value items (MCP server + replay-safe LLM events + tool safety + memory compaction). Everything else lives in plugin LLM client.
