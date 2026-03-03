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
| `LdapProtocol` | `FlowProtocol` | `LdapConfig` | `LdapState` | host, port, bind_dn, tls_mode |
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

## Config Types (FlowProtocol)

### OAuth2Config

```rust
pub struct OAuth2Config {
    pub auth_url:   String,
    pub token_url:  String,
    pub scopes:     Vec<String>,
    pub grant_type: GrantType,      // AuthorizationCode | ClientCredentials | DeviceCode
    pub auth_style: AuthStyle,      // Header (RFC default) | PostBody (GitHub, Slack)
    pub pkce:       bool,           // default: false (will become true in OAuth 2.1)
}
```

### LdapConfig

```rust
pub struct LdapConfig {
    pub tls:     TlsMode,   // None | Tls | StartTls
    pub timeout: Duration,  // default: 30s
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
    pub access_token:  String,
    pub token_type:    String,                  // "Bearer"
    pub refresh_token: Option<String>,
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
    pub password: String,   // encrypted at rest
}

impl BasicAuthState {
    pub fn encoded(&self) -> String { ... }  // Base64 "user:pass"
}
```

### LdapState

```rust
pub struct LdapState {
    pub host:          String,
    pub port:          u16,
    pub bind_dn:       String,
    pub bind_password: String,
    pub tls:           TlsMode,
}
```

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

### LDAP — 4 lines

```rust
#[derive(Credential)]
#[credential(key = "company-ldap", name = "Company LDAP", extends = LdapProtocol)]
#[ldap(tls = StartTls)]
pub struct CompanyLdap;
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
