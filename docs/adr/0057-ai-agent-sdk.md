# ADR-0057: AI agent SDK (`nebula-agent`)

**Status:** Proposed (2026-05-14)
**Tags:** ai, agent, sdk, llm, streaming

> **Amended by [ADR-0089](./0089-resource-advertised-agent-tools.md)
> (2026-06-04):** the primary agent-tool source is inverted from actions to
> **resources** (`impl ResourceTools for X`, discovered off acquired
> `ResourceGuard`s). `AgentTool: StatelessAction` (§1 below) is narrowed to a
> **secondary** provider; both feed one `ToolDefinition` and one registry. The
> `Llm`/`Memory`/ReAct-loop/streaming/multi-agent sections here are unchanged.
> Read 0089 before implementing the tool surface.

## Context

Charter §2 lists AI agent orchestration as one of three target
profiles. Current Nebula primitives (action, resource, credential,
schema) cover the **mechanism**; the **idiom** for building AI agents
on top remains undefined.

Existing tools — LangChain (Harrison Chase), LangGraph, LlamaIndex
(Jerry Liu), CrewAI (Joao Moura) — converged on common patterns:
typed tools, agent-as-graph-node, streaming outputs, dynamic DAG
where LLM picks next node, deterministic replay for debugging.

## Decision

Introduce `nebula-agent` SDK crate (separate from `nebula-sdk` core
facade) providing:

### 1. Typed agent tools

```rust
pub trait AgentTool: StatelessAction {
    fn description(&self) -> ToolDescription;       // for LLM tool-use
    fn examples(&self) -> &[ToolExample] { &[] }    // few-shot examples
}

// Auto-derived ToolDescription from #[derive(Schema)] on Self::Input:
impl<T: StatelessAction + AgentTool> ToTool for T { /* ... */ }

#[action("search.web", name = "Search Web")]
async fn search_web(query: String) -> Result<Vec<SearchResult>, ActionError> {
    /* ... */
}
// `search_web` automatically becomes a usable agent tool.
```

LLM receives JSON Schema from `Input` type, picks tool by name,
provides JSON args, engine deserializes + invokes.

### 2. Agent loop primitive

```rust
pub struct Agent<L: Llm> {
    llm: L,
    tools: ToolRegistry,
    memory: Box<dyn Memory>,
    max_iterations: u32,
}

impl<L: Llm> Agent<L> {
    pub async fn run(&self, initial_prompt: String) -> Result<AgentResult, AgentError> {
        // Standard ReAct loop:
        // 1. Send context + tool catalog to LLM
        // 2. LLM responds: either final answer or tool invocation
        // 3. If tool: invoke through engine, append result to context
        // 4. Goto 1 until max_iterations or final answer
    }
}
```

### 3. Streaming-first outputs

All agent operations return `StreamOutput` (per F5). Token-level
streaming from LLM, tool-result streaming, event streaming.

```rust
let mut stream = agent.run_streaming(prompt).await?;
while let Some(event) = stream.next().await {
    match event {
        AgentEvent::Token(t)        => print!("{t}"),
        AgentEvent::ToolCall { tool, args } => log_tool_call(tool, args),
        AgentEvent::ToolResult(r)   => log_tool_result(r),
        AgentEvent::FinalAnswer(a)  => return Ok(a),
    }
}
```

### 4. Multi-agent via workflow composition (per F-prior, ADR-0067?)

Multi-agent pattern: each agent is a workflow node. Agents communicate
via shared state (workflow context) or via message channels (engine
event bus).

```rust
type CustomerSupportFlow = Workflow<
    Connect<TriagingAgent, ResearchAgent>,
    Connect<ResearchAgent, ResponseAgent>,
    Connect<ResponseAgent, ReviewAgent>,
>;
```

`run_workflow!` macro for invoking sub-workflow as agent step (per
B-09).

### 5. Context budget tracking (per Jerry Liu B-04)

`OutputEnvelope::OutputMeta::TokenUsage` extended:

```rust
pub struct TokenUsage {
    pub input_tokens:    u32,
    pub output_tokens:   u32,
    pub context_tokens:  u32,        // RAG-retrieved
    pub max_context:     u32,        // model limit
    pub overflow_strategy: OverflowStrategy,  // Truncate | Summarize | Reject
}
```

Workflow editor visualization: token-budget bar per node showing
where context approaches limits.

### 6. Vector store as Resource

Per Jerry Liu Day 4 input: vector stores fit `Resource` model
naturally.

```rust
#[derive(Resource)]
struct PineconeStore { /* ... */ }

#[action("rag.retrieve")]
async fn retrieve(
    query: String,
    #[require("vector_store")] store: Handle<PineconeStore>,
    #[require("embedder")] embedder: Handle<OpenAICredential>,
) -> Result<Vec<Document>, ActionError> { /* ... */ }
```

## Consequences

### Positive

- AI agent authoring uses **same primitives** as other workflows —
  no separate SDK paradigm.
- Typed tools eliminate runtime "tool not found" / "args
  malformed" errors common in LangChain Python.
- Streaming first-class — no retrofit pain.
- LangChain compatibility: optional `nebula-agent-langchain-compat`
  contrib crate provides LangChain-like API on top.

### Negative

- AI vendor SDK churn (OpenAI / Anthropic / Google update API
  monthly) — `Llm` trait must remain stable; provider impls update
  separately.
- Replay determinism for LLM calls — requires response caching layer
  (separate crate `nebula-agent-replay` proposed).

### Neutral

- Multi-agent patterns vary widely (CrewAI hierarchy / LangGraph
  state machines / AutoGen conversation). Nebula provides primitives;
  doesn't pick one paradigm.

## References

- Conference Day 3 (CONFERENCE-NOTES.md) — Greg Brockman input.
- Conference Day 4 — Harrison Chase, Jerry Liu, Joao Moura input.
- LangChain LCEL streaming retrofit lessons.

## Out of scope

- Specific LLM provider implementations (OpenAI / Anthropic / etc.)
  — separate `nebula-agent-openai` etc. contrib crates.
- Vector store implementations — separate contrib crates per vendor.
- Reference ReAct agent implementation (the 200-line example) —
  ships in `examples/` workspace member.
