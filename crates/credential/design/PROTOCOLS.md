# Protocol Layer

Unified view of credential protocols in `nebula-credential`.

**Guiding principle:** Simple things should be trivial, complex things should be possible.

## Protocol → Trait Mapping

| Protocol | Trait | Config | State | Notes |
|----------|-------|--------|-------|-------|
| `ApiKeyProtocol` | `StaticProtocol` | — | `ApiKeyState` | server + token |
| `BasicAuthProtocol` | `StaticProtocol` | — | `BasicAuthState` | username + password |
| `HeaderAuthProtocol` | `StaticProtocol` | — | `HeaderAuthState` | header_name + header_value |
| `DatabaseProtocol` | `StaticProtocol` | — | `DatabaseState` | host, port, database, username, password, ssl_mode |
| `OAuth2Protocol` | `FlowProtocol` | `OAuth2Config` | `OAuth2State` | AuthCode, ClientCredentials, Device |
| `LdapProtocol` | `FlowProtocol` | `LdapConfig` | `LdapState` | host+port+tls in Config; bind_dn+bind_password (SecretString) in State |
| `SamlProtocol` | `FlowProtocol` | `SamlConfig` | — | Phase 5 stub |
| `KerberosProtocol` | `FlowProtocol` | `KerberosConfig` | — | Phase 5 stub |
| `MtlsProtocol` | `FlowProtocol` | `MtlsConfig` | — | Phase 5 stub |

## Protocol Support Matrix

| Protocol | Interactive | Refresh | Rotation | Scope Isolation | Audit |
|----------|-------------|---------|----------|-----------------|-------|
| **OAuth2** | Yes | Yes | Token refresh | Yes | Yes |
| **SAML 2.0** | Yes | No | No | Yes | Yes |
| **LDAP/AD** | Optional | No | Yes (password) | Yes | Yes |
| **mTLS** | No | No | Yes (cert renewal) | Yes | Yes |
| **JWT** | Optional | Yes | Reissue | Yes | Yes |
| **API Keys** | No | No | Yes | Yes | Yes |
| **Kerberos** | Yes | Yes (TGT) | No | Yes | Limited |
| **Basic Auth** | No | No | Yes (password) | Yes | Yes |
| **Header Auth** | No | No | Yes | Yes | Yes |
| **Database** | No | No | Yes (password) | Yes | Yes |

## StaticProtocol vs FlowProtocol

- **StaticProtocol**: sync, no IO. `parameters()` + `build_state(values)` → State. Use for credentials where initialization is a pure form-to-state transformation.
- **FlowProtocol**: async. `parameters()` + `initialize(config, values, ctx)` → `InitializeResult<State>`. Optional `refresh`/`revoke`. Use for credentials requiring network calls, user interaction, or multi-step flows.

Protocols do **not** know about storage or rotation; they only build/update State and return `InitializeResult`/`refresh`/`revoke`.

## Trait Definitions

### StaticProtocol

```rust
pub trait StaticProtocol: Send + Sync + 'static {
    type State: CredentialState;

    fn parameters() -> ParameterCollection where Self: Sized;

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError>
    where Self: Sized;
}
```

### FlowProtocol

```rust
pub trait FlowProtocol: Send + Sync + 'static {
    type Config: Send + Sync + 'static;
    type State: CredentialState;

    fn parameters() -> ParameterCollection where Self: Sized;

    async fn initialize(
        config: &Self::Config,
        values: &ParameterValues,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>
    where Self: Sized;

    async fn refresh(
        config: &Self::Config,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>
    where Self: Sized { Ok(()) }

    async fn revoke(
        config: &Self::Config,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>
    where Self: Sized { Ok(()) }
}
```

### CredentialResource

Links a Resource to its required credential type at compile time:

```rust
pub trait CredentialResource: Resource {
    type Credential: CredentialType;

    fn authorize(&mut self, state: &<Self::Credential as CredentialType>::State);
}
```

The runtime automatically retrieves and injects the credential State when creating or refreshing the resource instance. On credential rotation, `resource::Manager::notify_credential_rotated` calls `authorize(&new_state)` on all linked resources.

## Protocol Registry Architecture

### Two-layer design

Protocol support uses two complementary layers:

```
┌─────────────────────────────────────────────────────┐
│  TYPED LAYER  (compile-time safety)                 │
│  FlowProtocol<Config, State>  StaticProtocol<State> │
│  Used to implement protocols — fully type-safe      │
└──────────────────────┬──────────────────────────────┘
                       │ Bridge (automatic via ProtocolDriver)
┌──────────────────────▼──────────────────────────────┐
│  ERASED LAYER  (runtime flexibility)                │
│  ErasedProtocol  →  ProtocolRegistry                │
│  Used for dynamic registration, plugin loading,     │
│  API layer, and storage with serde_json::Value      │
└─────────────────────────────────────────────────────┘
```

### ErasedProtocol — object-safe trait

All state flows as `serde_json::Value` at this layer, making it object-safe and serializable:

```rust
#[async_trait]
pub trait ErasedProtocol: Send + Sync + 'static {
    fn credential_key(&self) -> &CredentialKey;  // from nebula-core; e.g. "oauth2_github"
    fn display_name(&self) -> &str;
    fn parameters(&self) -> ParameterCollection;
    fn capabilities(&self) -> ProtocolCapabilities;

    async fn initialize(
        &self,
        values: ParameterValues,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<serde_json::Value>, CredentialError>;

    async fn continue_flow(
        &self,
        partial: PartialState,
        input: UserInput,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<serde_json::Value>, CredentialError>;

    async fn refresh(
        &self,
        state: serde_json::Value,
        ctx: &mut CredentialContext,
    ) -> Result<serde_json::Value, CredentialError>;

    async fn revoke(
        &self,
        state: serde_json::Value,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>;
}

pub struct ProtocolCapabilities {
    pub interactive: bool,  // requires browser redirect (OAuth2, SAML)
    pub refresh:     bool,  // can refresh token
    pub revoke:      bool,  // can revoke token
    pub rotate:      bool,  // supports rotation policy
}
```

### ProtocolDriver — automatic bridge from typed to erased

`ProtocolDriver<P>` wraps a `FlowProtocol` and captures its config at registration time.
It automatically implements `ErasedProtocol` by serializing/deserializing state via `serde_json::Value`.
The protocol implementor never touches `ErasedProtocol` directly.

```rust
// Registration — config captured once at startup
let registry = ProtocolRegistry::builder()
    // Static protocols — one line each
    .static_protocol::<ApiKeyProtocol>(CredentialKey::new("api_key").unwrap(), "API Key")
    .static_protocol::<BasicAuthProtocol>(CredentialKey::new("basic_auth").unwrap(), "Basic Auth")
    // Flow protocols — config provided at registration
    .flow_protocol::<OAuth2Protocol>(
        CredentialKey::new("oauth2_github").unwrap(),
        "GitHub OAuth2",
        OAuth2Config {
            auth_url:   "https://github.com/login/oauth/authorize".into(),
            token_url:  "https://github.com/login/oauth/access_token".into(),
            scopes:     vec!["repo".into(), "user".into()],
            grant_type: GrantType::AuthorizationCode,
            auth_style: AuthStyle::PostBody,
        },
    )
    // Community plugin — implements ErasedProtocol directly
    .protocol(Arc::new(MyCustomSamlDriver::new(config)))
    .build();
```

### CredentialType — typed marker for action developers

Action developers use `CredentialType` to access credentials with compile-time safety.
The typed layer resolves through the registry without the developer knowing about `ErasedProtocol`:

```rust
pub trait CredentialType: Send + Sync + 'static {
    /// The CredentialKey registered in ProtocolRegistry (from nebula-core).
    fn credential_key() -> CredentialKey where Self: Sized;
    /// The typed state returned after successful authentication.
    type State: CredentialState + Serialize + DeserializeOwned;
}

// Defining a credential type — links Rust type to registry key
pub struct GitHubOAuth2;
impl CredentialType for GitHubOAuth2 {
    fn credential_key() -> CredentialKey {
        CredentialKey::new("oauth2_github").expect("valid key")
    }
    type State = OAuth2State;
}

// Using a credential in an action — compile-time safe
let token: OAuth2State = ctx.credentials()
    .credential::<GitHubOAuth2>(&ctx)  // resolves via GitHubOAuth2::credential_key()
    .await?;
token.access_token.expose_secret()  // explicit, auditable opt-in
```

`CredentialKey` (from `nebula-core`) is a normalized domain key (`[a-z][a-z0-9_]*`).
It identifies the protocol *type* in storage, API responses, and the registry.
`CredentialId` (UUID) identifies a specific credential *instance* in the database.

## Config Types (FlowProtocol)

### OAuth2Config

```rust
pub struct OAuth2Config {
    pub auth_url:   String,
    pub token_url:  String,
    pub scopes:     Vec<String>,
    pub grant_type: GrantType,   // AuthorizationCode | ClientCredentials | DeviceCode
    pub auth_style: AuthStyle,   // Header (RFC default) | PostBody (GitHub, Slack)
    // PKCE (RFC 7636) is always enforced for AuthorizationCode — not configurable (D-008).
    // ClientCredentials and DeviceCode do not use PKCE by spec.
}
```

### LdapConfig

```rust
pub struct LdapConfig {
    pub host:    String,
    pub port:    u16,            // default: 389 (LDAP/StartTLS), 636 (LDAPS)
    pub tls:     TlsMode,        // None | Tls | StartTls
    pub timeout: Duration,       // default: 30s
    pub ca_cert: Option<String>,
}
```

### SamlConfig

```rust
pub struct SamlConfig {
    pub binding:       SamlBinding,  // HttpPost | HttpRedirect
    pub sign_requests: bool,         // default: false
}
```

## Supporting Enums

```rust
#[derive(Default)]
pub enum GrantType {
    #[default] AuthorizationCode,
    ClientCredentials,
    DeviceCode,
}

#[derive(Default)]
pub enum AuthStyle {
    #[default] Header,    // RFC 6749: Authorization: Basic base64(id:secret)
    PostBody,             // GitHub, Slack: client_id/secret as POST fields
}

#[derive(Default)]
pub enum TlsMode {
    #[default] None,      // Plaintext (dev only)
    Tls,                  // LDAPS (port 636)
    StartTls,             // STARTTLS upgrade (port 389)
}

#[derive(Default)]
pub enum SamlBinding {
    #[default] HttpPost,
    HttpRedirect,
}
```

## State Types

### OAuth2State

```rust
pub struct OAuth2State {
    pub access_token:  SecretString,            // zeroized on drop — never expose raw
    pub token_type:    String,                  // "Bearer"
    pub refresh_token: Option<SecretString>,    // zeroized on drop
    pub expires_at:    Option<DateTime<Utc>>,
    pub scopes:        Vec<String>,
}

impl OAuth2State {
    pub fn is_expired(&self, margin: Duration) -> bool { ... }
    pub fn bearer_header(&self) -> String { ... }
}
```

### BasicAuthState

```rust
pub struct BasicAuthState {
    pub username: String,
    pub password: SecretString,  // zeroized on drop; expose_secret() for Base64 encoding
}

impl BasicAuthState {
    pub fn encoded(&self) -> String { ... }  // Base64 "user:pass" — calls expose_secret() internally
}
```

### LdapState

```rust
pub struct LdapState {
    pub bind_dn:       String,
    pub bind_password: SecretString,  // zeroized on drop
    // host, port, tls — server config; lives in LdapConfig, not session state
}
```

### ApiKeyState

```rust
pub struct ApiKeyState {
    pub server: Option<String>,  // optional base URL / endpoint
    pub token:  SecretString,    // zeroized on drop — the raw API key value
}
```

### HeaderAuthState

```rust
pub struct HeaderAuthState {
    pub header_name:  String,       // e.g. "X-Api-Key", "Authorization"
    pub header_value: SecretString, // zeroized on drop — the raw header value
}
```

### DatabaseState

```rust
pub struct DatabaseState {
    pub host:     String,
    pub port:     u16,
    pub database: String,
    pub username: String,
    pub password: SecretString,  // zeroized on drop — never log or expose
    pub ssl_mode: SslMode,       // Disable | Allow | Prefer | Require | VerifyCa | VerifyFull
}
```

> **Rule (D-006):** `password` is `SecretString` in ALL state types that carry a credential value. Storing a plain `String` for a password is a security violation, not a performance trade-off.

## Core Types (core::result)

- **InitializeResult\<S\>**: `Complete(S)` | `Pending { partial_state, next_step }` | `RequiresInteraction(InteractionRequest)`
- **PartialState**: data, step, created_at, ttl_seconds, metadata
- **UserInput**: Callback, Code, Poll, Custom
- **InteractionRequest**: Redirect, CodeInput, DisplayInfo, AwaitConfirmation, Custom

## Macro DX (Developer Experience)

### Simplest case — 3 lines

```rust
#[derive(Credential)]
#[credential(key = "stripe-api", name = "Stripe API", extends = ApiKeyProtocol)]
pub struct StripeApi;
// Input = ParameterValues, State = ApiKeyState (auto-derived)
```

### OAuth2 standard provider — 8 lines

```rust
#[derive(Credential)]
#[credential(key = "google-oauth2", name = "Google OAuth2", extends = OAuth2Protocol)]
#[oauth2(
    auth_url  = "https://accounts.google.com/o/oauth2/v2/auth",
    token_url = "https://oauth2.googleapis.com/token",
    scopes    = ["openid", "email", "profile"],
)]
pub struct GoogleOauth2;
```

### OAuth2 non-standard — 10 lines

```rust
#[derive(Credential)]
#[credential(key = "github-oauth2", name = "GitHub OAuth2", extends = OAuth2Protocol)]
#[oauth2(
    auth_url   = "https://github.com/login/oauth/authorize",
    token_url  = "https://github.com/login/oauth/access_token",
    scopes     = ["repo", "user", "workflow"],
    auth_style = PostBody,  // GitHub requires POST body, not Basic header
)]
pub struct GithubOauth2;
```

### LDAP — 5 lines

```rust
#[derive(Credential)]
#[credential(key = "company-ldap", name = "Company LDAP", extends = LdapProtocol)]
#[ldap(host = "ldap.company.com", port = 389, tls = StartTls)]
pub struct CompanyLdap;
// bind_dn + bind_password come from ParameterValues at runtime (not compile-time config)
```

### Full manual override (escape hatch)

When the protocol is non-standard, implement `CredentialType` manually:

```rust
pub struct AzureAd;

impl CredentialType for AzureAd {
    type Input = ParameterValues;
    type State = OAuth2State;

    fn description() -> CredentialDescription {
        let mut props = OAuth2Protocol::parameters();
        props.push(TextParameter::new("tenant_id", "Tenant ID").required(true));
        CredentialDescription::builder("azure-ad", "Azure Active Directory")
            .properties(props)
            .build()
    }

    async fn initialize(
        &self,
        input: &ParameterValues,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<OAuth2State>, CredentialError> {
        let tenant = input.get_string("tenant_id").unwrap_or("common");
        let config = OAuth2Config::authorization_code()
            .auth_url(format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"))
            .token_url(format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"))
            .scopes(["openid", "profile", "email", "offline_access"])
            .build();
        OAuth2Protocol::initialize(&config, input, ctx).await
    }
}
```

### Resource using credential — typed, compile-time safe

```rust
impl CredentialResource for GithubHttpClient {
    type Credential = GithubOauth2;  // compile-time link

    fn authorize(&mut self, state: &OAuth2State) {
        self.client = reqwest::Client::builder()
            .default_headers({
                let mut h = HeaderMap::new();
                h.insert(AUTHORIZATION, state.bearer_header().parse().unwrap());
                h
            })
            .build()
            .unwrap();
    }
}
```

## Non-Goals

- No HTTP client bundled in protocols — transport is the Resource's concern
- No string-based `extends` (n8n style) — everything is type-safe, compile-time checked
- No automatic token injection — `Resource::authorize()` handles this explicitly
- No raw `&str` type IDs in public API — use `CredentialKey` from `nebula-core` for all protocol type identifiers
