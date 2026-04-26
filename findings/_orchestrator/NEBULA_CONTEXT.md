# Nebula — Comparison Baseline

For each axis your competitor's approach is compared against this. Use these one-liners as the right column of the scorecard, but verify the specifics by reading source files in the Nebula workspace at `crates/` if your competitor's design forces a non-trivial comparison.

## Project context

Nebula = Rust workflow orchestration engine, n8n+Temporal+Airflow merged. Solo dev (Vanya). 26-crate workspace. Edition 2024. Stable Rust 1.95.0 (pinned).

Three deployment modes from one codebase: **desktop / self-hosted / cloud**.

Core differentiators:
- Type-safe DAG (4 levels: static generics → TypeId → refinement predicates → petgraph)
- Credential subsystem deeper than competitors (State/Material split, LiveCredential, blue-green refresh)
- Resilience as separate crate with unified error classification
- Plugin model targets WASM sandbox + Plugin Fund commercial model
- 5 action kinds covering n8n + Temporal use cases unified

## Per-axis Nebula approach (use in scorecard right column)

| Axis | Nebula approach (one-liner — verify specifics in `crates/` if needed) |
|------|----------------------------------------------------------------------|
| A1 Workspace | 26 crates, layered: nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant / nebula-eventbus / etc. Edition 2024. |
| A2 DAG | TypeDAG: L1 = static generics enforce port types at compile time; L2 = TypeId for dynamic registration; L3 = refinement predicates (e.g., `NonEmpty<String>`); L4 = petgraph soundness checks. |
| A3 Action | 5 action kinds (Process / Supply / Trigger / Event / Schedule). Sealed trait. Associated `Input` / `Output` / `Error`. Versioning via type identity. Derive macros via nebula-derive. |
| A4 Credential | State/Material split (typed state + opaque material). CredentialOps trait. LiveCredential with watch() for blue-green refresh. OAuth2Protocol blanket adapter. DynAdapter for type erasure. |
| A5 Resource | 4 scope levels (Global / Workflow / Execution / Action). ReloadOutcome enum (Reloaded / NoChange / Failed). Generation tracking for cache invalidation. on_credential_refresh per-resource hook. |
| A6 Resilience | nebula-resilience crate: retry / circuit breaker / bulkhead / timeout / hedging. Unified ErrorClassifier categorizing transient vs permanent. |
| A7 Expression | 60+ functions, type inference, sandboxed eval. Syntax: `$nodes.foo.result.email`. JSONPath-like + computed expressions. |
| A8 Storage | sqlx + PgPool. Pg*Repo per aggregate (PgWorkflowRepo, PgExecutionRepo, etc.). SQL migrations in `migrations/`. PostgreSQL RLS for tenancy. |
| A9 Persistence | Frontier-based scheduler with checkpoint recovery. Append-only execution log. State reconstruction via replay. |
| A10 Concurrency | tokio runtime. Frontier scheduler with work-stealing semantics. `!Send` action support via thread-local sandbox isolation. |
| A11 Plugin | WASM sandbox planned (wasmtime). plugin-v2 spec doc. Plugin Fund commercial model (royalties to plugin authors). Capability-based security. |
| A12 Trigger | TriggerAction with `Input = Config` (registration) and `Output = Event` (typed payload). Source trait normalizes raw inbound (HTTP req / Kafka msg / cron tick) into Event. 2-stage. |
| A13 Deployment | 3 modes from one binary: `nebula desktop` (single-user GUI), `nebula serve` (self-hosted), cloud (managed). |
| A14 Multi-tenancy | nebula-tenant crate. Three isolation modes (schema / RLS / database). RBAC. SSO planned. SCIM planned. |
| A15 Observability | OpenTelemetry. Structured tracing per execution (one trace = one workflow run). Metrics per action (latency / count / errors). |
| A16 API | REST API now. GraphQL + gRPC planned. OpenAPI spec generated. OwnerId-aware (per-tenant). |
| A17 Type safety | Sealed traits (extern crates can't impl core kinds). GATs for resource handles. HRTBs for lifetime polymorphism. typestate (Validated/Unvalidated). Validated<T> proof tokens. |
| A18 Errors | nebula-error crate. Contextual errors. ErrorClass enum (transient / permanent / cancelled / etc.). Used by ErrorClassifier in resilience. |
| A19 Testing | nebula-testing crate. resource-author-contracts.md (contract tests for resource implementors). insta + wiremock + mockall. |
| A20 Governance | Open core. Plugin Fund (commercial model for plugin authors). Planned SOC 2 (2-3 year horizon). Solo maintainer (Vanya). |
| A21 AI/LLM | **No first-class LLM abstraction yet.** Strategic bet: AI workflows realized through generic actions + plugin LLM client. Surge (separate project) handles agent orchestration on ACP. |

## How to use this in your scorecard

Right column ("Nebula approach") = paste the one-liner above. If you need more depth (because competitor's approach challenges Nebula's), grep:

```bash
# from worktree root
grep -r "TriggerAction" crates/nebula-action/src/ | head
grep -r "LiveCredential" crates/nebula-credential/src/ | head
grep -r "ReloadOutcome" crates/nebula-resource/src/ | head
```

…and quote the actual Nebula code. Don't make up specifics.

## Verdict column ("Who's deeper / simpler / more correct")

Honest assessment — Nebula is not always deeper. Use one of:
- **Nebula deeper** (with reason)
- **Competitor deeper** (with reason)
- **Different decomposition, neither dominates** (with reason)
- **Competitor simpler, Nebula richer** (trade-off)
- **Convergent** (similar approach independently arrived at)

## Borrow column

- **yes** — adopt approach (or compatible refinement)
- **no — Nebula's already better**
- **no — different goals**
- **refine** — adopt the idea but fit Nebula's existing decomposition
- **maybe — needs ADR review**
