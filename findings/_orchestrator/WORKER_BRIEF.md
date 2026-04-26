# Worker Brief — Nebula Competitor Research v2

**You are a research worker.** You will receive an assigned project (name + repo URL + tier). Your job is to clone it, decompose its architecture across 21 axes, and produce `findings/<project-name>/architecture.md` per the protocol below.

**Hard rules (rejection triggers):**
1. Code citations MUST include path + line number for every architectural claim. "I think it uses WASM" without `crates/foo/src/loader.rs:42` = rejected.
2. **Negative findings are mandatory.** If the project lacks a credential/plugin/AI/resource layer — explicit statement + grep evidence (show what you searched for and that you found nothing). Silence = rejected.
3. For Tier 1 projects: `architecture.md` must be ≥ 6K words, all 22 scorecard rows filled (A1-A21 with A11 split into BUILD + EXEC).
4. For Tier 2: ≥ 3K words, scorecard required.
5. **Every Deep Question** in §1.5 below for axes A3/A4/A5/A11/A12/A21 must be explicitly answered. "Project does not implement X" is a valid answer if backed by grep evidence.
6. Cite ≥ 3 GitHub issues for Tier 1/2 projects with >100 closed issues. Use `gh issue list`.

---

## §1.1 — 21 Architectural axes

| # | Axis | Nebula's approach | Look for in competitor |
|---|------|-------------------|------------------------|
| A1 | Workspace structure | 26 crates, layered | crate count, layers, feature flags, umbrella |
| A2 | DAG model | TypeDAG L1-L4 (generics → TypeId → predicates → petgraph) | how graph described, compile-time vs runtime check, port typing |
| A3 | Action/Node abstraction | 5 action kinds, sealed traits, assoc Input/Output/Error, versioning, derive macros | see §1.5 A3 deep Qs |
| A4 | Credentials | State/Material split, CredentialOps, LiveCredential, blue-green refresh, OAuth2Protocol | see §1.5 A4 deep Qs |
| A5 | Resource lifecycle | Scoped, ReloadOutcome enum, generation tracking, on_credential_refresh | see §1.5 A5 deep Qs |
| A6 | Resilience | retry / CB / bulkhead / timeout / hedging, ErrorClassifier | patterns, classification |
| A7 | Expression engine | 60+ funcs, type inference, sandbox, `$nodes.foo.result.email` | DSL, syntax, sandbox, types |
| A8 | Storage layer | sqlx + PgPool, Pg*Repo, migrations, RLS | DB, query layer, migrations |
| A9 | Persistence model | Frontier-based, checkpoint, append-only execution log | persistence model, recovery |
| A10 | Concurrency | tokio, frontier scheduler, !Send isolation | runtime, scheduler, !Send handling |
| A11 | Plugin system | WASM sandbox planned, plugin v2 spec, Plugin Fund | see §1.5 A11 — split BUILD + EXEC |
| A12 | Trigger/Event model | TriggerAction (Input=Config, Output=Event), Source normalizes | see §1.5 A12 deep Qs |
| A13 | Deployment modes | 3 modes from one codebase | binary / lib / SaaS / multi-mode |
| A14 | Multi-tenancy | nebula-tenant: schema/RLS/db, RBAC, SSO, SCIM | tenant isolation, RBAC, SSO |
| A15 | Observability | OpenTelemetry, structured tracing per execution | telemetry stack, granularity |
| A16 | API surface | REST + GraphQL/gRPC plan, OpenAPI, OwnerId-aware | API type, OpenAPI, versioning |
| A17 | Type safety | Sealed traits, GATs, HRTBs, typestate, Validated<T> | advanced type-system features |
| A18 | Errors | nebula-error, ErrorClass | own type / anyhow / eyre |
| A19 | Testing infra | nebula-testing crate, contract tests | public testing utils, contracts |
| A20 | Governance/commercial | Open core, Plugin Fund, planned SOC 2 | license, governance, commercial story |
| A21 | AI/LLM integration | (none yet — Nebula bets AI = generic actions + plugin LLM client) | see §1.5 A21 deep Qs |

---

## §1.5 — Deep Questions (mandatory for A3, A4, A5, A11, A12, A21)

### A3 — Action/Node structure

**A3.1 Trait shape:** sealed/open? trait-object compatible (`dyn Action`)? assoc types count (Input/Output/Error/Config/Context/State)? GAT? HRTB? typestate? default methods?

**A3.2 I/O shape:** Input serializable required? generic? type-erased (serde_json::Value / Box<dyn Any> / enum)? same for Output. Streaming output? Side-effects model?

**A3.3 Versioning:** v1 vs v2 distinguishable in workflow definition? migration support? `#[deprecated]`? referenced by name+version, name only, or type-tag?

**A3.4 Lifecycle hooks:** pre/execute/post/cleanup/on-failure? all async or mixed? cancellation points? idempotency key?

**A3.5 Resource & credential deps:** how does action declare "I need DB pool X + credential Y"? assoc types / attribute / config / constructor inject? compile-time check?

**A3.6 Retry/resilience attachment:** per-action policy or global? declared via metadata / attribute / runtime config? override workflow-level?

**A3.7 Authoring DX:** derive macro / builder / manual impl? "hello world action" line count? IDE support?

**A3.8 Metadata:** display name / description / icon / category — where? i18n? compile-time vs runtime?

**A3.9 vs Nebula:** Nebula has 5 action kinds (Process/Supply/Trigger/Event/Schedule). Competitor has how many? Sealed + assoc types vs ?

### A4 — Credentials

**A4.1 Existence:** separate credential layer? Or just env vars / config strings? If absent — documented design decision or omission?

**A4.2 Storage:** at-rest encryption (AES-256-GCM / ChaCha20-Poly1305 / none)? backend (own DB / external vault / OS keychain / file)? key rotation?

**A4.3 In-memory protection:** Zeroize? secrecy::Secret<T>? own wrapping? lifetime limits?

**A4.4 Lifecycle:** CRUD + revoke? refresh model (poll / push / on-demand)? expiry detection + auto-refresh? revocation (hard delete / soft / tombstone)?

**A4.5 OAuth2/OIDC:** authorization code? client credentials? device code? PKCE? multi-provider? refresh handling? scope mgmt?

**A4.6 Composition:** one credential per action or multiple? delegation? SSO patterns?

**A4.7 Scope:** per-user / workspace / tenant / global? cross-execution sharing? workflow ownership?

**A4.8 Type safety:** Validated/Unvalidated state distinction? compile-time leak prevention? phantom types per credential kind?

**A4.9 vs Nebula:** State/Material split, LiveCredential watch(), blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter erasure — competitor has which?

### A5 — Resource lifecycle

**A5.1 Existence:** separate resource abstraction (DB pools / HTTP clients / caches as first-class) or each action makes its own?

**A5.2 Scoping:** scope levels (Global / Workflow / Execution / Action / Step)? who decides — action / runtime / config?

**A5.3 Lifecycle hooks:** init / shutdown / health-check? async or sync? failure during init: block / degrade / skip?

**A5.4 Reload:** hot-reload? blue-green? ReloadOutcome enum? generation counter for cache invalidation?

**A5.5 Sharing:** one-per-execution or shared? pooling? Arc/Rc?

**A5.6 Credential deps:** resource declares which creds? notified on rotation? per-resource refresh hook?

**A5.7 Backpressure:** acquire timeout? bounded queue? priority levels?

**A5.8 vs Nebula:** Nebula has 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh — subset / superset / different?

### A11 — Plugin system (BUILD vs EXEC, separate sub-axes!)

**BUILD process:**

**A11.1 Format:** .tar.gz / OCI / cargo crate / WASM blob / dynamic library / workspace member? manifest format (TOML/JSON/YAML)? schema versioned? multi-plugins per package?

**A11.2 Toolchain:** where compile (in-tree / separate cargo project / SDK)? cross-compilation? reproducibility? build SDK (cli / cargo extension / scaffolding)?

**A11.3 Manifest content:** required fields? capability declaration (network / fs / crypto)? permission grants (write /tmp, contact api.example.com)? plugin deps? resource declarations?

**A11.4 Registry/discovery:** local dir? remote HTTP? OCI registry? signing? search/list/version pinning?

**EXEC sandbox:**

**A11.5 Sandbox type:** dyn library (libloading): hot-reload? memory isolation? ABI? — OR — WASM: runtime (wasmtime/wasmer/wasmi/components)? interface (WIT / wit-bindgen / raw)? — OR — subprocess: IPC (Unix socket / pipe / stdin / shmem)? — OR — remote RPC?

**A11.6 Trust boundary:** plugin treated as untrusted? capability-based? CPU/memory/wall-time limits? network/fs policy?

**A11.7 Host↔plugin calls:** host-provided fns? plugin-exposed fns? marshaling (serde / prost / wit-bindgen / custom)? async crossing? error propagation?

**A11.8 Lifecycle:** start/stop/reload? hot reload? crash recovery (restart/fail/degrade)?

**A11.9 vs Nebula:** Nebula targets WASM + capability security + Plugin Fund commercial. Competitor uses what? Has commercial monetization model?

### A12 — Trigger/Event

**A12.1 Trigger types:** webhook? schedule (cron/interval/one-shot)? external event (Kafka/RabbitMQ/NATS/pubsub/Redis streams)? FS watch? DB change (CDC/LISTEN-NOTIFY)? polling? internal event? manual?

**A12.2 Webhook:** registration time? URL allocation (stable/random/configurable)? idempotency key? HMAC verification? retry on init fail? rate limiting?

**A12.3 Schedule:** cron variant (POSIX/Quartz/Vixie)? timezone? DST? missed schedule recovery? distributed (no double-fire)?

**A12.4 External event:** direct broker integration vs external connector model? consumer groups / offsets? ordering?

**A12.5 Reactive vs polling:** default model? both supported? which preferred?

**A12.6 Trigger→workflow dispatch:** 1:1 or fan-out? trigger metadata as context? conditional triggers? replay support?

**A12.7 Trigger as Action:** is trigger a kind of Action (like Nebula TriggerAction) or separate? lifecycle (forever / one-shot)?

**A12.8 vs Nebula:** Source → Event → TriggerAction 2-stage. Competitor similar 2-stage? Backpressure model?

### A21 — AI/LLM integration ⭐ NEW

**A21.1 Existence:** built-in / separate crate / community plugin / nothing? central feature or nice-to-have?

**A21.2 Provider abstraction:** single (OpenAI only) or multi (OpenAI + Anthropic + local)? provider trait shape? BYOL endpoint? local model (llama.cpp / ollama / candle / mistral.rs)?

**A21.3 Prompt mgmt:** templating? system/user/assistant structure? few-shot? versioning? prompts checked into workflow definition?

**A21.4 Structured output:** JSON mode? schema enforcement (JSON Schema / serde / Pydantic-style)? function/tool calling? re-prompting on validation fail?

**A21.5 Tool calling:** definition format? multi-tools per call? execution sandbox (same proc / plugin / external)? feedback loop (multi-turn)? parallel exec?

**A21.6 Streaming:** SSE / chunked? streaming → workflow nodes? backpressure?

**A21.7 Multi-agent:** patterns (agents calling agents)? hand-off? shared memory? termination conditions?

**A21.8 RAG/vector:** embeddings built-in? vector store integration (Qdrant / Pinecone / pgvector / Weaviate)? retrieval as workflow node?

**A21.9 Memory/context:** conversation memory (per-execution / session / user)? context window mgmt (truncation / summarization / sliding)? long-term memory?

**A21.10 Cost/tokens:** counting? per-provider cost calc? budget circuit breakers? per-tenant attribution?

**A21.11 Observability:** per-LLM-call tracing? prompt+response logging (PII-safe)? eval hooks (LLM-as-judge)?

**A21.12 Safety:** content filtering pre/post? prompt injection mitigations? output validation?

**A21.13 vs Nebula+Surge:** Nebula has no first-class LLM (bet: AI = generic actions + plugin LLM). Surge = agent orchestrator on ACP. Competitor first-class? Working or over-coupled?

---

## §3 — Per-project investigation protocol

### §3.1 Acquisition

```bash
mkdir -p targets findings/<crate-name>
git clone --depth 50 <repo_url> ./targets/<crate-name>
cd ./targets/<crate-name>
git fetch --tags --depth 50
git log --oneline -20 > ../../findings/<crate-name>/structure/git-log.txt
git tag --sort=-creatordate | head -5 >> ../../findings/<crate-name>/structure/git-log.txt
```

### §3.2 Documentation harvest

Save to `findings/<crate-name>/docs/`:
- README.md (root + sub-crates)
- docs/ directory if present
- ARCHITECTURE.md, DESIGN.md, RFC*.md, ADR*.md, *.spec.md
- CHANGELOG.md
- examples/ structure + 2-3 representative examples

### §3.3 Code structure

```bash
fd -t d -d 3 . > findings/<crate-name>/structure/tree-d3.txt
fd Cargo.toml --type f -d 3 -X cat > findings/<crate-name>/structure/all-cargo-tomls.txt
tokei --output json . > findings/<crate-name>/structure/tokei.json 2>/dev/null || echo "tokei failed" > findings/<crate-name>/structure/tokei.json
```

Create `findings/<crate-name>/structure-summary.md`: crate count, deps graph, top-10 deps, LOC, test count.

### §3.4 — `architecture.md` template (THE MEAT)

```markdown
# <Crate name> — Architectural Decomposition

## 0. Project metadata
Repo, stars, forks, last activity, license, governance, maintainers.

## 1. Concept positioning [A1, A13, A20]
- One-sentence (from author's own README)
- One-sentence (mine, after reading code)
- Comparison with Nebula

## 2. Workspace structure [A1]
Crate count + names, layer separation, feature flags, umbrella.

## 3. Core abstractions [A3, A17] ⭐ DEEP
**ANSWER ALL A3.1-A3.9 questions. Code citations MANDATORY.**
Trait hierarchy for unit of work — full signatures (path:line).
Comparison with Nebula 5-types ProcessAction/SupplyAction/TriggerAction/EventAction/ScheduleAction.

## 4. DAG / execution graph [A2, A9, A10]
Graph desc, port typing, compile-time checks, scheduler model, concurrency.

## 5. Persistence & recovery [A8, A9]
Storage, schema/event-sourcing/journal, checkpoint, recovery semantics.

## 6. Credentials / secrets [A4] ⭐ DEEP
**ANSWER ALL A4.1-A4.9 questions.**
If "no credential layer" — explicit statement + grep evidence (searched: "credential" / "secret" / "token" / "auth" — found: ...).

## 7. Resource management [A5] ⭐ DEEP
**ANSWER ALL A5.1-A5.8 questions.**
If "no resource abstraction" — explicit statement + grep evidence.

## 8. Resilience [A6, A18]
Retry / CB / timeout / bulkhead, error classification, hedging.

## 9. Expression / data routing [A7]
DSL existence + syntax, type inference, sandbox.

## 10. Plugin / extension system [A11] ⭐ DEEP — TWO sub-sections
### 10.A — Plugin BUILD process
**ANSWER A11.1-A11.4. Cite manifest path, build script path.**

### 10.B — Plugin EXECUTION sandbox
**ANSWER A11.5-A11.9. Cite loader path, runtime path.**

## 11. Trigger / event model [A12] ⭐ DEEP
**ANSWER ALL A12.1-A12.8.**
Webhook / schedule / external / polling — each separately.

## 12. Multi-tenancy [A14]
Tenant isolation, RBAC, SSO, SCIM.

## 13. Observability [A15]
Tracing framework, metrics, granularity.

## 14. API surface [A16]
Programmatic API, network API, versioning.

## 15. Testing infrastructure [A19]
Unit density, integration patterns, public testing utils.

## 16. AI / LLM integration [A21] ⭐ DEEP NEW
**ANSWER ALL A21.1-A21.13.**
If "no AI integration" — explicit grep evidence (searched: "openai", "anthropic", "llm", "embedding", "completion" — found: ... or empty).

## 17. Notable design decisions
3-7 architectural decisions with trade-offs and applicability to Nebula.

## 18. Known limitations / pain points
From issues, discussions, blog posts, CHANGELOG breaking changes. Cite issue numbers + reaction count + URL.

## 19. Bus factor / sustainability
Maintainer count, commit cadence, issues ratio, last release age.

## 20. Final scorecard vs Nebula

| Axis | Their approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|---------------|-----------------|---------------------------------------|---------|
| A1 Workspace | ... | 26 crates layered | ... | yes/no/refine |
| A2 DAG | ... | TypeDAG L1-L4 | ... | ... |
| A3 Action | ... | 5 action kinds, sealed traits | ... | ... |
| A4 Credential | ... | State/Material split, LiveCredential | ... | ... |
| A5 Resource | ... | 4 scope levels, ReloadOutcome | ... | ... |
| A6 Resilience | ... | retry/CB/bulkhead/timeout/hedging | ... | ... |
| A7 Expression | ... | 60+ funcs, type inference, sandbox | ... | ... |
| A8 Storage | ... | sqlx + PgPool + RLS | ... | ... |
| A9 Persistence | ... | Frontier + checkpoint + append-only | ... | ... |
| A10 Concurrency | ... | tokio + frontier scheduler | ... | ... |
| A11 Plugin BUILD | ... | WASM, plugin-v2 spec | ... | ... |
| A11 Plugin EXEC | ... | WASM sandbox + capability security | ... | ... |
| A12 Trigger | ... | TriggerAction Source→Event 2-stage | ... | ... |
| A13 Deployment | ... | 3 modes one codebase | ... | ... |
| A14 Multi-tenancy | ... | nebula-tenant schema/RLS/db | ... | ... |
| A15 Observability | ... | OpenTelemetry per execution | ... | ... |
| A16 API | ... | REST + planned GraphQL/gRPC | ... | ... |
| A17 Type safety | ... | sealed/GAT/HRTB/typestate/Validated<T> | ... | ... |
| A18 Errors | ... | nebula-error + ErrorClass | ... | ... |
| A19 Testing | ... | nebula-testing crate | ... | ... |
| A20 Governance | ... | Open core, Plugin Fund | ... | ... |
| A21 AI/LLM | ... | (none yet — generic actions + LLM plugin) | ... | ... |

22 rows total (A11 split into BUILD + EXEC).
```

### §3.5 Issues sweep

```bash
gh issue list --repo <owner>/<repo> --state open --limit 100 \
  --json number,title,reactionGroups,labels \
  --jq 'sort_by(-.reactionGroups[0].users.totalCount) | .[0:20]' > findings/<crate-name>/issues-top20-open.json

gh issue list --repo <owner>/<repo> \
  --label "design,architecture,rfc,proposal,plugin,credential,trigger,ai,llm" \
  --state all --limit 50 > findings/<crate-name>/issues-architectural.json

gh api repos/<owner>/<repo>/discussions --paginate \
  | jq '.[] | {title, category, reactions}' > findings/<crate-name>/discussions.json 2>/dev/null || echo "no discussions or API unavailable" > findings/<crate-name>/discussions.json
```

Save `findings/<crate-name>/issues-architectural.md`: top issues by reaction with summary.

### §3.6 DeepWiki augmentation

For Tier 1 — all queries; Tier 2 — queries 1, 2, 3, 4, 6, 7, 9; Tier 3 — queries 1, 4, 7, 9.

1. "What is the core trait hierarchy for actions/nodes/activities?"
2. "How is workflow state persisted and recovered after crash?"
3. "What is the credential or secret management approach?"
4. "How are plugins or extensions implemented (WASM/dynamic/static)? Where do plugins compile and where do they execute?"
5. "What concurrency primitives are used and how is `!Send` handled?"
6. "How are triggers (webhooks, schedules, external events) modeled?"
7. "Is there built-in LLM or AI agent integration? What providers and abstractions are supported?"
8. "What are the major architectural trade-offs documented in design docs?"
9. "What known limitations or planned redesigns are documented?"

Use `mcp__deepwiki__ask_question` for each. Save raw responses in `findings/<crate-name>/deepwiki-findings.md`.

### §3.7 Context7 (only if mature crate >5K downloads on crates.io)

```
mcp__context7__resolve-library-id libraryName="<crate>"
mcp__context7__get-library-docs ... topic="architecture" / "traits" / "plugins" / "credentials" / "llm"
```

---

## Per-tier scope (use the row in the scorecard appropriate to your tier)

**Tier 1 — Direct competitors:** 21 axes A1-A21 with A11 split into BUILD+EXEC = **22 scorecard rows**. ≥6K words. All 9 DeepWiki queries (or 3-fail-then-stop pattern). All deep questions for A3/A4/A5/A11/A12/A21.

**Tier 2 — Adjacent / important:** 14 priority axes A1-A12 + A21 with A11 split into BUILD+EXEC = **14 scorecard rows**. ≥3K words. DeepWiki queries 1, 2, 3, 4, 6, 7, 9 (7 queries; or 3-fail-then-stop pattern). All deep questions for A3/A4/A5/A11/A12/A21.

**Tier 3 — Reference:** 6 axes A1, A2, A3, A11 (split BUILD+EXEC), A18, A21 = **7 scorecard rows**. ≥1.5K words. DeepWiki queries 1, 4, 7, 9 (4 queries; or 3-fail-then-stop pattern). Deep questions for A3 and A21 only (full A3.1-A3.9 and A21.1-A21.13). For A11 still split BUILD+EXEC. A18 just "what error type they use".

**Special note for AI-first Tier 3 projects** (rayclaw, cloudllm, orchestral, aofctl): full A21.1-A21.13 deep questions despite light tier — their positioning is AI-first, depth required there.

## Quality gates (auto-rejection)

- Tier 1 architecture.md < 6K words → reject
- Tier 2 architecture.md < 3K words → reject
- Tier 3 architecture.md < 1.5K words → reject
- Tier 1 scorecard < 22 filled rows → reject; Tier 2 < 14 → reject; Tier 3 < 7 → reject
- Any required A3 / A4 / A5 / A11 / A12 / A21 deep question without explicit answer → reject (per tier scope above)
- "Searched and found nothing" without grep evidence in text → reject
- < 3 cited issues for Tier 1/2 with > 100 closed issues → reject (Tier 3 N/A)

---

## Operational rules

1. NO subjective adjectives without doc/code citations. Path + line number.
2. Comparison with Nebula MANDATORY in every architecture.md section.
3. Document failures: DeepWiki null result → record query + null. Repo unclonable → escalation file.
4. Russian comments OK in synthesis (orchestrator phase). Per-project architecture.md remains English.
5. Don't fabricate LOC. tokei output or "tokei failed: <err>".
6. Cite issues with link + reactionCount.
7. Negative findings mandatory with grep evidence.
8. Code citations mandatory for every Deep Question.

---

## Final deliverable for your assigned project

```
findings/<your-project>/
├── docs/                          # harvested docs
├── structure/                     # tree, cargo tomls, tokei, git-log
├── structure-summary.md           # 200-500 words
├── architecture.md                # ⭐ THE 6K+ WORD DELIVERABLE (Tier 1) or 3K+ (Tier 2) or 1.5K+ (Tier 3)
├── issues-architectural.md        # cited issues
├── issues-top20-open.json         # raw output
├── deepwiki-findings.md           # raw DeepWiki responses
└── context7-findings.md           # if applicable (mature crates only)
```

When done, write a per-worker completion file (avoids race on shared log):

```
findings/_orchestrator/completions/<your-project>.md
```

Format:
```markdown
# Completion — <project> — <tier>

- timestamp: <ISO 8601>
- word_count: <int>
- key_finding: <one-line summary>
- gaps: <axes that lacked clear evidence — be honest>
- escalations: <any blockers logged in ESCALATIONS.md, or "none">
- artifacts:
  - architecture.md: <path>
  - issues count: <int>
  - deepwiki queries: <int / 9>
```

Do NOT append to DISPATCH_LOG.md directly (the orchestrator consolidates).
