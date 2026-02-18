# Credential Protocol System Design

**Date:** 2026-02-18  
**Status:** Approved — ready for implementation  
**Scope:** `nebula-credential` crate — `protocols/` module + `#[derive(Credential)]` macro extensions

---

## Goal

Design a layered protocol system that lets plugin developers authenticate against any service
(HTTP APIs, LDAP, SAML IdPs, Kerberos KDCs, mTLS endpoints) with minimal boilerplate,
while keeping full escape hatches for non-standard behaviour.

The guiding principle: **simple things should be trivial, complex things should be possible.**

---

## Context

### What exists today

- `CredentialType` trait — describes a credential (schema + initialize)
- `CredentialProtocol` trait — static form→State building block (`parameters()` + `build_state()`)
- `ApiKeyProtocol` — first concrete protocol (server + token)
- `#[derive(Credential)]` macro with `extends = ApiKeyProtocol`
- `Refreshable`, `Revocable`, `InteractiveCredential` — opt-in traits
- `InitializeResult` — supports `Complete`, `Pending`, `RequiresInteraction`
- `InteractionRequest` — 7 interaction types (Redirect, CodeInput, DisplayInfo, etc.)

### What is missing

- `StaticProtocol` / `FlowProtocol` distinction
- `OAuth2Protocol`, `LdapProtocol`, `SamlProtocol`, `BasicAuthProtocol`
- `#[oauth2(...)]`, `#[ldap(...)]`, `#[saml(...)]` macro attributes
- `CredentialResource` trait linking credential State to Resource clients
- `AuthStyle`, `GrantType`, `TlsMode` enums

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                    Plugin Developer API                      │
│                                                              │
│  #[derive(Credential)]                                       │
│  #[credential(key = "...", name = "...", extends = Foo)]     │
│  #[oauth2(auth_url = "...", token_url = "...", ...)]  ◄───── macro attributes
│  pub struct MyCredential;                                    │
└─────────────────────────────┬───────────────────────────────┘
                              │ macro generates
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    CredentialType (trait)                    │
│   type Input, type State, description(), initialize()       │
└──────────┬──────────────────────────────┬───────────────────┘
           │ implemented by               │ implemented by
           ▼                              ▼
┌──────────────────────┐      ┌───────────────────────────────┐
│   StaticProtocol     │      │       FlowProtocol            │
│   (trait)            │      │       (trait)                 │
│                      │      │                               │
│  Sync. No IO.        │      │  Async. Has Config.           │
│  Form → State        │      │  Multi-step flows.            │
│                      │      │  Refresh + Revoke.            │
└──────────┬───────────┘      └──────────┬────────────────────┘
           │                             │
     ┌─────┴──────┐              ┌───────┴──────────────┐
     │            │              │                      │
  ApiKey      BasicAuth       OAuth2Protocol        LdapProtocol
  Protocol    Protocol        Config=OAuth2Config   Config=LdapConfig
  Header      Database        (auth code, client    (host, port,
  Auth        Protocol         credentials, PKCE)    bind, TLS)
                              │
                         SamlProtocol        KerberosProtocol
                         Config=SamlConfig   Config=KerberosConfig
                         MtlsProtocol
                         Config=MtlsConfig
```

### Resource integration

```
CredentialType ◄──── CredentialResource::Credential (associated type)
                          │
                          └── impl Resource for MyHttpClient {
                                  type Credential = GithubApi;
                                  fn authorize(&mut self, state: &ApiKeyState) { ... }
                              }
```

---

## Trait Definitions

### `StaticProtocol`

For credentials where initialization is pure: form values in, State out. No network calls,
no user interaction. The current `CredentialProtocol` trait is renamed to `StaticProtocol`.

```rust
/// Synchronous form-to-State protocol. No IO, no async.
///
/// Use for: API keys, Basic Auth, database credentials, header auth.
/// Implement via `#[credential(extends = MyProtocol)]` macro attribute.
pub trait StaticProtocol: Send + Sync + 'static {
    /// State produced after initialization
    type State: CredentialState;

    /// Parameters shown to user in UI
    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    /// Build State from submitted form values
    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError>
    where
        Self: Sized;
}
```

**Built-in implementations:**

| Protocol | Fields | State |
|---|---|---|
| `ApiKeyProtocol` | `server` (URL), `token` (secret) | `ApiKeyState` |
| `BasicAuthProtocol` | `username`, `password` (secret) | `BasicAuthState` |
| `HeaderAuthProtocol` | `header_name`, `header_value` (secret) | `HeaderAuthState` |
| `DatabaseProtocol` | `host`, `port`, `database`, `username`, `password`, `ssl_mode` | `DatabaseState` |

### `FlowProtocol`

For credentials requiring async operations: HTTP token exchange, TCP bind, SAML assertion
validation. Parameterized by a `Config` type that the plugin provides to configure
provider-specific endpoints, scopes, and options.

```rust
/// Async multi-step protocol. Configurable per provider.
///
/// Use for: OAuth2, LDAP, SAML, Kerberos, mTLS.
/// Plugin implements `Config` type and uses macro attributes to wire it up.
pub trait FlowProtocol: Send + Sync + 'static {
    /// Provider-specific configuration (endpoints, scopes, options)
    type Config: Send + Sync + 'static;

    /// State produced after successful flow completion
    type State: CredentialState;

    /// Parameters shown to user in UI (client_id, client_secret, etc.)
    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    /// Execute the authentication flow
    ///
    /// Returns `InitializeResult::Complete` for non-interactive flows
    /// (client_credentials), or `RequiresInteraction`/`Pending` for
    /// interactive flows (authorization_code, device_code).
    async fn initialize(
        config: &Self::Config,
        values: &ParameterValues,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>
    where
        Self: Sized;

    /// Refresh an expired credential (default: not supported)
    ///
    /// Override for protocols with token refresh: OAuth2 refresh_token,
    /// Kerberos TGT renewal, etc.
    async fn refresh(
        config: &Self::Config,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>
    where
        Self: Sized,
    {
        let _ = (config, state, ctx);
        Ok(())
    }

    /// Revoke an active credential (default: not supported)
    ///
    /// Override for OAuth2 token revocation, LDAP unbind, session invalidation.
    async fn revoke(
        config: &Self::Config,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>
    where
        Self: Sized,
    {
        let _ = (config, state, ctx);
        Ok(())
    }
}
```

### `CredentialResource`

Links a Resource to its required credential type at compile time.

```rust
/// A Resource that requires a specific credential for authorization.
///
/// The runtime automatically retrieves and injects the credential State
/// when creating or refreshing the resource instance.
pub trait CredentialResource: Resource {
    /// The credential type required by this resource
    type Credential: CredentialType;

    /// Apply credential state to authorize this resource's client.
    ///
    /// Called after the resource is created and whenever the credential
    /// is refreshed (e.g. OAuth2 token rotation).
    fn authorize(
        &mut self,
        state: &<Self::Credential as CredentialType>::State,
    );
}
```

---

## Macro Attributes Design

### `extends` for `StaticProtocol`

Works exactly as today. Macro generates full `impl CredentialType` — no manual code.

```rust
#[derive(Credential)]
#[credential(
    key = "slack-api",
    name = "Slack API",
    description = "Authenticate with Slack using a bot token",
    extends = ApiKeyProtocol,      // StaticProtocol impl
)]
pub struct SlackApi;
// Input  = ParameterValues        (auto)
// State  = ApiKeyState            (auto, from Protocol::State)
// initialize() delegates to ApiKeyProtocol::build_state()
```

### `extends` + `#[oauth2(...)]` for `OAuth2Protocol`

The `#[oauth2(...)]` attribute configures the `OAuth2Config`. Macro generates:
- `impl CredentialType` with `initialize()` delegating to `OAuth2Protocol::initialize()`
- `impl Refreshable` delegating to `OAuth2Protocol::refresh()`
- The `OAuth2Config` instance as a const

```rust
#[derive(Credential)]
#[credential(
    key = "github-oauth2",
    name = "GitHub OAuth2",
    description = "Authenticate with GitHub via OAuth2",
    extends = OAuth2Protocol,
)]
#[oauth2(
    auth_url   = "https://github.com/login/oauth/authorize",
    token_url  = "https://github.com/login/oauth/access_token",
    scopes     = ["repo", "user", "workflow"],
    grant_type = AuthorizationCode,   // default, can be omitted
    auth_style = PostBody,            // GitHub requires this (non-standard)
    pkce       = false,               // default
)]
pub struct GithubOauth2;
```

For Google (standard OAuth2 — minimal config):

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

### `extends` + `#[ldap(...)]` for `LdapProtocol`

```rust
#[derive(Credential)]
#[credential(key = "company-ldap", name = "Company LDAP", extends = LdapProtocol)]
#[ldap(
    tls          = StartTls,    // None | Tls | StartTls, default: None
    timeout_secs = 30,          // default: 30
)]
pub struct CompanyLdap;
// Parameters: host, port, bind_dn, bind_password (from LdapProtocol::parameters())
// State: LdapState { host, port, bind_dn, bind_password, tls_mode }
```

### `extends` + `#[saml(...)]` for `SamlProtocol`

```rust
#[derive(Credential)]
#[credential(key = "okta-saml", name = "Okta SAML", extends = SamlProtocol)]
#[saml(
    binding       = HttpPost,      // HttpPost | HttpRedirect, default: HttpPost
    sign_requests = true,          // default: false
)]
pub struct OktaSaml;
// Parameters: idp_url, entity_id, certificate (from SamlProtocol::parameters())
```

### Full manual override (escape hatch)

When the protocol is non-standard, plugin implements `CredentialType` manually.
The protocol can still be used as a helper:

```rust
pub struct AzureAd;

impl CredentialType for AzureAd {
    type Input = ParameterValues;
    type State = OAuth2State;

    fn description() -> CredentialDescription {
        // Mix protocol parameters with custom ones
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

impl Refreshable for AzureAd {
    async fn refresh(&self, state: &mut OAuth2State, ctx: &mut CredentialContext)
        -> Result<(), CredentialError>
    {
        let tenant = /* extract from state or config */;
        let config = /* rebuild config */;
        OAuth2Protocol::refresh(&config, state, ctx).await
    }
}
```

---

## Supporting Enums

### `GrantType`

```rust
/// OAuth2 grant type (RFC 6749)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GrantType {
    /// Authorization Code flow — requires user browser redirect (default)
    #[default]
    AuthorizationCode,
    /// Client Credentials flow — server-to-server, no user interaction
    ClientCredentials,
    /// Device Authorization Grant (RFC 8628) — for CLI/TV apps
    DeviceCode,
}
```

### `AuthStyle`

How `client_id` / `client_secret` are sent in the token request.

```rust
/// How client credentials are sent in the OAuth2 token request
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuthStyle {
    /// RFC 6749 standard: Authorization: Basic base64(client_id:client_secret) (default)
    #[default]
    Header,
    /// client_id and client_secret as form fields in the POST body
    ///
    /// Required by: GitHub, Slack, some legacy providers
    PostBody,
}
```

### `TlsMode` (LDAP)

```rust
/// TLS mode for LDAP connections
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TlsMode {
    /// Plaintext connection (default, for development only)
    #[default]
    None,
    /// TLS from the start (ldaps://, port 636)
    Tls,
    /// STARTTLS upgrade on plaintext connection (port 389)
    StartTls,
}
```

### `SamlBinding`

```rust
/// SAML request binding method
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SamlBinding {
    /// HTTP POST binding (default)
    #[default]
    HttpPost,
    /// HTTP Redirect binding (GET with base64-encoded SAMLRequest)
    HttpRedirect,
}
```

---

## Config Structs

### `OAuth2Config`

```rust
/// Provider-specific OAuth2 configuration.
///
/// Built via builder — only `auth_url` and `token_url` are required.
/// Generated by the `#[oauth2(...)]` macro attribute as a `const`.
pub struct OAuth2Config {
    pub auth_url:   String,
    pub token_url:  String,
    pub scopes:     Vec<String>,
    pub grant_type: GrantType,      // default: AuthorizationCode
    pub auth_style: AuthStyle,      // default: Header
    pub pkce:       bool,           // default: false
}

impl OAuth2Config {
    pub fn authorization_code() -> OAuth2ConfigBuilder { ... }
    pub fn client_credentials() -> OAuth2ConfigBuilder { ... }
    pub fn device_code() -> OAuth2ConfigBuilder { ... }
}
```

### `LdapConfig`

```rust
pub struct LdapConfig {
    pub tls:          TlsMode,   // default: None
    pub timeout:      Duration,  // default: 30s
    pub ca_cert:      Option<String>,
}
```

### `SamlConfig`

```rust
pub struct SamlConfig {
    pub binding:       SamlBinding,  // default: HttpPost
    pub sign_requests: bool,         // default: false
}
```

---

## State Types

### `OAuth2State`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2State {
    pub access_token:  String,
    pub token_type:    String,                  // "Bearer" usually
    pub refresh_token: Option<String>,
    pub expires_at:    Option<DateTime<Utc>>,
    pub scopes:        Vec<String>,
}

impl OAuth2State {
    /// True if the access token is expired or expiring within `margin`
    pub fn is_expired(&self, margin: Duration) -> bool { ... }

    /// Bearer header value: "Bearer {access_token}"
    pub fn bearer_header(&self) -> String { ... }
}
```

### `BasicAuthState`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicAuthState {
    pub username: String,
    pub password: String,   // stored encrypted at rest
}

impl BasicAuthState {
    /// Base64-encoded "username:password" for Authorization: Basic header
    pub fn encoded(&self) -> String { ... }
}
```

### `LdapState`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapState {
    pub host:          String,
    pub port:          u16,
    pub bind_dn:       String,
    pub bind_password: String,
    pub tls:           TlsMode,
}
```

---

## File Layout

```
crates/credential/src/protocols/
├── mod.rs             — pub use for all protocols + enums
├── api_key.rs         — ApiKeyProtocol + ApiKeyState       (exists)
├── basic_auth.rs      — BasicAuthProtocol + BasicAuthState
├── header_auth.rs     — HeaderAuthProtocol + HeaderAuthState
├── database.rs        — DatabaseProtocol + DatabaseState
├── oauth2/
│   ├── mod.rs         — OAuth2Protocol impl + OAuth2Config + OAuth2State
│   ├── config.rs      — OAuth2Config, OAuth2ConfigBuilder
│   ├── state.rs       — OAuth2State + helpers
│   └── flow.rs        — token exchange, refresh, revoke HTTP logic
├── ldap/
│   ├── mod.rs         — LdapProtocol + LdapConfig + LdapState
│   └── config.rs      — LdapConfig, TlsMode
├── saml/
│   ├── mod.rs         — SamlProtocol + SamlConfig (Phase 5 stub)
│   └── config.rs      — SamlConfig, SamlBinding
├── kerberos/
│   └── mod.rs         — KerberosProtocol stub (Phase 5)
└── mtls/
    └── mod.rs         — MtlsProtocol stub (Phase 5)

crates/credential/src/traits/
└── credential.rs      — rename CredentialProtocol → StaticProtocol,
                         add FlowProtocol, CredentialResource

crates/macros/src/
└── credential.rs      — add #[oauth2(...)], #[ldap(...)], #[saml(...)] attribute parsing
```

---

## Developer Experience Examples

### Simplest case — 3 lines

```rust
#[derive(Credential)]
#[credential(key = "stripe-api", name = "Stripe API", extends = ApiKeyProtocol)]
pub struct StripeApi;
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

### OAuth2 non-standard provider — 10 lines

```rust
#[derive(Credential)]
#[credential(key = "github-oauth2", name = "GitHub OAuth2", extends = OAuth2Protocol)]
#[oauth2(
    auth_url   = "https://github.com/login/oauth/authorize",
    token_url  = "https://github.com/login/oauth/access_token",
    scopes     = ["repo", "user", "workflow"],
    auth_style = PostBody,
)]
pub struct GithubOauth2;
```

### LDAP — 7 lines

```rust
#[derive(Credential)]
#[credential(key = "company-ldap", name = "Company LDAP", extends = LdapProtocol)]
#[ldap(tls = StartTls)]
pub struct CompanyLdap;
```

### Resource using credential — typed, compile-time safe

```rust
pub struct GithubHttpClient {
    client: reqwest::Client,
    base_url: String,
}

impl Resource for GithubHttpClient {
    type Config = ();
    type Instance = Self;
    // ...
}

impl CredentialResource for GithubHttpClient {
    type Credential = GithubApi;  // compile-time link

    fn authorize(&mut self, state: &ApiKeyState) {
        // runtime injects the correct State automatically
        self.client = reqwest::Client::builder()
            .default_headers({
                let mut h = HeaderMap::new();
                h.insert(AUTHORIZATION, format!("Bearer {}", state.token()).parse().unwrap());
                h
            })
            .build()
            .unwrap();
        self.base_url = state.server().to_string();
    }
}
```

---

## What Changes vs Today

| | Before | After |
|---|---|---|
| `CredentialProtocol` | one trait for all | renamed → `StaticProtocol` |
| OAuth2 | no built-in | `FlowProtocol` impl + `#[oauth2(...)]` macro |
| LDAP | no built-in | `FlowProtocol` impl + `#[ldap(...)]` macro |
| SAML / Kerberos / mTLS | no built-in | Phase 5 stubs, same pattern |
| Resource↔Credential | manual, string-based | `CredentialResource::type Credential` |
| `#[derive(Credential)]` | only `extends` for StaticProtocol | also `extends` for FlowProtocol via sub-attributes |
| Plugin boilerplate | 3 lines (StaticProtocol) | 3–10 lines (all protocols) |

---

## Non-Goals

- No HTTP client bundled in protocols — transport is always the Resource's concern
- No string-based extends (n8n style) — everything is type-safe and compile-time checked  
- SAML, Kerberos, mTLS are stubs in this phase — interfaces only, no implementation
- No automatic token injection into HTTP requests — Resource::authorize() handles this explicitly

---

*Generated from brainstorming session 2026-02-18*
