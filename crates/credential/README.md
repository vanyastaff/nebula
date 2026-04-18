# nebula-credential

Universal credential management for workflow automation.

## Overview

Secure, extensible credential system with 12 universal auth scheme types, an open `AuthScheme` trait, composable storage layers, AES-256-GCM encryption with key rotation, and derive macros for low-boilerplate credential definitions.

## Key Features

- **12 Universal Auth Schemes** — SecretToken, IdentityPassword, OAuth2Token, KeyPair, Certificate, SigningKey, FederatedAssertion, ChallengeSecret, OtpSeed, ConnectionUri, InstanceBinding, SharedKey
- **Open `AuthScheme` Trait** — plugins add custom schemes via `#[derive(AuthScheme)]`
- **Unified `Credential` Trait** — resolve, refresh, test, revoke in one trait
- **Composable Storage** — EncryptionLayer, CacheLayer, AuditLayer, ScopeLayer
- **Encryption Key Rotation** — multi-key support with lazy re-encryption on read
- **AAD Enforcement** — credential ID bound as additional authenticated data, no legacy fallback
- **Interactive Flows** — OAuth2 with PKCE, device code, multi-step resolve
- **Derive Macros** — `#[derive(Credential)]` and `#[derive(AuthScheme)]`

## Quick Start

```rust,ignore
use nebula_credential::{Credential, scheme::SecretToken, SecretString};

// Static credentials use identity projection (State = Scheme)
struct ApiKeyCredential;

impl Credential for ApiKeyCredential {
    type Scheme = SecretToken;
    type State = SecretToken;
    type Pending = nebula_credential::NoPendingState;

    const KEY: &'static str = "api_key";

    // implement resolve(), project(), description(), parameters()
}
```

## Architecture

```
nebula-credential/
├── src/
│   ├── scheme/         # 12 universal auth scheme types
│   ├── credentials/    # Built-in credential impls (ApiKey, BasicAuth, OAuth2)
│   ├── layer/          # Composable storage layers
│   ├── rotation/       # Credential rotation (feature-gated)
│   ├── crypto.rs       # AES-256-GCM, key derivation, PKCE
│   ├── credential.rs   # Unified Credential trait
│   ├── store.rs        # CredentialStore trait
│   ├── resolver.rs     # Runtime resolution engine
│   └── registry.rs     # Type-erased dispatch
├── macros/             # #[derive(Credential)], #[derive(AuthScheme)]
└── tests/
```

## Security

- AES-256-GCM encryption at rest with Argon2id key derivation
- `SecretString` with automatic zeroization on drop
- `Zeroizing<Vec<u8>>` for all intermediate plaintext buffers
- `Debug` impls redact all secret fields
- `#![forbid(unsafe_code)]`

## License

Licensed under the same terms as the Nebula project.

### Authenticated encryption details (evicted from PRODUCT_CANON.md §12.5)

Credentials at rest are encrypted with **AES-256-GCM** using **Argon2id** as the key derivation function. The credential ID is bound as additional authenticated data (AAD), ensuring ciphertext is tied to the specific credential record — no legacy fallback without AAD. Key rotation is supported via multi-key storage with lazy re-encryption on read.

Specific algorithm/KDF/parameters: see `src/crypto.rs` for the authoritative implementation. These choices are L4 implementation detail — changing the algorithm or parameters requires updating this README and `src/crypto.rs`; no canon revision needed. The L2 invariant ("encryption at rest uses authenticated encryption; do not bypass for debugging") lives in canon §12.5.
