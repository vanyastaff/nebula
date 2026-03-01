# API Reference (Human-Oriented)

## High-level Entry Point

- `CredentialManager`
  - CRUD operations
  - listing/filtering
  - validation checks
  - optional cache integration
  - batch and scoped operations

Created via `CredentialManager::builder()`.

## Core Types

- `CredentialId`, `ScopeId`
- `CredentialContext`
- `CredentialMetadata`
- `CredentialDescription`
- `CredentialFilter`
- `CredentialRef`, `CredentialProvider`
- `CredentialState`
- `CredentialError`, `StorageError`, `ManagerError`, `ValidationError`, `CryptoError`

## Traits

- `StorageProvider`, `StateStore`
- `DistributedLock`
- `CredentialResource`, `CredentialType`
- `FlowProtocol`, `StaticProtocol`, `InteractiveCredential`
- `Refreshable`, `Revocable`
- `RotatableCredential`, `TestableCredential`

## Built-in Protocols

- `ApiKeyProtocol`
- `BasicAuthProtocol`
- `HeaderAuthProtocol`
- `DatabaseProtocol`
- `OAuth2Protocol` (+ config/state/flow helpers)
- `LdapProtocol`
- `SamlConfig`
- `KerberosConfig`
- `MtlsConfig`

## Storage Providers

- `MockStorageProvider` (always available)
- `LocalStorageProvider` (`storage-local`)
- `AwsSecretsManagerProvider` (`storage-aws`)
- `HashiCorpVaultProvider` (`storage-vault`)
- `KubernetesSecretsProvider` (`storage-k8s`)

Provider configuration and metrics are exposed via `providers::config` and `providers::metrics`.

## Rotation APIs

- `RotationPolicy` (`Periodic`, `BeforeExpiry`, `Scheduled`, `Manual`)
- `RotationTransaction`, `TransactionPhase`, `RotationState`
- grace period and backup APIs
- blue/green helpers for zero-downtime scenarios
- retry and failure classification helpers

## Utilities

- `EncryptionKey`, `EncryptedData`, `encrypt`, `decrypt`
- `SecretString`
- retry and time helpers
- validation helpers for encrypted payload constraints

## Interactive Flow Surface

Used by `nebula-api` to drive multi-step credential creation (OAuth2 Authorization Code, Device Flow, etc.).

### `InitializeResult<S>` — outcome of `FlowProtocol::initialize()`

```rust
pub enum InitializeResult<S> {
    /// Credential is ready; state S is the persisted credential state.
    Complete(S),
    /// Waiting for an external event (e.g. polling device flow).
    Pending { partial_state: S, next_step: InteractionRequest },
    /// User must perform an action in a browser or external system.
    RequiresInteraction(InteractionRequest),
}

pub enum InteractionRequest {
    /// Redirect user to URL (OAuth2 Authorization Code, SAML, etc.).
    Redirect { url: Url, state: Option<String> },
    /// Show code to user; poll until confirmed (OAuth2 Device Flow).
    DisplayInfo { user_code: String, verification_url: Url, expires_in: Duration },
    /// Prompt user to paste a code (e.g. copy-paste from external system).
    CodeInput { prompt: String },
    /// Show instructions; user confirms out-of-band.
    AwaitConfirmation { message: String },
    Custom(serde_json::Value),
}
```

### `UserInput` — caller delivers user action back to the flow

```rust
pub enum UserInput {
    /// OAuth2 callback params: { code, state } or error params.
    Callback { params: HashMap<String, String> },
    /// User-entered code (Device Flow confirmation or manual code).
    Code(String),
    /// Signal to re-poll (Device Flow pending).
    Poll,
    Custom(serde_json::Value),
}
```

### `InteractiveCredential` trait

```rust
pub trait InteractiveCredential: FlowProtocol {
    /// Continue an in-progress flow with user-supplied input.
    async fn continue_flow(
        &self,
        partial_state: Self::State,
        input: UserInput,
    ) -> Result<InitializeResult<Self::State>>;
}
```

## nebula-api Observable APIs *(Phase 4)*

Methods `nebula-api` calls on `CredentialManager` to serve credential endpoints:

| Manager method | API endpoint | Notes |
|----------------|-------------|-------|
| `list(filter)` → `Vec<CredentialMetadata>` | `GET /credentials` | Metadata only; no secret material |
| `get(id)` → `Option<(CredentialMetadata, CredentialStatus)>` | `GET /credentials/:id` | Status: active / pending_interaction / error |
| `create(type_id, input)` → `InitializeResult` | `POST /credentials` | Returns 202 if `RequiresInteraction` |
| `continue(id, UserInput)` → `InitializeResult<Complete>` | `POST /credentials/:id/callback` | Completes interactive flow |
| `delete(id)` → `Result<()>` | `DELETE /credentials/:id` | Revokes tokens; removes from storage |
| `list_types()` → `Vec<CredentialTypeSchema>` | `GET /credential-types` | Type ids, display names, parameter schemas |

### `CredentialTypeSchema`

```rust
pub struct CredentialTypeSchema {
    pub type_id: String,              // e.g. "oauth2_github"
    pub display_name: String,
    pub description: String,
    pub icon: Option<String>,
    pub params: ParameterCollection,  // schema for POST /credentials body
    pub capabilities: Vec<String>,    // e.g. ["refresh", "revoke", "rotate"]
}
```

## Credential ↔ Resource Cascade

When `CredentialManager` completes a rotation, linked resource instances are refreshed:

```rust
// Resource defines its credential binding at the type level:
impl CredentialResource<OAuth2GitHub> for GitHubApiResource {
    fn authorize(&mut self, state: &OAuth2State) {
        self.bearer_token = state.access_token.clone();
    }
}
```

On credential rotation, `resource::Manager::notify_credential_rotated(credential_id, &new_state)`:
- Finds all registered resources with `CredentialResource` bound to `credential_id`
- Calls `resource.authorize(&new_state)` on the resource handler
- Drains existing pool instances; new acquires create instances with updated auth
- In-flight instances complete normally; no cancellation
