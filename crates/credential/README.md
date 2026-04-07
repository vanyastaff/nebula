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
