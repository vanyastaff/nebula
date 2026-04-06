# Red Team Security Assessment — Findings & Mitigations

> 10 attack vectors identified by 5 security researchers against all 7 Nebula specs.

---

## Critical

### RT-5. In-process sandbox = no real isolation from plugins
**Attack:** Compiled plugin action runs in same address space. Raw pointer arithmetic bypasses `SandboxedContext` API boundary entirely.
**Status:** ACKNOWLEDGED. Phase 3 (WASM/Firecracker) is the real fix. Until then:
**Mitigation v1:** All v1 actions are first-party trusted code only. Third-party plugins require code review. `SandboxedContext` capability checks prevent ACCIDENTAL undeclared access. Documentation: "InProcessSandbox is NOT a security boundary — it is a correctness check."
**Mitigation v2:** WASM plugin loading with capability-based permissions.
**Mitigation v3:** Firecracker microVM isolation (see breakthrough ideas #9).

---

## High (4)

### RT-3. rkyv AlignedVec not zeroized on cache eviction
**Attack:** Credential cache uses rkyv `AlignedVec`. On eviction, freed memory retains plaintext credential bytes.
**Fix:** Wrap `AlignedVec` in `Zeroizing<AlignedVec>` (zeroize crate). The `Zeroizing<T>` wrapper calls `T.zeroize()` on drop. Requires `AlignedVec` to implement `Zeroize` — if it doesn't, copy to `Zeroizing<Vec<u8>>` before archiving.
**Spec change:** Credential v3 spec — add to Section 4.3: "All credential cache buffers use `Zeroizing<T>` wrapper. `AlignedVec` from rkyv copied to `Zeroizing<Vec<u8>>` for cache storage."

### RT-7. No framework-level webhook signature verification
**Attack:** Any HTTP POST to webhook endpoint is accepted. No HMAC verification at framework level.
**Fix:** Add optional `WebhookVerifier` trait to nebula-webhook:
```rust
pub trait WebhookVerifier: Send + Sync {
    fn verify(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), WebhookError>;
}
```
Built-in impls: `HmacSha256Verifier`, `TimestampVerifier`. TriggerAction declares its verifier. Framework calls verify BEFORE writing to Postgres queue.
**Spec change:** Runtime v2 — add webhook verification step before queue write.

### RT-8. EventBus rotation events unauthenticated
**Attack:** Any code with EventBus sender access can emit forged rotation events.
**Fix:** EventBus messages carry an `Origin` tag (`Origin::CredentialResolver`, `Origin::Admin`). `spawn_rotation_listener` only accepts events from known origins. Not cryptographic — in-process code can forge origins. But combined with (RT-5) it narrows the attack surface to malicious plugins which are already the threat model.
**Spec change:** Resource v2 — add origin validation to rotation event dispatch.

### RT-10. RLS session variable gap on pooled connections
**Attack:** Checkpoint writes via pooled Postgres connection may use wrong tenant's `app.current_owner`.
**Fix:** Set `app.current_owner` as FIRST statement in every transaction, not once per connection. Use `SET LOCAL` (transaction-scoped, auto-resets on commit/rollback).
```sql
SET LOCAL app.current_owner = $1;
-- then perform the actual query
```
**Spec change:** Workflow v2 W1 — specify `SET LOCAL` per-transaction, not per-connection.

---

## Medium (4)

### RT-1. Expression evaluation lacks allocation budget
**Attack:** Expression produces multi-GB intermediate allocations before step limit fires.
**Fix:** Add `max_eval_memory_bytes` to `EvaluationPolicy`. Track via allocator hooks or periodic RSS check during eval. Return `ExpressionError::MemoryBudgetExceeded` on breach.
**Spec change:** Expression spec (when written) — add memory budget alongside step limit.

### RT-2. Expression errors may leak node output content
**Attack:** Failed expression like `{{ $node.secret.output.key }}` leaks data structure in error message.
**Fix:** Expression error messages NEVER include node output VALUES — only field NAMES and types. Redact all `Value` content from error messages at the evaluator boundary.
**Spec change:** Expression spec — "Expression errors must not contain node output values."

### RT-4. TOCTOU on in-memory ScopeLayer
**Attack:** Credential owner changes between scope check and actual read.
**Fix:** Atomic check-and-read in `ScopeLayer::get()` — single operation that filters and returns. For in-memory store: hold read lock through both operations. For Postgres: RLS handles it atomically.
**Spec change:** Credential v3 — "ScopeLayer::get() must be atomic (check + read in one operation)."

### RT-6. Total execution byte budget unenforced
**Attack:** 100 pages × 9.9MB each = ~1GB, under per-node limit but over any sane total.
**Status:** ALREADY TRACKED. RT1 in runtime spec promotes `max_total_execution_bytes` enforcement to v1 blocker.

---

## Low (1)

### RT-9. Arc<RawValue> no execution-scoped cleanup
**Attack:** Leaked reference from previous execution reads stale data.
**Fix:** Execution arena (breakthrough idea #2) scopes all outputs to execution lifetime. Until then: engine drops all node output Arcs explicitly on execution completion.
**Status:** Covered by arena allocation (v2).

---

## Summary Table

| ID | Severity | Fixed by | When |
|----|----------|----------|------|
| RT-5 | Critical | WASM/Firecracker sandbox | Phase 3 (v1: trusted code only) |
| RT-3 | High | Zeroizing<T> on cache buffers | v1 |
| RT-7 | High | WebhookVerifier trait | v1 |
| RT-8 | High | EventBus origin tags | v1 |
| RT-10 | High | SET LOCAL per transaction | v1 |
| RT-1 | Medium | Expression memory budget | v1.1 |
| RT-2 | Medium | Redact values from expr errors | v1 |
| RT-4 | Medium | Atomic ScopeLayer::get() | v1 |
| RT-6 | Medium | Enforce total execution bytes | v1 (already tracked) |
| RT-9 | Low | Execution arena | v2 |
