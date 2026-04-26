# Master Matrix — Nebula vs 27 Rust Workflow/Orchestration Projects

**Tiers:** T1 = direct competitor (deep dive 6), T2 = adjacent/important (10), T3 = reference (11). Total 27 + Nebula baseline = 28 rows.

**Categories codified in cells:**
- ✓ = present, comparable to Nebula or better in this axis
- ◐ = partial / weak / aspirational only
- ✗ = absent (verified by grep evidence in source architecture.md)
- N/A = different problem domain (not a workflow engine in Nebula's sense)
- ⭐ = competitor deeper than Nebula on this axis (rare, flagged)

## Strategic axes (most relevant for Nebula)

| Project | Tier | A1 Workspace | A2 DAG | A3 Action | A4 Cred | A5 Res | A6 Resilience | A11-BUILD | A11-EXEC | A12 Trigger | A14 Tenancy | A17 TypeSafe | A21 AI/LLM |
|---------|:----:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| **Nebula** | — | 26 crates layered | TypeDAG L1-L4 | 5 sealed kinds + assoc types | State/Material split, LiveCredential | 4-scope, ReloadOutcome | retry/CB/bulkhead/timeout/hedge | WASM planned, plugin-v2 | WASM + capability sec | TriggerAction Source→Event 2-stage | nebula-tenant 3 modes | sealed/GAT/HRTB/typestate | ✗ (defensive bet) |
| z8run | T1 | n8n-style ✓ | runtime DAG | typed nodes | ◐ table no user_id | ✗ | ✗ (timeout only) | wasmtime v42 | ◐ caps unenforced | webhook+cron(broken) | ✗ | minimal | ⭐ **10 AI nodes shipping** |
| temporalio-sdk | T1 | 6 crates | none (replay) | Workflow + Activity | ✗ (TLS only) | ✗ | server-side retry | ✗ (workflows ARE plugins) | ✗ | server-owned schedule + signal | server namespace | minimal | ✗ |
| acts | T1 | 1 crate + 32K LOC | runtime tree | open ActPackageFn, type-erased | ✗ (7-line SecretsVar JS global) | ✗ | ✗ | inventory ✓ | ✗ (in-process) | manual/hook/chat (cron roadmap) | ✗ | minimal | ✗ |
| duroxide | T1 | 1 crate + tool | replay tree | OrchestrationHandler + ActivityHandler | ✗ | ✗ | retry/error class | ✗ (no plugins) | ✗ | raise_event/enqueue primitives | ✗ | minimal | ✗ (proposal: LlmRequested/LlmCompleted history events) |
| orka | T1 | 1 crate | sequential ✗ | Pipeline<TData,Err> | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | TData generic only | ✗ |
| dataflow-rs | T1 | 2 crates | priority list | AsyncFunctionHandler async-trait | ✗ | ✗ | retryable() class | ✗ (no plugins) | ✗ | ✗ | ✗ | ✗ Value | ✗ |
| acts-next | T2 | 1 crate (fork) | same as acts | +SetVars/SetProcessVars | ✗ same as acts | ✗ | ✗ | inventory same | ✗ | manual/hook/chat | ✗ | minimal | ✗ |
| runner_q | T2 | 2 crates | ✗ (queue) | ActivityHandler Value I/O | ✗ | ✗ | retry+OnDuplicate | ✗ | ✗ | imperative enqueue + delay | ✗ | minimal | ✗ |
| runtara-core | T2 | compile-to-WASM | JSON DSL → AOT | AiAgent step + saga | ✗ | ✗ | retry only | inventory + WASM target | static linkage | server-owned schedule | single-tenant | minimal | ⭐ **AiAgent step + MCP server (rmcp 1.2)** |
| dagx | T2 | 3 crates | typestate ⭐ | TaskBuilder consume | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | typestate cycle prevent ⭐ | ✗ |
| emergent-engine | T2 | OS process bus | cycles ALLOWED | Source/Handler/Sink | ✗ | ✗ | timeout only | git-repo registry | OS subprocess (no sandbox) | interval+webhook (ext) | ✗ | minimal | ✗ (CLI-wrap pattern) |
| flowlang | T2 | JSON-dataflow | runtime | Command structs | ✗ | ✗ | catch_unwind | ✗ | ✗ (in-process polyglot) | manual via API | ✗ | ✗ DataObject | ⭐ MCP server (flowmcp binary) |
| tianshu | T2 | coroutine-replay | ✗ | ctx.step()+toolregistry | ✗ | ✗ | ResilientLlmProvider fallback | ✗ | ✗ | polling (LLM IntentRouter) | ✗ | minimal | ⭐ **LangGraph-alt** LlmProvider+ToolSafety+ManagedConversation |
| treadle | T2 | 1 crate | linear stages | Stage+QualityGate(v2) | ✗ | ✗ | retry+v2 RetryBudget | ✗ | ✗ | manual | ✗ | minimal | ◐ v2 retry-with-feedback design only |
| raftoral | T2 | 2 crates Raft | none | WorkflowFunction(name,ver) | ✗ | ✗ | retry only | ✗ | ✗ (sidecar gRPC for polyglot) | imperative | ✗ | minimal | ✗ (checkpoint_compute! natural fit) |
| kotoba-workflow | T2 | research-grade | ✗ (archived) | Mediator process | ✗ (plaintext API key in archive) | ✗ | ✗ | ✗ | ✗ | ✗ (archived enum) | ✗ | minimal | ◐ archived OpenAI client only |
| fluxus | T3 | 8 crates | ✗ linear stream | Source/Op/Sink async-trait | ✗ | ✗ | backpressure+retry | ✗ | ✗ | source-driven | ✗ | minimal | ✗ |
| aqueducts-utils | T3 | 8 crates | YAML+SQL impl deps | Stage{name,query} only | ✗ | ✗ | DataFusion impl | ✗ | ✗ static linkage | manual | ✗ | minimal | ✗ |
| rayclaw | T3 | 1 crate | LLM-driven | Tool trait Value I/O | ✗ | ✗ | exp backoff+5-cat LLM err | ✗ | ✗ ACP/MCP subprocess | manual via ACP | ✗ | minimal | ⭐ **LLM-as-scheduler** Anthropic+OpenAI-compat+Bedrock+sqlite-vec RAG+token tracking |
| rust-rule-engine | T3 | 1 crate | RETE-UL network | 10-variant ActionType + RulePlugin | ✗ | ✗ | ✗ | same-binary | ✗ | ✗ | ✗ | minimal | ✗ (aspirational only) |
| cloudllm | T3 | 2 crates | LLM-driven | ClientWrapper trait | ✗ | ✗ | ContextStrategy fallback | ✗ | ✗ MCP subprocess | ✗ | ✗ | ✗ Value | ⭐ **7 orchestration modes** OpenAI/Claude/Gemini/Grok+MentisDB+AnthropicAgentTeams |
| aofctl | T3 | 17 crates | linear+fleet | Agent trait async-trait | ✗ | ✗ | Supervisor primitive ⭐ | ✗ | MCP subprocess + Docker bollard | manual via CLI | ✗ | minimal | ⭐ **5 fleet modes** Anthropic/OpenAI/Google/Groq/Ollama/Bedrock+JSON Schema retry |
| orchestral | T3 | 6 crates | LLM-as-planner | open Action Value I/O | ✗ | ✗ | exp backoff | ✗ | MCP subprocess | manual via CLI | ✗ | minimal | ⭐ **9 providers** + MCP bridge + skill system + action-selector pre-filter |
| dag_exec | T3 | 1 crate | Kahn BFS+pruning | closure (uniform O) | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | minimal | ✗ |
| ebi_bpmn | T3 | 1 crate | BPMN Petri-net | 22-variant enum + traits | ✗ | ✗ | ✗ | ✗ | ✗ | BPMN events (token-game) | ✗ | minimal | ✗ |
| durable-lambda-core | T3 | 6 crates | replay tree | DurableContextOps RPITIT | ✗ | ✗ | AWS-side | ✗ | ✗ | AWS-owned | AWS-owned | minimal | ✗ |
| deltaflow | T3 | 2 crates | linear chain | Step trait 2 assoc | ✗ | ✗ | retry only | ✗ | ✗ | interval | ✗ | minimal | ✗ |

## Aggregate signals

| Signal | Count | Projects |
|--------|------:|----------|
| **First-class AI/LLM integration** | 7/27 | z8run, runtara-core, tianshu, rayclaw, cloudllm, aofctl, orchestral |
| **MCP server / bridge integration** | 6/27 | runtara-core (rmcp 1.2 native), flowlang (flowmcp binary), rayclaw, aofctl, orchestral, cloudllm |
| **Distributed coordination** | 2/27 | temporalio-sdk (server), raftoral (embedded Raft) |
| **WASM plugin sandbox attempt** | 2/27 | z8run (capabilities unenforced), runtara-core (compile-target only) |
| **Real plugin EXEC isolation** | 1/27 | aofctl (Docker via bollard) |
| **Credential subsystem (any depth)** | 0/27 | none — Nebula is the only entry on this axis |
| **Resource lifecycle abstraction** | 0/27 | none — Nebula is the only entry on this axis |
| **Multi-tenancy (RBAC/RLS/SCIM)** | 0/27 | none |
| **3+ deployment modes from one codebase** | 0/27 | Nebula's 3-mode is unique |
| **Sealed traits + assoc types for action** | 0/27 | none — Nebula's 5 sealed kinds is unique |
| **Type-erased I/O (`serde_json::Value`)** | 22/27 | most projects |
| **typestate / phantom types for safety** | 1/27 | dagx (cycle prevention) |
| **`async_trait` macro use (1.95+ anti-pattern)** | many | acts, dataflow-rs, fluxus, aofctl, others — common |
| **Native AFIT / RPITIT** | 2/27 | duroxide (partial), durable-lambda-core (full) |
| **Provider/Backend trait pattern (test isolation)** | 6/27 | duroxide, raftoral, runner_q, cloudllm, durable-lambda-core, others |
| **Saga / compensation as first-class** | 1/27 | runtara-core (CompensationConfig) |
| **Human-in-the-loop pause** | 1/27 | treadle (StageOutcome::NeedsReview) |

## DeepWiki indexing reality check

| DeepWiki status | Count | Projects |
|-----------------|------:|----------|
| **Indexed (queries succeeded)** | 5/27 | acts (9/9), dataflow-rs (9/9), aqueducts-utils (4/4), fluxus (4/4), rust-rule-engine (4/4), rayclaw (4/4) |
| **NOT indexed (3-fail-stop)** | 22/27 | most projects, including all major Tier 1 (z8run, temporalio-sdk, duroxide, orka) |

**Implication**: DeepWiki is unreliable for niche / new repos. Direct code reading remains primary mechanism. Worker brief's 3-fail-stop pattern saved significant cycles.
