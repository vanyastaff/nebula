# Spec 15 — Delivery semantics and marketing language

> **Status:** draft
> **Canon target:** §9.6 (new), §4.5.1 (new), §11.3 (extend)
> **Depends on:** 09 (retry), 11 (triggers), 14 (stateful idempotency)
> **Depended on by:** product marketing site, public docs, customer SLAs

## Problem

Delivery semantics is the one area where honesty matters most and marketing lies most. Every serious distributed systems blog (Tyler Treat, Jay Kreps, Martin Kleppmann) has written «no, you can't have exactly-once». Yet every major player markets some form of it:

- **Temporal:** «exactly-once activity execution» — true in a narrow sense, misleading in practice
- **Kafka:** «exactly-once semantics» — true within Kafka topics only, falls apart at boundaries
- **AWS Step Functions:** correctly documents «at-least-once» — notable counterexample
- **Stripe docs:** explicit warning against «exactly once» claims, names Temporal and Kafka as examples

**Nebula has accumulated every primitive needed to make honest guarantees:** durable control queue (§12.2), idempotency keys per attempt (§11.3), retry cascade (spec 09), trigger dedup (spec 11), cancel cascade (spec 08). Now we need to **publicly commit** to what they mean together.

This spec is almost pure documentation. It codifies what we can say in marketing, docs, SLAs, and canon.

## Decision

**Four explicit guarantees**, each verifiable against the built primitives. **Forbidden marketing words list** with authorized alternatives. **Two-sided idempotency contract** documented as author-engine responsibility split.

## The four guarantees

### Guarantee 1 — Trigger ingestion: at-least-once with built-in dedup

> **Any trigger event we accept will result in at least one workflow execution attempt, unless explicitly cancelled or quota-rejected before dispatch. Duplicate events from the source are deduplicated via the `trigger_events` inbox table keyed by author-configured event identity.**

**What this means:**

- Webhook sender retries → we dedup via unique constraint on `(trigger_id, event_id)` — spec 11
- Event id comes from the source (e.g., GitHub `X-GitHub-Delivery`), not generated randomly
- Sources without reliable id fall back to body hash; authors are warned
- Accepted events are persisted before returning 202 to sender — we don't drop on the path between receive and store
- Sources retrying the same event don't cause double-processing

**What this does NOT mean:**

- Events we **rejected** (auth failed, quota exceeded, bad payload) are not persisted — sender sees 401/429/400 and retries. No automatic dead-letter for rejected events in v1.
- Events from sources that fire-and-forget without retry can still be lost if our ingestion is down — no magic.
- Extremely delayed duplicates (after retention period, default 30 days) will re-process. Operationally rare.

**Verifiable by tests:**
- Webhook replay → dedup prevents second execution
- Crash between receive and `trigger_events` insert → sender retries → no duplication after recovery

### Guarantee 2 — Node dispatch: at-least-once execution with stable per-attempt key

> **Once a node attempt is started, its `idempotency_key = {execution_id}:{logical_node_id}:{attempt}` is registered in storage before side effects. The engine never runs two attempts concurrently with the same key. A single attempt may be re-dispatched across process restarts if the previous process crashed before marking the attempt terminal — in that case the attempt retains the same idempotency key, so external systems using that key deduplicate.**

**What this means:**

- Each node attempt gets a stable key, recorded in `execution_nodes.idempotency_key`
- Worker crash during attempt → next worker resumes same attempt (via lease takeover) with same key
- External system can deduplicate based on the key (if it supports idempotency keys — Stripe, AWS, most modern APIs)
- Retries are **new attempts with new keys** (attempt counter increments)
- Engine never schedules two concurrent instances of same (execution_id, node_id, attempt)

**What this does NOT mean:**

- Side effects performed by the action — that's external system's job
- Guarantees about how many times action code actually executes physically (could be 1+ due to crashes)

### Guarantee 3 — Side effects: effectively-once when idempotency contract is honored

> **Nebula does not guarantee exactly-once external side effects. The engine provides stable idempotency keys; authors must propagate them to external systems that support deduplication. Where external systems support idempotency keys (Stripe, AWS APIs, most modern SaaS), end-to-end behavior is effectively-once. Where they do not, duplicate side effects are possible under partial failure, and authors must implement application-level reconciliation.**

**The key phrase: «effectively-once when idempotency contract is honored».**

**Two-sided contract:**

| Side | Responsibility |
|---|---|
| **Engine** | Provide stable `idempotency_key` per attempt. Provide `iteration_idempotency_key` per stateful iteration. Persist key before side effect. Never run two attempts with same key concurrently. |
| **Author** | Propagate key to external systems. Use external system's `Idempotency-Key` header (or equivalent). For systems without such support, implement reconciliation (query external state to verify side effect). Document per-action contract. |

**Example — Stripe (external dedup supported):**

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    let idempotency_key = ctx.idempotency_key();  // engine-provided
    
    let charge = stripe_client
        .charges()
        .create_with_idempotency(&idempotency_key, &input.charge_request)
        .await?;
    
    Ok(Output { charge_id: charge.id })
}
```

On retry: same key → Stripe returns cached result → no double charge.

**Example — legacy API (no dedup support):**

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    // First, check if this work is already done (reconciliation query)
    let check_url = format!("{}/status?order_id={}", input.base_url, input.order_id);
    let status = http_client.get(&check_url).send().await?;
    
    if status.body.contains("processed") {
        // Already done, return cached result
        return Ok(Output::already_processed(input.order_id));
    }
    
    // Not done yet, perform the side effect
    let result = http_client.post(&input.action_url).send().await?;
    Ok(Output::from(result))
}
```

Author pattern: «check then act». Each retry checks external state before acting. Works for systems without native idempotency but requires author discipline.

**Marketing implication:**

- «Effectively-once with Stripe / AWS / modern SaaS» — ✅ true, use in marketing
- «Guaranteed exactly-once for any integration» — ❌ lie, never in marketing

### Guarantee 4 — Cancellation: eventually-terminated within bounded grace

> **A cancel request will result in the execution reaching terminal state (`Cancelled` or `CancelledEscalated`) within bounded time, where «bounded» is the sum of layer grace periods from spec 08: up to ~2 minutes for graceful cascade, with hard escalation beyond. The engine does not promise that side effects in-flight at cancel time are not committed externally — that depends on whether the action's external calls were already past the point of no return.**

**What this means:**

- Cancel **will** eventually terminate the execution — no stuck «Cancelling» forever
- Graceful path runs action's cleanup (within ~30s node grace)
- Escalation path force-kills (within ~60s process grace)
- Operators can use `POST .../terminate` for immediate kill (bypass grace)

**What this does NOT mean:**

- In-flight HTTP call may have reached the server before cancel
- Server may or may not have processed it — action doesn't know
- External state may be inconsistent with Nebula's «Cancelled» state
- Reconciliation is operator's responsibility (or author's, via compensating actions)

**Marketing implication:**

- «Graceful cancellation with bounded grace periods» — ✅
- «Guaranteed no side effect on cancel» — ❌

## Marketing language rules

### Forbidden

These phrases are **banned** from marketing, documentation, and public messaging:

- ❌ **«exactly-once»** — anything. «Exactly-once delivery», «exactly-once execution», «exactly-once side effects». All lies.
- ❌ **«guaranteed no duplicates»** — depends on external system
- ❌ **«never lose data»** — work between checkpoints can be lost (see §11.5 durability matrix)
- ❌ **«100% reliable»** — meaningless
- ❌ **«unbreakable»** — meaningless
- ❌ **«zero-loss»** — same problem as «never lose data»
- ❌ **Vague superiority claims** like «better than Temporal» — invites technical challenge, weakens brand

### Allowed and recommended

These are the **authorized** phrases for marketing, docs, and positioning:

- ✅ **«Durable execution»** — state persists across restarts
- ✅ **«At-least-once delivery with automatic deduplication»** — honest and accurate
- ✅ **«Effectively-once with idempotent APIs»** — truthful with correct contract
- ✅ **«Automatic retry with idempotency keys»** — true per spec 09
- ✅ **«Graceful cancellation with bounded grace periods»** — true per spec 08
- ✅ **«Workflow state survives crashes»** — via checkpoint policy, spec 14
- ✅ **«Type-safe integration contracts»** — our real differentiator vs n8n
- ✅ **«Honest retry semantics»** — emphasize clarity over hype

### Competitive positioning

When positioning against specific competitors, use **concrete technical differences**, not hand-waving:

- vs **n8n:** «Rust-native typed integration contracts, persisted retry accounting, no vm2 sandbox CVEs, honest delivery semantics»
- vs **Temporal:** «No workflow replay determinism constraints, simpler operational model, single-binary self-host»
- vs **Airflow:** «Execution state pinned per run, no DAG re-parse drift, honest retry budgets»
- vs **Zapier/Make:** «Self-host, typed integrations, developer-first»

## Two-sided idempotency contract (canon §11.3 extension)

### What the engine provides

```rust
impl ActionContext {
    /// Stable idempotency key for this attempt.
    /// Format: {execution_id}:{logical_node_id}:{attempt}
    /// Changes between retries (new attempt = new key).
    pub fn idempotency_key(&self) -> String;
}

impl StatefulContext {
    /// Stable idempotency key for current stateful iteration.
    /// Format: {execution_id}:{logical_node_id}:{attempt}:iter:{iteration_count}
    /// Changes between iterations AND between retries.
    pub fn iteration_idempotency_key(&self) -> String;
}
```

- Key is **persisted** before side effect (via `execution_nodes.idempotency_key` unique constraint)
- Engine **never** runs two concurrent attempts with same key (unique constraint enforces)
- Key is **stable across process restart** within same attempt (lease takeover uses same attempt row)

### What the author must do

1. **Use the engine-provided key** when calling external systems that support idempotency
2. **For systems without idempotency support**, implement reconciliation pattern:
   - Query external state to check if side effect is already done
   - Only perform new action if state indicates not done
3. **Document per-action** whether idempotency is guaranteed, best-effort, or not supported
4. **Use business-level keys** when multiple retries should dedup (e.g., «charge for order_X» regardless of retry)
5. **Use per-attempt keys** when retries should create fresh operations (e.g., «ping healthcheck N»)

### What we do NOT do

- Engine does NOT verify author uses the key correctly (can't statically check external API calls)
- Engine does NOT dedup side effects itself — that's external system's job
- Engine does NOT rollback partial side effects on cancel or failure

## Canon §9.6 — new section text

Proposed wording for canon fold-in:

```markdown
### 9.6 Delivery semantics

Nebula provides four explicit guarantees, collectively defining what authors 
and operators can trust about message and execution delivery:

**1. Trigger ingestion — at-least-once with built-in dedup.**

Accepted trigger events will result in at least one workflow execution attempt. 
Duplicates are prevented by the `trigger_events` inbox table with unique 
constraint on `(trigger_id, event_id)`. Event identity comes from the source 
where available; fallback is body hash. Rejected events are not persisted.

**2. Node dispatch — at-least-once execution with stable idempotency key.**

Each node attempt registers `idempotency_key = {execution_id}:{node_id}:{attempt}` 
in storage before side effects. The engine never concurrently runs two instances 
with the same key. Worker crashes mid-attempt result in lease takeover by 
another worker, which reuses the same attempt row and same key. Retries are 
new attempts with new keys (see §11.2).

**3. Side effects — effectively-once when idempotency contract is honored.**

The engine provides stable idempotency keys; authors propagate them to external 
systems that support deduplication. For Stripe, AWS, most modern SaaS, this 
results in effectively-once end-to-end behavior. For systems without idempotency 
support, authors must implement reconciliation (query external state to check 
if side effect is already done). Nebula does not claim exactly-once for side 
effects — see §4.5.1 for marketing language rules.

**4. Cancellation — eventually-terminated within bounded grace.**

Cancel requests reach terminal state within bounded time (grace waterfall 
defined in §12.2, typically up to ~2 minutes graceful + escalation). In-flight 
external calls may commit before cancel reaches the action; reconciliation of 
such state is the author's responsibility.

**Anti-pattern: false capability.**

Any change that would claim behavior beyond these four guarantees requires 
updating this section deliberately, not by quiet documentation drift. If a 
prototype implementation is not yet honoring a guarantee end-to-end, the 
feature is a §4.5 false capability and must be hidden or narrowed until 
implementation matches.
```

## Canon §4.5.1 — new section text

```markdown
### 4.5.1 Marketing language

Public-facing marketing, documentation, and product messaging MUST match 
the delivery semantics defined in §9.6.

**Forbidden words and phrases:**

- «exactly-once» in any form
- «guaranteed no duplicates»
- «never lose data»
- «100% reliable»
- «zero-loss»
- Vague competitive superiority without concrete technical differences

**Authorized phrases:**

- «durable execution»
- «at-least-once delivery with automatic deduplication»
- «effectively-once with idempotent APIs»
- «automatic retry with idempotency keys»
- «graceful cancellation with bounded grace periods»
- «type-safe integration contracts»

**Rationale:** Dishonest marketing that over-claims gets disproved by 
customer war stories and attracts lasting reputation damage. Temporal and 
Kafka both suffer from this — serious distributed systems blog posts 
(Kreps, Kleppmann, Treat) publicly call out their «exactly-once» claims as 
misleading. Nebula chooses honest positioning as a brand differentiator.

**Enforcement:**
- Marketing website copy review checks against this list
- Docs PRs include reviewer check
- Anyone proposing a new guarantee must first prove implementation, then 
  update §9.6, then update marketing — in that order
```

## Public-facing FAQ (for docs site)

Draft answers that can be published as-is:

**Q: Does Nebula guarantee exactly-once delivery?**

A: No. Nothing does, at any meaningful layer. Nebula provides at-least-once delivery with automatic deduplication at ingestion, plus stable idempotency keys that you can propagate to external systems to achieve **effectively-once** behavior end-to-end. If your external system (Stripe, AWS, most modern SaaS) supports idempotency keys, your actions will not cause double side effects on retry.

**Q: What happens if my action crashes mid-way?**

A: If it was a transient crash (process died, network blip), another worker picks it up and resumes from the last checkpoint. The same idempotency key is reused, so external systems that honor idempotency keys return cached results instead of re-executing. Work since the last checkpoint is re-executed; idempotency keys prevent double side effects.

**Q: What if my retry creates a duplicate in the external system?**

A: This happens only when the external system does not support idempotency keys. Nebula cannot prevent duplicates in that case — it's physically impossible without cooperation from the external system. You have three options:
1. Use a system that supports idempotency keys (recommended)
2. Implement a reconciliation check in your action (query state before acting)
3. Accept duplicates and deduplicate downstream (e.g., unique constraint in your own DB)

**Q: I cancelled an execution but my external system still received the call. Is that a bug?**

A: No, that's the fundamental limit of distributed cancellation. The cancel signal races with the in-flight HTTP call. If the server received the call before cancel fired, it may have processed it. Nebula guarantees the execution reaches Cancelled state within bounded time, but not that external state rolls back. If you need rollback, design your workflow with explicit compensation actions.

**Q: Is my data safe if the process crashes?**

A: Durable state (execution rows, node attempts, journal, control queue, checkpointed stateful state) survives any crash — it's in Postgres, acked before continuing. Work between checkpoints (within a stateful action) may be re-executed on restart; idempotency keys prevent double side effects. Non-persistent state (rate limiter counters, in-memory caches) resets on restart — by design, these aren't authoritative.

## Tests that verify guarantees

Beyond unit / integration tests of individual components, these are **acceptance tests** for the delivery guarantees:

**Guarantee 1 test:**

```rust
#[test]
fn webhook_dedup_under_retry_storm() {
    // Send same webhook (same event_id) 100 times in parallel
    // Assert: exactly 1 execution created
    // Assert: other 99 responses say "accepted" but "deduplicated"
}

#[test]
fn webhook_received_and_stored_before_response() {
    // Kill process immediately after sender receives 202
    // Recover, verify event is in trigger_events (was flushed before response)
    // Worker picks it up and runs
}
```

**Guarantee 2 test:**

```rust
#[test]
fn attempt_reuses_same_key_across_worker_crash() {
    // Start execution, let attempt 1 start
    // Kill worker mid-attempt
    // Wait for lease expiration
    // Assert: new worker picks up same attempt row with same idempotency_key
    // Assert: external system (mocked) sees same key twice but dedups (returns cached)
}
```

**Guarantee 3 test:**

```rust
#[test]
fn effectively_once_with_stripe_mock() {
    // Start execution, action calls mock Stripe with idempotency_key
    // Force retry (action returns Transient)
    // Assert: mock Stripe received 2 requests with same key
    // Assert: mock Stripe returned cached result on 2nd
    // Assert: no double charge in ledger
}
```

**Guarantee 4 test:**

```rust
#[test]
fn cancel_eventually_terminates_within_grace() {
    // Start execution with action that sleeps 5 minutes
    // Send cancel
    // Assert: execution reaches Cancelled within 5 seconds (cooperative path)
    
    // Start execution with action that ignores cancellation
    // Send cancel
    // Assert: execution reaches CancelledEscalated within 60 seconds (grace + escalation)
}
```

## Open questions

- **Formal SLA document** — customers may ask for an SLA with specific uptimes, guarantees, credits. Draft version based on guarantees 1–4. Deferred to when first customer asks.
- **Compliance certifications** — SOC2, ISO 27001, HIPAA. Delivery semantics is one input; needs legal and audit review. Deferred until enterprise customer asks.
- **Cross-workflow transactional boundaries** — «when action A commits, also trigger workflow B in a single transaction». Not in v1, probably never — anti-pattern for distributed systems.
- **Formal model checking** — TLA+ or similar to prove guarantees hold under all schedules. Interesting for academic rigor, deferred. Tests + careful code review are sufficient for v1.
- **Fuzz testing delivery guarantees** — chaos engineering tests that randomly crash workers, drop packets, delay storage. Valuable, deferred to dedicated test infrastructure work.
