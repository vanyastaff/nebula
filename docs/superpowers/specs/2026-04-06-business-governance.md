# Nebula — Business & Governance Decisions

> From investor/business panel (Round 11). Items that are NOT technical spec changes but critical for project viability.

---

## 1. Commercial Model (a16z feedback)

**Open-core:**
- **Open source (MIT/Apache-2.0):** Engine, all crates, credential security, RLS, sandbox
- **Commercial (future):** Managed cloud, visual editor SaaS, enterprise admin console, SSO/RBAC, premium support with SLAs, compliance attestation

## 2. LTS Policy (Enterprise Japan feedback)

**Proposal:**
- Pre-1.0: no stability guarantee. Breaking changes in any release.
- 1.0 release: 2-year LTS. Security patches backported.
- Breaking changes: major versions only (1.0 → 2.0). 6-month deprecation cycle.
- `schema_version` on WorkflowDefinition ensures stored workflows remain readable.
- Migration tools shipped with every major version.

## 3. API Stability Commitment (Cloud Platform feedback)

**Stable surface in 1.0:**
- `WorkflowDefinition` JSON schema
- `Action`, `StatelessAction`, `StatefulAction`, `TriggerAction` trait signatures
- `ActionContext` method signatures
- `ActionResult<T>` enum variants (additive only via `#[non_exhaustive]`)
- `ParameterCollection`, `ParameterValues` types
- `CredentialStore`, `Storage` trait signatures

**Unstable (may change in minor versions):**
- Internal handler/adapter layer
- Serialization format (internal, versioned)
- Derive macro generated code (output, not trait impls)
- CLI flags and configuration format

## 4. Compliance Roadmap (Enterprise Japan + Fintech feedback)

| Standard | Status | Path |
|----------|--------|------|
| SOC2 CC6.1 (encryption) | Partial — credentials encrypted | Complete: encrypt execution state too |
| SOC2 CC6.3 (access control) | Partial — OwnerId + RLS | Complete: RBAC in nebula-auth |
| PCI-DSS (card data) | GAP — node outputs unencrypted | Fix: EncryptionLayer on execution state |
| ISO 27001 | Not started | Requires organizational controls, not just code |
| HIPAA | Not started | Requires BAA + encryption + audit |

## 5. Execution State Encryption (Fintech PCI gap)

**Problem:** Execution state (node outputs) stored as unencrypted MessagePack BYTEA. May contain sensitive data (card tokens, PII).

**Fix:** Apply same `EncryptionLayer` pattern used for credentials to execution state storage. Per-execution encryption key derived from master key + execution_id. Transparent to engine — handled at storage layer.

```rust
pub struct EncryptedExecutionStorage<S: ExecutionRepo> {
    inner: S,
    key: Arc<EncryptionKey>,
}

impl<S: ExecutionRepo> ExecutionRepo for EncryptedExecutionStorage<S> {
    async fn save_state(&self, state: &ExecutionState) -> Result<()> {
        let plaintext = Zeroizing::new(rmp_serde::to_vec(state)?);
        let encrypted = encrypt(&self.key, &plaintext, state.execution_id.as_bytes())?;
        self.inner.save_raw(state.execution_id, &encrypted).await
    }
}
```

## 6. Multi-Language Plugin SDK (a16z community feedback)

**Priority path to ecosystem growth:**
1. v1: Rust-only plugins (compiled into binary)
2. v1.1: C FFI (`libnebulaengine.so`) for Go/Python/Node embedding
3. v2: WASM plugin loading (Python/TS/Go compiled to WASM)
4. v3: Native language SDKs (Python SDK, TypeScript SDK) wrapping gRPC API

## 7. n8n Migration Tool (CTO feedback)

**v1.1 deliverable:** `nebula-migrate-n8n` CLI tool that reads n8n workflow JSON export and produces Nebula WorkflowDefinition JSON. Maps INodeType → ActionKey. Manual review required for complex nodes. Reduces 6-week migration to 2 weeks.
