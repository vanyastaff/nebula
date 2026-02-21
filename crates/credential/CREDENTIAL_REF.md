# CredentialRef and CredentialProvider

## Overview

Added type-safe credential references and a provider trait to `nebula-credential` for decoupled credential acquisition across the system.

## New Types

### `CredentialRef`

Type-safe reference to a credential that wraps a `TypeId`.

```rust
use nebula_credential::CredentialRef;
use std::any::TypeId;

struct GithubToken;

// Create a type-safe reference
const GITHUB: CredentialRef = CredentialRef::of::<GithubToken>();

// Check type ID
assert_eq!(GITHUB.type_id(), TypeId::of::<GithubToken>());
```

**Key Features:**
- **Pure TypeId**: Just wraps `TypeId` - no string IDs
- **Type-safe**: Ensures compile-time and runtime type checking
- **Const-friendly**: Can be used in const contexts
- **Zero-cost**: Single `TypeId` field (no overhead)
- **Implements**: `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`

**Design Philosophy:**
- No string IDs - type is the identifier
- Simple newtype wrapper around `TypeId`
- Enables both type-based and ID-based credential acquisition strategies

### `CredentialProvider`

Trait for acquiring credentials with two methods: type-safe and string-based.

```rust
pub trait CredentialProvider: Send + Sync {
    // Type-safe: acquire by credential type
    async fn credential<C: Send + 'static>(
        &self,
        ctx: &CredentialContext,
    ) -> Result<SecretString, CredentialError>;

    // Dynamic: acquire by string ID
    async fn get(
        &self,
        id: &str,
        ctx: &CredentialContext,
    ) -> Result<SecretString, CredentialError>;

    // Convenience methods
    async fn has_credential<C: Send + 'static>(&self, ctx: &CredentialContext) -> bool;
    async fn has(&self, id: &str, ctx: &CredentialContext) -> bool;
}
```

**Two Acquisition Strategies:**

1. **Type-based (preferred):**
```rust
// Define credential type
struct GithubToken;

// Acquire it
let token = provider.credential::<GithubToken>(&ctx).await?;
```

2. **ID-based (dynamic):**
```rust
let token = provider.get("github_token", &ctx).await?;
```

## Usage in Actions/Triggers

Action contexts can now hold a `CredentialProvider`:

```rust
use nebula_credential::{CredentialProvider, CredentialContext, SecretString};

pub struct ActionContext {
    credential_provider: Arc<dyn CredentialProvider>,
}

impl ActionContext {
    // Type-safe acquisition
    pub async fn get_credential<C: Send + 'static>(&self) -> Result<SecretString, CredentialError> {
        let ctx = CredentialContext::new(&self.user_id);
        self.credential_provider
            .credential::<C>(&ctx)
            .await
    }
    
    // Dynamic acquisition
    pub async fn get_by_id(&self, id: &str) -> Result<SecretString, CredentialError> {
        let ctx = CredentialContext::new(&self.user_id);
        self.credential_provider
            .get(id, &ctx)
            .await
    }
}
```

## Benefits

1. **Dual API**: Type-safe (`credential<C>()`) + dynamic (`get(id)`)
2. **No Circular Dependencies**: Clean separation between crates
3. **Type Safety**: `CredentialRef` enforces type correctness
4. **Flexibility**: Manager can map TypeId → credential or ID → credential
5. **Modern Rust**: Native async fn in traits (Rust 1.75+)
6. **Security**: All methods return `SecretString` with automatic zeroization

## Implementation Strategy

CredentialManager can support both approaches:

```rust
struct CredentialManager {
    credentials_by_id: HashMap<CredentialId, Credential>,
    credentials_by_type: HashMap<TypeId, CredentialId>, // Type → ID mapping
}

impl CredentialProvider for CredentialManager {
    async fn credential<C: Send + 'static>(
        &self,
        ctx: &CredentialContext,
    ) -> Result<SecretString, CredentialError> {
        let type_id = TypeId::of::<C>();
        let cred_id = self.credentials_by_type.get(&type_id)?;
        self.retrieve(cred_id, ctx).await
    }

    async fn get(
        &self,
        id: &str,
        ctx: &CredentialContext,
    ) -> Result<SecretString, CredentialError> {
        let cred_id = CredentialId::new(id)?;
        self.retrieve(&cred_id, ctx).await
    }
}
```

## File Structure

```
crates/credential/src/
├── lib.rs                    # Exports CredentialRef and CredentialProvider
├── core/
│   ├── mod.rs               # Re-exports from reference module
│   └── reference.rs         # CredentialRef + CredentialProvider trait (NEW)
└── manager/
    └── manager.rs           # Manager implements CredentialProvider (future)
```

## Migration Notes

- `CredentialRef` is now just `CredentialRef::of::<T>()` - no string ID needed
- Use `credential<C>()` for type-safe acquisition
- Use `get(id)` for dynamic acquisition when type unknown at compile time
- No `async-trait` dependency - native async fn in traits

## Future Work

- Implement `CredentialProvider` for `CredentialManager` with dual registry (TypeId + ID)
- Consider registration API: `register_typed::<C>(id, secret)` vs `store(id, secret)`
- Add credential discovery: `list_credentials() -> Vec<CredentialInfo>`
- Integration with action/trigger contexts
