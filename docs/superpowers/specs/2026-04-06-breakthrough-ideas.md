# Nebula — Breakthrough Ideas from Conference Lightning Talks

> Collected from Round 7: engineers presenting specific technologies from their production stacks.

---

## 1. Inline Caching for Expression Evaluation (Google V8)

**What:** Cache resolved paths in expression evaluation. After first `$node.output.data.name` access, subsequent accesses are a single pointer compare + dereference. No HashMap walk, no clone.

**Impact:** 10-50x speedup on repeated expressions in loop/template nodes.

**When:** v1.1 (expression crate)

**Key insight:** Within one execution, node outputs are append-only. Once an Arc<Value> exists for a node, its identity (pointer) never changes. Use pointer identity as cache key.

---

## 2. Arena-Based Node Output Allocation (Cap'n Proto/Cloudflare)

**What:** Per-execution `bumpalo` arena for all node outputs. No individual heap allocations, no fragmentation, instant bulk free on execution completion.

**Impact:** Eliminates per-node alloc/free overhead. Better cache locality.

**When:** v2 (requires engine refactor for lifetime management)

**Key insight:** Node outputs have execution-scoped lifetime. Arena allocation matches this naturally.

---

## 3. Deterministic Simulation Testing (TigerBeetle)

**What:** Abstract async runtime behind a `RuntimeEnv` trait. Swap in a deterministic simulator that controls scheduling order via seeded RNG. Reproduce any interleaving bug with a seed number.

**Impact:** Catch concurrency bugs that tokio::test cannot find. Inject faults deterministically.

**When:** v1.1 (start with `RuntimeEnv` trait, simulation test framework)

**Key insight:** Thread `RuntimeEnv` into `ActionRuntime` and `Engine`. Replace `Instant::now()` and `tokio::spawn()` with trait methods. Production = TokioEnv. Tests = SimEnv with fault injection.

---

## 4. AIMD Adaptive Rate Limiting (Netflix Zuul)

**What:** Replace static rate limits with AIMD (Additive Increase Multiplicative Decrease). Auto-discover external API rate limits: increase linearly on success, halve on 429/5xx.

**Impact:** Zero-config rate limiting per `(tenant, provider, route)`. Converges to optimal rate in minutes.

**When:** v1 — nebula-resilience already has `AdaptiveRateLimiter` with proportional adjustment. Replace with true AIMD.

**Key insight:** Current 0.9/1.1 multiplicative factors oscillate. AIMD's asymmetry (slow up, fast down) is what makes TCP stable.

---

## 5. Consistent Hashing for Trigger Affinity (Discord)

**What:** Hash ring assigns triggers to workers. WebSocket connections, cached state, rate limit buckets stay co-located. Worker failure redistributes only 1/N triggers.

**When:** v2 (distributed runtime)

---

## 6. Vectorized Batch Expression Evaluation (Databricks Photon)

**What:** `eval_batch(param, body, &[Value], context)` processes N items with one context clone instead of N. Eliminates 9,999 context clones for 10K-item array.

**Impact:** 3-5x on array-heavy workflows (batch API responses, data transforms).

**When:** v1.1 (expression crate, eval.rs)

**Key insight:** Reuse one mutable context for all iterations. Set lambda binding via `set_execution_var` without cloning.

---

## 7. USDT Probes for Zero-Overhead Tracing (Oxide Computer)

**What:** Compile-time NOPs that become DTrace/BPF tracepoints when enabled. Instrument node execution, credential resolution, resource acquisition. Zero overhead when disabled.

**When:** v1 — low effort, high value.

```rust
dtrace_provider!("nebula", {
    fn action__entry(execution_id: &str, node_id: &str, action_key: &str) {}
    fn action__return(execution_id: &str, node_id: &str, latency_us: u64, ok: u8) {}
    fn credential__resolve(credential_key: &str, scheme: &str, latency_us: u64) {}
});
```

**Key insight:** In production, attach to ONE execution by filtering on execution_id. No impact on others.

---

## 8. Embedded libSQL for Local-First Storage (Turso)

**What:** Replace Postgres/SQLite with libSQL for desktop/embedded use. Built-in sync replication, encryption at rest, SQLite-compatible API.

**Impact:** Desktop app runs offline, syncs when online. No Docker, no Postgres, no migrations server.

**When:** v1.1 (Storage trait already abstracts backends)

```rust
impl Storage for LibSqlStorage {
    // Same trait, embedded implementation
    // Optionally syncs to Turso cloud
}
```

---

## 9. Firecracker MicroVM Sandbox (Fly.io)

**What:** Run untrusted actions in Firecracker microVMs — full Linux kernel, 125ms boot, 5MB overhead. Communication via vsock (no network stack).

**When:** Phase 3 (action isolation)

**Key insight:** WASM can't run native libraries. MicroVMs run anything. `SandboxRunner` trait already fits.

---

## 10. Standard Tool Use Protocol for AgentAction (Anthropic)

**What:** AgentContext::invoke_tool() emits tool_use/tool_result blocks compatible with Claude, GPT, and Gemini APIs. One tool loop works across all LLM providers.

**When:** v1 when AgentAction ships (Phase 10)

**Key insight:** Anthropic, OpenAI, and Google all converged on the same tool schema shape. `ResolvedTool` → JSON Schema input_schema. No adapter per provider needed.

---

## Priority Summary

**v1 (ship now):**
- AIMD adaptive rate limiting (low effort, high impact)
- USDT probes (low effort, production debugging)

**v1.1 (next release):**
- Inline caching for expressions (10-50x on hot paths)
- Vectorized batch evaluation (3-5x on arrays)
- Deterministic simulation testing (RuntimeEnv trait)
- libSQL embedded storage (desktop/offline)

**v2 (future):**
- Arena allocation (engine refactor)
- Consistent hashing (distributed)
- Firecracker sandbox (Phase 3)
- Standard tool protocol (AgentAction Phase 10)
