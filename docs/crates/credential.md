# nebula-credential

Encrypted credential storage and lifecycle management. Handles secrets from creation
through automatic refresh to revocation, with a tiered cache and distributed locking
to prevent refresh stampedes.

## System Overview

```
Actions / Services / Workers
         │
         ▼
 CredentialManager API
   get_token(id) → AccessToken
   create_credential(type, input) → CredentialId
   refresh_credential(id) → AccessToken
   revoke_credential(id) → ()
         │
         ├── Manager ──── Registry ──── ClientAuthenticator
         │                              (HTTP, gRPC, DB)
         │
         ▼
  Storage & Cache Layer
   StateStore (Postgres)   TokenCache (L1 memory / L2 Redis)
         │
         ▼
  Infrastructure
   KMS encryption   Distributed lock (Redis)   Audit log
```

## Module Structure

```
nebula-credential/src/
├── core/
│   ├── credential.rs      Credential trait
│   ├── token.rs           AccessToken type
│   ├── secure.rs          SecureString (zeroize on drop)
│   ├── ephemeral.rs       Ephemeral<T> wrapper
│   ├── state.rs           CredentialState trait
│   └── context.rs         CredentialContext
├── manager/
│   ├── manager.rs         CredentialManager
│   ├── cache.rs           Token caching
│   ├── refresh.rs         Refresh orchestration
│   ├── policy.rs          RefreshPolicy
│   └── registry.rs        Credential type registry
├── storage/
│   ├── traits.rs          StateStore trait
│   ├── postgres.rs        PostgreSQL backend
│   └── memory.rs          In-memory backend (tests)
├── cache/
│   ├── redis.rs           Redis token cache
│   ├── encrypted.rs       Encrypted cache wrapper
│   └── tiered.rs          L1/L2 tiered cache
├── lock/
│   ├── traits.rs          DistributedLock trait
│   └── redis.rs           Redis-based locks
├── credentials/           Built-in credential types
│   ├── oauth2/            OAuth 2.0 (code, device, client creds flows)
│   │   └── providers/     Google, Microsoft, GitHub, Salesforce, custom
│   ├── apikey/            API key, Bearer token
│   ├── aws/               AWS SigV4 + STS assume-role
│   ├── database/          PostgreSQL, MySQL, MongoDB
│   └── ldap/              Simple bind, SASL
├── authn/                 Client authenticators
│   ├── http.rs            Bearer / Basic
│   ├── grpc.rs            gRPC metadata
│   └── database.rs        Database connection auth
└── security/
    ├── kms.rs             KMS client trait
    ├── encryption.rs      At-rest encryption (AES-256-GCM)
    └── audit.rs           Audit logging
```

## Credential Lifecycle

```rust
// 1. Define a credential type
pub struct MyApiCredential;

impl Credential for MyApiCredential {
    type Input = MyInput;   // Initial configuration
    type State = MyState;   // Persistent state (refresh tokens, etc.)

    async fn initialize(input: Self::Input) -> Result<(Self::State, Option<AccessToken>)>;
    async fn refresh(state: &mut Self::State) -> Result<AccessToken>;
}

// 2. Register the type
manager.register_credential_type::<MyApiCredential>();

// 3. Create an instance
let id = manager.create_credential("my_api", input).await?;

// 4. Use it — auto-refreshes transparently
let token = manager.get_token(&id).await?;

// 5. Revoke
manager.revoke_credential(&id).await?;
```

## State Model

```
Persistent State (stored in StateStore / Postgres)
  • Refresh tokens
  • Client credentials
  • Provider configuration
  • Expiry metadata

Ephemeral Data (in-memory / TokenCache only)
  • Access tokens              → TokenCache (L1 + L2)
  • Temporary values           → Ephemeral<T> (zeroized on drop)
  • Metrics                    → never persisted
```

## Refresh Flow

```
get_token(id)
    │
    ▼
L1 cache hit? ──yes──► return token
    │ no
    ▼
L2 cache hit? ──yes──► return token
    │ no
    ▼
Acquire distributed lock(id)
    │
    ▼
Re-check cache (another process may have refreshed)
    │
    ├─ valid ──► return token, release lock
    │
    └─ stale
         │
         ▼
    Load state from StateStore
    Call Credential::refresh(state)
    Store new state (CAS)
    Cache access token (L1 + L2)
    Release lock
    Return AccessToken
```

The distributed lock prevents multiple processes from refreshing the same credential
simultaneously (stampede prevention).

## Security

**At-rest encryption:**
```
User input → SecureString (zeroize on drop) → KMS-encrypted bytes → StateStore
```

- `SecureString` uses `secrecy` + `zeroize`; never appears in logs (redacted `Debug`)
- Constant-time comparison via `subtle`
- Serialized as Base64 only; never raw bytes in JSON
- KMS client trait — supports AWS KMS, HashiCorp Vault, or custom backend

**Concurrency:**
- Redis distributed lock with auto-renewal for long refresh operations
- CAS (compare-and-swap) for all state updates to prevent lost writes

## Tiered Cache

```
Request
  │
  ▼
L1 — local memory   (10 s TTL)
  │ miss
  ▼
L2 — Redis          (5 min TTL cap)
  │ miss
  ▼
Refresh + distributed lock
```

## Using Credentials in Actions

```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<...> {
    // Retrieve a token; auto-refreshed if expired
    let token = ctx.credentials().get_token(&self.credential_id).await?;

    let resp = client
        .get(&self.url)
        .bearer_auth(token.as_str())
        .send()
        .await?;

    Ok(json!({ "status": resp.status().as_u16() }))
}
```
