# A4 — Credentials / Secrets: Deep Cross-Project Analysis

**Strategic verdict for Nebula**: A4 is **Nebula's clearest unique advantage**. **Zero of 27 competitors have shipped a credential subsystem with comparable depth.** Most have nothing at all. The few that touch credentials have **active security gaps** (z8run vault has no `user_id`; acts has 7-line plaintext JS global; kotoba-workflow archive has plaintext API keys). Defensive moat for enterprise sales.

## Population

### Projects with comparable credential subsystem

**0/27.** None.

### Projects with partial / weak credential surface

| Project | What exists | What's missing | Security verdict |
|---------|-------------|----------------|------------------|
| **z8run** | `credentials` table, generic vault | **No `user_id` column** — any authenticated user can read any credential in multi-user deployment | **vulnerability** |
| **acts** / acts-next | 7-line `SecretsVar` JS global | no encryption, no lifecycle, no scoping, plaintext at rest | toy |
| **runtara-core** | api_key passed as plain `String` to LLM provider | no protection, no rotation | none |
| **tianshu** | api_key as plain `String` field in `LlmProviderConfig` | no zeroize, no secrecy::Secret, no rotation | none |
| **temporalio-sdk** | TLS/mTLS client certs + API key | only transport-level; **no credential management** | transport-only |
| **kotoba-workflow** (archived) | OpenAI client with plaintext api_key | code abandoned in `_archive/` | irrelevant |
| **rayclaw, cloudllm, aofctl, orchestral, etc.** | api_key in env var or config string | no credential layer | env-var-style |
| **emergent-engine** | env vars in TOML `env` table passed to subprocesses | env-var-only | env-var-style |

### Projects with confirmed-absent credential layer (verified by grep)

orka, dataflow-rs, dagx, runner_q, raftoral, fluxus, aqueducts-utils, ebi_bpmn, durable-lambda-core, deltaflow, dag_exec, rust-rule-engine, treadle, duroxide, flowlang.

Workers verified absence by grepping for `credential`, `secret`, `token`, `auth`, `oauth`, `password`, `zeroize`, `secrecy`, `vault`, `keyring` and found **zero matches** in source code.

## Cross-cutting findings

### Encryption at rest
**Not observed in any project.** No competitor encrypts credentials before persistence. z8run stores credential data in SQLite/PostgreSQL plaintext.

### Zeroize / secrecy in memory
**Not observed in any project.** No competitor uses `zeroize::Zeroize` or `secrecy::Secret<T>` wrappers to limit credential lifetime in memory. Every credential is a plain `String`.

### OAuth2 / OIDC support
**Not observed in any project as core feature.** Some projects have OAuth-named action types (workflow nodes), but these are application-level not credential-level. No OAuth2 token refresh handling, no PKCE, no scope management at the credential subsystem level.

### Live refresh / blue-green
**Not observed in any project.** Every project that has any credential storage assumes credentials are static. Refresh is the user's responsibility.

### Per-credential type erasure
**Not observed in any project.** All credential storage is generic ("opaque blob" or "JSON Value"). No competitor has typed credential kinds (e.g., `SlackToken` vs `DatabaseUrl` as distinct types).

### Multi-tenant credential ownership
**Not observed except as a gap (z8run no user_id).**

## Verdict for Nebula's strategy

### Position vs industry

Nebula's credential subsystem (State/Material split + LiveCredential + blue-green refresh + OAuth2Protocol blanket adapter + DynAdapter type erasure + nebula-tenant integration) is **uniquely deep in the entire competitive set**. This is not "Nebula slightly ahead" — it's "Nebula has one and the field doesn't."

### Strategic implications

1. **This is the defensive moat for enterprise sales.** Selling Nebula into regulated industries (finance, healthcare, government) requires:
   - Encryption at rest (Nebula has)
   - In-memory protection / Zeroize (Nebula has)
   - Refresh / rotation (Nebula has via LiveCredential)
   - OAuth2 with proper scope/refresh handling (Nebula has via OAuth2Protocol)
   - Audit trail of credential access (verify Nebula has — likely needs the on_credential_refresh hook to emit events)
   - Multi-tenant ownership / RBAC (Nebula has via nebula-tenant)
   
   No competitor offers this combination. SOC 2, HIPAA, PCI-DSS conversations become possible.

2. **The credential subsystem is an underused marketing angle.** Nebula's docs likely undersell this differentiator. Competitive collateral should emphasize:
   - "Most Rust workflow tools have no credential layer at all"
   - "z8run's SQL vault has a known security gap (no `user_id` in credentials table)"
   - "Nebula's credentials never leave the encrypted form except in `LiveCredential::material()` accessor with explicit scope"
   
   This is true and verifiable.

3. **Don't dilute by half-measures.** The temptation to add a "lite credential" surface for desktop mode (no encryption, just env vars) would erode the moat. Better to require the full subsystem in all modes — the implementation cost is one-time; the marketing cost of "Nebula encrypts credentials except in desktop mode" is permanent.

4. **Audit logging gap to verify.** Nebula has on_credential_refresh hook per resource — does the engine emit audit events for credential **read** operations (not just refresh)? If not, this is a gap to close before SOC 2 attempts. Estimated effort: 1-2 weeks (extend audit log to cover credential reads).

5. **Consider a "vault" interop story.** HashiCorp Vault, AWS Secrets Manager, Azure Key Vault, GCP Secret Manager are real vault systems. Nebula could position its credential subsystem as either:
   - **Replacement** (Nebula stores credentials directly) — current default
   - **Cache + refresh** (Nebula references vault entries, refreshes via LiveCredential) — value-add for orgs with existing vault investments
   
   The cache+refresh story would make Nebula adoptable in environments where credentials must live in a corporate vault. Estimated effort: 2-4 weeks for first vault adapter (Vault probably).

### What NOT to do

- **Don't expose credential plaintext in any user-facing API**, including admin/debug. The State/Material split design exists specifically to prevent this. Resist the temptation to add a `GET /credentials/{id}` that returns the plaintext for "convenience" — every competitor that has a credential surface fails this test.
- **Don't add "store this token" without a credential kind.** Generic "secret" storage is what acts has (and it's a toy). The typed credential kinds (SlackToken vs DatabaseUrl) are a real differentiator — keep enforcing them.
- **Don't downgrade encryption per deployment mode.** Keep AES-256-GCM (or ChaCha20-Poly1305) in all 3 modes. Desktop mode users still expect their stored OAuth tokens to be encrypted on disk.

## Borrow candidates

**None.** This is a one-way axis: Nebula's design is the reference, competitors have nothing to borrow back. The strongest competitor data point is z8run's vulnerability (no `user_id`) — a confirmation that "doing credentials right" is non-trivial.

## Marketing copy (suggested, verify with security-lead before using)

> "Of 27 Rust workflow / orchestration libraries surveyed in 2026, **none** ship a comparable credential subsystem. Most have no credential layer at all. The few that do — z8run, acts — have documented security gaps (z8run vault has no per-user ownership; acts uses a 7-line plaintext JavaScript global). Nebula's State/Material split, blue-green refresh, OAuth2 with scope/refresh handling, and per-tenant credential ownership are uniquely deep in this competitive set."
